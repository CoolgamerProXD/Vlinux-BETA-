import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { notify, HISTORY_LIMIT, TOAST_LIMIT, MERGE_WINDOW_MS } from './notify.svelte';

describe('notify', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    notify.clear();
  });
  afterEach(() => vi.useRealTimers());

  it('keeps every message in the history and counts unread', () => {
    notify.error('Disk full', 'files');
    notify.success('Saved', 'settings');
    expect(notify.items.map((i) => i.text)).toEqual(['Disk full', 'Saved']);
    expect(notify.unread).toBe(2);
    expect(notify.unreadErrors).toBe(1);
    notify.markAllRead();
    expect(notify.unread).toBe(0);
  });

  it('toasts disappear but the history keeps them (errors are never lost)', () => {
    notify.error('Could not launch Foo', 'launcher');
    expect(notify.toasts).toHaveLength(1);
    vi.advanceTimersByTime(10_000);
    expect(notify.toasts).toHaveLength(0);
    expect(notify.items).toHaveLength(1);
    expect(notify.unreadErrors).toBe(1);
  });

  it('merges identical messages from the same source instead of flooding', () => {
    for (let i = 0; i < 5; i++) notify.error('Network share is slow', 'files');
    expect(notify.items).toHaveLength(1);
    expect(notify.items[0]!.count).toBe(5);
    expect(notify.toasts).toHaveLength(1);
    vi.advanceTimersByTime(MERGE_WINDOW_MS + 1);
    notify.error('Network share is slow', 'files');
    expect(notify.items).toHaveLength(2);
    // A different source is a different message.
    notify.error('Network share is slow', 'distro');
    expect(notify.items).toHaveLength(3);
  });

  it('shows at most a few toasts at once', () => {
    for (let i = 0; i < 6; i++) notify.info(`m${i}`);
    expect(notify.toasts).toHaveLength(TOAST_LIMIT);
    expect(notify.toasts.map((t) => t.text)).toEqual(['m3', 'm4', 'm5']);
  });

  it('bounds the history', () => {
    for (let i = 0; i < HISTORY_LIMIT + 20; i++) notify.push({ text: `n${i}`, ms: 0 });
    expect(notify.items).toHaveLength(HISTORY_LIMIT);
    expect(notify.items[0]!.text).toBe('n20');
    expect(notify.toasts).toHaveLength(0);
  });

  it('supports details, dismissal and history-only messages', () => {
    const id = notify.push({ kind: 'warning', source: 'distro', text: 'Start was slow', details: 'took 31 s', ms: 0 });
    expect(notify.toasts).toHaveLength(0);
    expect(notify.items[0]!.details).toBe('took 31 s');
    notify.remove(id);
    expect(notify.items).toHaveLength(0);
  });

  it('ui.toast still works for existing call sites', async () => {
    const { ui } = await import('./ui.svelte');
    ui.toast('Opened it', 'info');
    ui.toast('Broke', 'error');
    expect(notify.items.map((i) => i.kind)).toEqual(['info', 'error']);
  });
});
