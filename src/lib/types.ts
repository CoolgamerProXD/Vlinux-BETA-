// Mirrors src-tauri/src/config.rs. Keep the two in sync.

export const WALLPAPERS = ['nebula', 'dusk', 'peaks', 'grid'] as const;
export const ACCENTS = ['violet', 'cyan', 'rose', 'amber', 'mint'] as const;
export const DOCK_POSITIONS = ['bottom', 'left', 'right'] as const;
export const THEMES = ['midnight', 'graphite', 'oled'] as const;
export const CURSORS = ['block', 'bar', 'underline'] as const;
/** What may open by itself at start-up. Never the terminal or Files: both start the distribution. */
export const STARTUP_APPS = ['none', 'distros', 'settings'] as const;

export type Wallpaper = (typeof WALLPAPERS)[number];
export type Accent = (typeof ACCENTS)[number];
export type DockPosition = (typeof DOCK_POSITIONS)[number];
export type Theme = (typeof THEMES)[number];
export type CursorStyle = (typeof CURSORS)[number];
export type StartupApp = (typeof STARTUP_APPS)[number];

export const FOLDER_IDS = ['downloads', 'documents', 'pictures', 'videos', 'music', 'desktop'] as const;
export type FolderId = (typeof FOLDER_IDS)[number];
export const MAPPING_MODES = ['shared', 'private'] as const;
export const PROJECT_MODES = ['linux_private', 'shared'] as const;
export type MappingMode = (typeof MAPPING_MODES)[number];
export type ProjectsMode = (typeof PROJECT_MODES)[number];

export interface FolderPref {
  mode: MappingMode;
  /** Custom Windows folder; null = the Windows Known Folder, resolved by the backend. */
  windows_path: string | null;
}

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
    theme: Theme;
    /** Backdrop blur behind panels (the biggest GPU cost on integrated graphics). */
    blur: boolean;
  };
  dock: {
    position: DockPosition;
    icon_size: number;
    labels: boolean;
    magnify: boolean;
  };
  topbar: {
    show_workspaces: boolean;
    show_title: boolean;
    show_windows: boolean;
    show_battery: boolean;
  };
  terminal: {
    font_size: number;
    cursor: CursorStyle;
    cursor_blink: boolean;
    scrollback: number;
  };
  startup: { open_app: StartupApp };
  /** Which distribution the user chose to use. Never set automatically. */
  runtime: { selected_distro: string | null };
  /** Saved shared-folder preferences (applied from M4 on). */
  storage: { projects: ProjectsMode } & Record<FolderId, FolderPref>;
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
  schema_version: 3,
  shell: { start_fullscreen: true, clock_24h: true, show_seconds: false, show_date: true, workspace_count: 5 },
  appearance: { wallpaper: 'nebula', accent: 'violet', glass: 60, animations: true, theme: 'midnight', blur: true },
  dock: { position: 'bottom', icon_size: 48, labels: false, magnify: true },
  topbar: { show_workspaces: true, show_title: true, show_windows: true, show_battery: true },
  terminal: { font_size: 13, cursor: 'block', cursor_blink: true, scrollback: 5000 },
  startup: { open_app: 'none' },
  runtime: { selected_distro: null },
  storage: defaultStorage(),
});

export const defaultFolderPref = (): FolderPref => ({ mode: 'shared', windows_path: null });

export function defaultStorage(): Config['storage'] {
  return {
    projects: 'linux_private',
    downloads: defaultFolderPref(),
    documents: defaultFolderPref(),
    pictures: defaultFolderPref(),
    videos: defaultFolderPref(),
    music: defaultFolderPref(),
    desktop: defaultFolderPref(),
  };
}

const DISTRO_NAME = /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/;

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
  const tb = (o.topbar ?? {}) as Record<string, unknown>;
  const tm = (o.terminal ?? {}) as Record<string, unknown>;
  const su = (o.startup ?? {}) as Record<string, unknown>;
  const r = (o.runtime ?? {}) as Record<string, unknown>;
  const st = (o.storage ?? {}) as Record<string, any>;
  const ds = defaultStorage();
  const storage = { projects: pick(PROJECT_MODES, st.projects, ds.projects) } as Config['storage'];
  for (const id of FOLDER_IDS) {
    const f = (st[id] ?? {}) as Record<string, unknown>;
    const wp = typeof f.windows_path === 'string' ? f.windows_path.trim() : '';
    storage[id] = {
      mode: pick(MAPPING_MODES, f.mode, 'shared'),
      windows_path: wp && wp.length <= 1024 && !wp.includes('\0') ? wp : null,
    };
  }
  return {
    schema_version: 3,
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
      theme: pick(THEMES, a.theme, d.appearance.theme),
      blur: bool(a.blur, d.appearance.blur),
    },
    dock: {
      position: pick(DOCK_POSITIONS, k.position, d.dock.position),
      icon_size: clamp(Number(k.icon_size ?? d.dock.icon_size), 32, 72),
      labels: bool(k.labels, d.dock.labels),
      magnify: bool(k.magnify, d.dock.magnify),
    },
    topbar: {
      show_workspaces: bool(tb.show_workspaces, d.topbar.show_workspaces),
      show_title: bool(tb.show_title, d.topbar.show_title),
      show_windows: bool(tb.show_windows, d.topbar.show_windows),
      show_battery: bool(tb.show_battery, d.topbar.show_battery),
    },
    terminal: {
      font_size: clamp(Number(tm.font_size ?? d.terminal.font_size), 10, 24),
      cursor: pick(CURSORS, tm.cursor, d.terminal.cursor),
      cursor_blink: bool(tm.cursor_blink, d.terminal.cursor_blink),
      scrollback: clamp(Number(tm.scrollback ?? d.terminal.scrollback), 500, 50000),
    },
    startup: { open_app: pick(STARTUP_APPS, su.open_app, d.startup.open_app) },
    runtime: { selected_distro: typeof r.selected_distro === 'string' && DISTRO_NAME.test(r.selected_distro) ? r.selected_distro : null },
    storage,
  };
}
