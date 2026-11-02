import { describe, expect, it } from 'vitest';
import {
  CARD_STATE_LABEL, NOT_INSPECTED, actionOf, cardFacts, draftToChanges, emptyState, groupResults, inspectedText, parsePackageNames,
  preferredNotice, reportHeadline, runPlanText, shownState, stateTone,
} from './distros-view';
import type { CheckResult, ConfView, DiagReport, DistroCard, Overview } from './distros-types';

const caps = (state: 'not_checked' | 'present' | 'absent' = 'not_checked'): DistroCard['packages'] => ({
  manager: 'pacman', manager_name: 'pacman', binary: 'pacman',
  availability: state === 'present' ? { state, version: '7.0.0' } : { state },
  detect: { support: 'available', note: '' }, version: { support: 'unsupported', note: '' }, search: { support: 'unsupported', note: '' },
  install: { support: 'unsupported', note: '' }, update: { support: 'unsupported', note: '' },
});

const card = (over: Partial<DistroCard> = {}): DistroCard => ({
  name: 'archlinux', display_name: 'Arch Linux', state: 'running', wsl_state: 'running', wsl_version: 2, is_windows_default: false, is_preferred: false,
  kind: 'regular', origin: 'existing', adapter: { id: 'arch', display_name: 'Arch Linux' } as DistroCard['adapter'], identity_source: 'registered_name',
  os_pretty: null, os_version: null, kernel: null, arch: null, default_user: null, packages: caps(), package_notes: [], inspected_at: null, busy: null,
  failure: null, actions: [], ...over,
});

const overview = (over: Partial<Overview> = {}): Overview => ({
  runtime: { kind: 'ready' }, cards: [card()], preferred: { kind: 'none' }, preferred_name: null, windows_default: null, warning: null, generated_at: 1, ...over,
});

const fact = (c: DistroCard, label: string) => cardFacts(c).find((f) => f.label === label)!;

describe('card facts never invent data', () => {
  it('shows "Not inspected yet" for everything that has not been read', () => {
    const c = card();
    for (const l of ['Version', 'Kernel', 'Default user', 'Architecture', 'Package manager']) {
      expect(fact(c, l).value).toBe(NOT_INSPECTED);
      expect(fact(c, l).known).toBe(false);
    }
  });

  it('always shows the facts that WSL itself reports', () => {
    const c = card({ name: 'Ubuntu-24.04', display_name: 'Ubuntu', wsl_version: 1 });
    expect(fact(c, 'Name').value).toBe('Ubuntu-24.04');
    expect(fact(c, 'Display name').value).toBe('Ubuntu');
    expect(fact(c, 'WSL version').value).toBe('WSL 1');
  });

  it('shows what inspection read', () => {
    const c = card({ inspected_at: 100, kernel: '5.15.1', arch: 'x86_64', default_user: 'dana', os_version: '24.04', packages: caps('present') });
    expect(fact(c, 'Kernel').value).toBe('5.15.1');
    expect(fact(c, 'Architecture').value).toBe('x86_64');
    expect(fact(c, 'Default user').value).toBe('dana');
    expect(fact(c, 'Version').value).toBe('24.04');
    expect(fact(c, 'Package manager').value).toBe('pacman');
    expect(fact(c, 'Kernel').known).toBe(true);
  });

  it('after inspection, a value that could not be read says so instead of "not inspected"', () => {
    const c = card({ inspected_at: 100, kernel: null, default_user: null });
    expect(fact(c, 'Kernel').value).toBe('Unknown');
    expect(fact(c, 'Default user').value).toBe('Could not be read');
    expect(fact(c, 'Kernel').known).toBe(false);
  });

  it('reports a package manager that was looked for and not found', () => {
    expect(fact(card({ inspected_at: 1, packages: caps('absent') }), 'Package manager').value).toContain('None VLINUX supports');
  });

  it('does not treat a name without digits as a version', () => {
    expect(fact(card({ inspected_at: 1, os_pretty: 'Arch Linux' }), 'Version').known).toBe(false);
    expect(fact(card({ inspected_at: 1, os_pretty: 'Ubuntu 24.04 LTS' }), 'Version').value).toBe('Ubuntu 24.04 LTS');
  });
});

describe('inspection age', () => {
  it('words the age of a cached inspection', () => {
    const now = 100000;
    expect(inspectedText(null, now)).toBe(NOT_INSPECTED);
    expect(inspectedText(now - 10, now)).toBe('Inspected just now');
    expect(inspectedText(now - 600, now)).toBe('Inspected 10 min ago');
    expect(inspectedText(now - 7200, now)).toBe('Inspected 2 h ago');
    expect(inspectedText(now - 3 * 86400, now)).toBe('Inspected 3 d ago');
    expect(inspectedText(now + 500, now)).toBe('Inspected just now');
  });
});

describe('states', () => {
  it('has a label for every state in the specification', () => {
    expect(Object.values(CARD_STATE_LABEL).map((l) => l.replace('…', ''))).toEqual(['Detected', 'Running', 'Stopped', 'Starting', 'Installing', 'Inspecting', 'Unavailable', 'Failed']);
  });
  it('maps states to tones', () => {
    expect(stateTone('running')).toBe('good');
    expect(stateTone('starting')).toBe('busy');
    expect(stateTone('failed')).toBe('bad');
    expect(stateTone('unavailable')).toBe('warn');
    expect(stateTone('stopped')).toBe('idle');
  });
  it('shows a requested operation at once, without inventing a backend state', () => {
    expect(shownState(card({ state: 'stopped' }), 'start')).toMatchObject({ key: 'starting', label: 'Starting…' });
    expect(shownState(card({ state: 'running' }), 'start').key).toBe('running');
    expect(shownState(card({ state: 'running' }), 'stop')).toMatchObject({ key: 'stopping', label: 'Stopping…' });
    expect(shownState(card({ state: 'running' }), 'inspect').key).toBe('inspecting');
    expect(shownState(card({ state: 'stopped' })).key).toBe('stopped');
  });
});

describe('actions', () => {
  it('treats a missing action as disabled with a reason', () => {
    const a = actionOf(card(), 'start');
    expect(a.enabled).toBe(false);
    expect(a.reason).toBeTruthy();
  });
  it('returns the backend decision', () => {
    const c = card({ actions: [{ action: 'stop', enabled: true, reason: null }] });
    expect(actionOf(c, 'stop').enabled).toBe(true);
  });
});

describe('empty, loading and failed discovery', () => {
  it('shows loading only before anything is known', () => {
    expect(emptyState(null, null, true)?.kind).toBe('loading');
    expect(emptyState(overview(), null, true)).toBeNull();
  });
  it('shows a failed discovery with the error', () => {
    const e = emptyState(null, 'boom', false);
    expect(e?.kind).toBe('error');
    expect(e?.detail).toBe('boom');
  });
  it('invites an explicit refresh when nothing has been checked', () => {
    expect(emptyState(null, null, false)?.detail).toContain('Refresh');
  });
  it('explains each runtime problem instead of an empty list', () => {
    expect(emptyState(overview({ runtime: { kind: 'not_installed' }, cards: [] }), null, false)?.title).toBe('WSL is not installed');
    expect(emptyState(overview({ runtime: { kind: 'no_distributions' }, cards: [] }), null, false)?.title).toContain('No Linux distribution');
    expect(emptyState(overview({ runtime: { kind: 'restart_required' }, cards: [] }), null, false)?.title).toContain('restart');
    expect(emptyState(overview({ runtime: { kind: 'unsupported_host', reason: 'not windows' }, cards: [] }), null, false)?.detail).toBe('not windows');
    expect(emptyState(overview({ runtime: { kind: 'error', message: 'bad' }, cards: [] }), null, false)?.detail).toBe('bad');
  });
  it('shows the cards for a ready runtime', () => {
    expect(emptyState(overview(), null, false)).toBeNull();
    expect(emptyState(overview({ cards: [card(), card({ name: 'Debian' })] }), null, false)).toBeNull();
  });
});

describe('preferred distribution notice', () => {
  it('shows the backend warning, which says VLINUX has not switched', () => {
    const n = preferredNotice(overview({ warning: 'gone; VLINUX has not switched', preferred_name: 'x' }));
    expect(n?.tone).toBe('warn');
    expect(n?.text).toContain('has not switched');
  });
  it('explains that nothing is chosen for the user', () => {
    expect(preferredNotice(overview())?.text).toContain('never picks one');
  });
  it('is silent when a valid preference exists or there is nothing to choose', () => {
    expect(preferredNotice(overview({ preferred_name: 'archlinux', preferred: { kind: 'ok', name: 'archlinux' } }))).toBeNull();
    expect(preferredNotice(overview({ cards: [] }))).toBeNull();
    expect(preferredNotice(overview({ cards: [card({ kind: 'internal' })] }))).toBeNull();
    expect(preferredNotice(null)).toBeNull();
  });
});

describe('package names', () => {
  it('splits on spaces, commas and newlines and drops duplicates', () => {
    expect(parsePackageNames('git, neovim  git\nbash')).toEqual(['git', 'neovim', 'bash']);
  });
  it('is empty for blank input', () => {
    expect(parsePackageNames('  ,, \n')).toEqual([]);
  });
  it('caps the list (the backend validates every name again)', () => {
    expect(parsePackageNames(Array.from({ length: 40 }, (_, i) => `p${i}`).join(' '))).toHaveLength(20);
  });
  it('does not "sanitise" hostile input: it is sent as typed and rejected by the backend', () => {
    expect(parsePackageNames('git;rm -rf ~')).toEqual(['git;rm', '-rf', '~']);
  });
});

describe('configuration drafts', () => {
  const view = (): ConfView => ({
    distro: 'd', path: '/etc/wsl.conf', exists: true, malformed: [], other_keys: [], fingerprint: 'f', size: 1, editable: true, notes: [],
    settings: [
      { id: 'automount.enabled', label: 'a', help: '', kind: 'bool', section: 'automount', key: 'enabled', value: 'true', default_note: '' },
      { id: 'user.default', label: 'u', help: '', kind: 'username', section: 'user', key: 'default', value: null, default_note: '' },
    ],
  });
  it('lists only what differs from the file', () => {
    expect(draftToChanges(view(), {})).toEqual([]);
    expect(draftToChanges(view(), { 'automount.enabled': 'true' })).toEqual([]);
    expect(draftToChanges(view(), { 'automount.enabled': 'false' })).toEqual([{ id: 'automount.enabled', value: 'false' }]);
  });
  it('treats an emptied value as "remove the key" and an untouched unset key as no change', () => {
    expect(draftToChanges(view(), { 'automount.enabled': '', 'user.default': '' })).toEqual([{ id: 'automount.enabled', value: null }]);
    expect(draftToChanges(view(), { 'user.default': null })).toEqual([]);
  });
  it('trims, and ignores settings the view does not know', () => {
    expect(draftToChanges(view(), { 'user.default': '  dana ', 'nope.key': 'x' })).toEqual([{ id: 'user.default', value: 'dana' }]);
  });
  it('skips an explicitly untouched entry', () => {
    expect(draftToChanges(view(), { 'user.default': undefined })).toEqual([]);
  });
});

describe('diagnostics wording', () => {
  const res = (over: Partial<CheckResult>): CheckResult => ({
    id: 'x', category: 'runtime', title: 't', checked: 'c', distro: null, outcome: 'passed', evidence: 'real_wsl', modified: false, message: 'm', guidance: null, ms: 1, ...over,
  });
  const report = (over: Partial<DiagReport> = {}): DiagReport => ({
    generated_at: 1, mode: 'real', distro: 'd', allow_write: false, allow_start: false, results: [], text: '', cancelled: false, diag_dir: null,
    summary: { passed: 3, failed: 1, skipped: 0, unsupported: 2, not_tested: 4, real_wsl_passed: 3 }, ...over,
  });
  it('a simulated run says that it proves nothing', () => {
    const h = reportHeadline(report({ mode: 'simulated', summary: { passed: 5, failed: 0, skipped: 0, unsupported: 0, not_tested: 0, real_wsl_passed: 0 } }));
    expect(h.startsWith('SIMULATED run')).toBe(true);
    expect(h).toContain('says nothing about this PC');
    expect(h).not.toContain('real WSL.');
  });
  it('a real run counts only what passed against the real WSL', () => {
    expect(reportHeadline(report())).toContain('3 passed against the real WSL');
    expect(reportHeadline(report({ summary: { passed: 2, failed: 0, skipped: 0, unsupported: 0, not_tested: 0, real_wsl_passed: 0 } }))).not.toContain('against the real WSL');
  });
  it('groups results by category in the fixed order and hides empty categories', () => {
    const g = groupResults([res({ id: 'a', category: 'desktop' }), res({ id: 'b', category: 'runtime' }), res({ id: 'c', category: 'runtime' })]);
    expect(g.map((x) => x.category)).toEqual(['runtime', 'desktop']);
    expect(g[0]!.rows.map((r) => r.id)).toEqual(['b', 'c']);
  });
  it('states before the run what it will and will not touch', () => {
    const none = runPlanText(false, false, false).join(' ');
    expect(none).toContain('Will NOT create any file');
    expect(none).toContain('Will not start the distribution');
    const all = runPlanText(true, true, false).join(' ');
    expect(all).toContain('/tmp/vlinux-diag-');
    expect(all).toContain('remove only that folder');
    expect(all).toContain('May start the distribution');
    expect(runPlanText(false, false, true)[0]).toContain('SIMULATED');
  });
});
