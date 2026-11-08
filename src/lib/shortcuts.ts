// The desktop's keyboard shortcuts: one table for behaviour (`desktopAction`), one for display
// (`SHORTCUTS`, used by the shortcut overlay and Settings). Alt+Tab is captured by Windows itself
// and can never reach the app, so the window switcher is Alt+` instead.
//
// Keys that readline, tmux or vim use inside a terminal (Alt+B/F/D/N/M, Alt+/, Ctrl+anything) are
// deliberately not taken. When a terminal has focus, these shortcuts still reach the desktop
// (`isShellShortcut` is what the terminal consults).

export type DesktopAction =
  | { type: 'return-to-windows' }
  | { type: 'fullscreen' }
  | { type: 'launcher' }
  | { type: 'shortcuts' }
  | { type: 'power' }
  | { type: 'notifications' }
  | { type: 'close-window' }
  | { type: 'workspace'; n: number }
  | { type: 'move-to-workspace'; n: number }
  | { type: 'snap'; target: 'left' | 'right' | 'maximize' }
  | { type: 'lower' }
  | { type: 'cycle'; dir: 1 | -1 };

type Keys = { ctrlKey: boolean; altKey: boolean; shiftKey: boolean; metaKey: boolean; code: string };

const DIGIT = /^Digit([1-9])$/;

export function desktopAction(e: Keys): DesktopAction | null {
  if (e.ctrlKey && e.altKey && !e.shiftKey && !e.metaKey && e.code === 'KeyQ') return { type: 'return-to-windows' };
  if (e.code === 'F11' && !e.altKey && !e.ctrlKey) return { type: 'fullscreen' };
  if (!e.altKey || e.ctrlKey || e.metaKey) return null;
  if (e.code === 'F1') return e.shiftKey ? { type: 'shortcuts' } : { type: 'launcher' };
  if (e.code === 'Backquote') return { type: 'cycle', dir: e.shiftKey ? -1 : 1 };
  const digit = DIGIT.exec(e.code);
  if (e.shiftKey) {
    if (e.code === 'KeyQ') return { type: 'power' };
    if (e.code === 'KeyN') return { type: 'notifications' };
    if (e.code === 'ArrowLeft') return { type: 'snap', target: 'left' };
    if (e.code === 'ArrowRight') return { type: 'snap', target: 'right' };
    if (e.code === 'ArrowUp') return { type: 'snap', target: 'maximize' };
    if (e.code === 'ArrowDown') return { type: 'lower' };
    if (digit) return { type: 'move-to-workspace', n: Number(digit[1]) - 1 };
    return null;
  }
  if (e.code === 'KeyW') return { type: 'close-window' };
  if (digit) return { type: 'workspace', n: Number(digit[1]) - 1 };
  return null;
}

export interface Shortcut {
  keys: string;
  label: string;
  group: 'Desktop' | 'Windows' | 'Workspaces' | 'Terminal' | 'Files' | 'Launcher';
}

export const SHORTCUTS: Shortcut[] = [
  { group: 'Desktop', keys: 'Alt + F1', label: 'Open / close the application launcher' },
  { group: 'Desktop', keys: 'Alt + Shift + F1', label: 'Show this shortcut list' },
  { group: 'Desktop', keys: 'Alt + Shift + N', label: 'Open the notification centre' },
  { group: 'Desktop', keys: 'F11', label: 'Toggle full screen' },
  { group: 'Desktop', keys: 'Alt + Shift + Q', label: 'Open the power / exit menu' },
  { group: 'Desktop', keys: 'Ctrl + Alt + Q', label: 'Return to Windows immediately (nothing is closed)' },
  { group: 'Desktop', keys: 'Esc', label: 'Close the open menu or panel' },
  { group: 'Windows', keys: 'Alt + `', label: 'Switch window (hold Alt, press ` again to go on; add Shift to go back)' },
  { group: 'Windows', keys: 'Alt + W', label: 'Close the focused window' },
  { group: 'Windows', keys: 'Alt + Shift + ← / →', label: 'Tile the focused window to the left / right half' },
  { group: 'Windows', keys: 'Alt + Shift + ↑', label: 'Maximise the focused window' },
  { group: 'Windows', keys: 'Alt + Shift + ↓', label: 'Restore the window, or minimise it if it is not tiled' },
  { group: 'Workspaces', keys: 'Alt + 1 … 9', label: 'Switch workspace' },
  { group: 'Workspaces', keys: 'Alt + Shift + 1 … 9', label: 'Send the focused window to a workspace' },
  { group: 'Launcher', keys: '↑ / ↓ · Enter', label: 'Choose and open an application' },
  { group: 'Terminal', keys: 'Ctrl + Shift + T / W', label: 'New tab / close tab' },
  { group: 'Terminal', keys: 'Ctrl + Shift + C / V', label: "Copy selection / paste (Ctrl + C stays the shell's interrupt)" },
  { group: 'Files', keys: 'Ctrl + L', label: 'Edit the path' },
  { group: 'Files', keys: 'F2', label: 'Rename' },
  { group: 'Files', keys: 'Delete', label: 'Delete (always asks first)' },
  { group: 'Files', keys: 'Ctrl + C / X / V', label: 'Copy / cut / paste' },
  { group: 'Files', keys: 'Ctrl + A', label: 'Select all' },
];

export const SHORTCUT_GROUPS: Shortcut['group'][] = ['Desktop', 'Windows', 'Workspaces', 'Launcher', 'Terminal', 'Files'];
