import type { Accent, Config } from './types';

export const ACCENT_COLORS: Record<Accent, { main: string; rgb: string; second: string }> = {
  violet: { main: '#a98bff', rgb: '169 139 255', second: '#5ee0ff' },
  cyan: { main: '#4fd8ff', rgb: '79 216 255', second: '#7a8bff' },
  rose: { main: '#ff7aa8', rgb: '255 122 168', second: '#ffb36b' },
  amber: { main: '#ffb454', rgb: '255 180 84', second: '#ff7a6b' },
  mint: { main: '#5ff0b0', rgb: '95 240 176', second: '#5ec8ff' },
};

/** Map configuration onto CSS custom properties on <html>. */
export function applyTheme(c: Config, root: HTMLElement = document.documentElement) {
  const a = ACCENT_COLORS[c.appearance.accent];
  const g = c.appearance.glass;
  root.style.setProperty('--accent', a.main);
  root.style.setProperty('--accent-rgb', a.rgb);
  root.style.setProperty('--accent-2', a.second);
  root.style.setProperty('--glass-a', (0.94 - g * 0.0066).toFixed(3));
  root.style.setProperty('--glass-blur', `${Math.round(6 + g * 0.24)}px`);
  root.classList.toggle('no-anim', !c.appearance.animations);
}
