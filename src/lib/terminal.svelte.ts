import type { FitAddon } from '@xterm/addon-fit';
import type { Terminal as XTerm } from '@xterm/xterm';
import * as ipc from './ipc';
import {
  AckBatcher,
  DEFAULT_COLS,
  DEFAULT_ROWS,
  PendingOutput,
  decodeBase64,
  isShellShortcut,
  terminalShortcut,
  type SessionInfo,
  type SessionState,
  type TerminalShortcut,
} from './terminal-types';

export interface TermSession {
  info: SessionInfo;
  state: SessionState;
  term: XTerm;
  fit: FitAddon;
  acks: AckBatcher;
  rows: number;
  cols: number;
  /** Set once the xterm instance has been attached to the DOM. */
  attached: boolean;
}

/** Loaded on first use so xterm.js is not part of the desktop's start-up cost. */
async function loadXterm() {
  const [x, f, u] = await Promise.all([import('@xterm/xterm'), import('@xterm/addon-fit'), import('@xterm/addon-unicode11')]);
  return { Terminal: x.Terminal, FitAddon: f.FitAddon, Unicode11Addon: u.Unicode11Addon };
}

function cssVar(name: string, fallback: string): string {
  if (typeof document === 'undefined') return fallback;
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim() || fallback;
}

/** xterm theme in the VLINUX palette; the accent follows the user's setting. */
export function terminalTheme() {
  const accent = cssVar('--accent', '#a98bff');
  const rgb = cssVar('--accent-rgb', '169 139 255').replace(/\s+/g, ',');
  return {
    background: '#0b0b14',
    foreground: '#e6e6f2',
    cursor: accent,
    cursorAccent: '#0b0b14',
    selectionBackground: `rgba(${rgb},0.38)`,
    black: '#1c1c2e',
    red: '#ff6b81',
    green: '#5ff0b0',
    yellow: '#ffd36b',
    blue: '#7aa2ff',
    magenta: '#c792ff',
    cyan: '#5ee0ff',
    white: '#d7d7e6',
    brightBlack: '#5b5b78',
    brightRed: '#ff8fa0',
    brightGreen: '#8cf7c8',
    brightYellow: '#ffe39a',
    brightBlue: '#a3bcff',
    brightMagenta: '#dcb8ff',
    brightCyan: '#9aeeff',
    brightWhite: '#ffffff',
  };
}

/**
 * Terminal sessions for the UI.
 *
 * All process management is in Rust. This store owns one xterm.js emulator per backend session,
 * routes output events to it, forwards keystrokes (`onData`) and size changes (`onResize`), and
 * reports rendering progress back so the backend can pace very large outputs.
 *
 * Nothing starts at launch: sessions exist only after `open()` (the user opened a terminal).
 */
class TerminalStore {
  sessions = $state<TermSession[]>([]);
  activeId = $state<string | null>(null);
  opening = $state(false);
  error = $state<string | null>(null);
  /** Set by "Open terminal" buttons elsewhere (e.g. Linux Setup); consumed by the Terminal window. */
  launchRequest = $state<string | null>(null);

  readonly active = $derived(this.sessions.find((s) => s.info.id === this.activeId) ?? null);
  readonly liveCount = $derived(this.sessions.filter((s) => s.state.kind === 'running').length);

  private pending = new PendingOutput();
  private closed = new Set<string>();
  private lastRows = DEFAULT_ROWS;
  private lastCols = DEFAULT_COLS;
  private listening: Promise<void> | null = null;
  private unlisten: (() => void)[] = [];

  /** Open a session in `distro`, or in the distribution selected in VLINUX when `null`. */
  async open(distro: string | null = null): Promise<TermSession | null> {
    this.opening = true;
    this.error = null;
    try {
      await this.ensureListening();
      const x = await loadXterm();
      const term = new x.Terminal({
        rows: this.lastRows,
        cols: this.lastCols,
        fontFamily: cssVar('--font-mono', 'monospace'),
        fontSize: 13,
        lineHeight: 1.18,
        cursorBlink: true,
        scrollback: 5000,
        allowProposedApi: true, // required by the Unicode 11 addon
        theme: terminalTheme(),
      });
      const fit = new x.FitAddon();
      term.loadAddon(fit);
      term.loadAddon(new x.Unicode11Addon());
      term.unicode.activeVersion = '11';

      let info: SessionInfo;
      try {
        info = await ipc.terminalOpen(distro, term.rows, term.cols);
      } catch (e) {
        term.dispose();
        throw e;
      }

      const id = info.id;
      const acks = new AckBatcher((n) => void ipc.terminalAck(id, n).catch(() => {}));
      // Keystrokes (and the emulator's own replies, e.g. cursor position) go to the session's PTY.
      term.onData((d) => void ipc.terminalInput(id, d).catch(() => {}));
      term.onResize(({ rows, cols }) => {
        this.lastRows = rows;
        this.lastCols = cols;
        const s = this.sessions.find((x) => x.info.id === id);
        if (s) {
          s.rows = rows;
          s.cols = cols;
        }
        void ipc.terminalResize(id, rows, cols).catch(() => {});
      });
      term.attachCustomKeyEventHandler((e) => this.keyHandler(id, e));

      this.sessions.push({ info, state: info.state, term, fit, acks, rows: term.rows, cols: term.cols, attached: false });
      const reg = this.sessions[this.sessions.length - 1]!;
      const early = this.pending.take(id);
      for (const chunk of early.chunks) this.write(reg, chunk);
      if (early.state) this.applyState(reg, early.state);
      this.activeId = id;
      return reg;
    } catch (e) {
      this.error = String((e as Error)?.message ?? e);
      return null;
    } finally {
      this.opening = false;
    }
  }

  /** Close one tab and its session. Other sessions, WSL and the distribution keep running. */
  async close(id: string): Promise<void> {
    const i = this.sessions.findIndex((s) => s.info.id === id);
    if (i < 0) return;
    const s = this.sessions[i]!;
    this.closed.add(id);
    s.acks.dispose();
    s.term.dispose();
    this.sessions.splice(i, 1);
    if (this.activeId === id) {
      const next = this.sessions[Math.min(i, this.sessions.length - 1)];
      this.activeId = next ? next.info.id : null;
    }
    try {
      await ipc.terminalClose(id);
    } catch {
      /* the session was already gone */
    }
  }

  /** Start a fresh session in the same distribution as an ended one, replacing it. */
  async restart(id: string): Promise<void> {
    const old = this.sessions.find((s) => s.info.id === id);
    if (!old) return;
    const fresh = await this.open(old.info.distro);
    if (fresh) await this.close(id);
  }

  /** Closes every session (the Terminal window was closed). */
  closeAll() {
    const all = [...this.sessions];
    this.sessions.splice(0, this.sessions.length);
    this.activeId = null;
    for (const s of all) {
      this.closed.add(s.info.id);
      s.acks.dispose();
      s.term.dispose();
      void ipc.terminalClose(s.info.id).catch(() => {});
    }
  }

  activate(id: string) {
    if (this.sessions.some((s) => s.info.id === id)) this.activeId = id;
  }

  /** Clear the visible terminal and its scrollback (the shell's own prompt/state is untouched). */
  clear() {
    this.active?.term.clear();
    this.active?.term.focus();
  }

  focus() {
    this.active?.term.focus();
  }

  /** Re-apply the VLINUX theme (the accent colour may have changed). */
  restyle() {
    const theme = terminalTheme();
    for (const s of this.sessions) s.term.options.theme = theme;
  }

  /** Called by the Terminal window for Ctrl+Shift+T / Ctrl+Shift+W. */
  onShortcut: ((s: TerminalShortcut) => void) | null = null;

  private keyHandler(id: string, e: KeyboardEvent): boolean {
    // Desktop shortcuts must reach the desktop and must not be typed into the shell.
    if (isShellShortcut(e)) return false;
    const sc = terminalShortcut(e);
    if (!sc) return true;
    if (e.type === 'keydown') {
      if (sc === 'copy') {
        const sel = this.sessions.find((s) => s.info.id === id)?.term.getSelection();
        if (sel) void navigator.clipboard?.writeText(sel).catch(() => {});
      } else if (sc === 'new-tab' || sc === 'close-tab') {
        this.onShortcut?.(sc);
      }
      // 'paste' is left to the browser's paste event, which xterm.js turns into input.
    }
    return false;
  }

  private write(s: TermSession, bytes: Uint8Array) {
    s.term.write(bytes, () => s.acks.rendered(bytes.length));
  }

  private applyState(s: TermSession, state: SessionState) {
    if (s.state.kind !== 'running') return;
    s.state = state;
    s.info = { ...s.info, state };
    if (state.kind === 'exited') {
      const code = state.code === null ? '' : ` with exit code ${state.code}`;
      s.term.write(`\r\n\x1b[2m[VLINUX] The shell ended${code}. Use Restart, or close this tab.\x1b[0m\r\n`);
    } else if (state.kind === 'failed') {
      s.term.write(`\r\n\x1b[31m[VLINUX] ${state.message}\x1b[0m\r\n`);
    }
  }

  private ensureListening(): Promise<void> {
    if (!this.listening) {
      this.listening = (async () => {
        this.unlisten.push(
          await ipc.onTerminalOutput((e) => {
            const bytes = decodeBase64(e.data);
            const s = this.sessions.find((x) => x.info.id === e.id);
            if (s) this.write(s, bytes);
            else if (!this.closed.has(e.id)) this.pending.push(e.id, bytes);
          }),
        );
        this.unlisten.push(
          await ipc.onTerminalState((e) => {
            const s = this.sessions.find((x) => x.info.id === e.id);
            if (s) this.applyState(s, e.state);
            else if (!this.closed.has(e.id)) this.pending.state(e.id, e.state);
          }),
        );
      })();
    }
    return this.listening;
  }
}

export const terminals = new TerminalStore();
