import { notify } from './notify.svelte';

export type Overlay = 'launcher' | 'power' | 'status' | 'notifications' | 'shortcuts' | 'distro' | null;

/** Transient shell UI state: which overlay is open. Messages live in `notify`. */
class UI {
  overlay = $state<Overlay>(null);

  open(o: Exclude<Overlay, null>) {
    this.overlay = o;
  }
  toggle(o: Exclude<Overlay, null>) {
    this.overlay = this.overlay === o ? null : o;
  }
  close() {
    this.overlay = null;
  }

  /** Kept for existing call sites; every message also lands in the notification history. */
  toast(text: string, kind: 'info' | 'error' = 'info', ms?: number, source = 'shell') {
    notify.push({ kind, source, text, ms });
  }
}

export const ui = new UI();
