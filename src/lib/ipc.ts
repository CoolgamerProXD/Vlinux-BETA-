// Single choke-point between the UI and the Rust core.
//
// The frontend can only call the fixed set of typed commands below; there is no
// generic "run this command" bridge. When running outside Tauri (plain browser,
// used for UI development and automated screenshots) every call falls back to a
// browser implementation so the UI stays testable — those fallbacks never touch
// a real system.

import { sanitizeConfig, defaultConfig, type Config, type ConfigPayload, type SystemInfo } from './types';

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
