import * as ipc from './ipc';
import { config } from './config.svelte';
import { ui } from './ui.svelte';
import type { CheckResult, DiagCategory, DiagReport, DistroCard, DistroOpEvent, InspectOutcome, Overview } from './distros-types';

export type DistroTab = 'overview' | 'packages' | 'config' | 'diagnostics';

/**
 * State of the Distribution Manager. It performs no work by itself: discovery is cached in the
 * backend and refreshed only on an explicit action (or after an operation the user started), and
 * the event listeners exist only while the Distribution Manager window is open (attach/detach).
 */
class DistrosStore {
  overview = $state<Overview | null>(null);
  loading = $state(false);
  error = $state<string | null>(null);
  selected = $state<string | null>(null);
  tab = $state<DistroTab>('overview');
  /** Latest operation message per distribution (from backend events and results). */
  notes = $state<Record<string, string>>({});
  /** Set when an inspection needs the user's permission to start a stopped distribution. */
  startPrompt = $state<string | null>(null);
  /** A stop awaiting the user's explicit confirmation. */
  stopPrompt = $state<string | null>(null);
  /** Operations the user has just asked for, shown at once (before the backend reports its own state). */
  pending = $state<Record<string, 'start' | 'stop' | 'inspect'>>({});

  diagRunning = $state(false);
  diagProgress = $state<CheckResult[]>([]);
  diagReport = $state<DiagReport | null>(null);
  diagError = $state<string | null>(null);

  private attached = 0;
  private unlistenOp: (() => void) | undefined;
  private unlistenDiag: (() => void) | undefined;
  private seq = 0;
  private again = false;
  private againForce = false;
  private inflight: Promise<void> | null = null;
  private activeRun = '';

  get cards(): DistroCard[] {
    return this.overview?.cards ?? [];
  }

  /** The distribution the tabs talk about: the user's pick, else the preferred one, else the first regular one. */
  get current(): DistroCard | null {
    const cards = this.cards;
    return (
      cards.find((c) => c.name === this.selected) ??
      cards.find((c) => c.is_preferred && c.kind === 'regular') ??
      cards.find((c) => c.kind === 'regular') ??
      null
    );
  }

  async attach() {
    this.attached++;
    if (this.attached > 1) return;
    const [op, diag] = await Promise.all([ipc.onDistroOp((e) => this.onOp(e)), ipc.onDiagProgress((e) => this.onDiag(e))]);
    if (this.attached === 0) {
      // Closed again before the listeners were registered.
      op();
      diag();
      return;
    }
    this.unlistenOp = op;
    this.unlistenDiag = diag;
    if (!this.overview) await this.refresh(false);
  }

  detach() {
    this.attached = Math.max(0, this.attached - 1);
    if (this.attached > 0) return;
    this.unlistenOp?.();
    this.unlistenDiag?.();
    this.unlistenOp = undefined;
    this.unlistenDiag = undefined;
  }

  /** `force` asks WSL again (explicit Refresh); otherwise the backend's cached discovery is used. */
  refresh(force = false): Promise<void> {
    if (this.inflight) {
      // An obsolete request is not cancelled mid-flight, but a fresh one follows it and callers wait for that.
      this.again = true;
      this.againForce = this.againForce || force;
      return this.inflight;
    }
    this.inflight = this.runRefresh(force).finally(() => {
      this.inflight = null;
    });
    return this.inflight;
  }

  private async runRefresh(first: boolean) {
    let force = first;
    do {
      this.again = false;
      this.loading = true;
      const mine = ++this.seq;
      try {
        const o = await ipc.distroOverview(force);
        if (mine === this.seq) {
          this.overview = o;
          this.error = null;
        }
      } catch (e) {
        this.error = String(e);
      } finally {
        this.loading = false;
      }
      force = this.againForce;
      this.againForce = false;
    } while (this.again);
  }

  select(name: string, tab?: DistroTab) {
    this.selected = name;
    if (tab) this.tab = tab;
  }

  private onOp(e: DistroOpEvent) {
    this.notes = { ...this.notes, [e.distro]: e.message };
    void this.refresh(false);
  }

  private fail(name: string, e: unknown) {
    const m = String(e);
    this.notes = { ...this.notes, [name]: m };
    ui.toast(m, 'error', 7000);
  }

  async setPreferred(name: string | null) {
    try {
      this.overview = await ipc.distroSetPreferred(name);
      // The preference lives in VLINUX's own config: reload it so Settings and the Terminal agree.
      await config.load();
      ui.toast(name ? `“${name}” is now your preferred distribution. The Windows default was not changed.` : 'Preferred distribution cleared.');
    } catch (e) {
      this.fail(name ?? '', e);
    }
  }

  private setPending(name: string, op: 'start' | 'stop' | 'inspect' | null) {
    const next = { ...this.pending };
    if (op) next[name] = op;
    else delete next[name];
    this.pending = next;
  }

  async start(name: string) {
    if (this.pending[name]) return;
    this.setPending(name, 'start');
    try {
      const r = await ipc.distroStart(name);
      this.notes = { ...this.notes, [name]: r.outcome.message };
    } catch (e) {
      this.fail(name, e);
    } finally {
      await this.refresh(false);
      this.setPending(name, null);
    }
  }

  requestStop(name: string) {
    this.stopPrompt = name;
  }

  /** Called only from the confirmation dialog. */
  async confirmStop(name: string) {
    this.stopPrompt = null;
    if (this.pending[name]) return;
    this.setPending(name, 'stop');
    try {
      const r = await ipc.distroStop(name, true);
      this.notes = { ...this.notes, [name]: r.outcome.message };
    } catch (e) {
      this.fail(name, e);
    } finally {
      await this.refresh(false);
      this.setPending(name, null);
    }
  }

  async inspect(name: string, allowStart = false): Promise<InspectOutcome | null> {
    this.startPrompt = null;
    if (this.pending[name]) return null;
    this.setPending(name, 'inspect');
    let needsStart = false;
    try {
      const r = await ipc.distroInspect(name, allowStart);
      if (r.kind === 'needs_start') {
        needsStart = true;
      } else {
        this.notes = {
          ...this.notes,
          [name]: r.inspection.started_by_inspection ? 'Inspected. This started the distribution (you allowed that).' : 'Inspected. The distribution was not started by this.',
        };
      }
      return r;
    } catch (e) {
      this.fail(name, e);
      return null;
    } finally {
      await this.refresh(false);
      this.setPending(name, null);
      // Asked only once the card is idle again, so the answer can be acted on at once.
      if (needsStart) this.startPrompt = name;
    }
  }

  async cancelInspect(name: string) {
    try {
      await ipc.distroInspectCancel(name);
    } catch (e) {
      this.fail(name, e);
    }
  }

  // ---- diagnostics (state lives here so a run survives switching tabs) ----

  async runDiagnostics(distro: string | null, allowStart: boolean, allowWrite: boolean, categories: DiagCategory[] | null) {
    if (this.diagRunning) return;
    this.diagRunning = true;
    this.diagProgress = [];
    this.diagReport = null;
    this.diagError = null;
    try {
      this.diagReport = await ipc.diagRun({ distro, allow_start: allowStart, allow_write: allowWrite, categories });
    } catch (e) {
      this.diagError = String(e);
    } finally {
      this.diagRunning = false;
    }
    // A run may have started or changed a distribution (only with the permissions given).
    void this.refresh(false);
  }

  async cancelDiagnostics() {
    try {
      await ipc.diagCancel();
    } catch (e) {
      this.diagError = String(e);
    }
  }

  private onDiag(e: { run_id: string; index: number; result: CheckResult }) {
    if (e.run_id !== this.activeRun) {
      this.activeRun = e.run_id;
      this.diagProgress = [];
    }
    if (this.diagRunning) this.diagProgress = [...this.diagProgress, e.result];
  }
}

export const distros = new DistrosStore();
