import { describe, it, expect } from 'vitest';
import { ridge, stars } from './wallpaper-gen';

describe('wallpaper generator', () => {
  it('is deterministic', () => {
    expect(ridge(1, 600, 200)).toBe(ridge(1, 600, 200));
    expect(stars(7, 50)).toEqual(stars(7, 50));
  });
  it('differs by seed and stays inside the viewBox', () => {
    expect(ridge(1, 600, 200)).not.toBe(ridge(2, 600, 200));
    for (const s of stars(3, 300)) {
      expect(s.x).toBeGreaterThanOrEqual(0);
      expect(s.x).toBeLessThanOrEqual(1600);
      expect(s.y).toBeLessThanOrEqual(900);
    }
  });
  it('produces a closed path', () => {
    const d = ridge(5, 500, 100);
    expect(d.startsWith('M0 900')).toBe(true);
    expect(d.endsWith('Z')).toBe(true);
    expect(d).not.toContain('NaN');
  });
});
