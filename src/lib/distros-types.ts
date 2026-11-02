// Mirrors the JSON produced by the Distribution Manager commands (src-tauri/src/distros.rs and the
// `vlinux-runtime` crate). The UI only displays these structures and sends the few typed requests
// in ipc.ts: it never builds a command line, a path, or file text for the backend.

import type { AdapterInfo, DistroState, RuntimeStatus, WslReport } from './runtime-types';

export type CardState = 'detected' | 'running' | 'stopped' | 'starting' | 'installing' | 'inspecting' | 'unavailable' | 'failed';

export type ActionId =
  | 'open_terminal' | 'open_files' | 'start' | 'stop' | 'inspect' | 'set_preferred' | 'refresh' | 'open_config' | 'view_diagnostics';

export interface ActionState {
  action: ActionId;
  enabled: boolean;
  reason: string | null;
}

export type Support = 'available' | 'plan_only' | 'unsupported';
export interface Capability { support: Support; note: string }
export type Availability = { state: 'not_checked' } | { state: 'present'; version: string | null } | { state: 'absent' };

export interface PackageCaps {
  manager: 'apt' | 'pacman' | 'dnf' | 'unknown';
  manager_name: string;
  binary: string | null;
  availability: Availability;
  detect: Capability;
  version: Capability;
  search: Capability;
  install: Capability;
  update: Capability;
}

export type OpKind = 'start' | 'stop' | 'inspect';

export interface DistroCard {
  name: string;
  display_name: string;
  state: CardState;
  wsl_state: DistroState;
  wsl_version: number;
  is_windows_default: boolean;
  is_preferred: boolean;
  kind: 'regular' | 'internal';
  origin: 'existing' | 'installed_by_vlinux';
  adapter: AdapterInfo;
  identity_source: 'registered_name' | 'os_release';
  os_pretty: string | null;
  os_version: string | null;
  kernel: string | null;
  arch: string | null;
  default_user: string | null;
  packages: PackageCaps;
  package_notes: string[];
  inspected_at: number | null;
  busy: OpKind | null;
  failure: string | null;
  actions: ActionState[];
}

export type PreferredStatus =
  | { kind: 'none' }
  | { kind: 'ok' | 'missing' | 'internal' | 'busy' | 'unchecked'; name: string };

export interface Overview {
  runtime: RuntimeStatus;
  cards: DistroCard[];
  preferred: PreferredStatus;
  preferred_name: string | null;
  windows_default: string | null;
  warning: string | null;
  generated_at: number;
}

export interface ManagerProbe { binary: string; present: boolean; version: string | null }

export interface Inspection {
  distro: string;
  at: number;
  was_running: boolean;
  started_by_inspection: boolean;
  os_release: { id: string; name?: string | null; pretty_name: string | null; version_id: string | null } | null;
  kernel: string | null;
  arch: string | null;
  default_user: string | null;
  uid: number | null;
  managers: ManagerProbe[];
  notes: string[];
}

export type InspectOutcome = { kind: 'done'; inspection: Inspection } | { kind: 'needs_start'; distro: string };

export interface OpOutcome {
  distro: string;
  op: OpKind;
  ran: boolean;
  verified: boolean;
  message: string;
}

export interface LifecycleResult { outcome: OpOutcome; report: WslReport }

export interface DistroOpEvent {
  distro: string;
  op: OpKind;
  phase: 'started' | 'finished';
  ok: boolean;
  message: string;
}

// ---- packages ----

export interface PackagePlan {
  distro: string;
  op: 'install' | 'update';
  manager_name: string;
  packages: string[];
  commands: string[];
  requires_root: boolean;
  /** Always false: VLINUX shows the command; the user runs it in the Terminal. */
  executed: boolean;
  notes: string[];
}

export interface PackageHit { name: string; version: string | null; description: string }

// ---- configuration ----

export interface SettingValue {
  id: string;
  label: string;
  help: string;
  kind: 'bool' | 'path' | 'username';
  section: string;
  key: string;
  value: string | null;
  default_note: string;
}

export interface ConfView {
  distro: string;
  path: string;
  exists: boolean;
  malformed: number[];
  settings: SettingValue[];
  other_keys: string[];
  fingerprint: string;
  size: number;
  editable: boolean;
  notes: string[];
}

export type WslConfRead = { kind: 'view'; view: ConfView; wsl_version: number } | { kind: 'needs_start'; distro: string };

export interface SettingChange { id: string; value: string | null }

export interface DiffLine { kind: 'same' | 'add' | 'remove' | 'gap'; text: string }

export interface ConfPlan {
  distro: string;
  based_on: string;
  creates_file: boolean;
  new_text: string;
  diff: DiffLine[];
  summary: string[];
  warnings: string[];
  restart_note: string;
}

export type ApplyOutcome =
  | { kind: 'applied'; backup_id: string; verified_on_disk: boolean; verified_active: boolean; message: string }
  | { kind: 'not_writable'; reason: string; proposed: string; instructions: string };

export interface BackupInfo { id: string; taken_at: number; existed: boolean; applied: boolean; restored_at: number | null; bytes: number }

export interface WinResource { label: string; value: string | null; note: string }
export interface WindowsConfig {
  display_path: string;
  exists: boolean;
  malformed: number[];
  resources: WinResource[];
  entries: { section: string; key: string; value: string }[];
  host_processors: number | null;
  notes: string[];
  read_error: string | null;
}

// ---- diagnostics ----

export type DiagCategory = 'runtime' | 'terminal' | 'filesystem' | 'shared_folders' | 'desktop';
export type DiagOutcome = 'passed' | 'failed' | 'skipped' | 'unsupported' | 'not_tested';
export type DiagEvidence = 'real_wsl' | 'host' | 'simulated' | 'none';

export interface CheckResult {
  id: string;
  category: DiagCategory;
  title: string;
  checked: string;
  distro: string | null;
  outcome: DiagOutcome;
  evidence: DiagEvidence;
  modified: boolean;
  message: string;
  guidance: string | null;
  ms: number;
}

export interface DiagOptions {
  distro: string | null;
  allow_start: boolean;
  allow_write: boolean;
  categories: DiagCategory[] | null;
}

export interface DiagReport {
  generated_at: number;
  mode: 'real' | 'simulated';
  distro: string | null;
  allow_write: boolean;
  allow_start: boolean;
  results: CheckResult[];
  summary: { passed: number; failed: number; skipped: number; unsupported: number; not_tested: number; real_wsl_passed: number };
  text: string;
  cancelled: boolean;
  diag_dir: string | null;
}

export interface DiagProgress { run_id: string; index: number; result: CheckResult }
