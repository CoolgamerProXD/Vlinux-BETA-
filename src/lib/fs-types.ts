// Types and pure helpers for the file manager (M4). They mirror the Rust serialisation in
// `vlinux-runtime::fs` and `vlinux-runtime::shared`. The UI never builds a host path: a place is
// either a Linux path inside the chosen distribution or a shared Windows folder id plus relative
// names, and the backend validates both.

export type Location =
  | { space: 'linux'; path: string }
  | { space: 'windows'; root: string; rel: string[] };

export type FsErrorKind =
  | 'not_found' | 'permission_denied' | 'already_exists' | 'not_a_directory' | 'is_a_directory'
  | 'not_empty' | 'invalid_path' | 'invalid_name' | 'outside_root' | 'unavailable' | 'refused'
  | 'cancelled' | 'io';

export interface FsError {
  kind: FsErrorKind;
  message: string;
}

/** Whatever `invoke` rejected with, as an `FsError`. */
export function asFsError(e: unknown): FsError {
  if (e && typeof e === 'object' && 'kind' in e && 'message' in e) {
    const o = e as { kind: unknown; message: unknown };
    if (typeof o.kind === 'string' && typeof o.message === 'string') return { kind: o.kind as FsErrorKind, message: o.message };
  }
  return { kind: 'io', message: typeof e === 'string' ? e : e instanceof Error ? e.message : String(e) };
}

export type EntryKind = 'file' | 'dir' | 'symlink' | 'other';

export interface Entry {
  name: string;
  kind: EntryKind;
  size: number | null;
  /** Seconds since the Unix epoch. */
  modified: number | null;
  hidden: boolean;
  readonly: boolean;
  class: { id: string; label: string };
  open: { allowed: boolean; reason: string | null };
  link_target: string | null;
  link_is_dir: boolean | null;
}

export interface Crumb { label: string; location: Location }
export interface Page { offset: number; total: number; entries: Entry[] }

export interface ListingHead {
  id: string;
  location: Location;
  resolved: Location;
  crumbs: Crumb[];
  parent: Location | null;
  total: number;
  truncated: boolean;
  unreadable: number;
  page: Page;
}

export type SortKey = 'name' | 'size' | 'modified';
export interface ListOptions { show_hidden: boolean; sort: SortKey; descending: boolean }

export interface Place {
  id: string;
  label: string;
  location: Location;
  detail: string | null;
  /** Verified live: this Linux folder IS the Windows folder. Never just "preferred". */
  linked: boolean;
  unc: boolean;
  warning: string | null;
  exists: boolean;
}

export interface Places {
  distro: string | null;
  linux_available: boolean;
  linux_note: string | null;
  home: Place | null;
  roots: Place[];
  folders: Place[];
  shared: Place[];
}

export type TransferMode = 'copy' | 'move';
export type Conflict = 'keep_both' | 'skip' | 'fail';

export interface Progress { files: number; bytes: number; current: string; skipped: number; failed: number }
export interface ItemFailure { path: string; message: string }
export interface OpReport {
  items_done: number;
  files: number;
  bytes: number;
  skipped: string[];
  failed: ItemFailure[];
  cancelled: boolean;
  results: Location[];
}

export interface JobEvent {
  job_id: string;
  kind: 'delete' | 'copy' | 'move';
  state: 'running' | 'done' | 'failed' | 'cancelled';
  progress: Progress | null;
  report: OpReport | null;
  error: FsError | null;
}

// ---- Shared-folder mappings ------------------------------------------------------------------

export type Resolution = 'replace_empty_folder' | 'replace_link' | 'move_aside';
export interface Offer { resolution: Resolution; label: string; description: string }

export type MappingState =
  | { state: 'unknown' }
  | { state: 'private' }
  | { state: 'ready'; needs: Resolution | null; note: string }
  | { state: 'applied'; target: string }
  | { state: 'conflict'; kind: string; detail: string; entries: string[]; more: number; offers: Offer[] }
  | { state: 'linked_but_private'; target: string }
  | { state: 'unavailable'; reason: string }
  | { state: 'failed'; message: string };

export interface MappingStatus {
  folder: string;
  label: string;
  linux_path: string;
  preference: 'shared' | 'private';
  windows_path: string | null;
  target: string | null;
  warning: string | null;
  state: MappingState;
  backup: string | null;
  applied_at: number | null;
}

export interface MappingReport {
  distro: string;
  checked: boolean;
  home: string | null;
  mount_root: string;
  automount: boolean;
  folders: MappingStatus[];
}

export interface MappingOutcome { message: string; status: MappingStatus }

/** Plain-language badge for a mapping: preferred / applied / pending user action / failed. */
export function mappingBadge(s: MappingStatus | undefined, preferred: 'shared' | 'private'): { text: string; tone: 'ok' | 'info' | 'warn' | 'bad' | 'idle' } {
  if (!s) return { text: preferred === 'shared' ? 'Preferred · not checked' : 'Linux-private', tone: 'idle' };
  switch (s.state.state) {
    case 'applied': return { text: 'Connected', tone: 'ok' };
    case 'ready': return s.state.needs ? { text: 'Pending · needs your confirmation', tone: 'warn' } : { text: 'Ready to apply', tone: 'info' };
    case 'conflict': return { text: 'Pending · needs your decision', tone: 'warn' };
    case 'unavailable': return { text: 'Unavailable', tone: 'bad' };
    case 'failed': return { text: 'Failed', tone: 'bad' };
    case 'linked_but_private': return { text: 'Still linked · undo to make private', tone: 'warn' };
    case 'private': return { text: 'Linux-private', tone: 'idle' };
    case 'unknown': return { text: 'Preferred · not checked', tone: 'idle' };
  }
}

// ---- Helpers ---------------------------------------------------------------------------------

export function locKey(l: Location): string {
  return l.space === 'linux' ? `linux:${l.path}` : `windows:${l.root}:${l.rel.join('/')}`;
}

export function sameLoc(a: Location | null, b: Location | null): boolean {
  return !!a && !!b && locKey(a) === locKey(b);
}

export function childOf(l: Location, name: string): Location {
  if (l.space === 'windows') return { space: 'windows', root: l.root, rel: [...l.rel, name] };
  return { space: 'linux', path: (l.path === '/' ? '' : l.path.replace(/\/+$/, '')) + '/' + name };
}

export function parentOf(l: Location): Location | null {
  if (l.space === 'windows') return l.rel.length ? { space: 'windows', root: l.root, rel: l.rel.slice(0, -1) } : null;
  const parts = l.path.split('/').filter(Boolean);
  if (!parts.length) return null;
  return { space: 'linux', path: '/' + parts.slice(0, -1).join('/') };
}

export function isDirEntry(e: Entry): boolean {
  return e.kind === 'dir' || (e.kind === 'symlink' && e.link_is_dir === true);
}

export function formatSize(n: number | null | undefined): string {
  if (n === null || n === undefined) return '—';
  if (n < 1024) return `${n} B`;
  const units = ['KB', 'MB', 'GB', 'TB'];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v >= 100 ? v.toFixed(0) : v >= 10 ? v.toFixed(1) : v.toFixed(2)} ${units[i]}`;
}

export function formatDate(sec: number | null | undefined, now = Date.now()): string {
  if (sec === null || sec === undefined) return '—';
  const d = new Date(sec * 1000);
  const sameYear = d.getFullYear() === new Date(now).getFullYear();
  return d.toLocaleString(undefined, sameYear
    ? { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }
    : { year: 'numeric', month: 'short', day: 'numeric' });
}

/**
 * A path typed in the location bar. Only Linux paths can be typed (`/…` or `~/…`): Windows places
 * are reached through the sidebar, because the UI never sends raw Windows paths.
 */
export function parseTypedPath(text: string, home: string | null): Location | { error: string } {
  let t = text.trim();
  if (!t) return { error: 'Type a path such as ~/Documents or /etc.' };
  if (/^[A-Za-z]:[\\/]/.test(t) || t.startsWith('\\\\')) {
    return { error: 'Windows folders are opened from the Windows section of the sidebar; type a Linux path here.' };
  }
  if (t === '~' || t.startsWith('~/')) {
    if (!home) return { error: 'The home folder is not known yet.' };
    t = home.replace(/\/+$/, '') + t.slice(1);
  }
  if (!t.startsWith('/')) return { error: 'Paths start with / or ~.' };
  if (/[\0-\x1f]/.test(t) || t.includes('\\')) return { error: 'That path contains characters that are not allowed.' };
  const parts = t.split('/').filter((p) => p && p !== '.');
  if (parts.includes('..')) return { error: '“..” is not allowed in a typed path.' };
  return { space: 'linux', path: '/' + parts.join('/') };
}

/** Name for the "new folder" prompt: avoids what is already listed. */
export function suggestName(base: string, taken: Iterable<string>): string {
  const set = new Set(taken);
  if (!set.has(base)) return base;
  for (let i = 2; i < 1000; i++) {
    const n = `${base} ${i}`;
    if (!set.has(n)) return n;
  }
  return base;
}

/** Quick client-side check, mirroring the backend (which is the real authority). */
export function nameProblem(name: string): string | null {
  if (!name.trim()) return 'Give it a name.';
  if (name === '.' || name === '..') return '“.” and “..” are not valid names.';
  if (/[\\/\0]/.test(name)) return 'Names cannot contain / or \\.';
  if (/[\0-\x1f]/.test(name)) return 'Names cannot contain control characters.';
  if (name.length > 255) return 'That name is too long.';
  return null;
}
