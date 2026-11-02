import { describe, expect, it, vi } from 'vitest';
import { AckBatcher, decodeBase64, PendingOutput } from './terminal-types';

describe('decodeBase64', () => {
  it('decodes bytes, including partial UTF-8 sequences', () => {
    expect(Array.from(decodeBase64('Zm9v'))).toEqual([102, 111, 111]);
    expect(Array.from(decodeBase64('4pw='))).toEqual([0xe2, 0x9c]);
    expect(Array.from(decodeBase64('AP/+AQ=='))).toEqual([0, 255, 254, 1]);
    expect(decodeBase64('').length).toBe(0);
  });
});

describe('PendingOutput', () => {
  it('keeps output for a not-yet-registered session and hands it over once, in order', () => {
    const p = new PendingOutput();
    p.push('t1', new Uint8Array([1]));
    p.push('t1', new Uint8Array([2, 3]));
    p.state('t1', { kind: 'exited', code: 0 });
    const got = p.take('t1');
    expect(got.chunks.map((c) => Array.from(c))).toEqual([[1], [2, 3]]);
    expect(got.state).toEqual({ kind: 'exited', code: 0 });
    expect(p.take('t1').chunks).toEqual([]);
  });

  it('is bounded per session and in the number of unknown sessions', () => {
    const p = new PendingOutput(10, 2);
    p.push('a', new Uint8Array(8));
    p.push('a', new Uint8Array(8)); // would exceed 10 bytes: dropped
    expect(p.take('a').chunks.length).toBe(1);
    p.push('a', new Uint8Array(1));
    p.push('b', new Uint8Array(1));
    p.push('c', new Uint8Array(1)); // evicts the oldest id
    expect(p.size).toBe(2);
    expect(p.take('a').chunks).toEqual([]);
  });
});

describe('AckBatcher', () => {
  it('acknowledges large amounts immediately and small remainders after a short delay', () => {
    vi.useFakeTimers();
    const send = vi.fn();
    const a = new AckBatcher(send, 100, 40);
    a.rendered(60);
    expect(send).not.toHaveBeenCalled();
    a.rendered(60); // 120 >= 100
    expect(send).toHaveBeenCalledWith(120);
    a.rendered(5);
    expect(send).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(50);
    expect(send).toHaveBeenLastCalledWith(5);
    // idle: no timer is left running
    expect(vi.getTimerCount()).toBe(0);
    a.dispose();
    vi.useRealTimers();
  });
});

import { isShellShortcut, terminalShortcut } from './terminal-types';

const key = (code: string, m: Partial<Record<'ctrlKey' | 'altKey' | 'shiftKey' | 'metaKey', boolean>> = {}) => ({
  code, ctrlKey: false, altKey: false, shiftKey: false, metaKey: false, ...m,
});

describe('keyboard routing', () => {
  it('leaves desktop shortcuts to the desktop', () => {
    expect(isShellShortcut(key('F1', { altKey: true }))).toBe(true);
    expect(isShellShortcut(key('KeyW', { altKey: true }))).toBe(true);
    expect(isShellShortcut(key('Digit3', { altKey: true }))).toBe(true);
    expect(isShellShortcut(key('KeyQ', { altKey: true, shiftKey: true }))).toBe(true);
    expect(isShellShortcut(key('KeyQ', { ctrlKey: true, altKey: true }))).toBe(true);
    expect(isShellShortcut(key('F11'))).toBe(true);
  });
  it('does not steal ordinary terminal keys', () => {
    expect(isShellShortcut(key('KeyC', { ctrlKey: true }))).toBe(false); // Ctrl+C is SIGINT
    expect(isShellShortcut(key('KeyW', { ctrlKey: true }))).toBe(false); // readline: delete word
    expect(isShellShortcut(key('KeyL', { ctrlKey: true }))).toBe(false);
    expect(isShellShortcut(key('KeyD', { ctrlKey: true }))).toBe(false);
    expect(isShellShortcut(key('ArrowUp'))).toBe(false);
    expect(isShellShortcut(key('Tab'))).toBe(false);
    expect(isShellShortcut(key('KeyB', { altKey: true }))).toBe(false); // readline: back a word
  });
  it('maps the terminal window shortcuts', () => {
    const cs = { ctrlKey: true, shiftKey: true };
    expect(terminalShortcut(key('KeyT', cs))).toBe('new-tab');
    expect(terminalShortcut(key('KeyW', cs))).toBe('close-tab');
    expect(terminalShortcut(key('KeyC', cs))).toBe('copy');
    expect(terminalShortcut(key('KeyV', cs))).toBe('paste');
    expect(terminalShortcut(key('KeyC', { ctrlKey: true }))).toBe(null);
  });
});
