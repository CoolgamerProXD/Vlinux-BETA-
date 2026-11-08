// Browser-preview stand-in for Linux application discovery.
//
// SIMULATED: no distribution is read and nothing is launched. It exists so the launcher's states
// (loading, stopped, empty, list, launch failure) can be developed and tested without Windows.
// The desktop app never loads this module.

import type { AppsReport, LaunchRecord, LinuxApp } from './linux-apps-types';
import { wslDetect } from './ipc';
import { scenario } from './preview';

const NOTE = 'SIMULATED PREVIEW DATA — these applications are not installed anywhere; nothing is launched in the browser preview.';
const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

const mk = (id: string, name: string, over: Partial<LinuxApp> = {}): LinuxApp => ({
  id, name, generic_name: null, comment: null, categories: [], keywords: [], icon: null, terminal: false, launchable: true, reason: null, source: 'system', ...over,
});

const SAMPLE: LinuxApp[] = [
  mk('org.example.TextEditor.desktop', 'Text Editor', { generic_name: 'Text Editor', comment: 'Edit text files', categories: ['Utility', 'TextEditor'], keywords: ['notes'] }),
  mk('org.example.Browser.desktop', 'Web Browser', { generic_name: 'Web Browser', comment: 'Browse the web', categories: ['Network'], keywords: ['internet'] }),
  mk('org.example.Calculator.desktop', 'Calculator', { comment: 'Perform calculations', categories: ['Utility'] }),
  mk('org.example.Monitor.desktop', 'System Monitor', { comment: 'Processes and resources', categories: ['System'] }),
  mk('org.example.Htop.desktop', 'Htop', { terminal: true, launchable: false, reason: 'This program runs inside a terminal. Open it from the Terminal app.' }),
  mk('org.example.Broken.desktop', 'Broken Example', { comment: 'Always fails to start (to show the error path)' }),
  mk('org.example.Ghost.desktop', 'Missing Program', { launchable: false, reason: "'/usr/bin/ghost' does not exist in this distribution." }),
  mk('org.example.Flat.desktop', 'Flatpak Example', { source: 'flatpak', comment: 'Installed with Flatpak' }),
  mk('Terminal.desktop', 'Terminal', { comment: 'Terminal emulator', categories: ['System'] }),
  mk('Terminal-flatpak.desktop', 'Terminal', { source: 'flatpak', comment: 'Terminal emulator' }),
];

type Listener = (r: LaunchRecord) => void;
const listeners = new Set<Listener>();
export const onLaunch = (cb: Listener): (() => void) => {
  listeners.add(cb);
  return () => void listeners.delete(cb);
};
const emit = (r: LaunchRecord) => listeners.forEach((l) => l(r));

let counter = 1;
const records: LaunchRecord[] = [];

export async function discover(distro: string): Promise<AppsReport> {
  await wait(220);
  const base = { distro, wslg: true, apps: [] as LinuxApp[], folders: [] as string[], skipped: 0, warnings: [NOTE], elapsed_ms: 220 };
  let running = false;
  try {
    const r = await wslDetect(false);
    const d = r.distros.find((x) => x.name === distro);
    if (!d) return { ...base, state: 'unavailable', message: `No distribution named '${distro}' is registered with WSL.`, warnings: [] };
    running = d.state === 'running';
  } catch {
    return { ...base, state: 'unavailable', message: 'WSL is not available in the browser preview.', warnings: [] };
  }
  if (!running) return { ...base, state: 'stopped', message: `'${distro}' is not running. Start it to see and launch its applications.`, warnings: [] };
  if (scenario() === 'empty') return { ...base, state: 'ready', message: 'No desktop applications are installed in this distribution. Install some with its package manager (Linux Setup can search packages).' };
  return { ...base, state: 'ready', message: null, apps: SAMPLE, folders: ['/usr/share/applications', '/var/lib/flatpak/exports/share/applications'] };
}

export async function icons(_distro: string, ids: string[]): Promise<Record<string, string>> {
  await wait(60);
  // One simple generated SVG so the icon path is exercised; the rest use the generated tile.
  const svg = "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 48 48'><rect width='48' height='48' rx='10' fill='%23ff8a3d'/><path d='M14 24h20M24 14v20' stroke='white' stroke-width='5' stroke-linecap='round'/></svg>";
  return Object.fromEntries(ids.filter((i) => i === 'org.example.Calculator.desktop').map((i) => [i, `data:image/svg+xml;utf8,${svg}`]));
}

export async function launch(_distro: string, id: string): Promise<LaunchRecord> {
  const app = SAMPLE.find((a) => a.id === id);
  if (!app || !app.launchable) throw new Error(app?.reason ?? 'That application is no longer installed in this distribution.');
  const rec: LaunchRecord = { launch_id: counter++, id, name: app.name, state: 'running', code: null, message: null };
  records.push(rec);
  setTimeout(() => emit(rec), 0);
  if (id === 'org.example.Broken.desktop') {
    setTimeout(() => emit({ ...rec, state: 'failed', code: 127, message: 'exec: broken-example: not found' }), 350);
  }
  return rec;
}
