// Context-menu contents. Every entry calls an existing, real action; nothing here is decorative.

import { appById, type AppDef } from './apps';
import { config } from './config.svelte';
import type { MenuItem } from './ctx.svelte';
import { ui } from './ui.svelte';
import { WALLPAPERS } from './types';
import { wm, type WinState } from './wm.svelte';

export function windowMenu(w: WinState, workspaceCount: number): MenuItem[] {
  const items: MenuItem[] = [
    { label: w.minimized ? 'Restore' : 'Minimise', action: () => (w.minimized ? wm.reveal(w.id) : wm.minimize(w.id)) },
    { label: w.maximized || w.snapped ? 'Restore size' : 'Maximise', hint: 'Alt+Shift+↑', action: () => (w.maximized || w.snapped ? wm.snap(w.id, 'restore') : wm.snap(w.id, 'maximize')) },
    { label: 'Tile left', hint: 'Alt+Shift+←', action: () => wm.snap(w.id, 'left') },
    { label: 'Tile right', hint: 'Alt+Shift+→', action: () => wm.snap(w.id, 'right') },
    { separator: true },
  ];
  for (let i = 0; i < workspaceCount; i++) {
    if (i === w.workspace) continue;
    items.push({ label: `Move to workspace ${i + 1}`, hint: i < 9 ? `Alt+Shift+${i + 1}` : undefined, action: () => wm.moveToWorkspace(w.id, i, workspaceCount) });
  }
  items.push({ separator: true }, { label: 'Close', hint: 'Alt+W', danger: true, action: () => wm.close(w.id) });
  return items;
}

export function dockMenu(app: AppDef): MenuItem[] {
  const win = wm.byApp(app.id);
  const items: MenuItem[] = [{ label: win ? 'Show' : 'Open', action: () => wm.open(app) }];
  if (win) {
    items.push({ label: win.minimized ? 'Restore' : 'Minimise', disabled: false, action: () => (win.minimized ? wm.reveal(win.id) : wm.minimize(win.id)) });
    items.push({ separator: true }, { label: 'Close', danger: true, action: () => wm.close(win.id) });
  }
  return items;
}

export function desktopMenu(): MenuItem[] {
  const open = (id: string) => () => {
    const a = appById(id);
    if (a) wm.open(a);
  };
  return [
    { label: 'Applications…', hint: 'Alt+F1', action: () => ui.open('launcher') },
    { separator: true },
    { label: 'Open Terminal', action: open('terminal') },
    { label: 'Open Files', action: open('files') },
    { label: 'Settings', action: open('settings') },
    { separator: true },
    {
      label: 'Next wallpaper',
      action: () =>
        config.update((c) => {
          const i = WALLPAPERS.indexOf(c.appearance.wallpaper);
          c.appearance.wallpaper = WALLPAPERS[(i + 1) % WALLPAPERS.length]!;
        }),
    },
    { label: 'Notifications', hint: 'Alt+Shift+N', action: () => ui.open('notifications') },
    { label: 'Keyboard shortcuts', hint: 'Alt+Shift+F1', action: () => ui.open('shortcuts') },
  ];
}

