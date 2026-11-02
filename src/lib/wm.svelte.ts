import type { AppDef } from './apps';

export interface Rect { x: number; y: number; w: number; h: number }

export interface WinState extends Rect {
  id: number;
  appId: string;
  title: string;
  workspace: number;
  z: number;
  maximized: boolean;
  minimized: boolean;
}

export const MIN_W = 380;
export const MIN_H = 260;
const GRAB = 120; // px of a window that must stay reachable on screen

/**
 * Minimal floating window manager for the VLINUX shell.
 *
 * This manages *shell* windows (Settings, System, and later Terminal/Files). It is not
 * a Wayland compositor and does not manage Linux GUI windows.
 */
export class WindowManager {
  windows = $state<WinState[]>([]);
  workspace = $state(0);
  focusedId = $state<number | null>(null);
  bounds = $state({ w: 1280, h: 720 });
  /**
   * Apps may veto an immediate close by returning a message (e.g. a terminal with running
   * shells). The shell then asks the user; `closeNow` closes without asking.
   */
  guards: Record<string, () => string | null> = {};
  pendingClose = $state<{ id: number; title: string; message: string } | null>(null);
  private nextId = 1;
  private zTop = 1;
  private cascade = 0;

  get visible(): WinState[] {
    return this.windows.filter((w) => w.workspace === this.workspace && !w.minimized);
  }

  occupied(ws: number): boolean {
    return this.windows.some((w) => w.workspace === ws);
  }

  byApp(appId: string): WinState | undefined {
    return this.windows.find((w) => w.appId === appId);
  }

  setBounds(w: number, h: number) {
    this.bounds = { w, h };
    for (const win of this.windows) this.clamp(win);
  }

  /** Open an app window, or focus it if it is already open (all apps are single-instance). */
  open(app: Pick<AppDef, 'id' | 'name' | 'size'>): number {
    const existing = this.byApp(app.id);
    if (existing) {
      existing.minimized = false;
      this.workspace = existing.workspace;
      this.focus(existing.id);
      return existing.id;
    }
    const w = Math.max(MIN_W, Math.min(app.size.w, Math.floor(this.bounds.w * 0.92)));
    const h = Math.max(MIN_H, Math.min(app.size.h, Math.floor(this.bounds.h * 0.92)));
    const off = (this.cascade++ % 6) * 28;
    const win: WinState = {
      id: this.nextId++,
      appId: app.id,
      title: app.name,
      workspace: this.workspace,
      x: Math.max(0, Math.round((this.bounds.w - w) / 2) - 60 + off),
      y: Math.max(0, Math.round((this.bounds.h - h) / 2) - 40 + off),
      w,
      h,
      z: ++this.zTop,
      maximized: false,
      minimized: false,
    };
    this.windows.push(win);
    this.clamp(win);
    this.focusedId = win.id;
    return win.id;
  }

  private get(id: number) {
    return this.windows.find((w) => w.id === id);
  }

  focus(id: number) {
    const w = this.get(id);
    if (!w) return;
    w.minimized = false;
    w.z = ++this.zTop;
    this.focusedId = id;
  }

  /** Focus the top-most visible window of the current workspace (or nothing). */
  private refocus() {
    const top = [...this.visible].sort((a, b) => b.z - a.z)[0];
    this.focusedId = top ? top.id : null;
  }

  /** Close without asking (used after the user confirmed, and by apps that have no guard). */
  closeNow(id: number) {
    this.windows = this.windows.filter((w) => w.id !== id);
    if (this.pendingClose?.id === id) this.pendingClose = null;
    if (this.focusedId === id) this.refocus();
  }

  /** User-initiated close: asks first when the app registered a guard that objects. */
  close(id: number) {
    const w = this.get(id);
    const message = w ? this.guards[w.appId]?.() ?? null : null;
    if (w && message) {
      this.pendingClose = { id, title: w.title, message };
      return;
    }
    this.closeNow(id);
  }

  closeFocused() {
    if (this.focusedId !== null) this.close(this.focusedId);
  }

  minimize(id: number) {
    const w = this.get(id);
    if (!w) return;
    w.minimized = true;
    if (this.focusedId === id) this.refocus();
  }

  /** Dock behaviour: focused → minimise, otherwise bring to front (switching workspace if needed). */
  activate(app: Pick<AppDef, 'id' | 'name' | 'size'>) {
    const w = this.byApp(app.id);
    if (w && !w.minimized && w.workspace === this.workspace && this.focusedId === w.id) this.minimize(w.id);
    else this.open(app);
  }

  toggleMaximize(id: number) {
    const w = this.get(id);
    if (w) w.maximized = !w.maximized;
  }

  move(id: number, x: number, y: number) {
    const w = this.get(id);
    if (!w || w.maximized) return;
    w.x = x;
    w.y = y;
    this.clamp(w);
  }

  resize(id: number, width: number, height: number) {
    const w = this.get(id);
    if (!w || w.maximized) return;
    w.w = Math.max(MIN_W, Math.min(width, this.bounds.w - w.x));
    w.h = Math.max(MIN_H, Math.min(height, this.bounds.h - w.y));
  }

  switchWorkspace(n: number, count: number) {
    if (n < 0 || n >= count) return;
    this.workspace = n;
    this.refocus();
  }

  /** Called when the configured workspace count shrinks: orphaned windows move to the last workspace. */
  limitWorkspaces(count: number) {
    for (const w of this.windows) if (w.workspace >= count) w.workspace = count - 1;
    if (this.workspace >= count) this.workspace = count - 1;
    this.refocus();
  }

  private clamp(w: WinState) {
    const { w: bw, h: bh } = this.bounds;
    w.w = Math.max(MIN_W, Math.min(w.w, Math.max(MIN_W, bw)));
    w.h = Math.max(MIN_H, Math.min(w.h, Math.max(MIN_H, bh)));
    w.x = Math.min(Math.max(w.x, GRAB - w.w), Math.max(0, bw - GRAB));
    w.y = Math.min(Math.max(w.y, 0), Math.max(0, bh - 44));
  }
}

export const wm = new WindowManager();
