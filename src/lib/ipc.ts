// Single choke-point between the UI and the Rust core.
//
// The frontend can only call the fixed set of typed commands below; there is no
// generic "run this command" bridge. When running outside Tauri (plain browser,
// used for UI development and automated screenshots) every call falls back to a
// browser implementation so the UI stays testable — those fallbacks never touch
// a real system.

import { sanitizeConfig, defaultConfig, type Config, type ConfigPayload, type SystemInfo } from './types';
import type { Catalog, InstallEvent, InstallPlan, StorageOverview, Verdict, WslReport } from './runtime-types';
import type {
  ApplyOutcome, BackupInfo, ConfPlan, DiagOptions, DiagProgress, DiagReport, DistroOpEvent, InspectOutcome, LifecycleResult, Overview,
  PackageHit, PackagePlan, SettingChange, WindowsConfig, WslConfRead,
} from './distros-types';

const distrosPreview = () => import('./distros-preview');

export const isTauri = (): boolean => typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<T>(cmd, args);
}

const LS_KEY = 'vlinux.config';

export async function getConfig(): Promise<ConfigPayload> {
  if (isTauri()) return call<ConfigPayload>('get_config');
  let config = defaultConfig();
  let warning: string | null = null;
  try {
    const raw = localStorage.getItem(LS_KEY);
    if (raw) config = sanitizeConfig(JSON.parse(raw));
  } catch {
    warning = 'Stored preview settings were unreadable; defaults are in use.';
  }
  return { config, path: 'Browser preview (localStorage) — desktop app uses %APPDATA%\\app.vlinux.desktop\\config.toml', warning };
}

export async function saveConfig(config: Config): Promise<Config> {
  if (isTauri()) return call<Config>('save_config', { config });
  const clean = sanitizeConfig(config);
  localStorage.setItem(LS_KEY, JSON.stringify(clean));
  return clean;
}

export async function getSystemInfo(): Promise<SystemInfo> {
  if (isTauri()) return call<SystemInfo>('get_system_info');
  const ua = navigator.userAgent;
  return {
    app_version: __APP_VERSION__,
    os_name: /Windows/.test(ua) ? 'Windows' : /Mac/.test(ua) ? 'macOS' : /Linux/.test(ua) ? 'Linux' : 'Unknown',
    os_version: 'browser preview',
    os_arch: 'n/a',
    cpu_threads: navigator.hardwareConcurrency || 1,
    is_windows: /Windows/.test(ua),
  };
}

export async function isFullscreen(): Promise<boolean> {
  if (isTauri()) return call<boolean>('is_fullscreen');
  return !!document.fullscreenElement;
}

export async function setFullscreen(on: boolean): Promise<boolean> {
  if (isTauri()) return call<boolean>('set_fullscreen', { fullscreen: on });
  if (on) await document.documentElement.requestFullscreen();
  else if (document.fullscreenElement) await document.exitFullscreen();
  return !!document.fullscreenElement;
}

/** Leave full-screen and minimise to the Windows taskbar. Nothing is terminated. */
export async function exitToWindows(): Promise<void> {
  if (isTauri()) return call<void>('exit_to_windows');
  if (document.fullscreenElement) await document.exitFullscreen();
  throw new Error('Browser preview: there is no host window to minimise. Use the desktop app.');
}

export async function quitApp(): Promise<void> {
  if (isTauri()) return call<void>('quit_app');
  throw new Error('Browser preview: quitting is only available in the desktop app.');
}

/** Subscribe to full-screen changes pushed by the Rust core. Returns an unsubscribe function. */
export async function onFullscreenChange(cb: (on: boolean) => void): Promise<() => void> {
  if (isTauri()) {
    const { listen } = await import('@tauri-apps/api/event');
    return listen<boolean>('vlinux://fullscreen', (e) => cb(e.payload));
  }
  const h = () => cb(!!document.fullscreenElement);
  document.addEventListener('fullscreenchange', h);
  return () => document.removeEventListener('fullscreenchange', h);
}

// ---- Linux runtime (WSL) --------------------------------------------------------------------
// Every call is a typed, narrow request. The backend validates names against the real WSL state
// and builds all command lines itself. In the browser these are SIMULATED (see preview.ts).

const preview = () => import('./preview');

/** Read-only snapshot of WSL and installed distributions. Cached by the backend. */
export async function wslDetect(force = false): Promise<WslReport> {
  if (isTauri()) return call<WslReport>('wsl_detect', { force });
  return (await preview()).detect();
}

/** The official list of installable distributions (needs internet). */
export async function wslCatalog(force = false): Promise<Catalog> {
  if (isTauri()) return call<Catalog>('wsl_catalog', { force });
  return (await preview()).catalog();
}

/** Ask the backend for a reviewable plan. Installs nothing. */
export async function installPrepare(kind: 'distribution' | 'wsl_platform', name: string | null): Promise<InstallPlan> {
  if (isTauri()) return call<InstallPlan>('install_prepare', { kind, name });
  return (await preview()).prepare(kind, name);
}

/** Start a reviewed plan. `approved` must be true: it is the user's explicit consent. */
export async function installStart(planId: string, approved: boolean): Promise<{ job_id: string }> {
  if (isTauri()) return call<{ job_id: string }>('install_start', { planId, approved });
  return (await preview()).start(planId, approved);
}

export async function installCancel(jobId: string): Promise<boolean> {
  if (isTauri()) return call<boolean>('install_cancel', { jobId });
  return (await preview()).cancel(jobId);
}

export async function onInstallEvent(cb: (e: InstallEvent) => void): Promise<() => void> {
  if (isTauri()) {
    const { listen } = await import('@tauri-apps/api/event');
    return listen<InstallEvent>('vlinux://install', (e) => cb(e.payload));
  }
  return (await preview()).onInstall(cb);
}

/** Start one distribution (explicit user action). The backend de-duplicates and verifies. */
export async function distroStart(name: string): Promise<LifecycleResult> {
  if (isTauri()) return call<LifecycleResult>('distro_start', { name });
  return (await distrosPreview()).start(name);
}

/** Stop one distribution. `confirmed` is the user's explicit confirmation; the backend refuses without it. */
export async function distroStop(name: string, confirmed: boolean): Promise<LifecycleResult> {
  if (isTauri()) return call<LifecycleResult>('distro_stop', { name, confirmed });
  return (await distrosPreview()).stop(name, confirmed);
}

export async function storageOverview(): Promise<StorageOverview> {
  if (isTauri()) return call<StorageOverview>('storage_overview');
  return (await preview()).storageOverview((await getConfig()).config);
}

export async function storageValidatePath(path: string): Promise<Verdict> {
  if (isTauri()) return call<Verdict>('storage_validate_path', { path });
  return (await preview()).validatePath(path);
}

// ---- Terminal (M3) ----------------------------------------------------------------------------
// The UI names at most a distribution; the backend validates it, builds the wsl.exe command line
// and owns the PTY. `terminalInput` carries keystrokes to that PTY and nothing else. In the browser
// preview a clearly-labelled loopback stands in (see terminal-preview.ts): it runs no shell.

import type { SessionInfo, TerminalOutputEvent, TerminalStateEvent } from './terminal-types';

const termPreview = () => import('./terminal-preview');

/** Open a session in `distro`, or in the distribution selected in VLINUX when `null`. */
export async function terminalOpen(distro: string | null, rows: number, cols: number): Promise<SessionInfo> {
  if (isTauri()) return call<SessionInfo>('terminal_open', { distro, rows, cols });
  return (await termPreview()).open(distro, rows, cols);
}

export async function terminalInput(id: string, data: string): Promise<void> {
  if (isTauri()) return call<void>('terminal_input', { id, data });
  return (await termPreview()).input(id, data);
}

export async function terminalResize(id: string, rows: number, cols: number): Promise<void> {
  if (isTauri()) return call<void>('terminal_resize', { id, rows, cols });
  return (await termPreview()).resize(id, rows, cols);
}

/** Flow control: the terminal has rendered `bytes` more output. */
export async function terminalAck(id: string, bytes: number): Promise<void> {
  if (isTauri()) return call<void>('terminal_ack', { id, bytes });
}

export async function terminalClose(id: string): Promise<void> {
  if (isTauri()) return call<void>('terminal_close', { id });
  return (await termPreview()).close(id);
}

export async function onTerminalOutput(cb: (e: TerminalOutputEvent) => void): Promise<() => void> {
  if (isTauri()) {
    const { listen } = await import('@tauri-apps/api/event');
    return listen<TerminalOutputEvent>('vlinux://terminal-output', (e) => cb(e.payload));
  }
  return (await termPreview()).onOutput(cb);
}

export async function onTerminalState(cb: (e: TerminalStateEvent) => void): Promise<() => void> {
  if (isTauri()) {
    const { listen } = await import('@tauri-apps/api/event');
    return listen<TerminalStateEvent>('vlinux://terminal-state', (e) => cb(e.payload));
  }
  return (await termPreview()).onState(cb);
}

// ---- Files and shared folders (M4) -------------------------------------------------------------
// Typed file-manager commands. A place is a `Location` (a Linux path in the chosen distribution, or
// a shared Windows folder id plus names): the UI never sends a host path and never builds a shell
// command. In the browser preview a clearly-labelled in-memory demo filesystem stands in
// (fs-preview.ts); it touches nothing real.

import type {
  Conflict, JobEvent, ListingHead, ListOptions, Location, MappingOutcome, MappingReport, Page, Places, Resolution, TransferMode,
} from './fs-types';

const fsPreview = () => import('./fs-preview');

export async function fsPlaces(distro: string | null, refresh = false): Promise<Places> {
  if (isTauri()) return call<Places>('fs_places', { distro, refresh });
  return (await fsPreview()).places(distro, (await getConfig()).config);
}

export async function fsListOpen(distro: string | null, location: Location, options: ListOptions, pageSize = 200): Promise<ListingHead> {
  if (isTauri()) return call<ListingHead>('fs_list_open', { distro, location, options, pageSize });
  return (await fsPreview()).listOpen(distro, location, options, pageSize);
}

export async function fsListPage(id: string, offset: number, limit = 200): Promise<Page> {
  if (isTauri()) return call<Page>('fs_list_page', { id, offset, limit });
  return (await fsPreview()).listPage(id, offset, limit);
}

export async function fsListClose(id: string): Promise<void> {
  if (isTauri()) return call<void>('fs_list_close', { id });
  (await fsPreview()).listClose(id);
}

export async function fsCreate(distro: string | null, parent: Location, name: string, kind: 'folder' | 'file'): Promise<Location> {
  if (isTauri()) return call<Location>('fs_create', { distro, parent, name, kind });
  return (await fsPreview()).create(distro, parent, name, kind);
}

export async function fsRename(distro: string | null, location: Location, newName: string): Promise<Location> {
  if (isTauri()) return call<Location>('fs_rename', { distro, location, newName });
  return (await fsPreview()).rename(distro, location, newName);
}

/** Delete is a background job; `confirmed` must be true (the UI asks first, the backend insists). */
export async function fsDelete(distro: string | null, targets: Location[], confirmed: boolean): Promise<{ job_id: string }> {
  if (isTauri()) return call<{ job_id: string }>('fs_delete', { distro, targets, confirmed });
  return (await fsPreview()).deleteItems(distro, targets, confirmed);
}

export async function fsTransfer(distro: string | null, sources: Location[], dest: Location, mode: TransferMode, conflict: Conflict = 'keep_both'): Promise<{ job_id: string }> {
  if (isTauri()) return call<{ job_id: string }>('fs_transfer', { distro, sources, dest, mode, conflict });
  return (await fsPreview()).transfer(distro, sources, dest, mode, conflict);
}

export async function fsJobCancel(jobId: string): Promise<boolean> {
  if (isTauri()) return call<boolean>('fs_job_cancel', { jobId });
  return false;
}

export async function fsOpen(distro: string | null, location: Location): Promise<void> {
  if (isTauri()) return call<void>('fs_open', { distro, location });
  (await fsPreview()).open(distro, location);
}

export async function fsReveal(distro: string | null, location: Location): Promise<void> {
  if (isTauri()) return call<void>('fs_reveal', { distro, location });
  (await fsPreview()).reveal(distro, location);
}

export async function onFsJob(cb: (e: JobEvent) => void): Promise<() => void> {
  if (isTauri()) {
    const { listen } = await import('@tauri-apps/api/event');
    return listen<JobEvent>('vlinux://fs-job', (e) => cb(e.payload));
  }
  return (await fsPreview()).onJob(cb);
}

/** `check = false` starts no Linux process and claims nothing as connected. */
export async function mappingStatus(distro: string, check = false): Promise<MappingReport> {
  if (isTauri()) return call<MappingReport>('mapping_status', { distro, check });
  return (await fsPreview()).mappingStatus(distro, check, (await getConfig()).config);
}

export async function mappingApply(distro: string, folder: string, resolution: Resolution | null): Promise<MappingOutcome> {
  if (isTauri()) return call<MappingOutcome>('mapping_apply', { distro, folder, resolution });
  return (await fsPreview()).mappingApply(distro, folder, resolution, (await getConfig()).config);
}

export async function mappingUndo(distro: string, folder: string): Promise<MappingOutcome> {
  if (isTauri()) return call<MappingOutcome>('mapping_undo', { distro, folder });
  return (await fsPreview()).mappingUndo(distro, folder, (await getConfig()).config);
}

// ---- Distribution Manager (M5) ------------------------------------------------------------------
// Typed requests only. The UI names a distribution and, for configuration, a list of
// `{ id, value }` pairs for the few settings VLINUX manages; the backend validates the name against
// the registered distributions, re-reads the real file, builds the new content itself and refuses
// without `confirmed`. Nothing here accepts a command line, a path, or file text.

/** Cached discovery. `force` re-queries WSL (explicit refresh); nothing is polled. */
export async function distroOverview(force = false): Promise<Overview> {
  if (isTauri()) return call<Overview>('distro_overview', { force });
  return (await distrosPreview()).overview(force);
}

/** Record the preferred distribution in VLINUX's own config. Never changes the Windows default. */
export async function distroSetPreferred(name: string | null): Promise<Overview> {
  if (isTauri()) return call<Overview>('distro_set_preferred', { name });
  return (await distrosPreview()).setPreferred(name);
}

/** Read-only inspection. A stopped distribution is started only when `allowStart` is true. */
export async function distroInspect(name: string, allowStart: boolean): Promise<InspectOutcome> {
  if (isTauri()) return call<InspectOutcome>('distro_inspect', { name, allowStart });
  return (await distrosPreview()).inspect(name, allowStart);
}

export async function distroInspectCancel(name: string): Promise<boolean> {
  if (isTauri()) return call<boolean>('distro_inspect_cancel', { name });
  return false;
}

/** A reviewable package command. It is never executed by VLINUX. */
export async function distroPkgPlan(name: string, op: 'install' | 'update', packages: string[]): Promise<PackagePlan> {
  if (isTauri()) return call<PackagePlan>('distro_pkg_plan', { name, op, packages });
  return (await distrosPreview()).pkgPlan(name, op, packages);
}

export async function distroPkgSearch(name: string, query: string): Promise<PackageHit[]> {
  if (isTauri()) return call<PackageHit[]>('distro_pkg_search', { name, query });
  return (await distrosPreview()).pkgSearch(name, query);
}

export async function wslconfRead(name: string): Promise<WslConfRead> {
  if (isTauri()) return call<WslConfRead>('wslconf_read', { name });
  return (await distrosPreview()).confRead(name);
}

export async function wslconfPreview(name: string, changes: SettingChange[]): Promise<ConfPlan> {
  if (isTauri()) return call<ConfPlan>('wslconf_preview', { name, changes });
  return (await distrosPreview()).confPreview(name, changes);
}

export async function wslconfApply(name: string, changes: SettingChange[], basedOn: string, confirmed: boolean): Promise<ApplyOutcome> {
  if (isTauri()) return call<ApplyOutcome>('wslconf_apply', { name, changes, basedOn, confirmed });
  return (await distrosPreview()).confApply(name, changes, basedOn, confirmed);
}

export async function wslconfBackups(name: string): Promise<BackupInfo[]> {
  if (isTauri()) return call<BackupInfo[]>('wslconf_backups', { name });
  return (await distrosPreview()).confBackups(name);
}

export async function wslconfRestore(name: string, backupId: string, force: boolean, confirmed: boolean): Promise<ApplyOutcome> {
  if (isTauri()) return call<ApplyOutcome>('wslconf_restore', { name, backupId, force, confirmed });
  return (await distrosPreview()).confRestore(name, backupId, force, confirmed);
}

/** Read-only view of the Windows-owned `.wslconfig`. */
export async function wslconfigWindows(): Promise<WindowsConfig> {
  if (isTauri()) return call<WindowsConfig>('wslconfig_windows');
  return (await distrosPreview()).windowsConfig();
}

export async function diagRun(options: DiagOptions): Promise<DiagReport> {
  if (isTauri()) return call<DiagReport>('diag_run', { options });
  return (await distrosPreview()).diagRun(options);
}

export async function diagCancel(): Promise<boolean> {
  if (isTauri()) return call<boolean>('diag_cancel');
  return (await distrosPreview()).diagCancelRun();
}

export async function onDistroOp(cb: (e: DistroOpEvent) => void): Promise<() => void> {
  if (isTauri()) {
    const { listen } = await import('@tauri-apps/api/event');
    return listen<DistroOpEvent>('vlinux://distro-op', (e) => cb(e.payload));
  }
  return (await distrosPreview()).onOp(cb);
}

export async function onDiagProgress(cb: (e: DiagProgress) => void): Promise<() => void> {
  if (isTauri()) {
    const { listen } = await import('@tauri-apps/api/event');
    return listen<DiagProgress>('vlinux://diag-progress', (e) => cb(e.payload));
  }
  return (await distrosPreview()).onDiag(cb);
}

// ---- Linux applications (M7) -----------------------------------------------------------------
// Discovery only reads Desktop Entries of a *running* distribution (it never starts one). Launching
// sends a desktop-file id only; the backend re-reads the entry and builds the argument list itself.

import type { AppsReport, LaunchRecord } from './linux-apps-types';

const appsPreview = () => import('./linux-apps-preview');

export async function appsDiscover(distro: string): Promise<AppsReport> {
  if (isTauri()) return call<AppsReport>('apps_discover', { distro });
  return (await appsPreview()).discover(distro);
}

export async function appsIcons(distro: string, ids: string[]): Promise<Record<string, string>> {
  if (isTauri()) return call<Record<string, string>>('apps_icons', { distro, ids });
  return (await appsPreview()).icons(distro, ids);
}

export async function appsLaunch(distro: string, id: string): Promise<LaunchRecord> {
  if (isTauri()) return call<LaunchRecord>('apps_launch', { distro, id });
  return (await appsPreview()).launch(distro, id);
}

export async function onAppLaunch(cb: (r: LaunchRecord) => void): Promise<() => void> {
  if (isTauri()) {
    const { listen } = await import('@tauri-apps/api/event');
    return listen<LaunchRecord>('vlinux://app-launch', (e) => cb(e.payload));
  }
  return (await appsPreview()).onLaunch(cb);
}
