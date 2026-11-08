// Linux application discovery — shared types and the pure helpers behind the launcher.
// Mirrors src-tauri/src/apps.rs and crates/runtime/src/apps.rs.

export interface LinuxApp {
  /** Desktop-file id (`org.example.App.desktop`) — the only thing sent back to launch it. */
  id: string;
  name: string;
  generic_name: string | null;
  comment: string | null;
  categories: string[];
  keywords: string[];
  icon: string | null;
  terminal: boolean;
  launchable: boolean;
  /** Why it cannot be launched from here. */
  reason: string | null;
  source: 'user' | 'system' | 'flatpak' | 'snap' | string;
}

export interface AppsReport {
  distro: string;
  state: 'ready' | 'stopped' | 'unavailable';
  message: string | null;
  /** The distribution has WSLg's runtime folder, which Linux GUI apps need. */
  wslg: boolean;
  apps: LinuxApp[];
  folders: string[];
  skipped: number;
  warnings: string[];
  elapsed_ms: number;
}

export interface LaunchRecord {
  launch_id: number;
  id: string;
  name: string;
  state: 'running' | 'exited' | 'failed';
  code: number | null;
  message: string | null;
}

/** How well `query` matches an app: 0 = no match, higher = better. */
export function matchScore(query: string, app: Pick<LinuxApp, 'name' | 'generic_name' | 'comment' | 'categories' | 'keywords'>): number {
  const q = query.trim().toLowerCase();
  if (!q) return 1;
  const name = app.name.toLowerCase();
  if (name === q) return 100;
  if (name.startsWith(q)) return 80;
  if (name.split(/[\s\-_.]+/).some((w) => w.startsWith(q))) return 60;
  if (name.includes(q)) return 40;
  const extra = [app.generic_name ?? '', ...app.keywords, ...app.categories].map((s) => s.toLowerCase());
  if (extra.some((s) => s.startsWith(q))) return 25;
  if (extra.some((s) => s.includes(q))) return 20;
  if ((app.comment ?? '').toLowerCase().includes(q)) return 10;
  return 0;
}

/** Filter and rank by `query`; ties keep the incoming (alphabetical) order. */
export function rankApps<T extends Pick<LinuxApp, 'name' | 'generic_name' | 'comment' | 'categories' | 'keywords'>>(apps: T[], query: string): T[] {
  if (!query.trim()) return apps;
  return apps
    .map((a, i) => ({ a, i, s: matchScore(query, a) }))
    .filter((x) => x.s > 0)
    .sort((x, y) => y.s - x.s || x.i - y.i)
    .map((x) => x.a);
}

/**
 * Entries that share a display name are not hidden (they may be different programs), but each gets a
 * short qualifier so the user can tell them apart.
 */
export function describeApp(app: LinuxApp, all: LinuxApp[]): string {
  const clash = all.filter((a) => a.name.toLowerCase() === app.name.toLowerCase()).length > 1;
  const base = app.comment ?? app.generic_name ?? '';
  if (!clash) return base;
  const tag = app.source === 'system' ? app.id.replace(/\.desktop$/, '') : `${app.source} · ${app.id.replace(/\.desktop$/, '')}`;
  return base ? `${base} — ${tag}` : tag;
}

export const RECENTS_MAX = 8;

export function pushRecent(list: string[], id: string): string[] {
  return [id, ...list.filter((x) => x !== id)].slice(0, RECENTS_MAX);
}

export const recentsKey = (distro: string) => `vlinux.recents.${distro}`;

export function loadRecents(distro: string, storage: Pick<Storage, 'getItem'> | null = typeof localStorage === 'undefined' ? null : localStorage): string[] {
  try {
    const raw = storage?.getItem(recentsKey(distro));
    const v: unknown = raw ? JSON.parse(raw) : [];
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string' && /^[A-Za-z0-9._+-]{1,200}\.desktop$/.test(x)).slice(0, RECENTS_MAX) : [];
  } catch {
    return [];
  }
}

export function saveRecents(distro: string, list: string[], storage: Pick<Storage, 'setItem'> | null = typeof localStorage === 'undefined' ? null : localStorage) {
  try {
    storage?.setItem(recentsKey(distro), JSON.stringify(list.slice(0, RECENTS_MAX)));
  } catch {
    /* storage unavailable: recents just aren't remembered */
  }
}

/** Initial for the generated tile (no icon found). */
export function tileLetter(name: string): string {
  const m = name.trim().match(/[\p{L}\p{N}]/u);
  return (m ? m[0] : '?').toUpperCase();
}

/** Stable tint for the generated tile, derived from the name (not random). */
export function tileHue(name: string): number {
  let h = 0;
  for (const ch of name) h = (h * 31 + ch.codePointAt(0)!) % 360;
  return h;
}
