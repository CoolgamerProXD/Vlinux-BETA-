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
});
