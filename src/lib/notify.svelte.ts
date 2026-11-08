// Notification system: one place for every message the shell shows.
//
// * Transient toasts for what just happened, plus a history (the notification centre) so that a
//   message is never lost just because the user looked away. Errors stay in the history, unread,
//   until the user has seen them.
// * Identical messages from the same source within a few seconds are merged (a counter), so a
//   failing operation cannot flood the screen.
// * Nothing here reports success on its own: callers say what really happened.

export type NoticeKind = 'info' | 'success' | 'warning' | 'error';

export interface Notice {
  id: number;
  kind: NoticeKind;
  /** Which part of the shell raised it (`files`, `distro`, `launcher`, `settings`, `shell`…). */
  source: string;
  text: string;
  /** Longer technical detail, shown on demand in the notification centre. */
  details?: string;
  at: number;
  /** How many identical messages were merged into this one. */
  count: number;
  read: boolean;
}

export interface PushOptions {
  kind?: NoticeKind;
  source?: string;
  text: string;
  details?: string;
  /** How long the toast stays; 0 = no toast (history only). */
  ms?: number;
}

export const HISTORY_LIMIT = 100;
export const TOAST_LIMIT = 3;
export const MERGE_WINDOW_MS = 5000;

const DEFAULT_MS: Record<NoticeKind, number> = { info: 4000, success: 4000, warning: 7000, error: 9000 };

class Notify {
  /** History, oldest first. */
  items = $state<Notice[]>([]);
  /** Ids currently shown as toasts. */
  toastIds = $state<number[]>([]);
  private n = 0;
  private timers = new Map<number, ReturnType<typeof setTimeout>>();

  get toasts(): Notice[] {
    return this.toastIds.map((id) => this.items.find((i) => i.id === id)).filter((x): x is Notice => !!x);
  }

  get unread(): number {
    return this.items.reduce((n, i) => n + (i.read ? 0 : 1), 0);
  }

  get unreadErrors(): number {
    return this.items.reduce((n, i) => n + (!i.read && i.kind === 'error' ? 1 : 0), 0);
  }

  push(o: PushOptions): number {
    const kind = o.kind ?? 'info';
    const source = o.source ?? 'shell';
    const now = Date.now();
    const same = [...this.items]
      .reverse()
      .find((i) => i.kind === kind && i.source === source && i.text === o.text && now - i.at < MERGE_WINDOW_MS);
    let id: number;
    if (same) {
      same.count += 1;
      same.at = now;
      same.read = false;
      if (o.details) same.details = o.details;
      id = same.id;
    } else {
      id = ++this.n;
      this.items.push({ id, kind, source, text: o.text, details: o.details, at: now, count: 1, read: false });
      if (this.items.length > HISTORY_LIMIT) {
        const drop = this.items.length - HISTORY_LIMIT;
        const gone = this.items.splice(0, drop);
        for (const g of gone) this.hideToast(g.id);
      }
    }
    const ms = o.ms ?? DEFAULT_MS[kind];
    if (ms > 0) this.showToast(id, ms);
    return id;
  }

  private showToast(id: number, ms: number) {
    const ids = this.toastIds.filter((x) => x !== id);
    ids.push(id);
    const overflow = ids.length - TOAST_LIMIT;
    if (overflow > 0) {
      for (const dropped of ids.splice(0, overflow)) this.clearTimer(dropped);
    }
    this.toastIds = ids;
    this.clearTimer(id);
    this.timers.set(id, setTimeout(() => this.hideToast(id), ms));
  }

  private clearTimer(id: number) {
    const t = this.timers.get(id);
    if (t !== undefined) clearTimeout(t);
    this.timers.delete(id);
  }

  hideToast(id: number) {
    this.clearTimer(id);
    if (this.toastIds.includes(id)) this.toastIds = this.toastIds.filter((x) => x !== id);
  }

  /** The user has seen the history (the centre was opened). */
  markAllRead() {
    for (const i of this.items) i.read = true;
  }

  remove(id: number) {
    this.hideToast(id);
    this.items = this.items.filter((i) => i.id !== id);
  }

  clear() {
    for (const id of [...this.timers.keys()]) this.clearTimer(id);
    this.toastIds = [];
    this.items = [];
  }

  // Convenience wrappers: say what happened, in the right tone.
  info(text: string, source?: string) {
    return this.push({ kind: 'info', source, text });
  }
  success(text: string, source?: string) {
    return this.push({ kind: 'success', source, text });
  }
  warning(text: string, source?: string, details?: string) {
    return this.push({ kind: 'warning', source, text, details });
  }
  error(text: string, source?: string, details?: string) {
    return this.push({ kind: 'error', source, text, details });
  }
}

export const notify = new Notify();
