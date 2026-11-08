// A single context menu for the whole shell (desktop, windows, dock, window list).

export interface MenuItem {
  label?: string;
  hint?: string;
  action?: () => void;
  disabled?: boolean;
  danger?: boolean;
  checked?: boolean;
  separator?: boolean;
}

class ContextMenuState {
  menu = $state<{ x: number; y: number; items: MenuItem[]; label: string } | null>(null);

  open(x: number, y: number, items: MenuItem[], label = 'Context menu') {
    const clean = items.filter((it, i, a) => !(it.separator && (i === 0 || i === a.length - 1 || a[i - 1]?.separator)));
    this.menu = clean.length ? { x, y, items: clean, label } : null;
  }

  /** Open from a pointer or keyboard (Shift+F10 / Menu key) event, anchored sensibly. */
  openFrom(e: MouseEvent | KeyboardEvent, items: MenuItem[], label?: string) {
    e.preventDefault();
    e.stopPropagation();
    if ('clientX' in e && (e.clientX || e.clientY)) {
      this.open(e.clientX, e.clientY, items, label);
      return;
    }
    const r = (e.currentTarget as HTMLElement | null)?.getBoundingClientRect();
    this.open(r ? r.left + 8 : 24, r ? r.bottom : 24, items, label);
  }

  close() {
    this.menu = null;
  }
}

export const ctx = new ContextMenuState();
