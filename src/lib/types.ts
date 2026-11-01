// Mirrors src-tauri/src/config.rs. Keep the two in sync.

export const WALLPAPERS = ['nebula', 'dusk', 'peaks', 'grid'] as const;
export const ACCENTS = ['violet', 'cyan', 'rose', 'amber', 'mint'] as const;
export const DOCK_POSITIONS = ['bottom', 'left'] as const;

export type Wallpaper = (typeof WALLPAPERS)[number];
export type Accent = (typeof ACCENTS)[number];
export type DockPosition = (typeof DOCK_POSITIONS)[number];

export interface Config {
  schema_version: number;
  shell: {
    start_fullscreen: boolean;
    clock_24h: boolean;
    show_seconds: boolean;
    show_date: boolean;
    workspace_count: number;
  };
  appearance: {
    wallpaper: Wallpaper;
    accent: Accent;
    glass: number;
    animations: boolean;
  };
  dock: {
    position: DockPosition;
    icon_size: number;
  };
}

export interface ConfigPayload {
  config: Config;
  path: string;
  warning: string | null;
}

export interface SystemInfo {
  app_version: string;
  os_name: string;
  os_version: string;
  os_arch: string;
  cpu_threads: number;
  is_windows: boolean;
}

export const defaultConfig = (): Config => ({
  schema_version: 1,
  shell: { start_fullscreen: true, clock_24h: true, show_seconds: false, show_date: true, workspace_count: 5 },
  appearance: { wallpaper: 'nebula', accent: 'violet', glass: 60, animations: true },
  dock: { position: 'bottom', icon_size: 48 },
});

const clamp = (n: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, Math.round(Number.isFinite(n) ? n : lo)));
const pick = <T extends string>(list: readonly T[], v: unknown, fallback: T): T =>
  typeof v === 'string' && (list as readonly string[]).includes(v) ? (v as T) : fallback;
const bool = (v: unknown, fallback: boolean) => (typeof v === 'boolean' ? v : fallback);

/** Validate untrusted/partial data (e.g. from localStorage) into a full, valid Config. */
export function sanitizeConfig(input: unknown): Config {
  const d = defaultConfig();
  const o = (input && typeof input === 'object' ? input : {}) as Record<string, any>;
  const s = (o.shell ?? {}) as Record<string, unknown>;
  const a = (o.appearance ?? {}) as Record<string, unknown>;
  const k = (o.dock ?? {}) as Record<string, unknown>;
  return {
    schema_version: 1,
    shell: {
      start_fullscreen: bool(s.start_fullscreen, d.shell.start_fullscreen),
      clock_24h: bool(s.clock_24h, d.shell.clock_24h),
      show_seconds: bool(s.show_seconds, d.shell.show_seconds),
      show_date: bool(s.show_date, d.shell.show_date),
      workspace_count: clamp(Number(s.workspace_count ?? d.shell.workspace_count), 2, 9),
    },
    appearance: {
      wallpaper: pick(WALLPAPERS, a.wallpaper, d.appearance.wallpaper),
      accent: pick(ACCENTS, a.accent, d.appearance.accent),
      glass: clamp(Number(a.glass ?? d.appearance.glass), 0, 100),
      animations: bool(a.animations, d.appearance.animations),
    },
    dock: {
      position: pick(DOCK_POSITIONS, k.position, d.dock.position),
      icon_size: clamp(Number(k.icon_size ?? d.dock.icon_size), 32, 72),
    },
  };
}
