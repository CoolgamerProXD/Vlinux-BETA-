// BROWSER PREVIEW ONLY. This is NOT a shell and NOT a terminal emulator of Linux.
//
// Outside the desktop app there is no PTY, no WSL and no Linux, so the real backend cannot run.
// To keep the terminal UI (tabs, xterm.js rendering, keyboard capture, resizing) testable in a
// plain browser, this loopback transport echoes typed characters back and prints a banner saying
// so. It never interprets commands. The desktop app always uses the Rust backend instead.

import type { SessionInfo, SessionState, TerminalOutputEvent, TerminalStateEvent } from './terminal-types';
import * as ipc from './ipc';


const outputListeners = new Set<(e: TerminalOutputEvent) => void>();
const stateListeners = new Set<(e: TerminalStateEvent) => void>();
const sessions = new Map<string, SessionInfo>();
let counter = 1;

/** Observable by the automated UI test (the preview has no other way to show what was sent). */
export interface PreviewLog {
  opened: { id: string; distro: string; rows: number; cols: number }[];
  inputs: { id: string; data: string }[];
  resizes: { id: string; rows: number; cols: number }[];
  closed: string[];
  end?: (id: string, state: SessionState) => void;
}
const log: PreviewLog = { opened: [], inputs: [], resizes: [], closed: [] };
/** Test hook: end a preview session as a shell would (`exit`). */
export function endSession(id: string, state: SessionState) {
  stateListeners.forEach((l) => l({ id, state }));
}
log.end = endSession;
if (typeof window !== 'undefined') (window as unknown as { __vlinuxTerminalPreview: PreviewLog }).__vlinuxTerminalPreview = log;

function b64(text: string): string {
  const bytes = new TextEncoder().encode(text);
  let s = '';
  for (const b of bytes) s += String.fromCharCode(b);
  return btoa(s);
}

function emit(id: string, text: string) {
  const data = b64(text);
  queueMicrotask(() => outputListeners.forEach((l) => l({ id, data })));
}

export async function open(distro: string | null, rows: number, cols: number): Promise<SessionInfo> {
  const report = await ipc.wslDetect(false);
  if (report.status.kind !== 'ready') throw new Error('A Linux terminal needs WSL, which is not ready in this preview. Open Linux Setup to see what is needed.');
  let name = distro;
  if (!name) {
    const { config } = await import('./config.svelte');
    name = config.value.runtime.selected_distro;
  }
  if (!name) throw new Error('No distribution is selected for VLINUX. Pick one in Linux Setup, or choose one from the terminal\'s new-tab menu.');
  const d = report.distros.find((x) => x.name === name);
  if (!d || d.kind !== 'regular') throw new Error(`The distribution '${name}' is not installed (it may have been removed). Open Linux Setup to choose another.`);
  const id = `p${counter++}`;
  const info: SessionInfo = {
    id,
    distro: d.name,
    label: d.display_label,
    state: { kind: 'running' },
    started_at: Math.floor(Date.now() / 1000),
    shell: 'preview loopback',
  };
  sessions.set(id, info);
  log.opened.push({ id, distro: d.name, rows, cols });
  emit(
    id,
    '\x1b[33mVLINUX browser preview\x1b[0m — loopback only. No shell is running and nothing is executed;\r\n' +
      'typed characters are just echoed back. Use the desktop app for a real Linux shell.\r\n\r\n',
  );
  return info;
}

export async function input(id: string, data: string): Promise<void> {
  if (!sessions.has(id)) throw new Error(`There is no terminal session '${id}'.`);
  log.inputs.push({ id, data });
  let out = '';
  for (const ch of data) {
    if (ch === '\r') out += '\r\n';
    else if (ch === '\x7f') out += '\b \b';
    else if (ch === '\x03') out += '^C\r\n';
    else if (ch === '\x1b') out += '^[';
    else out += ch;
  }
  emit(id, out);
}

export async function resize(id: string, rows: number, cols: number): Promise<void> {
  if (!sessions.has(id)) throw new Error(`There is no terminal session '${id}'.`);
  log.resizes.push({ id, rows, cols });
}

export async function close(id: string): Promise<void> {
  if (!sessions.delete(id)) throw new Error(`There is no terminal session '${id}'.`);
  log.closed.push(id);
}

export function onOutput(cb: (e: TerminalOutputEvent) => void): () => void {
  outputListeners.add(cb);
  return () => outputListeners.delete(cb);
}

export function onState(cb: (e: TerminalStateEvent) => void): () => void {
  stateListeners.add(cb);
  return () => stateListeners.delete(cb);
}
