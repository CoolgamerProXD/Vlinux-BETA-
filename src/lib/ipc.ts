// Single choke-point between the UI and the Rust core.
//
// The frontend can only call the fixed set of typed commands below; there is no
// generic "run this command" bridge. When running outside Tauri (plain browser,
// used for UI development and automated screenshots) every call falls back to a
// browser implementation so the UI stays testable — those fallbacks never touch
// a real system.

import { sanitizeConfig, defaultConfig, type Config, type ConfigPayload, type SystemInfo } from './types';
import type { Catalog, InstallEvent, InstallPlan, StorageOverview, Verdict, WslReport } from './runtime-types';

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

export async function distroStart(name: string): Promise<WslReport> {
  if (isTauri()) return call<WslReport>('distro_start', { name });
  return (await preview()).distroState(name, 'running');
}

export async function distroStop(name: string): Promise<WslReport> {
  if (isTauri()) return call<WslReport>('distro_stop', { name });
  return (await preview()).distroState(name, 'stopped');
}

export async function storageOverview(): Promise<StorageOverview> {
  if (isTauri()) return call<StorageOverview>('storage_overview');
  return (await preview()).storageOverview((await getConfig()).config);
}

export async function storageValidatePath(path: string): Promise<Verdict> {
  if (isTauri()) return call<Verdict>('storage_validate_path', { path });
  return (await preview()).validatePath(path);
}
