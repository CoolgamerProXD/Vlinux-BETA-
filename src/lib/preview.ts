// Browser-preview implementations of the runtime commands.
//
// Used ONLY when the UI runs outside the desktop app (UI development, automated screenshots).
// Everything here is SIMULATED: no WSL is queried and nothing is ever installed. The scenario
// is chosen with `?wsl=missing|empty|ready|reboot|multi` (`multi` = six distributions with different
// package managers and states, for the Distribution Manager) (default: "unavailable", which honestly says
// there is no WSL in a browser). The desktop app never loads this module.

import type {
  AdapterInfo, Catalog, Distro, InstallEvent, InstallPlan, StorageOverview, Verdict, WslReport, FolderStatus,
} from './runtime-types';
import type { Config } from './types';
import { FOLDER_IDS } from './types';

export type Scenario = 'unavailable' | 'missing' | 'empty' | 'ready' | 'reboot' | 'multi';

export function scenario(): Scenario {
  const q = typeof location === 'undefined' ? '' : new URLSearchParams(location.search).get('wsl');
  return q === 'missing' || q === 'empty' || q === 'ready' || q === 'reboot' || q === 'multi' ? q : 'unavailable';
}

export const SIMULATION_NOTE =
  'SIMULATED PREVIEW DATA — this is the browser preview. No WSL was queried and nothing can be installed here.';

const adapters: Record<string, AdapterInfo> = {
  arch: { id: 'arch', display_name: 'Arch Linux', package_manager: 'pacman', package_manager_name: 'pacman', rolling_release: true, notes: 'Rolling release.' },
  ubuntu: { id: 'ubuntu', display_name: 'Ubuntu', package_manager: 'apt', package_manager_name: 'APT', rolling_release: false, notes: '' },
  debian: { id: 'debian', display_name: 'Debian', package_manager: 'apt', package_manager_name: 'APT', rolling_release: false, notes: '' },
  fedora: { id: 'fedora', display_name: 'Fedora', package_manager: 'dnf', package_manager_name: 'DNF', rolling_release: false, notes: '' },
  generic: { id: 'generic', display_name: 'Other Linux', package_manager: 'unknown', package_manager_name: 'unknown', rolling_release: false, notes: '' },
};

const ad = (k: string): AdapterInfo => adapters[k] ?? adapters.generic!;

const distro = (name: string, label: string, adapter: string, state: Distro['state'], over: Partial<Distro> = {}): Distro => ({
  name, state, wsl_version: 2, is_default: false, kind: 'regular', origin: 'existing', adapter: ad(adapter), identity_source: 'registered_name',
  os_release: null, display_label: label, ...over,
});

function baseReport(status: WslReport['status'], over: Partial<WslReport> = {}): WslReport {
  return {
    status, host: { is_windows: true, windows_build: 26100 }, wsl_version: null, kernel_version: null, wslg_version: null,
    default_wsl_version: null, wsl2_available: null, distros: [], default_distro: null, suggested_distro: null,
    pending_reboot: null, warnings: [SIMULATION_NOTE], detected_at: Math.floor(Date.now() / 1000), ...over,
  };
}

function initialReport(): WslReport {
  switch (scenario()) {
    case 'missing': return baseReport({ kind: 'not_installed' }, { pending_reboot: false });
    case 'reboot': return baseReport({ kind: 'restart_required' }, { pending_reboot: true });
    case 'empty': return baseReport({ kind: 'no_distributions' }, { wsl_version: '2.4.13.0', kernel_version: '5.15.167.4-1', default_wsl_version: 2, wsl2_available: true });
    case 'ready':
      return baseReport({ kind: 'ready' }, {
        wsl_version: '2.4.13.0', kernel_version: '5.15.167.4-1', wslg_version: '1.0.65', default_wsl_version: 2, wsl2_available: true,
        default_distro: 'archlinux', suggested_distro: 'archlinux',
        distros: [
          distro('archlinux', 'Arch Linux', 'arch', 'running', { is_default: true, identity_source: 'os_release', os_release: { id: 'arch', pretty_name: 'Arch Linux', version_id: null } }),
          distro('Ubuntu-24.04', 'Ubuntu', 'ubuntu', 'stopped'),
          distro('Debian', 'Debian', 'debian', 'stopped', { wsl_version: 1 }),
          distro('docker-desktop', 'docker-desktop', 'generic', 'running', { kind: 'internal' }),
        ],
      });
    case 'multi':
      return baseReport({ kind: 'ready' }, {
        wsl_version: '2.4.13.0', kernel_version: '5.15.167.4-1', wslg_version: '1.0.65', default_wsl_version: 2, wsl2_available: true,
        default_distro: 'archlinux', suggested_distro: 'archlinux',
        distros: [
          distro('archlinux', 'Arch Linux', 'arch', 'running', { is_default: true, identity_source: 'os_release', os_release: { id: 'arch', pretty_name: 'Arch Linux', version_id: null } }),
          distro('Ubuntu-24.04', 'Ubuntu', 'ubuntu', 'stopped'),
          distro('Debian', 'Debian', 'debian', 'running', { wsl_version: 1 }),
          distro('FedoraLinux-42', 'Fedora', 'fedora', 'stopped'),
          distro('Alpine', 'Other Linux', 'generic', 'stopped'),
          distro('docker-desktop', 'docker-desktop', 'generic', 'running', { kind: 'internal' }),
        ],
      });
    default:
      return baseReport({ kind: 'unsupported_host', reason: 'This is the browser preview, which has no WSL. Open the desktop app on Windows to see your real setup. Add ?wsl=ready, ?wsl=empty, ?wsl=missing or ?wsl=reboot to the address to preview the screens with simulated data.' }, { host: { is_windows: false, windows_build: null } });
  }
}

let current: WslReport | null = null;
const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

export async function detect(): Promise<WslReport> {
  await wait(250);
  current ??= initialReport();
  return structuredClone(current);
}

export async function catalog(): Promise<Catalog> {
  await wait(300);
  const rep = await detect();
  const mk = (wsl_name: string, friendly_name: string, adapter: string | null) => ({
    wsl_name, friendly_name, supported: adapter !== null, adapter: adapter ? ad(adapter) : null,
    already_installed: rep.distros.some((d) => d.name.toLowerCase() === wsl_name.toLowerCase()),
  });
  return {
    entries: [mk('Ubuntu', 'Ubuntu', 'ubuntu'), mk('Debian', 'Debian GNU/Linux', 'debian'), mk('FedoraLinux-42', 'Fedora Linux 42', 'fedora'), mk('archlinux', 'Arch Linux', 'arch'), mk('kali-linux', 'Kali Linux Rolling', null)],
    error: null,
    fetched_at: Math.floor(Date.now() / 1000),
  };
}

const plans = new Map<string, InstallPlan>();
let planN = 0;

export async function prepare(kind: 'distribution' | 'wsl_platform', name: string | null): Promise<InstallPlan> {
  const rep = await detect();
  const blockers: string[] = [];
  let plan: InstallPlan;
  if (kind === 'wsl_platform') {
    if (rep.status.kind === 'ready' || rep.status.kind === 'no_distributions') blockers.push('WSL is already installed and working.');
    if (rep.status.kind === 'restart_required') blockers.push('Windows already has a restart pending. Restart Windows first, then check again.');
    if (rep.status.kind === 'unsupported_host') blockers.push(rep.status.reason);
    plan = {
      plan_id: '', kind, wsl_name: null, display_name: 'Windows Subsystem for Linux', adapter: null,
      command_preview: 'wsl.exe --install --no-distribution',
      steps: [
        "Enable the Windows features WSL needs ('Windows Subsystem for Linux' and 'Virtual Machine Platform') and install the WSL kernel, using Windows' official installer.",
        'No Linux distribution is installed in this step.',
        'Windows may show an administrator (UAC) prompt and may ask you to restart afterwards.',
      ],
      requires_admin: 'may_prompt', restart_possible: true, needs_internet: true,
      existing_untouched: rep.distros.filter((d) => d.kind === 'regular').map((d) => d.name), blockers,
    };
  } else {
    const cat = await catalog();
    const entry = cat.entries.find((e) => e.wsl_name === name);
    if (!entry) blockers.push(`'${name}' is not in Microsoft's official list of installable distributions.`);
    else if (!entry.supported) blockers.push(`VLINUX has no adapter for '${name}' yet, so it is not offered for installation.`);
    if (rep.status.kind !== 'ready' && rep.status.kind !== 'no_distributions') blockers.push('WSL is not ready yet. Set up WSL first.');
    if (rep.distros.some((d) => d.name.toLowerCase() === String(name).toLowerCase())) {
      blockers.push(`A distribution named '${name}' is already installed. VLINUX never overwrites, resets or replaces an existing distribution; use it instead.`);
    }
    const display = entry?.friendly_name ?? String(name);
    plan = {
      plan_id: '', kind, wsl_name: name, display_name: display, adapter: entry?.adapter ?? null,
      command_preview: `wsl.exe --install ${name} --no-launch`,
      steps: [
        `Download ${display} from Microsoft's official WSL distribution list (internet connection required).`,
        `Register it with WSL as '${name}', side by side with your existing distributions.`,
        'Nothing is launched and no account is created in this step; first-run setup happens when the distribution is first opened.',
      ],
      requires_admin: 'never', restart_possible: false, needs_internet: true,
      existing_untouched: rep.distros.filter((d) => d.kind === 'regular').map((d) => d.name), blockers,
    };
  }
  plan.plan_id = `preview-plan-${++planN}`;
  plans.set(plan.plan_id, plan);
  return structuredClone(plan);
}

const listeners = new Set<(e: InstallEvent) => void>();
export const onInstall = (cb: (e: InstallEvent) => void) => {
  listeners.add(cb);
  return () => listeners.delete(cb);
};
const emit = (e: InstallEvent) => listeners.forEach((l) => l(e));

let cancelled = new Set<string>();
let jobN = 0;

export async function start(planId: string, approved: boolean): Promise<{ job_id: string }> {
  if (!approved) throw new Error('The installation was not approved.');
  const plan = plans.get(planId);
  if (!plan) throw new Error('This installation plan is unknown or was already used. Review it again.');
  if (plan.blockers.length) throw new Error(`This installation cannot start: ${plan.blockers.join(' ')}`);
  plans.delete(planId);
  const job_id = `preview-job-${++jobN}`;
  void (async () => {
    emit({ job_id, kind: 'progress', message: `Running ${plan.command_preview} (simulated)`, percent: null, outcome: null });
    for (let p = 10; p <= 100; p += 15) {
      await wait(450);
      if (cancelled.has(job_id)) {
        emit({ job_id, kind: 'finished', message: 'The installation was cancelled.', percent: null, outcome: { kind: 'cancelled' } });
        return;
      }
      emit({ job_id, kind: 'progress', message: `Installing: ${plan.display_name} ${Math.min(p, 100)}% (simulated)`, percent: Math.min(p, 100), outcome: null });
    }
    if (plan.kind === 'distribution' && plan.wsl_name) {
      current ??= initialReport();
      current.distros.push(distro(plan.wsl_name, plan.display_name, plan.adapter?.id ?? 'generic', 'stopped', { origin: 'installed_by_vlinux' }));
      current.status = { kind: 'ready' };
    }
    emit({ job_id, kind: 'finished', message: `${plan.display_name} was installed. (Simulated: nothing really changed.)`, percent: null, outcome: { kind: 'succeeded' } });
  })();
  return { job_id };
}

export async function cancel(jobId: string): Promise<boolean> {
  cancelled.add(jobId);
  return true;
}

export async function distroState(name: string, state: Distro['state']): Promise<WslReport> {
  await wait(300);
  current ??= initialReport();
  const d = current.distros.find((x) => x.name === name && x.kind === 'regular');
  if (!d) throw new Error(`No distribution named '${name}' is registered with WSL.`);
  d.state = state;
  return structuredClone(current);
}

// ---- storage ----

const KNOWN: Record<string, string> = { downloads: 'Downloads', documents: 'Documents', pictures: 'Pictures', videos: 'Videos', music: 'Music', desktop: 'Desktop' };

export function validatePath(path: string): Verdict {
  const p = path.trim();
  if (!p) return { kind: 'unsupported', message: 'No folder was given.' };
  if (/^(\\\\|\/\/)/.test(p)) {
    if (/^(\\\\|\/\/)(wsl\$|wsl\.localhost)[\\/]/i.test(p)) return { kind: 'unsupported', message: 'This location is inside a Linux distribution. Sharing it back into Linux would be circular.' };
    return { kind: 'unsupported', message: 'This is a network (UNC) location. WSL cannot reach it through a normal drive mount; choose a folder on a local drive.' };
  }
  const m = /^([A-Za-z]):[\\/](.*)$/.exec(p);
  if (!m || m[1] === undefined) return { kind: 'unsupported', message: 'Only absolute paths on a drive letter (for example D:\\Folder) are supported.' };
  const parts = (m[2] ?? '').split(/[\\/]/).filter((x) => x && x !== '.');
  if (!parts.length) return { kind: 'unsupported', message: 'A whole drive cannot be used as a mapping target; choose a folder on it.' };
  if (parts.includes('..')) return { kind: 'unsupported', message: "'..' is not allowed in a mapped path." };
  if (parts.some((x) => /[<>:"|?*]/.test(x))) return { kind: 'unsupported', message: 'The path contains characters Windows does not allow in names.' };
  return { kind: 'supported', wsl_path: `/mnt/${m[1].toLowerCase()}/${parts.join('/')}` };
}

/** Where the SIMULATED Windows keeps a Known Folder (Downloads is "redirected" to another drive). */
export function previewKnownFolder(id: string): string {
  const label = KNOWN[id] ?? id;
  return id === 'downloads' ? 'D:\\Media\\Downloads' : `C:\\Users\\Preview\\${label}`;
}

export function storageOverview(cfg: Config): StorageOverview {
  const folders: FolderStatus[] = FOLDER_IDS.map((id) => {
    const pref = cfg.storage[id];
    const label = KNOWN[id] ?? id;
    const def = previewKnownFolder(id);
    const eff = pref.windows_path ?? def;
    const verdict = pref.mode === 'private' ? null : validatePath(eff);
    const points_to =
      pref.mode === 'private' ? 'Linux-private storage inside the distribution (not visible from Windows).'
      : verdict?.kind === 'unsupported' ? `Cannot be shared: ${verdict.message}`
      : `${eff} (Windows) via ${verdict && 'wsl_path' in verdict ? verdict.wsl_path : ''}`;
    return { id, label, linux_path: `~/${label}`, mode: pref.mode, custom_windows_path: pref.windows_path, default_windows_path: def, effective_windows_path: eff, verdict, points_to };
  });
  return {
    folders, projects: cfg.storage.projects, mount_root: '/mnt',
    notes: [
      'These are saved preferences. A folder is only linked to Windows after it has been checked and applied for a distribution; the status shown here comes from that check, never from the preference alone. Existing Linux folders are never replaced, merged or deleted without an explicit choice.',
      'Linux-native storage is recommended for development projects: reading and writing files through /mnt is slower than the distribution\'s own filesystem.',
      SIMULATION_NOTE,
    ],
  };
}
