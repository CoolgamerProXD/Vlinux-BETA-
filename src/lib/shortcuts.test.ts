import { describe, it, expect } from 'vitest';
import { desktopAction, SHORTCUTS, SHORTCUT_GROUPS } from './shortcuts';
import { isShellShortcut } from './terminal-types';

const k = (code: string, m: Partial<Record<'ctrlKey' | 'altKey' | 'shiftKey' | 'metaKey', boolean>> = {}) => ({
  code, ctrlKey: false, altKey: false, shiftKey: false, metaKey: false, ...m,
});
const alt = { altKey: true };
const altShift = { altKey: true, shiftKey: true };

describe('desktopAction', () => {
  it('keeps every existing shortcut', () => {
    expect(desktopAction(k('F1', alt))).toEqual({ type: 'launcher' });
    expect(desktopAction(k('KeyW', alt))).toEqual({ type: 'close-window' });
    expect(desktopAction(k('Digit3', alt))).toEqual({ type: 'workspace', n: 2 });
    expect(desktopAction(k('KeyQ', altShift))).toEqual({ type: 'power' });
    expect(desktopAction(k('KeyQ', { ctrlKey: true, altKey: true }))).toEqual({ type: 'return-to-windows' });
    expect(desktopAction(k('F11'))).toEqual({ type: 'fullscreen' });
  });
  it('accepts the arrows typed on a numpad with Shift held (code NumpadN, key ArrowX)', () => {
    expect(desktopAction({ ...k('Numpad4', altShift), key: 'ArrowLeft' })).toEqual({ type: 'snap', target: 'left' });
    expect(desktopAction({ ...k('Numpad8', altShift), key: 'ArrowUp' })).toEqual({ type: 'snap', target: 'maximize' });
    // a numpad digit that is really a digit is not an arrow
    expect(desktopAction({ ...k('Numpad4', altShift), key: '4' })).toBeNull();
  });
  it('maps the M7 window shortcuts', () => {
    expect(desktopAction(k('Backquote', alt))).toEqual({ type: 'cycle', dir: 1 });
    expect(desktopAction(k('Backquote', altShift))).toEqual({ type: 'cycle', dir: -1 });
    expect(desktopAction(k('ArrowLeft', altShift))).toEqual({ type: 'snap', target: 'left' });
    expect(desktopAction(k('ArrowRight', altShift))).toEqual({ type: 'snap', target: 'right' });
    expect(desktopAction(k('ArrowUp', altShift))).toEqual({ type: 'snap', target: 'maximize' });
    expect(desktopAction(k('ArrowDown', altShift))).toEqual({ type: 'lower' });
    expect(desktopAction(k('Digit5', altShift))).toEqual({ type: 'move-to-workspace', n: 4 });
    expect(desktopAction(k('F1', altShift))).toEqual({ type: 'shortcuts' });
    expect(desktopAction(k('KeyN', altShift))).toEqual({ type: 'notifications' });
  });
  it('leaves terminal and editor keys alone', () => {
    for (const code of ['KeyB', 'KeyF', 'KeyD', 'KeyN', 'KeyM', 'Slash', 'ArrowLeft', 'Tab', 'Period'] as const) {
      expect(desktopAction(k(code, alt)), `Alt+${code}`).toBeNull();
    }
    expect(desktopAction(k('KeyC', { ctrlKey: true }))).toBeNull();
    expect(desktopAction(k('Backquote', { ctrlKey: true, altKey: true }))).toBeNull(); // AltGr on some layouts
    expect(desktopAction(k('Digit1', { ctrlKey: true, altKey: true }))).toBeNull();
    expect(desktopAction(k('KeyW', { altKey: true, metaKey: true }))).toBeNull();
    expect(desktopAction(k('Backquote'))).toBeNull();
  });
  it('the terminal defers to exactly these shortcuts', () => {
    expect(isShellShortcut(k('Backquote', alt))).toBe(true);
    expect(isShellShortcut(k('ArrowLeft', altShift))).toBe(true);
    expect(isShellShortcut(k('ArrowLeft', alt))).toBe(false); // readline/tmux word movement
    expect(isShellShortcut(k('KeyB', alt))).toBe(false);
  });
});

describe('shortcut reference', () => {
  it('every group is used and every row has text', () => {
    for (const g of SHORTCUT_GROUPS) expect(SHORTCUTS.some((s) => s.group === g), g).toBe(true);
    for (const s of SHORTCUTS) expect(s.keys && s.label).toBeTruthy();
  });
  it('lists no duplicated key combination within a group', () => {
    const seen = new Set<string>();
    for (const s of SHORTCUTS) {
      const key = `${s.group}:${s.keys}`;
      expect(seen.has(key), key).toBe(false);
      seen.add(key);
    }
  });
});
