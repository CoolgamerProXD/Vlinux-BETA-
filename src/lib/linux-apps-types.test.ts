import { describe, it, expect } from 'vitest';
import { matchScore, rankApps, describeApp, pushRecent, loadRecents, saveRecents, tileLetter, tileHue, RECENTS_MAX, type LinuxApp } from './linux-apps-types';

const app = (name: string, over: Partial<LinuxApp> = {}): LinuxApp => ({
  id: `${name.toLowerCase().replace(/\s+/g, '-')}.desktop`, name, generic_name: null, comment: null, categories: [], keywords: [], icon: null,
  terminal: false, launchable: true, reason: null, source: 'system', ...over,
});

describe('launcher search', () => {
  it('ranks exact > prefix > word prefix > substring > keywords > comment', () => {
    const list = [
      app('Image Viewer', { comment: 'view a file' }),
      app('Files'),
      app('Profile Manager'),
      app('Firefox', { keywords: ['browser'] }),
      app('File Roller'),
      app('Other', { comment: 'opens a file' }),
    ];
    expect(rankApps(list, 'files').map((a) => a.name)).toEqual(['Files']);
    expect(rankApps(list, 'file').map((a) => a.name)).toEqual(['Files', 'File Roller', 'Profile Manager', 'Image Viewer', 'Other']);
    expect(rankApps(list, 'brow').map((a) => a.name)).toEqual(['Firefox']);
  });
  it('is case-insensitive, trims, and returns everything for an empty query', () => {
    const list = [app('Zeta'), app('Alpha')];
    expect(rankApps(list, '  ALPHA ').map((a) => a.name)).toEqual(['Alpha']);
    expect(rankApps(list, '').map((a) => a.name)).toEqual(['Zeta', 'Alpha']);
    expect(rankApps(list, 'nothing')).toEqual([]);
  });
  it('matches category and generic names', () => {
    expect(matchScore('editor', app('Kate', { generic_name: 'Text Editor' }))).toBeGreaterThan(0);
    expect(matchScore('graphics', app('Gimp', { categories: ['Graphics'] }))).toBeGreaterThan(0);
  });
  it('word-prefix match handles separators', () => {
    expect(matchScore('mon', app('System-Monitor'))).toBe(60);
  });
});

describe('duplicates are explained, not hidden', () => {
  it('qualifies entries that share a name', () => {
    const a = app('Terminal', { id: 'org.a.Terminal.desktop' });
    const b = app('Terminal', { id: 'org.b.Terminal.desktop', source: 'flatpak' });
    const all = [a, b];
    expect(describeApp(a, all)).toBe('org.a.Terminal');
    expect(describeApp(b, all)).toBe('flatpak · org.b.Terminal');
    expect(describeApp(app('Unique', { comment: 'x' }), all)).toBe('x');
  });
});

describe('recents', () => {
  it('moves to the front, de-duplicates and is bounded', () => {
    let l: string[] = [];
    for (let i = 0; i < 12; i++) l = pushRecent(l, `a${i}.desktop`);
    expect(l).toHaveLength(RECENTS_MAX);
    expect(l[0]).toBe('a11.desktop');
    l = pushRecent(l, 'a8.desktop');
    expect(l[0]).toBe('a8.desktop');
    expect(l.filter((x) => x === 'a8.desktop')).toHaveLength(1);
  });
  it('survives corrupt storage and rejects non-ids', () => {
    const mem = (v: string | null) => ({ getItem: () => v });
    expect(loadRecents('d', mem('not json'))).toEqual([]);
    expect(loadRecents('d', mem('{"a":1}'))).toEqual([]);
    expect(loadRecents('d', mem(JSON.stringify(['ok.desktop', '../x.desktop', 5, 'a b.desktop'])))).toEqual(['ok.desktop']);
    expect(loadRecents('d', null)).toEqual([]);
  });
  it('round-trips per distribution', () => {
    const store: Record<string, string> = {};
    const s = { getItem: (k: string) => store[k] ?? null, setItem: (k: string, v: string) => void (store[k] = v) };
    saveRecents('arch', ['a.desktop'], s);
    saveRecents('ubuntu', ['b.desktop'], s);
    expect(loadRecents('arch', s)).toEqual(['a.desktop']);
    expect(loadRecents('ubuntu', s)).toEqual(['b.desktop']);
  });
});

describe('generated tiles', () => {
  it('uses the first letter and a stable hue', () => {
    expect(tileLetter('  firefox')).toBe('F');
    expect(tileLetter('???')).toBe('?');
    expect(tileHue('Firefox')).toBe(tileHue('Firefox'));
    expect(tileHue('Firefox')).toBeGreaterThanOrEqual(0);
    expect(tileHue('Firefox')).toBeLessThan(360);
  });
});
