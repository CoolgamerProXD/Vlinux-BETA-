import { describe, it, expect, beforeEach } from 'vitest';
import { WindowManager, MIN_W, MIN_H } from './wm.svelte';

const app = (id: string) => ({ id, name: id, size: { w: 700, h: 500 } });

describe('WindowManager', () => {
  let wm: WindowManager;
  beforeEach(() => {
    wm = new WindowManager();
    wm.setBounds(1200, 700);
  });

  it('opens a window on the current workspace and focuses it', () => {
    const id = wm.open(app('settings'));
    expect(wm.windows).toHaveLength(1);
    expect(wm.focusedId).toBe(id);
    expect(wm.windows[0]!.workspace).toBe(0);
  });

  it('is single-instance: reopening focuses the existing window', () => {
    const a = wm.open(app('settings'));
    wm.open(app('system'));
    const again = wm.open(app('settings'));
    expect(again).toBe(a);
    expect(wm.windows).toHaveLength(2);
    expect(wm.focusedId).toBe(a);
  });

  it('reopening an app from another workspace switches to it', () => {
    wm.open(app('settings'));
    wm.switchWorkspace(3, 5);
    expect(wm.visible).toHaveLength(0);
    wm.open(app('settings'));
    expect(wm.workspace).toBe(0);
    expect(wm.visible).toHaveLength(1);
  });

  it('close refocuses the top-most remaining window', () => {
    const a = wm.open(app('a'));
    const b = wm.open(app('b'));
    const c = wm.open(app('c'));
    wm.focus(a);
    wm.close(a);
    expect(wm.focusedId).toBe(c);
    wm.close(c);
    expect(wm.focusedId).toBe(b);
    wm.close(b);
    expect(wm.focusedId).toBeNull();
  });

  it('minimize hides and refocuses; dock activate toggles', () => {
    const a = wm.open(app('a'));
    const b = wm.open(app('b'));
    wm.minimize(b);
    expect(wm.visible.map((w) => w.id)).toEqual([a]);
    expect(wm.focusedId).toBe(a);
    wm.activate(app('b')); // restores
    expect(wm.focusedId).toBe(b);
    wm.activate(app('b')); // focused → minimise
    expect(wm.windows.find((w) => w.id === b)!.minimized).toBe(true);
  });

  it('workspaces isolate windows and ignore out-of-range switches', () => {
    wm.open(app('a'));
    wm.switchWorkspace(2, 5);
    wm.open(app('b'));
    expect(wm.visible.map((w) => w.appId)).toEqual(['b']);
    expect(wm.occupied(0)).toBe(true);
    expect(wm.occupied(1)).toBe(false);
    wm.switchWorkspace(9, 5);
    wm.switchWorkspace(-1, 5);
    expect(wm.workspace).toBe(2);
  });

  it('shrinking workspace count moves orphaned windows to the last workspace', () => {
    wm.switchWorkspace(4, 5);
    wm.open(app('a'));
    wm.limitWorkspaces(3);
    expect(wm.workspace).toBe(2);
    expect(wm.windows[0]!.workspace).toBe(2);
    expect(wm.visible).toHaveLength(1);
  });

  it('keeps windows reachable when dragged off-screen', () => {
    const id = wm.open(app('a'));
    wm.move(id, 99999, 99999);
    const w = wm.windows[0]!;
    expect(w.x).toBeLessThan(1200);
    expect(w.y).toBeLessThanOrEqual(700 - 44);
    wm.move(id, -99999, -50);
    expect(w.x + w.w).toBeGreaterThanOrEqual(100);
    expect(w.y).toBe(0);
  });

  it('enforces minimum size and bounds on resize', () => {
    const id = wm.open(app('a'));
    wm.resize(id, 10, 10);
    expect(wm.windows[0]!.w).toBe(MIN_W);
    expect(wm.windows[0]!.h).toBe(MIN_H);
    wm.resize(id, 99999, 99999);
    expect(wm.windows[0]!.x + wm.windows[0]!.w).toBeLessThanOrEqual(1200);
  });

  it('re-clamps windows when the display shrinks', () => {
    wm.open(app('a'));
    wm.setBounds(600, 400);
    const w = wm.windows[0]!;
    expect(w.w).toBeLessThanOrEqual(600);
    expect(w.h).toBeLessThanOrEqual(400);
  });

  it('maximized windows ignore move/resize', () => {
    const id = wm.open(app('a'));
    const { x, y } = wm.windows[0]!;
    wm.toggleMaximize(id);
    wm.move(id, 5, 5);
    expect([wm.windows[0]!.x, wm.windows[0]!.y]).toEqual([x, y]);
  });
});
