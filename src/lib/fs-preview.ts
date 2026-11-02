// Browser-preview implementation of the file-manager commands.
//
// Used ONLY outside the desktop app (UI development and automated tests). Everything here is
// SIMULATED: an in-memory demo filesystem, no WSL, no Windows folder is ever read or written. It
// is only available in the `?wsl=ready` scenario for Linux places; the Windows shared folders are
// simulated in every scenario. It follows the same rules as the real backend (no overwrite, delete
// needs confirmation, links are never followed blindly, conflicts need an explicit choice) so the
// UI behaves the same, but it proves nothing about real files. The desktop app never loads this.

import {
  childOf, nameProblem,
  type Conflict, type Entry, type FsError, type JobEvent, type ListingHead, type ListOptions, type Location, type MappingOutcome,
  type MappingReport, type MappingState, type MappingStatus, type OpReport, type Page, type Place, type Places, type Resolution,
  type TransferMode,
} from './fs-types';
import { previewKnownFolder, scenario, validatePath } from './preview';
import type { Config } from './types';
import { FOLDER_IDS } from './types';

type Node =
  | { t: 'dir'; kids: Map<string, Node>; mtime: number; denied?: boolean }
  | { t: 'file'; size: number; mtime: number }
  | { t: 'link'; target: string; win?: string; mtime: number };

const HOME = '/home/preview';
const NOW = Math.floor(Date.now() / 1000);
const KNOWN_LABEL: Record<string, string> = { downloads: 'Downloads', documents: 'Documents', pictures: 'Pictures', videos: 'Videos', music: 'Music', desktop: 'Desktop' };
const SIDEBAR = ['desktop', 'documents', 'downloads', 'pictures', 'music', 'videos'];

const dir = (kids: Record<string, Node> = {}, extra: Partial<Extract<Node, { t: 'dir' }>> = {}): Node => ({ t: 'dir', kids: new Map(Object.entries(kids)), mtime: NOW - 3600, ...extra });
const file = (size: number, age = 86400): Node => ({ t: 'file', size, mtime: NOW - age });

function makeWindows(): Record<string, Node> {
  return {
    documents: dir({ 'Budget 2026.xlsx': file(48_213), 'Letter.docx': file(21_900, 400000), Receipts: dir({ 'march.pdf': file(120_311) }) }),
    downloads: dir({ 'installer-notes.txt': file(1204), 'photo.jpg': file(2_450_000, 5000), 'setup.exe': file(14_000_000, 90000) }),
    pictures: dir({ 'holiday.png': file(3_100_000), Screenshots: dir() }),
    videos: dir({ 'clip.mp4': file(58_000_000) }),
    music: dir({ 'song.flac': file(24_000_000) }),
    desktop: dir({ 'todo.md': file(311) }),
  };
}

function makeLinux(): Node {
  const big: Record<string, Node> = {};
  for (let i = 1; i <= 3000; i++) big[`file-${String(i).padStart(4, '0')}.txt`] = file(100 + (i % 50) * 10, i * 60);
  return dir({
    home: dir({
      preview: dir({
        '.bashrc': file(3771), '.profile': file(807), '.config': dir({ 'app.conf': file(120) }),
        Desktop: dir(),
        // A non-empty folder in the way of a mapping: needs the user's decision.
        Documents: dir({ 'thesis.odt': file(88_400), 'notes.txt': file(1900), drafts: dir({ 'ch1.md': file(4000) }) }),
        // Downloads does not exist: a mapping can simply be created.
        // An empty folder: replacing it loses nothing but needs confirmation.
        Pictures: dir(),
        // A symbolic link that VLINUX did not create.
        Music: { t: 'link', target: '/opt/music', mtime: NOW - 99999 },
        Videos: dir({ 'old-recording.mkv': file(9_000_000) }),
        projects: dir({ big: dir(big), 'hello.py': file(220), 'README.md': file(900) }),
        broken: { t: 'link', target: '/nowhere', mtime: NOW - 10 },
      }),
    }),
    etc: dir({ hostname: file(8), 'os-release': file(300), 'wsl.conf': file(40) }),
    opt: dir({ music: dir({ 'private.ogg': file(5_000_000) }) }),
    usr: dir({ bin: dir(), lib: dir() }),
    var: dir({ log: dir() }),
    tmp: dir(),
    root: dir({}, { denied: true }),
  });
}

interface World { linux: Node; backups: Record<string, { backup: string; replaced: string; old?: string }>; applied: Record<string, { target: string; windows: string }> }
const worlds = new Map<string, World>();
let windows: Record<string, Node> | null = null;
const win = () => (windows ??= makeWindows());
const world = (d: string): World => {
  let w = worlds.get(d);
  if (!w) {
    w = { linux: makeLinux(), backups: {}, applied: {} };
    worlds.set(d, w);
  }
  return w;
};

const err = (kind: FsError['kind'], message: string): FsError => ({ kind, message });

function linuxAvailable(distro: string | null): string | null {
  if (scenario() !== 'ready') return 'SIMULATED: WSL is not available in this preview scenario. Add ?wsl=ready to preview the Linux side.';
  if (!distro) return 'No Linux distribution is selected. Choose one in Linux Setup to browse its files.';
  return null;
}

// ---- resolution ------------------------------------------------------------------------------

function comps(path: string): string[] {
  return path.split('/').filter((p) => p && p !== '.');
}

type Resolved = { loc: Location; node: Node | null };

function resolveLinux(w: World, path: string): Resolved {
  const parts = comps(path);
  if (parts.includes('..')) throw err('invalid_path', 'A path may not contain “..”.');
  let cur: string[] = [];
  let node: Node = w.linux;
  let follows = 0;
  const queue = [...parts];
  while (queue.length) {
    const c = queue.shift()!;
    if (node.t !== 'dir') throw err('not_a_directory', `“${cur[cur.length - 1] ?? '/'}” is not a folder.`);
    cur.push(c);
    if (cur.length === 2 && cur[0] === 'mnt' && /^[a-z]$/i.test(c)) throw err('outside_root', `/mnt/${c} is a Windows drive mounted into Linux. Windows files are reached through the shared folders in the sidebar.`);
    const next = node.kids.get(c);
    if (!next) {
      // Missing: nothing below can exist.
      return { loc: { space: 'linux', path: '/' + [...cur, ...queue].join('/') }, node: null };
    }
    if (next.t === 'link') {
      if (++follows > 20) throw err('refused', 'Too many levels of symbolic links (possible loop).');
      if (next.win) return { loc: { space: 'windows', root: next.win, rel: queue }, node: queue.length ? walkWin(next.win, queue) : win()[next.win] ?? null };
      cur.pop();
      const t = next.target;
      if (t.startsWith('/')) {
        cur = [];
        node = w.linux;
      }
      queue.unshift(...comps(t));
      continue;
    }
    node = next;
  }
  return { loc: { space: 'linux', path: '/' + cur.join('/') }, node };
}

function walkWin(root: string, rel: string[]): Node | null {
  let n: Node | null = win()[root] ?? null;
  for (const c of rel) {
    if (!n || n.t !== 'dir') return null;
    n = n.kids.get(c) ?? null;
  }
  return n;
}

function resolve(distro: string | null, loc: Location): Resolved {
  if (loc.space === 'windows') {
    if (!win()[loc.root]) throw err('outside_root', 'That is not one of the shared Windows folders.');
    if (loc.rel.some((c) => c === '..' || c === '.' || /[\\/:*?"<>|]/.test(c) || !c)) throw err('invalid_path', 'That path is not valid.');
    return { loc, node: walkWin(loc.root, loc.rel) };
  }
  const why = linuxAvailable(distro);
  if (why) throw err('unavailable', why);
  return resolveLinux(world(distro!), loc.path);
}

function nodeAt(distro: string | null, loc: Location): { parent: Node & { t: 'dir' }; name: string; node: Node | null; parentLoc: Location } {
  const comp = loc.space === 'windows' ? loc.rel : comps(loc.path);
  const name = comp[comp.length - 1];
  if (!name) throw err('refused', loc.space === 'windows' ? 'A shared folder itself cannot be changed here.' : 'The root folder cannot be changed.');
  const parentLoc: Location = loc.space === 'windows' ? { space: 'windows', root: loc.root, rel: loc.rel.slice(0, -1) } : { space: 'linux', path: '/' + comp.slice(0, -1).join('/') };
  const p = resolve(distro, parentLoc);
  if (!p.node) throw err('not_found', 'The folder does not exist any more.');
  if (p.node.t !== 'dir') throw err('not_a_directory', 'That is not a folder.');
  return { parent: p.node, name, node: p.node.kids.get(name) ?? null, parentLoc: p.loc };
}

// ---- listing ---------------------------------------------------------------------------------

const EXT_CLASS: [RegExp, string, string][] = [
  [/\.(txt|md|log)$/i, 'text', 'Text document'], [/\.pdf$/i, 'pdf', 'PDF document'], [/\.(docx?|odt)$/i, 'document', 'Word-processing document'],
  [/\.(xlsx?|ods|csv)$/i, 'spreadsheet', 'Spreadsheet'], [/\.(png|jpe?g|gif|webp|svg)$/i, 'image', 'Image'], [/\.(mp3|flac|ogg|wav)$/i, 'audio', 'Audio'],
  [/\.(mp4|mkv|webm|mov)$/i, 'video', 'Video'], [/\.(zip|7z|tar|gz)$/i, 'archive', 'Archive'], [/\.(py|js|ts|rs|sh|json|toml|yml|conf)$/i, 'code', 'Source or config'],
  [/\.(exe|msi|dll)$/i, 'program', 'Windows program'],
];
const BLOCKED = /\.(exe|com|scr|bat|cmd|ps1|vbs|js|msi|lnk|url|reg|dll|py|jar)$/i;

function describe(w: World | null, name: string, n: Node): Entry {
  const base = { name, hidden: name.startsWith('.'), readonly: false, modified: n.mtime };
  if (n.t === 'dir') return { ...base, kind: 'dir', size: null, class: { id: 'folder', label: 'Folder' }, open: { allowed: true, reason: null }, link_target: null, link_is_dir: null };
  if (n.t === 'link') {
    let isDir: boolean | null = null;
    if (n.win) isDir = true;
    else if (w) {
      try {
        isDir = resolveLinux(w, n.target).node ? resolveLinux(w, n.target).node!.t === 'dir' : null;
      } catch {
        isDir = null;
      }
    }
    return { ...base, kind: 'symlink', size: null, class: { id: 'link', label: 'Link' }, open: { allowed: true, reason: null }, link_target: n.win ? `${n.target} (shared with Windows)` : n.target, link_is_dir: isDir };
  }
  const cls = EXT_CLASS.find(([re]) => re.test(name));
  const blocked = BLOCKED.test(name);
  return {
    ...base, kind: 'file', size: n.size, class: cls ? { id: cls[1], label: cls[2] } : { id: 'file', label: 'File' },
    open: blocked ? { allowed: false, reason: `“${name.split('.').pop()}” files can run code or change Windows, so VLINUX does not launch them from the file manager.` } : { allowed: true, reason: null },
    link_target: null, link_is_dir: null,
  };
}

const sessions = new Map<string, Entry[]>();
let sessionN = 0;

function crumbsFor(loc: Location): { label: string; location: Location }[] {
  if (loc.space === 'windows') {
    const out = [{ label: KNOWN_LABEL[loc.root] ?? loc.root, location: { space: 'windows', root: loc.root, rel: [] } as Location }];
    loc.rel.forEach((c, i) => out.push({ label: c, location: { space: 'windows', root: loc.root, rel: loc.rel.slice(0, i + 1) } }));
    return out;
  }
  const out = [{ label: '/', location: { space: 'linux', path: '/' } as Location }];
  comps(loc.path).forEach((c, i, a) => out.push({ label: c, location: { space: 'linux', path: '/' + a.slice(0, i + 1).join('/') } }));
  return out;
}

export function listOpen(distro: string | null, location: Location, opts: ListOptions, pageSize: number): ListingHead {
  const r = resolve(distro, location);
  if (!r.node) throw err('not_found', `Can't open “${crumbsFor(location).at(-1)?.label ?? ''}”: it does not exist or can't be reached any more.`);
  if (r.node.t !== 'dir') throw err('not_a_directory', 'That is not a folder.');
  if (r.node.denied) throw err('permission_denied', 'You don’t have permission to open this folder.');
  const w = location.space === 'linux' || r.loc.space === 'linux' ? world(distro!) : null;
  let rows = [...r.node.kids.entries()].filter(([n]) => opts.show_hidden || !n.startsWith('.')).map(([n, node]) => describe(w, n, node));
  const dirs = (e: Entry) => (e.kind === 'dir' || (e.kind === 'symlink' && e.link_is_dir) ? 0 : 1);
  rows.sort((a, b) => {
    if (dirs(a) !== dirs(b)) return dirs(a) - dirs(b);
    let c = 0;
    if (opts.sort === 'size') c = (a.size ?? -1) - (b.size ?? -1);
    else if (opts.sort === 'modified') c = (a.modified ?? 0) - (b.modified ?? 0);
    if (c === 0) c = a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: 'base' });
    return opts.descending ? -c : c;
  });
  rows = rows.slice(0, 500_000);
  const id = `pv-list-${++sessionN}`;
  sessions.set(id, rows);
  if (sessions.size > 12) sessions.delete(sessions.keys().next().value!);
  const parentLoc = r.loc.space === 'windows' ? (r.loc.rel.length ? { space: 'windows', root: r.loc.root, rel: r.loc.rel.slice(0, -1) } as Location : null) : comps(r.loc.path).length ? { space: 'linux', path: '/' + comps(r.loc.path).slice(0, -1).join('/') } as Location : null;
  return {
    id, location, resolved: r.loc, crumbs: crumbsFor(r.loc), parent: parentLoc, total: rows.length, truncated: false, unreadable: 0,
    page: { offset: 0, total: rows.length, entries: rows.slice(0, Math.min(pageSize, 1000)) },
  };
}

export function listPage(id: string, offset: number, limit: number): Page {
  const rows = sessions.get(id);
  if (!rows) throw err('not_found', 'This listing was closed. Refresh to load it again.');
  return { offset, total: rows.length, entries: rows.slice(offset, offset + Math.min(limit, 1000)) };
}

export function listClose(id: string) {
  sessions.delete(id);
}

// ---- places ----------------------------------------------------------------------------------

export function places(distro: string | null, cfg: Config): Places {
  const why = linuxAvailable(distro);
  const w = why ? null : world(distro!);
  const winPath = (id: string) => (cfg.storage[id as (typeof FOLDER_IDS)[number]]?.mode === 'shared' ? cfg.storage[id as (typeof FOLDER_IDS)[number]].windows_path : null) ?? previewKnownFolder(id);
  const shared: Place[] = SIDEBAR.map((id) => ({
    id, label: KNOWN_LABEL[id]!, location: { space: 'windows', root: id, rel: [] }, detail: winPath(id),
    linked: !!w?.applied[id], unc: winPath(id).startsWith('\\\\'), warning: null, exists: true,
  }));
  const folders: Place[] = [];
  let home: Place | null = null;
  const roots: Place[] = [];
  if (w) {
    home = { id: 'home', label: 'Home', location: { space: 'linux', path: HOME }, detail: HOME, linked: false, unc: false, warning: null, exists: true };
    roots.push({ id: 'root', label: 'File System', location: { space: 'linux', path: '/' }, detail: '/', linked: false, unc: false, warning: null, exists: true });
    for (const id of SIDEBAR) {
      const a = w.applied[id];
      folders.push({
        id, label: KNOWN_LABEL[id]!, location: { space: 'linux', path: `${HOME}/${KNOWN_LABEL[id]}` }, linked: !!a, unc: false, warning: null, exists: true,
        detail: a ? `Shared with Windows: ${a.windows}` : null,
      });
    }
  } else {
    for (const p of shared) folders.push({ ...p, linked: false });
  }
  return { distro, linux_available: !why, linux_note: why, home, roots, folders, shared };
}

// ---- operations ------------------------------------------------------------------------------

export function create(distro: string | null, parent: Location, name: string, kind: 'folder' | 'file'): Location {
  const bad = nameProblem(name);
  if (bad) throw err('invalid_name', bad);
  const r = resolve(distro, parent);
  if (!r.node) throw err('not_found', 'The folder does not exist any more.');
  if (r.node.t !== 'dir') throw err('not_a_directory', 'That is not a folder.');
  if (r.node.denied) throw err('permission_denied', 'You don’t have permission to create items here.');
  if (r.node.kids.has(name)) throw err('already_exists', `“${name}” already exists here.`);
  r.node.kids.set(name, kind === 'folder' ? dir() : file(0, 0));
  return childOf(r.loc, name);
}

export function rename(distro: string | null, loc: Location, newName: string): Location {
  const bad = nameProblem(newName);
  if (bad) throw err('invalid_name', bad);
  const n = nodeAt(distro, loc);
  if (!n.node) throw err('not_found', `Can't rename “${n.name}”: it does not exist any more.`);
  if (loc.space === 'linux' && (loc.path === HOME || comps(loc.path).length === 1)) throw err('refused', 'This is a protected system location and can’t be renamed.');
  if (newName === n.name) return loc;
  if (n.parent.kids.has(newName)) throw err('already_exists', `“${newName}” already exists here.`);
  n.parent.kids.delete(n.name);
  n.parent.kids.set(newName, n.node);
  return childOf(n.parentLoc, newName);
}

const listeners = new Set<(e: JobEvent) => void>();
export const onJob = (cb: (e: JobEvent) => void) => {
  listeners.add(cb);
  return () => listeners.delete(cb);
};
let jobN = 0;

function emitJob(kind: JobEvent['kind'], job_id: string, state: JobEvent['state'], report: OpReport | null, error: FsError | null = null) {
  for (const l of listeners) l({ job_id, kind, state, progress: null, report, error });
}

function count(n: Node): { files: number; bytes: number } {
  if (n.t === 'file') return { files: 1, bytes: n.size };
  if (n.t === 'link') return { files: 1, bytes: 0 };
  let f = 0, b = 0;
  for (const k of n.kids.values()) {
    const c = count(k);
    f += c.files;
    b += c.bytes;
  }
  return { files: f + 1, bytes: b };
}

function clone(n: Node, report: OpReport): Node | null {
  if (n.t === 'link') {
    report.skipped.push('A symbolic link was skipped: links are never copied or followed.');
    return null;
  }
  if (n.t === 'file') return { ...n };
  const d = dir();
  if (d.t === 'dir') for (const [k, v] of n.kids) {
    const c = clone(v, report);
    if (c) d.kids.set(k, c);
  }
  return d;
}

function freeName(kids: Map<string, Node>, name: string): string {
  if (!kids.has(name)) return name;
  const dot = name.lastIndexOf('.');
  const [stem, ext] = dot > 0 ? [name.slice(0, dot), name.slice(dot)] : [name, ''];
  for (let i = 1; i < 1000; i++) {
    const cand = `${stem} (copy${i > 1 ? ` ${i}` : ''})${ext}`;
    if (!kids.has(cand)) return cand;
  }
  return name;
}

function job(kind: JobEvent['kind'], work: (report: OpReport) => void): { job_id: string } {
  const job_id = `pv-job-${++jobN}`;
  const report: OpReport = { items_done: 0, files: 0, bytes: 0, skipped: [], failed: [], cancelled: false, results: [] };
  setTimeout(() => emitJob(kind, job_id, 'running', null), 0);
  setTimeout(() => {
    try {
      work(report);
      emitJob(kind, job_id, 'done', report);
    } catch (e) {
      emitJob(kind, job_id, 'failed', report, e as FsError);
    }
  }, 60);
  return { job_id };
}

export function deleteItems(distro: string | null, targets: Location[], confirmed: boolean): { job_id: string } {
  if (!confirmed) throw err('refused', 'Deleting needs your confirmation. Nothing was deleted.');
  const resolved = targets.map((t) => nodeAt(distro, t));
  for (const [i, t] of targets.entries()) {
    if (t.space === 'linux' && (t.path === HOME || comps(t.path).length === 1)) throw err('refused', `“${resolved[i]!.name}” is a protected system location and can’t be deleted.`);
  }
  return job('delete', (report) => {
    for (const n of resolved) {
      if (!n.node) {
        report.failed.push({ path: n.name, message: `Can't delete “${n.name}”: it does not exist any more.` });
        continue;
      }
      const c = count(n.node);
      n.parent.kids.delete(n.name);
      report.items_done++;
      report.files += c.files;
      report.bytes += c.bytes;
    }
  });
}

export function transfer(distro: string | null, sources: Location[], dest: Location, mode: TransferMode, conflict: Conflict): { job_id: string } {
  if (!sources.length) throw err('invalid_path', 'Nothing was selected.');
  const d = resolve(distro, dest);
  if (!d.node || d.node.t !== 'dir') throw err(d.node ? 'not_a_directory' : 'not_found', 'The destination folder is not available.');
  const srcs = sources.map((s) => ({ loc: s, ...nodeAt(distro, s) }));
  const dkids = d.node.kids;
  const dloc = d.loc;
  return job(mode, (report) => {
    for (const s of srcs) {
      if (!s.node) {
        report.failed.push({ path: s.name, message: `Can't ${mode} “${s.name}”: it does not exist any more.` });
        continue;
      }
      if (s.node.t === 'dir' && s.node === d.node) {
        report.failed.push({ path: s.name, message: 'A folder can’t be copied or moved into itself.' });
        continue;
      }
      let name = s.name;
      if (dkids.has(name)) {
        if (mode === 'move' && s.parent.kids === dkids) {
          report.skipped.push(`“${name}” is already in this folder.`);
          continue;
        }
        if (conflict === 'skip') {
          report.skipped.push(`“${name}” already exists in the destination, so it was skipped.`);
          continue;
        }
        if (conflict === 'fail') {
          report.failed.push({ path: name, message: `“${name}” already exists in the destination.` });
          continue;
        }
        name = freeName(dkids, name);
      }
      const c = count(s.node);
      const copy = mode === 'move' ? s.node : clone(s.node, report);
      if (!copy) continue;
      dkids.set(name, copy);
      if (mode === 'move') s.parent.kids.delete(s.name);
      report.items_done++;
      report.files += c.files;
      report.bytes += c.bytes;
      report.results.push(childOf(dloc, name));
    }
  });
}

export function open(distro: string | null, loc: Location): void {
  const n = nodeAt(distro, loc);
  if (!n.node) throw err('not_found', `“${n.name}” does not exist any more.`);
  if (n.node.t === 'dir') throw err('is_a_directory', 'Folders open inside Files.');
  if (n.node.t === 'file' && BLOCKED.test(n.name)) throw err('refused', `“${n.name.split('.').pop()}” files can run code or change Windows, so VLINUX does not launch them from the file manager.`);
  lastOpened = { name: n.name, how: 'open' };
}

export function reveal(distro: string | null, loc: Location): void {
  const n = nodeAt(distro, loc);
  lastOpened = { name: n.name, how: 'reveal' };
}

/** What the simulated "open in Windows" did last, for the e2e tests. */
let lastOpened: { name: string; how: 'open' | 'reveal' } | null = null;
export const previewLastOpened = () => lastOpened;

// ---- mappings (a faithful, simulated copy of the engine's decisions) -----------------------------

type FolderPref = { mode: 'shared' | 'private'; windows_path: string | null };
const prefOf = (cfg: Config, id: string): FolderPref => cfg.storage[id as (typeof FOLDER_IDS)[number]];

function intended(cfg: Config, id: string): { windows: string; target: string; warning: string | null } | { error: string } {
  const p = prefOf(cfg, id);
  const windows = p.windows_path ?? previewKnownFolder(id);
  const v = validatePath(windows);
  if (v.kind === 'unsupported') return { error: v.message };
  if (!/^[CD]:/i.test(windows)) return { error: `The Windows folder ${windows} does not exist or can't be reached right now.` };
  return { windows, target: v.wsl_path, warning: v.kind === 'warning' ? v.message : null };
}

function status(w: World, cfg: Config, id: string, check: boolean): MappingStatus {
  const p = prefOf(cfg, id);
  const want = intended(cfg, id);
  const home = w.linux.t === 'dir' ? ((w.linux.kids.get('home') as Node & { t: 'dir' }).kids.get('preview') as Node & { t: 'dir' }) : null;
  const name = KNOWN_LABEL[id]!;
  const rec = w.backups[id];
  const base = {
    folder: id, label: name, linux_path: `~/${name}`, preference: p.mode,
    windows_path: 'error' in want ? p.windows_path ?? previewKnownFolder(id) : want.windows,
    target: 'error' in want ? null : want.target, warning: 'error' in want ? null : want.warning,
    backup: rec?.backup || null, applied_at: w.applied[id] ? NOW : null,
  };
  const make = (state: MappingState): MappingStatus => ({ ...base, state });
  if (!check) return make(p.mode === 'private' ? { state: 'private' } : { state: 'unknown' });
  const node = home?.kids.get(name);
  const ours = !!w.applied[id];
  if (p.mode === 'private') return make(node?.t === 'link' && ours ? { state: 'linked_but_private', target: node.target } : { state: 'private' });
  if ('error' in want) return make({ state: 'unavailable', reason: want.error });
  if (!node) return make({ state: 'ready', needs: null, note: `~/${name} doesn't exist yet; a link will be created.` });
  if (node.t === 'link') {
    if (node.target === want.target) return make({ state: 'applied', target: want.target });
    if (ours) return make({ state: 'ready', needs: 'replace_link', note: `~/${name} currently points to ${node.target}. It will be re-pointed to ${want.target}.` });
    return make({
      state: 'conflict', kind: 'other_link', entries: [], more: 0,
      detail: `~/${name} is already a symbolic link to ${node.target}, which VLINUX didn't create.`,
      offers: [{ resolution: 'replace_link', label: 'Replace the link', description: `Re-points ~/${name} to the Windows folder. Only the link changes; ${node.target} is untouched, and Undo restores the old link.` }],
    });
  }
  if (node.t === 'dir' && node.kids.size === 0) return make({ state: 'ready', needs: 'replace_empty_folder', note: `~/${name} is an empty folder. It will be replaced by the link (nothing is lost).` });
  const names = node.t === 'dir' ? [...node.kids.keys()].sort() : [];
  return make({
    state: 'conflict', kind: node.t === 'file' ? 'file' : 'non_empty_folder',
    detail: node.t === 'file' ? 'A file with this name exists in your Linux home. VLINUX will not overwrite it.' : 'The folder in Linux already contains files. VLINUX will not replace it. Move or merge its contents yourself, or keep this folder private.',
    entries: names.slice(0, 8), more: Math.max(0, names.length - 8),
    offers: [{ resolution: 'move_aside', label: 'Move my existing items aside', description: `Renames ~/${name} to ~/${name}.vlinux-backup (nothing is copied or deleted), then links ~/${name} to the Windows folder. Undo moves it back.` }],
  });
}

export function mappingStatus(distro: string, check: boolean, cfg: Config): MappingReport {
  const why = linuxAvailable(distro);
  if (why) throw err('unavailable', why);
  const w = world(distro);
  return { distro, checked: check, home: check ? HOME : null, mount_root: '/mnt', automount: true, folders: ['downloads', 'documents', 'pictures', 'videos', 'music', 'desktop'].map((id) => status(w, cfg, id, check)) };
}

const homeDir = (w: World) => ((w.linux as Node & { t: 'dir' }).kids.get('home') as Node & { t: 'dir' }).kids.get('preview') as Node & { t: 'dir' };

export function mappingApply(distro: string, id: string, resolution: Resolution | null, cfg: Config): MappingOutcome {
  const why = linuxAvailable(distro);
  if (why) throw err('unavailable', why);
  const w = world(distro);
  const st = status(w, cfg, id, true);
  const home = homeDir(w);
  const name = KNOWN_LABEL[id]!;
  const want = intended(cfg, id);
  const refuse = (m: string) => err('refused', m);
  if (st.state.state === 'unavailable') throw refuse(`${st.state.reason} Nothing was changed.`);
  if (prefOf(cfg, id).mode !== 'shared' || 'error' in want) throw refuse('This folder is set to Linux-private storage, so there is nothing to link.');
  const link: Node = { t: 'link', target: want.target, win: id, mtime: NOW };
  const done = (message: string): MappingOutcome => {
    w.applied[id] = { target: want.target, windows: want.windows };
    return { message, status: status(w, cfg, id, true) };
  };
  const s = st.state;
  if (s.state === 'applied') return done(`~/${name} was already linked to ${want.windows}. Nothing was changed.`);
  if (s.state === 'ready' && s.needs === null) {
    home.kids.set(name, link);
    w.backups[id] = { backup: '', replaced: 'nothing' };
    return done(`~/${name} now opens ${want.windows} directly.`);
  }
  if (s.state === 'ready' && s.needs === 'replace_empty_folder') {
    if (resolution !== 'replace_empty_folder') throw refuse(`Replacing the empty folder ~/${name} needs your confirmation first. Nothing was changed.`);
    home.kids.set(name, link);
    w.backups[id] = { backup: '', replaced: 'empty' };
    return done(`~/${name} now opens ${want.windows} directly.`);
  }
  if (s.state === 'ready' && s.needs === 'replace_link') {
    if (resolution !== 'replace_link') throw refuse(`Re-pointing the link ~/${name} needs your confirmation first. Nothing was changed.`);
    home.kids.set(name, link);
    return done(`~/${name} now opens ${want.windows} directly.`);
  }
  if (s.state === 'conflict' && s.kind === 'other_link') {
    if (resolution !== 'replace_link') throw refuse(`${s.detail} Nothing was changed.`);
    const old = home.kids.get(name) as Node & { t: 'link' };
    home.kids.set(name, link);
    w.backups[id] = { backup: '', replaced: 'link', old: old.target };
    return done(`~/${name} now opens ${want.windows} directly.`);
  }
  if (s.state === 'conflict') {
    if (resolution !== 'move_aside') throw refuse(`${s.detail} Nothing was changed.`);
    let backup = `${HOME}/${name}.vlinux-backup`;
    for (let i = 2; home.kids.has(backup.split('/').pop()!); i++) backup = `${HOME}/${name}.vlinux-backup-${i}`;
    home.kids.set(backup.split('/').pop()!, home.kids.get(name)!);
    home.kids.set(name, link);
    w.backups[id] = { backup, replaced: 'moved' };
    return done(`~/${name} now opens ${want.windows}. Your previous contents are kept at ${backup}; nothing was copied or deleted.`);
  }
  throw refuse('This folder cannot be applied right now. Nothing was changed.');
}

export function mappingUndo(distro: string, id: string, cfg: Config): MappingOutcome {
  const why = linuxAvailable(distro);
  if (why) throw err('unavailable', why);
  const w = world(distro);
  const home = homeDir(w);
  const name = KNOWN_LABEL[id]!;
  const node = home.kids.get(name);
  const rec = w.backups[id];
  if (!node || node.t !== 'link' || !w.applied[id]) {
    if (node && node.t !== 'link') throw err('refused', `~/${name} is a regular folder or file, not a link VLINUX created. Nothing was changed.`);
    if (node?.t === 'link') throw err('refused', `~/${name} is a link to ${node.target}, which VLINUX didn't create, so it was left alone.`);
    return { message: `~/${name} isn't linked, so there was nothing to undo.`, status: status(w, cfg, id, true) };
  }
  home.kids.delete(name);
  let message: string;
  if (rec?.replaced === 'moved' && rec.backup && home.kids.has(rec.backup.split('/').pop()!)) {
    home.kids.set(name, home.kids.get(rec.backup.split('/').pop()!)!);
    home.kids.delete(rec.backup.split('/').pop()!);
    message = `The link was removed and your original ~/${name} was moved back.`;
  } else if (rec?.replaced === 'link' && rec.old) {
    home.kids.set(name, { t: 'link', target: rec.old, mtime: NOW });
    message = `The link was removed and ~/${name} points to ${rec.old} again.`;
  } else {
    home.kids.set(name, dir());
    message = `The link was removed. ~/${name} is now a private folder inside the distribution. Files in the Windows folder were not touched.`;
  }
  delete w.applied[id];
  delete w.backups[id];
  return { message, status: status(w, cfg, id, true) };
}
