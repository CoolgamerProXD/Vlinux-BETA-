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

describe('WindowManager — M7 window management', () => {
  let wm: WindowManager;
  beforeEach(() => {
    wm = new WindowManager();
    wm.setBounds(1200, 700);
  });

  it('snaps to the left/right half and restores the earlier geometry', () => {
    const id = wm.open(app('a'));
    const before = { ...wm.windows[0]! };
    wm.snap(id, 'left');
    expect(wm.windows[0]).toMatchObject({ x: 0, y: 0, w: 600, h: 700, snapped: 'left' });
    wm.snap(id, 'right');
    expect(wm.windows[0]).toMatchObject({ x: 600, w: 600, snapped: 'right' });
    wm.snap(id, 'restore');
    expect(wm.windows[0]).toMatchObject({ x: before.x, y: before.y, w: before.w, h: before.h, snapped: null });
  });

  it('dragging or resizing a snapped window releases the snap and brings back its size', () => {
    const id = wm.open(app('a'));
    const w0 = wm.windows[0]!.w;
    wm.snap(id, 'left');
    wm.move(id, 200, 100);
    expect(wm.windows[0]).toMatchObject({ snapped: null, w: w0 });
    wm.snap(id, 'right');
    wm.resize(id, 500, 400);
    expect(wm.windows[0]!.snapped).toBeNull();
  });

  it('keeps snapped windows tiled when the desktop is resized', () => {
    const id = wm.open(app('a'));
    wm.snap(id, 'right');
    wm.setBounds(1000, 600);
    expect(wm.windows[0]).toMatchObject({ x: 500, w: 500, h: 600 });
  });

  it('never makes a snapped window smaller than the minimum', () => {
    wm.setBounds(500, 400);
    const id = wm.open(app('a'));
    wm.snap(id, 'left');
    expect(wm.windows[0]!.w).toBeGreaterThanOrEqual(MIN_W);
    expect(wm.windows[0]!.h).toBeGreaterThanOrEqual(MIN_H);
  });

  it('maximise/restore round-trips and snapped windows restore from toggleMaximize', () => {
    const id = wm.open(app('a'));
    wm.snap(id, 'maximize');
    expect(wm.windows[0]!.maximized).toBe(true);
    wm.snap(id, 'restore');
    expect(wm.windows[0]!.maximized).toBe(false);
    wm.snap(id, 'left');
    wm.toggleMaximize(id);
    expect(wm.windows[0]!.snapped).toBeNull();
  });

  it('lower: undoes a snap, otherwise minimises', () => {
    const id = wm.open(app('a'));
    wm.snap(id, 'left');
    wm.lower(id);
    expect(wm.windows[0]).toMatchObject({ snapped: null, minimized: false });
    wm.lower(id);
    expect(wm.windows[0]!.minimized).toBe(true);
  });

  it('moves a window to another workspace without following it', () => {
    const a = wm.open(app('a'));
    const b = wm.open(app('b'));
    wm.moveToWorkspace(b, 2, 5);
    expect(wm.workspace).toBe(0);
    expect(wm.windows.find((w) => w.id === b)!.workspace).toBe(2);
    expect(wm.focusedId).toBe(a);
    expect(wm.occupied(2)).toBe(true);
    wm.moveToWorkspace(a, 7, 5); // out of range: ignored
    expect(wm.windows.find((w) => w.id === a)!.workspace).toBe(0);
  });

  it('cycles windows most-recently-used first and commits on release', () => {
    const a = wm.open(app('a'));
    const b = wm.open(app('b'));
    const c = wm.open(app('c'));
    expect(wm.focusedId).toBe(c);
    wm.cycleStep(); // opens, picks the previous window
    expect(wm.switcher?.order).toEqual([c, b, a]);
    expect(wm.switcher?.index).toBe(1);
    wm.cycleStep();
    expect(wm.switcher?.index).toBe(2);
    wm.cycleCommit();
    expect(wm.switcher).toBeNull();
    expect(wm.focusedId).toBe(a);
    // The most recent two alternate.
    wm.cycleStep();
    wm.cycleCommit();
    expect(wm.focusedId).toBe(c);
  });

  it('cycling backwards wraps, cancel changes nothing, one window cannot cycle', () => {
    const a = wm.open(app('a'));
    wm.cycleStep();
    expect(wm.switcher).toBeNull();
    const b = wm.open(app('b'));
    wm.cycleStep(-1);
    expect(wm.switcher?.index).toBe(1);
    wm.cycleCancel();
    expect(wm.focusedId).toBe(b);
    expect(a).not.toBe(b);
  });

  it('cycling only covers the current workspace and restores minimised windows', () => {
    const a = wm.open(app('a'));
    const b = wm.open(app('b'));
    wm.moveToWorkspace(b, 1, 5);
    wm.open(app('c'));
    wm.minimize(a);
    wm.cycleStep();
    expect(wm.switcher?.order).not.toContain(b);
    expect(wm.switcher?.order).toContain(a);
    wm.cycleCommit();
    expect(wm.windows.find((w) => w.id === a)!.minimized).toBe(false);
    expect(wm.focusedId).toBe(a);
  });

  it('reveal switches workspace and restores', () => {
    const a = wm.open(app('a'));
    wm.moveToWorkspace(a, 3, 5);
    wm.minimize(a);
    wm.reveal(a);
    expect(wm.workspace).toBe(3);
    expect(wm.windows[0]!.minimized).toBe(false);
    expect(wm.focusedId).toBe(a);
  });
});
