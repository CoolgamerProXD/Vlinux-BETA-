import * as ipc from './ipc';
import { config } from './config.svelte';
import type { Catalog, Distro, InstallEvent, InstallOutcome, InstallPlan, WslReport } from './runtime-types';

export interface JobState {
  id: string;
  title: string;
  message: string;
  percent: number | null;
  outcome: InstallOutcome | null;
  /** Final message from the backend (may include notes about leftovers). */
  finalMessage: string | null;
  cancelRequested: boolean;
}

/**
 * Linux runtime state for the UI.
 *
 * Nothing is detected at startup: `refresh()` runs when the user opens Linux Setup (or asks to
 * check again). The backend caches results, and there is no polling timer — install progress
 * arrives as events.
 */
class RuntimeStore {
  report = $state<WslReport | null>(null);
  catalog = $state<Catalog | null>(null);
  loading = $state(false);
  catalogLoading = $state(false);
  error = $state<string | null>(null);
  plan = $state<InstallPlan | null>(null);
  planLoading = $state(false);
  job = $state<JobState | null>(null);
  busyDistro = $state<string | null>(null);
  private unlisten: (() => void) | undefined;

  readonly selected = $derived(config.value.runtime.selected_distro);
  readonly selectedDistro = $derived<Distro | null>(
    this.report?.distros.find((d) => d.name === this.selected && d.kind === 'regular') ?? null,
  );
  readonly suggested = $derived<Distro | null>(
    this.report?.distros.find((d) => d.name === this.report?.suggested_distro) ?? null,
  );
  readonly installing = $derived(this.job !== null && this.job.outcome === null);

  /** One-line summary for compact surfaces (status panel). Never triggers detection. */
  readonly summary = $derived.by(() => {
    const r = this.report;
    if (!r) return 'Not checked yet — open Linux Setup';
    const n = r.distros.filter((d) => d.kind === 'regular').length;
    switch (r.status.kind) {
      case 'ready': return `Ready · ${n} distribution${n === 1 ? '' : 's'}`;
      case 'no_distributions': return 'WSL ready · no distribution installed';
      case 'not_installed': return 'WSL is not installed';
      case 'restart_required': return 'Windows restart required';
      case 'feature_problem': return 'Needs attention';
      case 'unsupported_host': return 'Not available on this PC';
      case 'error': return 'Not responding';
    }
  });

  async refresh(force = false) {
    this.loading = true;
    this.error = null;
    try {
      this.report = await ipc.wslDetect(force);
    } catch (e) {
      this.error = String(e);
    } finally {
      this.loading = false;
    }
  }

  async loadCatalog(force = false) {
    this.catalogLoading = true;
    try {
      this.catalog = await ipc.wslCatalog(force);
    } catch (e) {
      this.catalog = { entries: [], error: String(e), fetched_at: 0 };
    } finally {
      this.catalogLoading = false;
    }
  }

  /** Remember the user's choice. Nothing is started or changed in WSL. */
  select(name: string | null) {
    config.update((c) => (c.runtime.selected_distro = name));
  }

  async review(kind: 'distribution' | 'wsl_platform', name: string | null) {
    this.planLoading = true;
    this.error = null;
    try {
      this.plan = await ipc.installPrepare(kind, name);
    } catch (e) {
      this.error = String(e);
    } finally {
      this.planLoading = false;
    }
  }

  discardPlan() {
    this.plan = null;
  }

  private async listen() {
    if (this.unlisten) return;
    this.unlisten = await ipc.onInstallEvent((e) => this.onEvent(e));
  }

  private onEvent(e: InstallEvent) {
    // Only one install runs at a time. Events can arrive before the start call returns, so a
    // job whose id is not known yet adopts the id of the first event.
    if (!this.job || (this.job.id !== '' && this.job.id !== e.job_id)) return;
    const base = { ...this.job, id: e.job_id };
    if (e.kind === 'progress') {
      this.job = { ...base, message: e.message, percent: e.percent ?? base.percent };
    } else {
      this.job = { ...base, message: e.message, finalMessage: e.message, percent: e.outcome?.kind === 'succeeded' ? 100 : base.percent, outcome: e.outcome };
      void this.refresh(true);
    }
  }

  /** Start the reviewed plan. `approved` must reflect an explicit user action. */
  async startPlan(approved: boolean) {
    const plan = this.plan;
    if (!plan || !approved) return;
    this.error = null;
    this.job = { id: '', title: plan.display_name, message: 'Starting…', percent: null, outcome: null, finalMessage: null, cancelRequested: false };
    try {
      await this.listen();
      const started = await ipc.installStart(plan.plan_id, true);
      if (this.job && this.job.id === '') this.job = { ...this.job, id: started.job_id };
      this.plan = null;
    } catch (e) {
      this.job = null;
      this.error = String(e);
    }
  }

  async cancelJob() {
    if (!this.job || this.job.outcome) return;
    const id = this.job.id;
    this.job = { ...this.job, cancelRequested: true, message: 'Cancelling…' };
    try {
      await ipc.installCancel(id);
    } catch (e) {
      this.error = String(e);
    }
  }

  dismissJob() {
    if (this.job?.outcome) this.job = null;
  }

  async start(name: string) {
    await this.act(name, () => ipc.distroStart(name));
  }
  async stop(name: string) {
    await this.act(name, () => ipc.distroStop(name));
  }
  private async act(name: string, f: () => Promise<WslReport>) {
    this.busyDistro = name;
    this.error = null;
    try {
      this.report = await f();
    } catch (e) {
      this.error = String(e);
    } finally {
      this.busyDistro = null;
    }
  }
}

export const runtime = new RuntimeStore();
