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
    expect(c.schema_version).toBe(2);
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
});
