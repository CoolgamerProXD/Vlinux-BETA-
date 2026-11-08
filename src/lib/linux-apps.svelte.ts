import * as ipc from './ipc';
import { notify } from './notify.svelte';
import {
  loadRecents, pushRecent, saveRecents,
  type AppsReport, type LaunchRecord, type LinuxApp,
} from './linux-apps-types';

export type AppsPhase = 'idle' | 'loading' | 'ready' | 'stopped' | 'unavailable' | 'error';

/** Applications are re-read when the launcher opens if the list is older than this. */
export const APPS_TTL_MS = 60_000;

/**
 * The launcher's view of the selected distribution's installed applications.
 *
 * Nothing runs until the launcher asks, and the backend only reads Desktop Entry files of a distribution
 * that is *already running*. There is no polling: one global listener (attached once) reports how
 * launches end.
 */
class LinuxApps {
  distro = $state<string | null>(null);
  phase = $state<AppsPhase>('idle');
  report = $state<AppsReport | null>(null);
  error = $state<string | null>(null);
  icons = $state<Record<string, string>>({});
  recents = $state<string[]>([]);
  launching = $state<Record<string, boolean>>({});

  private loadedAt = 0;
  private seq = 0;
  private asked = new Set<string>();
  private unlisten: (() => void) | undefined;
  private attaching = false;
  private warnedNoWslg = new Set<string>();

  get apps(): LinuxApp[] {
    return this.report?.state === 'ready' ? this.report.apps : [];
  }

  get message(): string | null {
    return this.report?.message ?? null;
  }

  async load(distro: string | null, force = false): Promise<void> {
    if (!distro) {
      this.distro = null;
      this.phase = 'idle';
      this.report = null;
      return;
    }
    if (distro !== this.distro) {
      this.distro = distro;
      this.report = null;
      this.icons = {};
      this.asked.clear();
      this.recents = loadRecents(distro);
      this.phase = 'idle';
      this.loadedAt = 0;
    }
    if (!force && this.phase === 'ready' && Date.now() - this.loadedAt < APPS_TTL_MS) return;
    const seq = ++this.seq;
    this.phase = 'loading';
    this.error = null;
    try {
      const r = await ipc.appsDiscover(distro);
      if (seq !== this.seq) return;
      this.report = r;
      this.phase = r.state;
      this.loadedAt = Date.now();
      if (r.state === 'ready') this.asked.clear();
    } catch (e) {
      if (seq !== this.seq) return;
      this.error = String((e as Error)?.message ?? e);
      this.phase = 'error';
    }
  }

  /** Fetch icons for what is on screen. Each id is asked for once; failures just leave the tile. */
  async loadIcons(ids: string[]): Promise<void> {
    const distro = this.distro;
    if (!distro) return;
    const want = ids.filter((i) => !this.asked.has(i)).slice(0, 64);
    if (!want.length) return;
    for (const i of want) this.asked.add(i);
    try {
      const got = await ipc.appsIcons(distro, want);
      if (distro === this.distro) this.icons = { ...this.icons, ...got };
    } catch {
      /* no icons: the generated tiles are used */
    }
  }

  async launch(app: LinuxApp): Promise<boolean> {
    const distro = this.distro;
    if (!distro || !app.launchable || this.launching[app.id]) return false;
    this.launching[app.id] = true;
    try {
      await ipc.appsLaunch(distro, app.id);
      this.recents = pushRecent(this.recents, app.id);
      saveRecents(distro, this.recents);
      return true;
    } catch (e) {
      const msg = String((e as Error)?.message ?? e);
      notify.error(`Couldn't launch ${app.name}.`, 'launcher', msg);
      return false;
    } finally {
      delete this.launching[app.id];
    }
  }

  /** Subscribe to launch outcomes once for the lifetime of the shell. */
  async attach(): Promise<void> {
    if (this.unlisten || this.attaching) return;
    this.attaching = true;
    try {
      this.unlisten = await ipc.onAppLaunch((r) => this.onEvent(r));
    } finally {
      this.attaching = false;
    }
  }

  detach() {
    this.unlisten?.();
    this.unlisten = undefined;
  }

  private onEvent(r: LaunchRecord) {
    if (r.state === 'running') {
      notify.push({ kind: 'info', source: 'launcher', text: `Starting ${r.name}… Linux apps open as their own Windows windows (WSLg), not inside VLINUX.`, ms: 3500 });
      const d = this.distro;
      if (d && this.report && !this.report.wslg && !this.warnedNoWslg.has(d)) {
        this.warnedNoWslg.add(d);
        notify.warning(`WSLg was not detected in “${d}”, so ${r.name} may not show a window. Update WSL (wsl --update from Windows) if nothing appears.`, 'launcher');
      }
    } else if (r.state === 'failed') {
      notify.error(`${r.name} stopped with an error.`, 'launcher', r.message ?? (r.code !== null ? `Exit code ${r.code}` : undefined));
    }
    // A normal exit is not news: the user closed the app.
  }
}

export const linuxApps = new LinuxApps();
