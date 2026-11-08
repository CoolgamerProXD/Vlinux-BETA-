import { desktopAction } from './shortcuts';
// Types mirrored from the Rust terminal module (`vlinux_runtime::terminal`).

export type SessionState =
  | { kind: 'running' }
  | { kind: 'exited'; code: number | null }
  | { kind: 'failed'; message: string };

export interface SessionInfo {
  id: string;
  /** Registered WSL name, e.g. `archlinux`. */
  distro: string;
  /** Human label, e.g. `Arch Linux`. */
  label: string;
  state: SessionState;
  started_at: number;
  /** Which shell attempt is running: `default shell` or the `/bin/sh` fallback. */
  shell: string;
}

export interface TerminalOutputEvent {
  id: string;
  /** Raw terminal bytes, base64. */
  data: string;
}

export interface TerminalStateEvent {
  id: string;
  state: SessionState;
}

export function decodeBase64(b64: string): Uint8Array {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

/** Size used before the first fit; a real fit happens as soon as the terminal is on screen. */
export const DEFAULT_ROWS = 30;
export const DEFAULT_COLS = 100;

/**
 * Output that arrives for a session id the UI has not registered yet (the backend starts streaming
 * before `terminal_open` has returned to the UI). Bounded so unknown ids cannot grow memory.
 */
export class PendingOutput {
  private chunks = new Map<string, Uint8Array[]>();
  private sizes = new Map<string, number>();
  private states = new Map<string, SessionState>();
  constructor(private readonly maxBytesPerId = 1 << 20, private readonly maxIds = 8) {}

  push(id: string, data: Uint8Array) {
    if (!this.chunks.has(id)) {
      if (this.chunks.size >= this.maxIds) {
        const oldest = this.chunks.keys().next().value;
        if (oldest !== undefined) this.drop(oldest);
      }
      this.chunks.set(id, []);
      this.sizes.set(id, 0);
    }
    const used = this.sizes.get(id) ?? 0;
    if (used + data.length > this.maxBytesPerId) return;
    this.chunks.get(id)!.push(data);
    this.sizes.set(id, used + data.length);
  }

  state(id: string, s: SessionState) {
    this.states.set(id, s);
  }

  /** Hand over (and forget) everything stored for `id`. */
  take(id: string): { chunks: Uint8Array[]; state: SessionState | null } {
    const chunks = this.chunks.get(id) ?? [];
    const state = this.states.get(id) ?? null;
    this.drop(id);
    return { chunks, state };
  }

  private drop(id: string) {
    this.chunks.delete(id);
    this.sizes.delete(id);
    this.states.delete(id);
  }

  get size(): number {
    return this.chunks.size;
  }
}

/**
 * Batches flow-control acknowledgements: tells the backend how many bytes the terminal has
 * rendered, but not more often than every `threshold` bytes (plus one trailing flush, so a small
 * remainder is never held back). No timer runs while the terminal is idle.
 */
export class AckBatcher {
  private owed = 0;
  private timer: ReturnType<typeof setTimeout> | undefined;
  constructor(
    private readonly send: (bytes: number) => void,
    private readonly threshold = 32 * 1024,
    private readonly flushMs = 40,
  ) {}

  rendered(bytes: number) {
    this.owed += bytes;
    if (this.owed >= this.threshold) {
      this.flush();
    } else if (this.timer === undefined) {
      this.timer = setTimeout(() => this.flush(), this.flushMs);
    }
  }

  flush() {
    if (this.timer !== undefined) {
      clearTimeout(this.timer);
      this.timer = undefined;
    }
    if (this.owed > 0) {
      const n = this.owed;
      this.owed = 0;
      this.send(n);
    }
  }

  dispose() {
    if (this.timer !== undefined) clearTimeout(this.timer);
    this.timer = undefined;
    this.owed = 0;
  }
}

/**
 * Shell-level shortcuts (Alt+F1, Alt+W, Alt+1…9, Alt+Shift+Q, Ctrl+Alt+Q, F11). When the terminal
 * has focus, xterm.js must leave these alone so the desktop still receives them — and so they are
 * not also typed into the shell.
 */
export function isShellShortcut(e: Pick<KeyboardEvent, 'ctrlKey' | 'altKey' | 'shiftKey' | 'metaKey' | 'code'>): boolean {
  return desktopAction(e) !== null;
}

/** Terminal-window shortcuts: Ctrl+Shift+T new tab, Ctrl+Shift+W close tab, Ctrl+Shift+C/V copy/paste. */
export type TerminalShortcut = 'new-tab' | 'close-tab' | 'copy' | 'paste';
export function terminalShortcut(e: Pick<KeyboardEvent, 'ctrlKey' | 'altKey' | 'shiftKey' | 'metaKey' | 'code'>): TerminalShortcut | null {
  if (!e.ctrlKey || !e.shiftKey || e.altKey || e.metaKey) return null;
  switch (e.code) {
    case 'KeyT': return 'new-tab';
    case 'KeyW': return 'close-tab';
    case 'KeyC': return 'copy';
    case 'KeyV': return 'paste';
    default: return null;
  }
}
