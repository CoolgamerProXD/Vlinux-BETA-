// Pure view-model helpers for the Distribution Manager. They decide what the UI says, so that the
// wording and the "never show something we do not know" rules can be unit-tested without a DOM.

import type {
  ActionId, ActionState, CheckResult, Capability, ConfView, DiagCategory, DiagEvidence, DiagOutcome, DiagReport, DistroCard, CardState,
  Overview, SettingChange, OpKind,
} from './distros-types';

export const CARD_STATE_LABEL: Record<CardState, string> = {
  detected: 'Detected',
  running: 'Running',
  stopped: 'Stopped',
  starting: 'Starting…',
  installing: 'Installing…',
  inspecting: 'Inspecting…',
  unavailable: 'Unavailable',
  failed: 'Failed',
};

/** Colour family for a state badge. */
export function stateTone(s: CardState): 'good' | 'idle' | 'busy' | 'warn' | 'bad' {
  switch (s) {
    case 'running': return 'good';
    case 'starting': case 'installing': case 'inspecting': return 'busy';
    case 'failed': return 'bad';
    case 'unavailable': return 'warn';
    default: return 'idle';
  }
}

/**
 * What a card shows for its state. An operation the user has just asked for is shown at once
 * ("Starting…") instead of after the backend's next report; "stopping" has no backend state of
 * its own because the distribution is Running until it has verifiably stopped.
 */
export function shownState(c: DistroCard, pending?: OpKind): { key: CardState | 'stopping'; label: string; tone: ReturnType<typeof stateTone> } {
  if (pending === 'stop' && c.state === 'running') return { key: 'stopping', label: 'Stopping…', tone: 'busy' };
  const key: CardState = pending === 'start' && c.state !== 'running' ? 'starting' : pending === 'inspect' ? 'inspecting' : c.state;
  return { key, label: CARD_STATE_LABEL[key], tone: stateTone(key) };
}

export function actionOf(card: DistroCard, id: ActionId): ActionState {
  return card.actions.find((a) => a.action === id) ?? { action: id, enabled: false, reason: 'Not available.' };
}

export const NOT_INSPECTED = 'Not inspected yet';

export interface Fact { label: string; value: string; known: boolean }

/**
 * The facts on a card. A value VLINUX has not actually read is shown as "Not inspected yet" (or
 * "Unknown" when inspection could not find it); nothing is guessed from the distribution's name.
 */
export function cardFacts(c: DistroCard): Fact[] {
  const inspected = c.inspected_at !== null;
  const read = (v: string | null, unknownText = 'Unknown'): Fact['value'] => (v ? v : inspected ? unknownText : NOT_INSPECTED);
  const fact = (label: string, v: string | null, unknownText?: string): Fact => ({ label, value: read(v, unknownText), known: !!v });
  const version = c.os_version ?? (c.os_pretty && /\d/.test(c.os_pretty) ? c.os_pretty : null);
  const pm = c.packages.availability.state === 'present' ? c.packages.manager_name : null;
  return [
    { label: 'Name', value: c.name, known: true },
    { label: 'Display name', value: c.display_name, known: true },
    fact('Version', version, 'Not reported by the distribution'),
    fact('Kernel', c.kernel),
    fact('Default user', c.default_user, 'Could not be read'),
    fact('Architecture', c.arch),
    {
      label: 'Package manager',
      value: pm ?? (c.packages.availability.state === 'absent' ? 'None VLINUX supports was found' : NOT_INSPECTED),
      known: pm !== null,
    },
    { label: 'WSL version', value: `WSL ${c.wsl_version}`, known: true },
  ];
}

export function inspectedText(at: number | null, now = Math.floor(Date.now() / 1000)): string {
  if (at === null) return NOT_INSPECTED;
  const s = Math.max(0, now - at);
  if (s < 90) return 'Inspected just now';
  if (s < 3600) return `Inspected ${Math.round(s / 60)} min ago`;
  if (s < 86400) return `Inspected ${Math.round(s / 3600)} h ago`;
  return `Inspected ${Math.round(s / 86400)} d ago`;
}

export function capabilityText(c: Capability): string {
  switch (c.support) {
    case 'available': return 'Supported';
    case 'plan_only': return 'Shows the command only';
    case 'unsupported': return 'Not supported';
  }
}

/** What to show instead of the cards (empty/failed discovery); `null` means show the cards. */
export function emptyState(o: Overview | null, error: string | null, loading: boolean): { kind: 'loading' | 'error' | 'runtime' | 'empty'; title: string; detail: string } | null {
  if (loading && !o) return { kind: 'loading', title: 'Looking at WSL…', detail: 'Reading the list of distributions. Nothing is started.' };
  if (error && !o) return { kind: 'error', title: 'Could not read WSL', detail: error };
  if (!o) return { kind: 'empty', title: 'Not checked yet', detail: 'Press Refresh to list your distributions. Nothing is started.' };
  switch (o.runtime.kind) {
    case 'ready': return o.cards.length ? null : { kind: 'empty', title: 'No distributions', detail: 'WSL has no distribution registered.' };
    case 'no_distributions': return { kind: 'empty', title: 'No Linux distribution is installed', detail: 'Open Linux Setup to install one (it asks for your approval first).' };
    case 'not_installed': return { kind: 'runtime', title: 'WSL is not installed', detail: 'Open Linux Setup to install it.' };
    case 'restart_required': return { kind: 'runtime', title: 'Windows needs a restart', detail: 'Restart Windows, then press Refresh.' };
    case 'feature_problem': return { kind: 'runtime', title: 'WSL needs attention', detail: o.runtime.hint };
    case 'unsupported_host': return { kind: 'runtime', title: 'WSL is not available on this PC', detail: o.runtime.reason };
    case 'error': return { kind: 'runtime', title: 'WSL did not respond properly', detail: o.runtime.message };
  }
}

// ---- package names ----

export const MAX_PACKAGES = 20;

/** Split what the user typed into package names. The backend validates them again. */
export function parsePackageNames(text: string): string[] {
  const seen = new Set<string>();
  for (const p of text.split(/[\s,]+/)) if (p) seen.add(p);
  return [...seen].slice(0, MAX_PACKAGES);
}

// ---- configuration drafts ----

/** The settings whose draft differs from what the file says. `undefined` in the draft = untouched. */
export function draftToChanges(view: ConfView, draft: Record<string, string | null | undefined>): SettingChange[] {
  const out: SettingChange[] = [];
  for (const s of view.settings) {
    if (!(s.id in draft)) continue;
    const d = draft[s.id];
    if (d === undefined) continue;
    const next = d === null || d.trim() === '' ? null : d.trim();
    if (next !== s.value) out.push({ id: s.id, value: next });
  }
  return out;
}

// ---- diagnostics ----

export const DIAG_CATEGORIES: { id: DiagCategory; label: string }[] = [
  { id: 'runtime', label: 'Runtime' },
  { id: 'terminal', label: 'Terminal' },
  { id: 'filesystem', label: 'Filesystem' },
  { id: 'shared_folders', label: 'Shared folders' },
  { id: 'desktop', label: 'Desktop' },
];

export const OUTCOME_LABEL: Record<DiagOutcome, string> = {
  passed: 'Passed',
  failed: 'Failed',
  skipped: 'Skipped',
  unsupported: 'Unsupported',
  not_tested: 'Not tested',
};

export const EVIDENCE_LABEL: Record<DiagEvidence, string> = {
  real_wsl: 'Real WSL',
  host: 'Windows host only',
  simulated: 'SIMULATED',
  none: 'Did not run',
};

export function groupResults(results: CheckResult[]): { category: DiagCategory; label: string; rows: CheckResult[] }[] {
  return DIAG_CATEGORIES.map((c) => ({ category: c.id, label: c.label, rows: results.filter((r) => r.category === c.id) })).filter((g) => g.rows.length > 0);
}

/** One honest line about a run. A simulated run says outright that it proves nothing. */
export function reportHeadline(r: DiagReport): string {
  const s = r.summary;
  const counts = `${s.passed} passed · ${s.failed} failed · ${s.skipped} skipped · ${s.unsupported} unsupported · ${s.not_tested} not tested`;
  if (r.mode === 'simulated') return `SIMULATED run: no real WSL was contacted, so this says nothing about this PC. ${counts}`;
  return `${counts}${s.real_wsl_passed ? ` — ${s.real_wsl_passed} passed against the real WSL` : ''}`;
}

/** Before a run: what it will touch, in words. */
export function runPlanText(allowStart: boolean, allowWrite: boolean, simulated: boolean): string[] {
  const lines = [
    simulated
      ? 'This is the browser preview: the run is SIMULATED and contacts no real WSL.'
      : 'Reads WSL status and the distribution\'s files (read-only) and runs a few fixed, harmless commands.',
  ];
  lines.push(
    allowWrite
      ? 'Will create a temporary folder /tmp/vlinux-diag-… inside the distribution, create files in it, open a throw-away terminal session, and remove only that folder afterwards.'
      : 'Will NOT create any file or open a terminal session: those checks stay "Not tested".',
  );
  lines.push(allowStart ? 'May start the distribution if it is stopped.' : 'Will not start the distribution if it is stopped.');
  return lines;
}

export function preferredNotice(o: Overview | null): { tone: 'warn' | 'info'; text: string } | null {
  if (!o) return null;
  if (o.warning) return { tone: 'warn', text: o.warning };
  if (!o.preferred_name && o.cards.some((c) => c.kind === 'regular')) {
    return { tone: 'info', text: 'No preferred distribution is chosen. VLINUX never picks one for you; terminals and Files ask or use the one you set here.' };
  }
  return null;
}
