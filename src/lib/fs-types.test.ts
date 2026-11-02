import { describe, expect, it } from 'vitest';
import {
  asFsError, childOf, formatSize, mappingBadge, nameProblem, parentOf, parseTypedPath, suggestName, sameLoc, type MappingStatus,
} from './fs-types';

describe('locations', () => {
  it('builds children and parents without ever producing a raw host path', () => {
    expect(childOf({ space: 'linux', path: '/' }, 'etc')).toEqual({ space: 'linux', path: '/etc' });
    expect(childOf({ space: 'linux', path: '/home/me/' }, 'a b')).toEqual({ space: 'linux', path: '/home/me/a b' });
    expect(childOf({ space: 'windows', root: 'downloads', rel: ['x'] }, 'y')).toEqual({ space: 'windows', root: 'downloads', rel: ['x', 'y'] });
    expect(parentOf({ space: 'linux', path: '/home/me' })).toEqual({ space: 'linux', path: '/home' });
    expect(parentOf({ space: 'linux', path: '/home' })).toEqual({ space: 'linux', path: '/' });
    expect(parentOf({ space: 'linux', path: '/' })).toBeNull();
    expect(parentOf({ space: 'windows', root: 'documents', rel: [] })).toBeNull();
    expect(parentOf({ space: 'windows', root: 'documents', rel: ['a', 'b'] })).toEqual({ space: 'windows', root: 'documents', rel: ['a'] });
    expect(sameLoc({ space: 'linux', path: '/a' }, { space: 'linux', path: '/a' })).toBe(true);
    expect(sameLoc({ space: 'linux', path: '/a' }, null)).toBe(false);
  });
});

describe('typed paths', () => {
  it('accepts Linux paths and expands ~', () => {
    expect(parseTypedPath('~/Docs', '/home/me')).toEqual({ space: 'linux', path: '/home/me/Docs' });
    expect(parseTypedPath('~', '/home/me/')).toEqual({ space: 'linux', path: '/home/me' });
    expect(parseTypedPath('/etc//ssh/./', null)).toEqual({ space: 'linux', path: '/etc/ssh' });
    expect(parseTypedPath('/', null)).toEqual({ space: 'linux', path: '/' });
  });
  it('refuses traversal, Windows paths and junk', () => {
    for (const bad of ['', '  ', '../x', '/a/../b', 'relative', 'C:\\Users', '\\\\nas\\share', '/a\\b', '/a\nb']) {
      expect('error' in parseTypedPath(bad, '/home/me'), bad).toBe(true);
    }
    expect('error' in parseTypedPath('~/x', null)).toBe(true);
  });
});

describe('names and formatting', () => {
  it('validates names', () => {
    expect(nameProblem('ok.txt')).toBeNull();
    for (const bad of ['', ' ', '.', '..', 'a/b', 'a\\b', 'x'.repeat(300)]) expect(nameProblem(bad), bad).not.toBeNull();
  });
  it('suggests free names', () => {
    expect(suggestName('New folder', ['a'])).toBe('New folder');
    expect(suggestName('New folder', ['New folder', 'New folder 2'])).toBe('New folder 3');
  });
  it('formats sizes', () => {
    expect(formatSize(null)).toBe('—');
    expect(formatSize(12)).toBe('12 B');
    expect(formatSize(1536)).toBe('1.50 KB');
    expect(formatSize(5 * 1024 * 1024)).toBe('5.00 MB');
  });
  it('normalises errors', () => {
    expect(asFsError({ kind: 'not_found', message: 'gone' })).toEqual({ kind: 'not_found', message: 'gone' });
    expect(asFsError('boom')).toEqual({ kind: 'io', message: 'boom' });
    expect(asFsError(new Error('x')).message).toBe('x');
  });
});

describe('mapping badges never claim a connection that was not verified', () => {
  const st = (state: MappingStatus['state']): MappingStatus => ({
    folder: 'downloads', label: 'Downloads', linux_path: '~/Downloads', preference: 'shared', windows_path: 'D:\\x', target: '/mnt/d/x',
    warning: null, state, backup: null, applied_at: null,
  });
  it('maps each state to a distinct, honest label', () => {
    expect(mappingBadge(undefined, 'shared')).toEqual({ text: 'Preferred · not checked', tone: 'idle' });
    expect(mappingBadge(st({ state: 'unknown' }), 'shared').tone).toBe('idle');
    expect(mappingBadge(st({ state: 'applied', target: '/mnt/d/x' }), 'shared')).toEqual({ text: 'Connected', tone: 'ok' });
    expect(mappingBadge(st({ state: 'ready', needs: null, note: '' }), 'shared').text).toBe('Ready to apply');
    expect(mappingBadge(st({ state: 'ready', needs: 'replace_empty_folder', note: '' }), 'shared').tone).toBe('warn');
    expect(mappingBadge(st({ state: 'conflict', kind: 'file', detail: '', entries: [], more: 0, offers: [] }), 'shared').text).toContain('decision');
    expect(mappingBadge(st({ state: 'failed', message: 'x' }), 'shared').tone).toBe('bad');
    expect(mappingBadge(st({ state: 'unavailable', reason: 'x' }), 'shared').tone).toBe('bad');
    expect(mappingBadge(st({ state: 'linked_but_private', target: 't' }), 'private').tone).toBe('warn');
  });
});
