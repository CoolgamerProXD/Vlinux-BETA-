// Mirrors the JSON produced by the `vlinux-runtime` crate (src-tauri/crates/runtime).
// The UI never sees or builds a command line: it only displays these structures and sends the
// few typed requests defined in ipc.ts.

export type DistroState = 'running' | 'stopped' | 'installing' | 'uninstalling' | 'converting' | 'unknown';

export interface AdapterInfo {
  id: string;
  display_name: string;
  package_manager: 'apt' | 'pacman' | 'dnf' | 'unknown';
  package_manager_name: string;
  rolling_release: boolean;
  notes: string;
}

export interface Distro {
  name: string;
  state: DistroState;
  wsl_version: number;
  is_default: boolean;
  /** `internal` = owned by another application (for example Docker Desktop); never touched. */
  kind: 'regular' | 'internal';
  origin: 'existing' | 'installed_by_vlinux';
  adapter: AdapterInfo;
  identity_source: 'registered_name' | 'os_release';
  os_release: { id: string; pretty_name: string | null; version_id: string | null } | null;
  display_label: string;
}

export type RuntimeStatus =
  | { kind: 'unsupported_host'; reason: string }
  | { kind: 'not_installed' }
  | { kind: 'restart_required' }
  | { kind: 'feature_problem'; code: string | null; hint: string }
  | { kind: 'no_distributions' }
  | { kind: 'ready' }
  | { kind: 'error'; message: string };

export interface WslReport {
  status: RuntimeStatus;
  host: { is_windows: boolean; windows_build: number | null };
  wsl_version: string | null;
  kernel_version: string | null;
  wslg_version: string | null;
  default_wsl_version: number | null;
  wsl2_available: boolean | null;
  distros: Distro[];
  default_distro: string | null;
  suggested_distro: string | null;
  pending_reboot: boolean | null;
  warnings: string[];
  detected_at: number;
}

export interface CatalogEntry {
  wsl_name: string;
  friendly_name: string;
  supported: boolean;
  adapter: AdapterInfo | null;
  already_installed: boolean;
}

export interface Catalog {
  entries: CatalogEntry[];
  error: string | null;
  fetched_at: number;
}

export interface InstallPlan {
  plan_id: string;
  kind: 'distribution' | 'wsl_platform';
  wsl_name: string | null;
  display_name: string;
  adapter: AdapterInfo | null;
  command_preview: string;
  steps: string[];
  requires_admin: 'never' | 'may_prompt';
  restart_possible: boolean;
  needs_internet: boolean;
  existing_untouched: string[];
  blockers: string[];
}

export type InstallOutcome =
  | { kind: 'succeeded' }
  | { kind: 'restart_required'; message: string }
  | { kind: 'failed'; exit_code: number | null; message: string; hint: string }
  | { kind: 'cancelled' }
  | { kind: 'timed_out' };

export interface InstallEvent {
  job_id: string;
  kind: 'progress' | 'finished';
  message: string;
  percent: number | null;
  outcome: InstallOutcome | null;
}

export type Verdict =
  | { kind: 'supported'; wsl_path: string }
  | { kind: 'warning'; wsl_path: string; message: string }
  | { kind: 'unsupported'; message: string };

export interface FolderStatus {
  id: string;
  label: string;
  linux_path: string;
  mode: 'shared' | 'private';
  custom_windows_path: string | null;
  default_windows_path: string | null;
  effective_windows_path: string | null;
  verdict: Verdict | null;
  points_to: string;
}

export interface StorageOverview {
  folders: FolderStatus[];
  projects: 'linux_private' | 'shared';
  mount_root: string;
  notes: string[];
}

/** Text for the one-line status headline. */
export function statusHeadline(s: RuntimeStatus): string {
  switch (s.kind) {
    case 'unsupported_host': return 'This PC cannot run the WSL backend';
    case 'not_installed': return 'WSL is not installed';
    case 'restart_required': return 'Windows needs a restart';
    case 'feature_problem': return 'WSL needs attention';
    case 'no_distributions': return 'No Linux distribution is installed';
    case 'ready': return 'WSL is ready';
    case 'error': return 'WSL did not respond properly';
  }
}
