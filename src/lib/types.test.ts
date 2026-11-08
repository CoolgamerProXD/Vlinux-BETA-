import { describe, it, expect } from 'vitest';
import { sanitizeConfig, defaultConfig } from './types';

describe('sanitizeConfig', () => {
  it('returns defaults for garbage input', () => {
    expect(sanitizeConfig(null)).toEqual(defaultConfig());
    expect(sanitizeConfig('x')).toEqual(defaultConfig());
    expect(sanitizeConfig(42)).toEqual(defaultConfig());
  });
  it('rejects unknown enum values and clamps numbers (matches Rust rules)', () => {
    const c = sanitizeConfig({
      shell: { workspace_count: 99 },
      appearance: { wallpaper: 'evil', accent: 'cyan', glass: -5 },
      dock: { position: 'top', icon_size: 500 },
    });
    expect(c.shell.workspace_count).toBe(9);
    expect(c.appearance.wallpaper).toBe('nebula');
    expect(c.appearance.accent).toBe('cyan');
    expect(c.appearance.glass).toBe(0);
    expect(c.dock.position).toBe('bottom');
    expect(c.dock.icon_size).toBe(72);
  });
  it('is idempotent', () => {
    const once = sanitizeConfig({ dock: { icon_size: 20 }, shell: { workspace_count: 1 } });
    expect(sanitizeConfig(once)).toEqual(once);
    expect(once.dock.icon_size).toBe(32);
    expect(once.shell.workspace_count).toBe(2);
  });
  it('handles NaN', () => {
    expect(sanitizeConfig({ dock: { icon_size: NaN } }).dock.icon_size).toBe(32);
  });
  it('validates the runtime and storage sections (schema 2)', () => {
    const c = sanitizeConfig({
      runtime: { selected_distro: 'archlinux' },
      storage: { projects: 'shared', music: { mode: 'private' }, downloads: { mode: 'sideways', windows_path: '  D:\\Dl  ' }, pictures: { windows_path: 'x\0y' } },
    });
    expect(c.schema_version).toBe(3);
    expect(c.runtime.selected_distro).toBe('archlinux');
    expect(c.storage.projects).toBe('shared');
    expect(c.storage.music.mode).toBe('private');
    expect(c.storage.downloads).toEqual({ mode: 'shared', windows_path: 'D:\\Dl' });
    expect(c.storage.pictures.windows_path).toBeNull();
    expect(c.storage.documents).toEqual({ mode: 'shared', windows_path: null });
  });
  it('drops unsafe distribution names and defaults to Linux-private projects', () => {
    expect(sanitizeConfig({ runtime: { selected_distro: 'x; calc' } }).runtime.selected_distro).toBeNull();
    expect(sanitizeConfig({ runtime: { selected_distro: '-rf' } }).runtime.selected_distro).toBeNull();
    expect(defaultConfig().storage.projects).toBe('linux_private');
    expect(defaultConfig().runtime.selected_distro).toBeNull();
  });
  it('loads a version-1 config without losing anything', () => {
    const c = sanitizeConfig({ schema_version: 1, appearance: { accent: 'rose' } });
    expect(c.appearance.accent).toBe('rose');
    expect(c.storage).toEqual(defaultConfig().storage);
  });
  it('validates the M7 settings (schema 3) with the same rules as Rust', () => {
    const c = sanitizeConfig({
      appearance: { theme: 'neon', blur: 'yes' },
      dock: { position: 'right', labels: true, magnify: 5 },
      topbar: { show_title: false, show_windows: 'no' },
      terminal: { font_size: 99, cursor: 'bar', scrollback: 3 },
      startup: { open_app: 'terminal' },
    });
    expect(c.appearance.theme).toBe('midnight');
    expect(c.appearance.blur).toBe(true);
    expect(c.dock).toMatchObject({ position: 'right', labels: true, magnify: true });
    expect(c.topbar).toEqual({ show_workspaces: true, show_title: false, show_windows: true, show_battery: true });
    expect(c.terminal).toEqual({ font_size: 24, cursor: 'bar', cursor_blink: true, scrollback: 500 });
    // Neither the terminal nor Files can open at start-up: both start the distribution.
    expect(c.startup.open_app).toBe('none');
    expect(sanitizeConfig({ startup: { open_app: 'files' } }).startup.open_app).toBe('none');
    expect(sanitizeConfig({ startup: { open_app: 'settings' } }).startup.open_app).toBe('settings');
  });
  it('loads a version-2 config and fills the new settings with defaults', () => {
    const c = sanitizeConfig({ schema_version: 2, dock: { position: 'left', icon_size: 40 }, runtime: { selected_distro: 'archlinux' } });
    expect(c.dock).toEqual({ position: 'left', icon_size: 40, labels: false, magnify: true });
    expect(c.terminal).toEqual(defaultConfig().terminal);
    expect(c.runtime.selected_distro).toBe('archlinux');
  });
});
