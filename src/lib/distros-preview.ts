// Browser-preview implementation of the Distribution Manager commands.
//
// Used ONLY outside the desktop app (UI development, automated screenshots). EVERYTHING here is
// SIMULATED: no WSL is queried, no distribution is touched, and every diagnostics result is stamped
// `simulated`. The desktop app never loads this module. It mirrors the behaviour rules of the real
// backend (explicit start, confirmation for stop, read-only inspection, plans that are never run,
// reviewable configuration edits) so the UI can be exercised end to end.

import type {
  ApplyOutcome, BackupInfo, CardState, CheckResult, ConfPlan, ConfView, DiagCategory, DiagOptions, DiagProgress, DiagReport, DiffLine,
  DistroCard, DistroOpEvent, Inspection, InspectOutcome, LifecycleResult, OpKind, Overview, PackageCaps, PackageHit, PackagePlan,
  SettingChange, SettingValue, WindowsConfig, WslConfRead, ActionId, ActionState, Capability, Availability,
} from './distros-types';
import type { Distro, DistroState } from './runtime-types';
import { detect, distroState, scenario, SIMULATION_NOTE } from './preview';
import { defaultConfig, sanitizeConfig } from './types';

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));
const now = () => Math.floor(Date.now() / 1000);

export { SIMULATION_NOTE };

// ---- what the simulated distributions "contain" -------------------------------------------------

interface Sim {
  pretty: string;
  version: string | null;
  kernel: string;
  arch: string;
  user: string;
  managers: { binary: string; present: boolean; version: string | null }[];
  conf: string | null;
  /** Simulates a distribution whose /etc/wsl.conf the Windows side may not write. */
  readonly?: boolean;
}

const KERNEL2 = '5.15.167.4-microsoft-standard-WSL2';
const SIM: Record<string, Sim> = {
  archlinux: {
    pretty: 'Arch Linux', version: null, kernel: KERNEL2, arch: 'x86_64', user: 'preview',
    managers: [{ binary: 'pacman', present: true, version: '7.0.0' }],
    conf: '[automount]\nenabled = true\nroot = /mnt/ # drives\n\n[network]\nhostname = arch-box\n\n[interop]\nenabled = true\n',
  },
  'Ubuntu-24.04': {
    pretty: 'Ubuntu 24.04.1 LTS', version: '24.04', kernel: KERNEL2, arch: 'x86_64', user: 'preview',
    managers: [{ binary: 'apt-get', present: true, version: '2.7.14' }],
    conf: null,
  },
  Debian: {
    pretty: 'Debian GNU/Linux 12 (bookworm)', version: '12', kernel: '4.4.0-19041-Microsoft', arch: 'x86_64', user: 'preview',
    managers: [{ binary: 'apt-get', present: true, version: '2.6.1' }],
    conf: '[boot]\nsystemd = false\n',
    readonly: true,
  },
  'FedoraLinux-42': {
    pretty: 'Fedora Linux 42', version: '42', kernel: KERNEL2, arch: 'x86_64', user: 'preview',
    managers: [{ binary: 'dnf', present: true, version: '4.21.0' }],
    conf: '[user]\ndefault = preview\n',
  },
  Alpine: {
    pretty: 'Alpine Linux v3.20', version: '3.20.3', kernel: KERNEL2, arch: 'x86_64', user: 'preview',
    managers: [{ binary: 'apk', present: true, version: '2.14.4' }],
    conf: null,
  },
};

const inspections = new Map<string, Inspection>();
const busy = new Map<string, OpKind>();
const failures = new Map<string, string>();
const confText = new Map<string, string | null>();
const backups = new Map<string, { id: string; at: number; existed: boolean; content: string; writtenFp: string | null; restoredAt: number | null }[]>();
let seq = 0;

const opListeners = new Set<(e: DistroOpEvent) => void>();
const emitOp = (e: DistroOpEvent) => opListeners.forEach((l) => l(e));
export const onOp = (cb: (e: DistroOpEvent) => void) => {
  opListeners.add(cb);
  return () => opListeners.delete(cb);
};

const diagListeners = new Set<(e: DiagProgress) => void>();
if (typeof window !== 'undefined') {
  // Test hook for the end-to-end run: proves the Distribution Manager removes its listeners when closed.
  (window as unknown as { __vlinuxDistrosPreview: unknown }).__vlinuxDistrosPreview = {
    listeners: () => ({ op: opListeners.size, diag: diagListeners.size }),
  };
}

export const onDiag = (cb: (e: DiagProgress) => void) => {
  diagListeners.add(cb);
  return () => diagListeners.delete(cb);
};

// ---- overview ----------------------------------------------------------------------------------

function preferredName(): string | null {
  try {
    const raw = localStorage.getItem('vlinux.config');
    return raw ? sanitizeConfig(JSON.parse(raw)).runtime.selected_distro : null;
  } catch {
    return null;
  }
}

function capabilities(manager: PackageCaps['manager'], availability: Availability): PackageCaps {
  const names = { apt: 'APT', pacman: 'pacman', dnf: 'DNF', unknown: 'unknown' } as const;
  const bins = { apt: 'apt-get', pacman: 'pacman', dnf: 'dnf', unknown: null } as const;
  const supported = manager !== 'unknown';
  const present = availability.state === 'present';
  const checked = availability.state !== 'not_checked';
  const name = names[manager];
  const cap = (support: Capability['support'], note: string): Capability => ({ support, note });
  const gate = (ok: string, plan: boolean): Capability =>
    !supported ? cap('unsupported', "VLINUX has no adapter for this distribution's package manager.")
    : !checked ? cap('unsupported', 'Inspect the distribution first: availability is checked, not assumed.')
    : !present ? cap('unsupported', `${name} was not found in this distribution.`)
    : cap(plan ? 'plan_only' : 'available', ok);
  return {
    manager, manager_name: name, binary: bins[manager], availability,
    detect: supported ? cap('available', "VLINUX looks for the package manager inside the distribution; it is never assumed from the distribution's name.") : cap('unsupported', 'VLINUX has no package-manager adapter for this distribution.'),
    version: !supported ? cap('unsupported', 'No adapter.')
      : availability.state === 'present' ? (availability.version ? cap('available', 'Read from the package manager.') : cap('unsupported', 'The package manager did not report a version.'))
      : availability.state === 'absent' ? cap('unsupported', `${name} was not found in this distribution.`)
      : cap('unsupported', 'Inspect the distribution to find out.'),
    search: gate('Read-only search, run only when you ask. Nothing is installed or changed.', false),
    install: gate('VLINUX shows the exact command to review and run in your terminal. It never installs packages itself: that needs root, and VLINUX does not escalate privileges.', true),
    update: gate('VLINUX shows the exact commands to run in your terminal. It never starts a system upgrade on its own.', true),
  };
}

const MANAGER_OF: Record<string, PackageCaps['manager']> = { arch: 'pacman', ubuntu: 'apt', debian: 'apt', fedora: 'dnf', generic: 'unknown' };

function packagesFor(d: Distro, insp: Inspection | undefined): { caps: PackageCaps; notes: string[] } {
  const manager = MANAGER_OF[d.adapter.id] ?? 'unknown';
  if (!insp) return { caps: capabilities(manager, { state: 'not_checked' }), notes: [] };
  const bin = { apt: 'apt-get', pacman: 'pacman', dnf: 'dnf', unknown: '' }[manager];
  const own = insp.managers.find((m) => m.binary === bin && m.present);
  if (own) return { caps: capabilities(manager, { state: 'present', version: own.version }), notes: [] };
  const supported = insp.managers.filter((m) => m.present && ['apt-get', 'pacman', 'dnf'].includes(m.binary));
  if (manager === 'unknown' && supported.length === 1 && supported[0]) {
    const m = supported[0];
    const pm = ({ 'apt-get': 'apt', pacman: 'pacman', dnf: 'dnf' } as const)[m.binary as 'apt-get' | 'pacman' | 'dnf'];
    return { caps: capabilities(pm, { state: 'present', version: m.version }), notes: [`${capabilities(pm, { state: 'absent' }).manager_name} was found by probing; the distribution itself is not one VLINUX has an adapter for.`] };
  }
  const others = insp.managers.filter((m) => m.present).map((m) => m.binary);
  return { caps: capabilities(manager, { state: 'absent' }), notes: others.length ? [`Found ${others.join(', ')} here, which VLINUX has no package adapter for.`] : [] };
}

function actionsFor(d: Distro, b: OpKind | null, preferred: boolean): ActionState[] {
  const all: ActionId[] = ['open_terminal', 'open_files', 'start', 'stop', 'inspect', 'set_preferred', 'refresh', 'open_config', 'view_diagnostics'];
  const act = (action: ActionId, enabled: boolean, reason: string): ActionState => ({ action, enabled, reason: enabled || !reason ? null : reason });
  if (d.kind === 'internal') return all.map((a) => act(a, a === 'refresh', 'This distribution belongs to another application; VLINUX never touches it.'));
  const changing = ['installing', 'uninstalling', 'converting'].includes(d.state);
  const verbs: Record<OpKind, string> = { start: 'starting', stop: 'stopping', inspect: 'being inspected' };
  return all.map((a) => {
    if (a === 'refresh') return act(a, true, '');
    if (changing) return act(a, false, 'WSL is changing this distribution right now.');
    if (b) return act(a, false, `It is ${verbs[b]} right now.`);
    switch (a) {
      case 'start': return act(a, d.state !== 'running', 'Already running.');
      case 'stop': return act(a, d.state === 'running', 'Only a running distribution can be stopped.');
      case 'set_preferred': return act(a, !preferred, 'Already your preferred distribution.');
      default: return act(a, true, '');
    }
  });
}

function cardState(d: Distro, b: OpKind | null, failed: boolean): CardState {
  if (d.kind === 'internal') return 'unavailable';
  if (b === 'start') return 'starting';
  if (b === 'inspect') return 'inspecting';
  if (b === 'stop') return d.state === 'running' ? 'running' : 'stopped';
  if (d.state === 'installing') return 'installing';
  if (d.state === 'uninstalling' || d.state === 'converting') return 'unavailable';
  if (failed) return 'failed';
  return d.state === 'running' ? 'running' : d.state === 'stopped' ? 'stopped' : 'detected';
}

export async function overview(force: boolean): Promise<Overview> {
  await wait(force ? 350 : 120);
  if (force) failures.clear();
  const report = await detect();
  const pref = preferredName();
  const cards: DistroCard[] = report.distros.map((d) => {
    const insp = inspections.get(d.name);
    const b = busy.get(d.name) ?? null;
    const failure = failures.get(d.name) ?? null;
    const { caps, notes } = packagesFor(d, insp);
    const os = insp?.os_release ?? d.os_release;
    return {
      name: d.name, display_name: d.display_label, state: cardState(d, b, failure !== null), wsl_state: d.state, wsl_version: d.wsl_version,
      is_windows_default: d.is_default, is_preferred: pref === d.name, kind: d.kind, origin: d.origin, adapter: d.adapter,
      identity_source: d.identity_source, os_pretty: os?.pretty_name ?? null, os_version: os?.version_id ?? null,
      kernel: insp?.kernel ?? null, arch: insp?.arch ?? null, default_user: insp?.default_user ?? null,
      packages: caps, package_notes: notes, inspected_at: insp?.at ?? null, busy: b, failure, actions: actionsFor(d, b, pref === d.name),
    };
  });
  const status = report.status;
  let preferred: Overview['preferred'] = { kind: 'none' };
  let warning: string | null = null;
  if (pref) {
    if (status.kind !== 'ready') {
      preferred = { kind: 'unchecked', name: pref };
      warning = `WSL is not ready, so VLINUX cannot check that '${pref}' still exists.`;
    } else {
      const d = report.distros.find((x) => x.name === pref);
      if (!d) {
        preferred = { kind: 'missing', name: pref };
        warning = `Your preferred distribution '${pref}' is no longer registered with WSL. VLINUX has not switched to another one${report.default_distro ? ` (the Windows default, '${report.default_distro}', is not used automatically either)` : ''} — choose a distribution yourself.`;
      } else if (d.kind === 'internal') {
        preferred = { kind: 'internal', name: pref };
        warning = `'${pref}' belongs to another application and cannot be used as your preferred distribution.`;
      } else {
        preferred = { kind: 'ok', name: pref };
      }
    }
  }
  return { runtime: status, cards, preferred, preferred_name: pref, windows_default: report.default_distro, warning, generated_at: now() };
}

export async function setPreferred(name: string | null): Promise<Overview> {
  if (name !== null) {
    const report = await detect();
    const d = report.distros.find((x) => x.name === name);
    if (!d) throw new Error(`No distribution named '${name}' is registered with WSL.`);
    if (d.kind === 'internal') throw new Error(`'${name}' belongs to another application and is never touched by VLINUX.`);
  }
  // Same place the preview config lives. Nothing about WSL changes: in particular not the Windows default.
  let cfg = defaultConfig();
  try {
    const raw = localStorage.getItem('vlinux.config');
    if (raw) cfg = sanitizeConfig(JSON.parse(raw));
  } catch { /* defaults */ }
  cfg.runtime.selected_distro = name;
  localStorage.setItem('vlinux.config', JSON.stringify(sanitizeConfig(cfg)));
  return overview(false);
}

// ---- lifecycle ---------------------------------------------------------------------------------

async function lifecycleOp(name: string, kind: 'start' | 'stop', confirmed: boolean): Promise<LifecycleResult> {
  const report = await detect();
  const d = report.distros.find((x) => x.name === name);
  if (!d) throw new Error(`No distribution named '${name}' is registered with WSL.`);
  if (d.kind === 'internal') throw new Error(`'${name}' belongs to another application and is never touched by VLINUX.`);
  if (kind === 'stop' && !confirmed) throw new Error('Stopping a distribution needs your confirmation. Nothing was stopped.');
  if (busy.has(name)) throw new Error(`'${name}' is ${busy.get(name) === 'start' ? 'starting' : busy.get(name) === 'stop' ? 'stopping' : 'being inspected'} right now.`);
  busy.set(name, kind);
  emitOp({ distro: name, op: kind, phase: 'started', ok: true, message: `${kind === 'start' ? 'Starting' : 'Stopping'} '${name}'… (simulated)` });
  try {
    await wait(kind === 'start' ? 1100 : 600);
    const target: DistroState = kind === 'start' ? 'running' : 'stopped';
    const ran = d.state !== target;
    const after = await distroState(name, target);
    failures.delete(name);
    const message = ran ? `'${name}' is now ${target}. (Simulated.)` : `'${name}' was already ${target}. Nothing was run. (Simulated.)`;
    emitOp({ distro: name, op: kind, phase: 'finished', ok: true, message });
    return { outcome: { distro: name, op: kind, ran, verified: true, message }, report: after };
  } finally {
    busy.delete(name);
  }
}

export const start = (name: string) => lifecycleOp(name, 'start', true);
export const stop = (name: string, confirmed: boolean) => lifecycleOp(name, 'stop', confirmed);

export async function inspect(name: string, allowStart: boolean): Promise<InspectOutcome> {
  const report = await detect();
  const d = report.distros.find((x) => x.name === name);
  if (!d) throw new Error(`No distribution named '${name}' is registered with WSL.`);
  if (d.kind === 'internal') throw new Error(`'${name}' belongs to another application and is never touched by VLINUX.`);
  if (busy.has(name)) throw new Error(`'${name}' is busy right now.`);
  const wasRunning = d.state === 'running';
  if (!wasRunning && !allowStart) return { kind: 'needs_start', distro: name };
  busy.set(name, 'inspect');
  emitOp({ distro: name, op: 'inspect', phase: 'started', ok: true, message: `Inspecting '${name}'… (simulated)` });
  try {
    await wait(700);
    if (!wasRunning) await distroState(name, 'running');
    const sim = SIM[name];
    const insp: Inspection = {
      distro: name, at: now(), was_running: wasRunning, started_by_inspection: !wasRunning,
      os_release: sim ? { id: d.adapter.id, pretty_name: sim.pretty, version_id: sim.version } : null,
      kernel: sim?.kernel ?? null, arch: sim?.arch ?? null, default_user: sim?.user ?? null, uid: sim ? 1000 : null,
      managers: sim?.managers ?? [],
      notes: sim ? [] : ['This distribution is not part of the simulation, so nothing could be read.'],
    };
    inspections.set(name, insp);
    failures.delete(name);
    emitOp({ distro: name, op: 'inspect', phase: 'finished', ok: true, message: `Inspected '${name}'. (Simulated.)` });
    return { kind: 'done', inspection: insp };
  } finally {
    busy.delete(name);
  }
}

// ---- packages ----------------------------------------------------------------------------------

async function support(name: string): Promise<PackageCaps> {
  const report = await detect();
  const d = report.distros.find((x) => x.name === name);
  if (!d || d.kind === 'internal') throw new Error(`No usable distribution named '${name}'.`);
  const insp = inspections.get(name);
  if (!insp) throw new Error(`'${name}' has not been inspected yet. Inspect it first so VLINUX can see which package manager it really has.`);
  const { caps } = packagesFor(d, insp);
  if (caps.availability.state !== 'present') throw new Error('No package manager VLINUX supports was found in this distribution.');
  return caps;
}

const valid = (p: string) => /^[a-z0-9][a-z0-9+._-]{0,63}$/.test(p);

export async function pkgPlan(name: string, op: 'install' | 'update', packages: string[]): Promise<PackagePlan> {
  const caps = await support(name);
  let commands: string[];
  if (op === 'install') {
    if (!packages.length) throw new Error('Enter at least one package name.');
    if (packages.length > 20) throw new Error('At most 20 packages can be reviewed at once.');
    const bad = packages.find((p) => !valid(p));
    if (bad) throw new Error(`'${bad.slice(0, 40)}' is not a valid package name.`);
    const list = packages.join(' ');
    commands = { apt: [`sudo apt install ${list}`], pacman: [`sudo pacman -S --needed ${list}`], dnf: [`sudo dnf install ${list}`], unknown: [] }[caps.manager];
  } else {
    commands = { apt: ['sudo apt update', 'sudo apt upgrade'], pacman: ['sudo pacman -Syu'], dnf: ['sudo dnf upgrade --refresh'], unknown: [] }[caps.manager];
  }
  if (!commands.length) throw new Error("VLINUX has no package-manager adapter for this distribution, so it can't prepare a command.");
  const notes = caps.manager === 'pacman' ? ['Arch does not support partial upgrades: update the whole system before installing new packages.'] : caps.manager === 'apt' ? ['Run `sudo apt update` first if the package lists are old.'] : [];
  notes.push('VLINUX has not run this. Copy it into the Terminal for this distribution if you want to go ahead; `sudo` will ask for your password there.');
  return { distro: name, op, manager_name: caps.manager_name, packages: op === 'install' ? packages : [], commands, requires_root: true, executed: false, notes };
}

const CATALOG: Record<string, PackageHit[]> = {
  pacman: [{ name: 'neovim', version: '0.10.2-1', description: 'Fork of Vim aiming to improve user experience, plugins, and GUIs' }, { name: 'nano', version: '8.2-1', description: 'Pico editor clone with enhancements' }],
  apt: [{ name: 'neovim', version: '0.9.5-6', description: 'heavily refactored vim fork' }, { name: 'nano', version: '7.2-2', description: 'small, friendly text editor inspired by Pico' }],
  dnf: [{ name: 'neovim', version: '0.10.1', description: 'Vim-fork focused on extensibility and usability' }, { name: 'nano', version: '8.1', description: 'A small text editor' }],
};

export async function pkgSearch(name: string, query: string): Promise<PackageHit[]> {
  const report = await detect();
  const d = report.distros.find((x) => x.name === name);
  if (d && d.state !== 'running') throw new Error(`'${name}' is stopped. Start it first: searching never starts a distribution.`);
  if (!/^[A-Za-z0-9][A-Za-z0-9._+-]{0,63}$/.test(query)) throw new Error('Use 1–64 letters, digits, and . _ + - in the search.');
  const caps = await support(name);
  await wait(300);
  return (CATALOG[caps.manager] ?? []).filter((h) => h.name.includes(query.toLowerCase()) || h.description.toLowerCase().includes(query.toLowerCase()));
}

// ---- /etc/wsl.conf -----------------------------------------------------------------------------

const DEFS: Omit<SettingValue, 'value'>[] = [
  { id: 'automount.enabled', section: 'automount', key: 'enabled', kind: 'bool', label: 'Mount Windows drives', help: 'Make Windows drives (C:, D:, …) available inside the distribution.', default_note: 'WSL default: on' },
  { id: 'automount.root', section: 'automount', key: 'root', kind: 'path', label: 'Drive mount folder', help: 'Where Windows drives appear. VLINUX shared-folder links point into this folder.', default_note: 'WSL default: /mnt/' },
  { id: 'interop.enabled', section: 'interop', key: 'enabled', kind: 'bool', label: 'Run Windows programs', help: 'Let Linux start Windows programs (including opening files with Windows apps).', default_note: 'WSL default: on' },
  { id: 'interop.appendWindowsPath', section: 'interop', key: 'appendWindowsPath', kind: 'bool', label: 'Windows PATH in Linux', help: "Add the Windows PATH to the distribution's PATH.", default_note: 'WSL default: on' },
  { id: 'boot.systemd', section: 'boot', key: 'systemd', kind: 'bool', label: 'systemd at startup', help: 'Run systemd as the init process (WSL 2 only).', default_note: 'WSL default: off unless the distribution sets it' },
  { id: 'user.default', section: 'user', key: 'default', kind: 'username', label: 'Default user', help: 'The user a new terminal opens as. It must already exist in the distribution.', default_note: 'Not set: the user chosen when the distribution was set up' },
];

const fp = (t: string | null): string => {
  let h = 2166136261;
  const s = t === null ? '\u0000none' : t;
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 16777619) >>> 0;
  return h.toString(16).padStart(8, '0');
};

const textOf = (name: string): string | null => (confText.has(name) ? (confText.get(name) ?? null) : (SIM[name]?.conf ?? null));

const KEYLINE = /^\s*([A-Za-z0-9_.-]+)\s*=\s*([^#]*?)\s*(#.*)?$/;

function getVal(text: string | null, section: string, key: string): string | null {
  let cur = '';
  for (const line of (text ?? '').split('\n')) {
    const sec = /^\s*\[([^\]]+)\]\s*$/.exec(line);
    if (sec && sec[1] !== undefined) { cur = sec[1].toLowerCase(); continue; }
    const m = KEYLINE.exec(line);
    if (m && cur === section.toLowerCase() && m[1] !== undefined && m[1].toLowerCase() === key.toLowerCase()) return m[2] ?? '';
  }
  return null;
}

function setVal(text: string | null, section: string, key: string, value: string | null): string {
  const lines = (text ?? '').split('\n');
  if (lines.length && lines[lines.length - 1] === '') lines.pop();
  let cur = '';
  let secEnd = -1;
  let found = -1;
  lines.forEach((line, i) => {
    const sec = /^\s*\[([^\]]+)\]\s*$/.exec(line);
    if (sec && sec[1] !== undefined) { cur = sec[1].toLowerCase(); if (cur === section.toLowerCase()) secEnd = i; return; }
    if (cur === section.toLowerCase()) {
      if (line.trim() !== '') secEnd = i;
      const m = KEYLINE.exec(line);
      if (m && m[1] !== undefined && m[1].toLowerCase() === key.toLowerCase()) found = i;
    }
  });
  if (found >= 0) {
    if (value === null) lines.splice(found, 1);
    else {
      const m = KEYLINE.exec(lines[found] ?? '');
      lines[found] = `${key} = ${value}${m && m[3] ? ` ${m[3]}` : ''}`;
    }
  } else if (value !== null) {
    if (secEnd >= 0) lines.splice(secEnd + 1, 0, `${key} = ${value}`);
    else lines.push(...(lines.length ? [''] : []), `[${section}]`, `${key} = ${value}`);
  }
  return lines.length ? lines.join('\n') + '\n' : '';
}

function viewOf(name: string): ConfView {
  const text = textOf(name);
  const managed = new Set(DEFS.map((d) => `${d.section}.${d.key}`.toLowerCase()));
  const other: string[] = [];
  let cur = '';
  for (const line of (text ?? '').split('\n')) {
    const sec = /^\s*\[([^\]]+)\]\s*$/.exec(line);
    if (sec && sec[1] !== undefined) { cur = sec[1]; continue; }
    const m = KEYLINE.exec(line);
    if (m && m[1] !== undefined && !managed.has(`${cur}.${m[1]}`.toLowerCase())) other.push(`${cur}.${m[1]}`);
  }
  return {
    distro: name, path: '/etc/wsl.conf', exists: text !== null, malformed: [],
    settings: DEFS.map((d) => ({ ...d, value: getVal(text, d.section, d.key) })),
    other_keys: other, fingerprint: fp(text), size: text?.length ?? 0, editable: true,
    notes: text === null ? ["The distribution has no /etc/wsl.conf. WSL's defaults apply; saving a change creates the file."] : [],
  };
}

const RESTART_NOTE = 'A saved change takes effect only after this distribution restarts. VLINUX will not restart or stop it for you, and it cannot confirm the change has taken effect until you have restarted it.';

async function accessible(name: string): Promise<boolean> {
  const report = await detect();
  const d = report.distros.find((x) => x.name === name);
  if (!d) throw new Error(`No distribution named '${name}' is registered with WSL.`);
  if (d.kind === 'internal') throw new Error(`'${name}' belongs to another application and is never touched by VLINUX.`);
  return d.state === 'running';
}

export async function confRead(name: string): Promise<WslConfRead> {
  if (!(await accessible(name))) return { kind: 'needs_start', distro: name };
  await wait(150);
  return { kind: 'view', view: viewOf(name), wsl_version: (await detect()).distros.find((d) => d.name === name)?.wsl_version ?? 2 };
}

function applyChanges(name: string, changes: SettingChange[]): string {
  let text = textOf(name);
  for (const c of changes) {
    const def = DEFS.find((d) => d.id === c.id);
    if (!def) throw new Error(`'${c.id}' is not a setting VLINUX manages.`);
    if (c.value !== null) {
      if (def.kind === 'bool' && c.value !== 'true' && c.value !== 'false') throw new Error(`'${def.label}' must be true or false.`);
      if (def.kind === 'path' && !/^\/[A-Za-z0-9_./-]*$/.test(c.value)) throw new Error(`'${def.label}' must be an absolute Linux folder such as /mnt/ (no spaces, quotes, '..' or '#').`);
      if (def.kind === 'username' && !/^[a-z_][a-z0-9_-]{0,31}$/.test(c.value)) throw new Error(`'${def.label}' must be a valid Linux user name (lower-case letters, digits, '_' and '-').`);
    }
    text = setVal(text, def.section, def.key, c.value);
  }
  return text ?? '';
}

function diff(oldText: string, newText: string): DiffLine[] {
  // Plain longest-common-subsequence line diff (files here are tiny).
  const a = oldText.split('\n');
  const b = newText.split('\n');
  const dp: number[][] = Array.from({ length: a.length + 1 }, () => new Array<number>(b.length + 1).fill(0));
  for (let i = a.length - 1; i >= 0; i--) {
    for (let j = b.length - 1; j >= 0; j--) {
      dp[i]![j] = a[i] === b[j] ? dp[i + 1]![j + 1]! + 1 : Math.max(dp[i + 1]![j]!, dp[i]![j + 1]!);
    }
  }
  const out: DiffLine[] = [];
  let i = 0;
  let j = 0;
  while (i < a.length || j < b.length) {
    if (i < a.length && j < b.length && a[i] === b[j]) {
      out.push({ kind: 'same', text: a[i]! });
      i++;
      j++;
    } else if (j < b.length && (i === a.length || dp[i]![j + 1]! >= dp[i + 1]![j]!)) {
      out.push({ kind: 'add', text: b[j]! });
      j++;
    } else {
      out.push({ kind: 'remove', text: a[i]! });
      i++;
    }
  }
  // Keep two lines of context around each change, collapse the rest.
  const keep = out.map((l, k) => l.kind !== 'same' || out.slice(Math.max(0, k - 2), k + 3).some((x) => x.kind !== 'same'));
  const res: DiffLine[] = [];
  out.forEach((l, k) => {
    if (keep[k]) res.push(l);
    else if (res.length && res[res.length - 1]!.kind !== 'gap') res.push({ kind: 'gap', text: '…' });
  });
  return out.some((l) => l.kind !== 'same') ? res : [{ kind: 'same', text: '(no change)' }];
}

export async function confPreview(name: string, changes: SettingChange[]): Promise<ConfPlan> {
  await accessible(name);
  const cur = textOf(name);
  const next = applyChanges(name, changes);
  const warnings: string[] = [];
  if (changes.some((c) => c.id === 'boot.systemd')) warnings.push('Changing systemd affects every service in this distribution after its next start.');
  if (changes.some((c) => c.id === 'automount.root' || c.id === 'automount.enabled')) warnings.push('VLINUX shared-folder links point into the drive mount folder; re-check them in Settings after changing this.');
  return {
    distro: name, based_on: fp(cur), creates_file: cur === null, new_text: next,
    diff: diff(cur ?? '', next),
    summary: changes.map((c) => `${DEFS.find((d) => d.id === c.id)?.label ?? c.id}: ${c.value === null ? "remove (WSL's default applies)" : c.value}`),
    warnings, restart_note: RESTART_NOTE,
  };
}

export async function confApply(name: string, changes: SettingChange[], basedOn: string, confirmed: boolean): Promise<ApplyOutcome> {
  if (!confirmed) throw new Error('The change was not confirmed. Nothing was written.');
  await accessible(name);
  const cur = textOf(name);
  if (fp(cur) !== basedOn) throw new Error('/etc/wsl.conf changed after the preview. Review the change again.');
  const next = applyChanges(name, changes);
  if (SIM[name]?.readonly) {
    return {
      kind: 'not_writable', reason: 'Permission denied: /etc/wsl.conf can only be written by root in this distribution. (Simulated.)', proposed: next,
      instructions: 'VLINUX does not use administrator or root access. To make this change yourself, open a terminal in this distribution and edit /etc/wsl.conf with sudo (for example `sudo nano /etc/wsl.conf`), pasting the proposed content shown here. Then restart the distribution.',
    };
  }
  const id = `bak-${++seq}`;
  const list = backups.get(name) ?? [];
  list.push({ id, at: now(), existed: cur !== null, content: cur ?? '', writtenFp: fp(next), restoredAt: null });
  backups.set(name, list);
  confText.set(name, next);
  return { kind: 'applied', backup_id: id, verified_on_disk: true, verified_active: false, message: `Saved to /etc/wsl.conf and read back successfully. ${RESTART_NOTE}` };
}

export async function confBackups(name: string): Promise<BackupInfo[]> {
  return (backups.get(name) ?? [])
    .map((b) => ({ id: b.id, taken_at: b.at, existed: b.existed, applied: b.writtenFp !== null, restored_at: b.restoredAt, bytes: b.content.length }))
    .sort((a, b) => b.taken_at - a.taken_at || (a.id < b.id ? 1 : -1));
}

export async function confRestore(name: string, id: string, force: boolean, confirmed: boolean): Promise<ApplyOutcome> {
  if (!confirmed) throw new Error('The restore was not confirmed. Nothing was changed.');
  await accessible(name);
  const list = backups.get(name) ?? [];
  const b = list.find((x) => x.id === id);
  if (!b) throw new Error('That backup no longer exists.');
  const cur = textOf(name);
  if (b.writtenFp !== null && fp(cur) !== b.writtenFp && !force) throw new Error("The file has changed since VLINUX wrote it, so restoring would discard someone else's edit. Nothing was changed. You can force the restore if you are sure.");
  const safety = `bak-${++seq}`;
  list.push({ id: safety, at: now(), existed: cur !== null, content: cur ?? '', writtenFp: null, restoredAt: null });
  confText.set(name, b.existed ? b.content : null);
  b.restoredAt = now();
  return { kind: 'applied', backup_id: safety, verified_on_disk: true, verified_active: false, message: `Restored. The state before the restore was saved as a new backup, so this can be undone. ${RESTART_NOTE}` };
}

export async function windowsConfig(): Promise<WindowsConfig> {
  await wait(80);
  return {
    display_path: '%USERPROFILE%\\.wslconfig', exists: true, malformed: [],
    resources: [
      { label: 'Memory limit for WSL 2', value: '8GB', note: '' },
      { label: 'Processor limit for WSL 2', value: null, note: 'Not set: WSL 2 may use all logical processors.' },
      { label: 'Swap size', value: null, note: "Not set: WSL's default swap applies." },
      { label: 'Localhost forwarding', value: null, note: "Not set: WSL's default applies." },
    ],
    entries: [{ section: 'wsl2', key: 'memory', value: '8GB' }],
    host_processors: navigator.hardwareConcurrency || null,
    notes: [
      'This file is shared by every distribution and is owned by Windows, not by a distribution.',
      'VLINUX only reads it. Changing it needs the whole WSL VM to restart (`wsl --shutdown`), which VLINUX never does, so edit it yourself if you want to change it.',
      'SIMULATED: this is example data from the browser preview, not your .wslconfig.',
    ],
    read_error: null,
  };
}

// ---- diagnostics (always simulated here) -------------------------------------------------------

interface Row { id: string; category: DiagCategory; title: string; checked: string; writes?: boolean; running?: boolean; host?: boolean; note?: string }
const ROWS: Row[] = [
  { id: 'runtime.wsl_available', category: 'runtime', title: 'WSL is available', checked: 'Whether wsl.exe answered and reports a usable WSL.' },
  { id: 'runtime.wsl_version', category: 'runtime', title: 'WSL version', checked: 'Whether `wsl --version` reports the WSL and kernel versions.' },
  { id: 'runtime.distros_listed', category: 'runtime', title: 'Distributions listed', checked: 'Whether WSL lists at least one distribution that VLINUX may use.' },
  { id: 'runtime.distro_state', category: 'runtime', title: 'Selected distribution', checked: 'Whether the selected distribution is registered, usable, and what state it is in.' },
  { id: 'runtime.exec', category: 'runtime', title: 'Commands run inside the distribution', checked: 'A fixed read-only command is run through wsl.exe and its output must come back exactly.', running: true },
  { id: 'terminal.pty_create', category: 'terminal', title: 'Terminal session opens', checked: 'A throw-away pseudo-terminal session is opened in the distribution.', writes: true, running: true },
  { id: 'terminal.io', category: 'terminal', title: 'Input and output', checked: 'Typed input reaches the shell and its output comes back.', writes: true, running: true },
  { id: 'terminal.ansi', category: 'terminal', title: 'Colour and control sequences', checked: 'ANSI escape sequences pass through unchanged.', writes: true, running: true },
  { id: 'terminal.unicode', category: 'terminal', title: 'Unicode', checked: 'A non-ASCII character survives the round trip.', writes: true, running: true },
  { id: 'terminal.resize', category: 'terminal', title: 'Resize', checked: 'Resizing the terminal is seen by the shell.', writes: true, running: true },
  { id: 'terminal.close', category: 'terminal', title: 'Session closes', checked: 'The session ends and no process of the diagnostics terminal is left.', writes: true, running: true },
  { id: 'fs.share_access', category: 'filesystem', title: 'File share reachable', checked: "Whether the distribution's files can be reached from Windows and the root listed.", running: true },
  { id: 'fs.enumerate', category: 'filesystem', title: 'Folder listing', checked: 'Listing /etc through the same code the Files app uses.', running: true },
  { id: 'fs.create_dir', category: 'filesystem', title: 'Create a folder', checked: 'Creates a temporary folder /tmp/vlinux-diag-<token>.', writes: true, running: true },
  { id: 'fs.create_file', category: 'filesystem', title: 'Create a file', checked: 'Creates an empty file in the diagnostic folder.', writes: true, running: true },
  { id: 'fs.rename', category: 'filesystem', title: 'Rename', checked: 'Renames a file and checks the old name is gone.', writes: true, running: true },
  { id: 'fs.copy', category: 'filesystem', title: 'Copy', checked: 'Copies a file into a sub-folder and compares the content.', writes: true, running: true },
  { id: 'fs.move', category: 'filesystem', title: 'Move', checked: 'Moves the copy into another sub-folder.', writes: true, running: true },
  { id: 'fs.symlink', category: 'filesystem', title: 'Symbolic link detection', checked: 'Creates a link inside the distribution and checks it is recognised as a link.', writes: true, running: true },
  { id: 'fs.permission_error', category: 'filesystem', title: 'Permission errors are reported', checked: 'Listing /root as the default user must be refused and reported as a permission error.', running: true },
  { id: 'fs.cleanup', category: 'filesystem', title: 'Clean-up', checked: 'Removes only the temporary diagnostic folder this run created.', writes: true, running: true },
  { id: 'shared.known_folders', category: 'shared_folders', title: 'Windows Known Folders', checked: 'Whether Windows reports the locations of Downloads, Documents, Pictures, Music, Videos and Desktop.', host: true },
  { id: 'shared.mount_root', category: 'shared_folders', title: 'Drive mount root', checked: 'Reads /etc/wsl.conf for the drive mount root and whether automount is enabled.', running: true },
  { id: 'shared.mount_access', category: 'shared_folders', title: 'Drive mounts reachable', checked: 'Lists the drive mount root to see that Windows drives are mounted.', running: true },
  { id: 'shared.link_roundtrip', category: 'shared_folders', title: 'Link create / verify / reverse', checked: 'Creates a link with the same operations the mapping feature uses, verifies it, removes it.', writes: true, running: true },
  { id: 'shared.conflict_detection', category: 'shared_folders', title: 'Mapping state evaluation', checked: "Evaluates each known folder's current state (read-only).", running: true },
  { id: 'desktop.explorer', category: 'desktop', title: 'Windows Explorer', checked: "Whether explorer.exe exists.", host: true },
  { id: 'desktop.path_logic', category: 'desktop', title: 'Windows to WSL path conversion', checked: "Converts a sample drive path and a network path with VLINUX's own rules.", host: true },
  { id: 'desktop.wslpath', category: 'desktop', title: 'wslpath agrees', checked: 'Asks the distribution (`wslpath -u`) to convert a Windows folder and compares it.', running: true },
  { id: 'desktop.unusual_names', category: 'desktop', title: 'Unusual file names', checked: 'Creates files with spaces and symbols in their names and checks they list back exactly.', writes: true, running: true },
  { id: 'desktop.file_association', category: 'desktop', title: 'File associations', checked: 'Whether Windows would open a file type with the expected application.', host: true, note: 'VLINUX has no safe, read-only way to query Windows file associations yet, so this is not tested.' },
];

let diagCancel = false;
let diagBusy = false;

export async function diagRun(o: DiagOptions): Promise<DiagReport> {
  if (diagBusy) throw new Error('A diagnostics run is already in progress.');
  diagBusy = true;
  diagCancel = false;
  try {
    const report = await detect();
    const d = o.distro ? report.distros.find((x) => x.name === o.distro) : undefined;
    const usable = !!d && d.kind === 'regular';
    let running = d?.state === 'running';
    const results: CheckResult[] = [];
    let index = 0;
    const runId = `preview-diag-${++seq}`;
    const cats = o.categories;
    for (const row of ROWS) {
      if (cats && !cats.includes(row.category) && row.category !== 'runtime' && !(row.id.startsWith('fs.') && (row.id === 'fs.share_access' || row.id === 'fs.create_dir' || row.id === 'fs.cleanup') && (cats.includes('shared_folders') || cats.includes('desktop')))) continue;
      await wait(35);
      let outcome: CheckResult['outcome'] = 'passed';
      let message = 'Passed (simulated).';
      let guidance: string | null = null;
      let modified = false;
      if (diagCancel) { outcome = 'not_tested'; message = 'The run was cancelled before this check.'; }
      else if (row.writes && !o.allow_write) { outcome = 'not_tested'; message = 'Needs your permission to create files in a temporary folder inside the distribution (not given for this run).'; }
      else if (row.note) { outcome = 'not_tested'; message = row.note; }
      else if (row.id === 'desktop.explorer') { outcome = 'unsupported'; message = 'This host has no Windows Explorer.'; }
      else if (row.id === 'runtime.distro_state' && !o.distro) { outcome = 'not_tested'; message = 'No distribution was selected for this run.'; }
      else if (row.id === 'runtime.distro_state' && !usable) { outcome = 'failed'; message = `'${o.distro}' is not a usable distribution.`; guidance = 'Choose another distribution.'; }
      else if (row.id === 'runtime.distro_state') { message = `'${o.distro}' is ${running ? 'running' : 'stopped'}.`; }
      else if (row.running && !o.distro) { outcome = 'not_tested'; message = 'No distribution was selected for this run.'; }
      else if (row.running && !usable) { outcome = 'skipped'; message = 'The selected distribution is not usable (see the Runtime checks).'; }
      else if (row.running && !running && o.allow_start && row.id === 'runtime.exec') {
        running = true;
        modified = true;
        await distroState(o.distro!, 'running');
        message = 'Started and a command ran (simulated).';
      }
      else if (row.running && !running) { outcome = 'not_tested'; message = 'The distribution is stopped, and starting it was not allowed for this run.'; }
      else if (row.id === 'fs.permission_error') { message = 'Access to /root was refused and reported as a permission error (simulated).'; }
      else if (row.id === 'shared.known_folders') { message = '6 of 6 resolved (simulated).'; }
      if (outcome === 'passed' && row.writes) modified = true;
      const evidence = outcome === 'passed' || outcome === 'failed' ? ('simulated' as const) : ('none' as const);
      const r: CheckResult = { id: row.id, category: row.category, title: row.title, checked: row.checked, distro: row.host ? null : o.distro, outcome, evidence, modified: evidence === 'none' ? false : modified, message, guidance, ms: 35 };
      results.push(r);
      diagListeners.forEach((l) => l({ run_id: runId, index: ++index, result: r }));
    }
    const count = (x: CheckResult['outcome']) => results.filter((r) => r.outcome === x).length;
    const summary = { passed: count('passed'), failed: count('failed'), skipped: count('skipped'), unsupported: count('unsupported'), not_tested: count('not_tested'), real_wsl_passed: 0 };
    const text = [
      'VLINUX compatibility report',
      'Run type: SIMULATED — no real WSL was contacted. These results say NOTHING about this PC.',
      `Distribution: ${o.distro ?? '(none selected)'}`,
      `Permissions for this run: start distribution = ${o.allow_start ? 'yes' : 'no'}, create temporary files = ${o.allow_write ? 'yes' : 'no'}`,
      `Summary: ${summary.passed} passed, ${summary.failed} failed, ${summary.skipped} skipped, ${summary.unsupported} unsupported, ${summary.not_tested} not tested (0 passed against the real WSL)`,
      ...(diagCancel ? ['The run was cancelled; the remaining checks are marked NOT TESTED.'] : []),
      '',
      ...results.map((r) => `${r.outcome.toUpperCase().replace('_', ' ').padEnd(11)} ${r.title} — ${r.message} [${r.evidence === 'simulated' ? 'SIMULATED' : 'did not run'}]`),
    ].join('\n');
    return { generated_at: now(), mode: 'simulated', distro: o.distro, allow_write: o.allow_write, allow_start: o.allow_start, results, summary, text, cancelled: diagCancel, diag_dir: null };
  } finally {
    diagBusy = false;
  }
}

export async function diagCancelRun(): Promise<boolean> {
  diagCancel = true;
  return diagBusy;
}

export const available = () => scenario() === 'multi' || scenario() === 'ready';
