export type Overlay = 'launcher' | 'power' | 'status' | null;

export interface Toast {
  id: number;
  kind: 'info' | 'error';
  text: string;
}

/** Transient shell UI state: which overlay is open and toast messages. */
class UI {
  overlay = $state<Overlay>(null);
  toasts = $state<Toast[]>([]);
  private n = 0;

  open(o: Exclude<Overlay, null>) {
    this.overlay = o;
  }
  toggle(o: Exclude<Overlay, null>) {
    this.overlay = this.overlay === o ? null : o;
  }
  close() {
    this.overlay = null;
  }

  toast(text: string, kind: Toast['kind'] = 'info', ms = 4500) {
    const id = ++this.n;
    this.toasts = [...this.toasts.slice(-3), { id, kind, text }];
    setTimeout(() => (this.toasts = this.toasts.filter((t) => t.id !== id)), ms);
  }
}

export const ui = new UI();
