import type { Accent, Config, Theme } from './types';

export const ACCENT_COLORS: Record<Accent, { main: string; rgb: string; second: string }> = {
  violet: { main: '#a98bff', rgb: '169 139 255', second: '#5ee0ff' },
  cyan: { main: '#4fd8ff', rgb: '79 216 255', second: '#7a8bff' },
  rose: { main: '#ff7aa8', rgb: '255 122 168', second: '#ffb36b' },
  amber: { main: '#ffb454', rgb: '255 180 84', second: '#ff7a6b' },
  mint: { main: '#5ff0b0', rgb: '95 240 176', second: '#5ec8ff' },
};

/** Surface colours per theme (CSS custom properties; applied to <html>). */
export const THEME_VARS: Record<Theme, Record<string, string>> = {
  midnight: { '--ink': '#08080f', '--panel-rgb': '16 16 28', '--panel2-rgb': '20 20 34', '--win-rgb': '14 14 24', '--text': '#ecebf7', '--muted': '#9b9bb8', '--faint': '#82829e', '--wall-dim': '0', '--term-bg': '#0b0b14' },
  graphite: { '--ink': '#0c0d10', '--panel-rgb': '24 25 30', '--panel2-rgb': '28 29 35', '--win-rgb': '20 21 26', '--text': '#e9eaee', '--muted': '#a0a2ad', '--faint': '#868894', '--wall-dim': '0.3', '--term-bg': '#0f1013' },
  oled: { '--ink': '#000000', '--panel-rgb': '6 6 8', '--panel2-rgb': '10 10 13', '--win-rgb': '3 3 5', '--text': '#f4f4f8', '--muted': '#a6a6b8', '--faint': '#8a8aa0', '--wall-dim': '0.82', '--term-bg': '#000000' },
};

/** Opacity of panels. Without blur, glass would be muddy, so panels are made mostly solid. */
export function glassAlpha(glass: number, blur: boolean): number {
  const a = 0.94 - glass * 0.0066;
  return blur ? a : Math.max(a, 0.86);
}

/** Map configuration onto CSS custom properties on <html>. */
export function applyTheme(c: Config, root: HTMLElement = document.documentElement) {
  const a = ACCENT_COLORS[c.appearance.accent];
  const g = c.appearance.glass;
  root.style.setProperty('--accent', a.main);
  root.style.setProperty('--accent-rgb', a.rgb);
  root.style.setProperty('--accent-2', a.second);
  for (const [k, v] of Object.entries(THEME_VARS[c.appearance.theme])) root.style.setProperty(k, v);
  root.style.setProperty('--glass-a', glassAlpha(g, c.appearance.blur).toFixed(3));
  root.style.setProperty('--glass-blur', `${Math.round(6 + g * 0.24)}px`);
  root.classList.toggle('no-anim', !c.appearance.animations);
  root.classList.toggle('no-blur', !c.appearance.blur);
  root.dataset.theme = c.appearance.theme;
}
