import * as ipc from './ipc';
import { runtime } from './runtime.svelte';
import { ui } from './ui.svelte';
import {
  asFsError, childOf, isDirEntry, locKey, parentOf, sameLoc,
  type Crumb, type Entry, type FsError, type JobEvent, type ListingHead, type Location, type OpReport, type Places, type SortKey, type TransferMode,
} from './fs-types';

export interface JobView {
  id: string;
  kind: JobEvent['kind'];
  state: JobEvent['state'];
  files: number;
  bytes: number;
  current: string;
  skipped: number;
  failed: number;
}

export interface Finished {
  title: string;
  report: OpReport | null;
  error: FsError | null;
}

const PAGE = 200;
const VERB: Record<JobEvent['kind'], [string, string]> = { delete: ['Deleting', 'Deleted'], copy: ['Copying', 'Copied'], move: ['Moving', 'Moved'] };

/**
 * State of the Files window. Everything the UI shows comes from the backend (the real distribution
 * filesystem or the shared Windows folders); directories are loaded one page at a time and only on
 * demand, so a folder with hundreds of thousands of entries never sits in the frontend.
 */
class FilesStore {
  /** Distribution whose Linux filesystem this window shows. `null` = Windows folders only. */
  distro = $state<string | null>(null);
  places = $state<Places | null>(null);
  placesError = $state<string | null>(null);
  head = $state<ListingHead | null>(null);
  entries = $state<Entry[]>([]);
  location = $state<Location | null>(null);
  loading = $state(false);
  error = $state<FsError | null>(null);
  history = $state<Location[]>([]);
  hIndex = $state(-1);
  showHidden = $state(false);
  sort = $state<SortKey>('name');
  descending = $state(false);
  selected = $state<string[]>([]);
  anchor = $state<string | null>(null);
  clipboard = $state<{ mode: TransferMode; sources: Location[]; names: string[] } | null>(null);
  jobs = $state<JobView[]>([]);
  finished = $state<Finished | null>(null);
  /** In-app drag in progress. */
  drag = $state<{ sources: Location[]; names: string[] } | null>(null);
  /** Somebody (Settings, Setup) asked Files to show a place; the window picks it up when it mounts. */
  pending = $state<{ distro: string | null; location: Location | null } | null>(null);
  booted = $state(false);

  private gen = 0;
  private moreBusy = false;
  private unlisten: (() => void) | undefined;

  readonly selectedSet = $derived(new Set(this.selected));
  readonly canBack = $derived(this.hIndex > 0);
  readonly canForward = $derived(this.hIndex >= 0 && this.hIndex < this.history.length - 1);
  readonly canUp = $derived(this.location ? parentOf(this.location) !== null : false);
  readonly busy = $derived(this.jobs.some((j) => j.state === 'running'));
  /** Linux path shown in the location bar when browsing Linux. */
  readonly linuxPath = $derived(this.location?.space === 'linux' ? this.location.path : null);
  /** A Linux folder that is really a shared Windows folder. */
  readonly sharedHere = $derived(this.head !== null && this.head.location.space === 'linux' && this.head.resolved.space === 'windows');
  readonly crumbs = $derived<Crumb[]>(this.head ? this.head.crumbs : this.location ? this.fallbackCrumbs(this.location) : []);

  private fallbackCrumbs(loc: Location): Crumb[] {
    if (loc.space === 'linux') {
      const parts = loc.path.split('/').filter(Boolean);
      return [{ label: '/', location: { space: 'linux', path: '/' } }, ...parts.map((p, i): Crumb => ({ label: p, location: { space: 'linux', path: '/' + parts.slice(0, i + 1).join('/') } }))];
    }
    const label = this.places?.shared.find((p) => p.id === loc.root)?.label ?? loc.root;
    return [{ label, location: { space: 'windows', root: loc.root, rel: [] } }, ...loc.rel.map((p, i): Crumb => ({ label: p, location: { space: 'windows', root: loc.root, rel: loc.rel.slice(0, i + 1) } }))];
  }

  private options() {
    return { show_hidden: this.showHidden, sort: this.sort, descending: this.descending };
  }

  // ---- start ---------------------------------------------------------------------------------

  /** Called when the window opens: a read-only look at WSL (cached), then places and a first folder. */
  async init() {
    if (this.booted) return;
    this.booted = true;
    this.unlisten = await ipc.onFsJob((e) => this.onJob(e));
    if (!runtime.report) await runtime.refresh(false);
    const req = this.pending;
    this.pending = null;
    const ready = runtime.report?.status.kind === 'ready';
    // Only the distribution the user chose (or explicitly asked for) is touched: nothing is guessed.
    this.distro = req?.distro ?? (ready ? runtime.selectedDistro?.name ?? null : null);
    await this.loadPlaces(false);
    const start = req?.location ?? this.places?.home?.location ?? this.places?.folders[0]?.location ?? this.places?.shared[0]?.location ?? null;
    if (start) await this.navigate(start);
  }

  dispose() {
    this.unlisten?.();
    this.unlisten = undefined;
    if (this.head) void ipc.fsListClose(this.head.id);
    this.head = null;
    this.entries = [];
    this.booted = false;
    this.location = null;
    this.history = [];
    this.hIndex = -1;
    this.selected = [];
  }

  async loadPlaces(refresh: boolean) {
    this.placesError = null;
    try {
      this.places = await ipc.fsPlaces(this.distro, refresh);
    } catch (e) {
      this.placesError = asFsError(e).message;
    }
  }

  async setDistro(name: string | null) {
    this.distro = name;
    await this.loadPlaces(true);
    const start = this.places?.home?.location ?? this.places?.folders[0]?.location ?? null;
    this.history = [];
    this.hIndex = -1;
    if (start) await this.navigate(start);
  }

  // ---- navigation ----------------------------------------------------------------------------

  async navigate(loc: Location, push = true) {
    const g = ++this.gen;
    this.loading = true;
    this.error = null;
    const previous = this.head?.id;
    try {
      const head = await ipc.fsListOpen(this.distro, loc, this.options(), PAGE);
      if (g !== this.gen) {
        void ipc.fsListClose(head.id);
        return;
      }
      if (previous) void ipc.fsListClose(previous);
      const keep = push ? [] : this.selected;
      this.head = head;
      this.entries = [...head.page.entries];
      this.selected = keep.filter((n) => this.entries.some((e) => e.name === n));
      this.anchor = this.selected[0] ?? null;
    } catch (e) {
      if (g !== this.gen) return;
      if (previous) void ipc.fsListClose(previous);
      this.head = null;
      this.entries = [];
      this.selected = [];
      this.error = asFsError(e);
    } finally {
      if (g === this.gen) {
        this.loading = false;
        this.location = loc;
        if (push && !sameLoc(this.history[this.hIndex] ?? null, loc)) {
          this.history = [...this.history.slice(0, this.hIndex + 1), loc].slice(-100);
          this.hIndex = this.history.length - 1;
        }
      }
    }
  }

  async refresh() {
    if (this.location) await this.navigate(this.location, false);
  }

  async back() {
    if (!this.canBack) return;
    this.hIndex -= 1;
    await this.navigate(this.history[this.hIndex]!, false);
  }

  async forward() {
    if (!this.canForward) return;
    this.hIndex += 1;
    await this.navigate(this.history[this.hIndex]!, false);
  }

  async up() {
    const p = this.location ? parentOf(this.location) : null;
    if (p) await this.navigate(p);
  }

  /** Load pages until `upto` rows are present (or the folder ends). */
  async ensureLoaded(upto: number) {
    const head = this.head;
    if (!head || this.moreBusy) return;
    const want = Math.min(upto, head.total);
    if (this.entries.length >= want) return;
    this.moreBusy = true;
    try {
      while (this.head === head && this.entries.length < want) {
        const page = await ipc.fsListPage(head.id, this.entries.length, PAGE);
        if (this.head !== head) return;
        if (!page.entries.length) break;
        this.entries = [...this.entries, ...page.entries];
      }
    } catch (e) {
      if (this.head === head) this.error = asFsError(e);
    } finally {
      this.moreBusy = false;
    }
  }

  async setHidden(on: boolean) {
    this.showHidden = on;
    await this.refresh();
  }

  async setSort(key: SortKey) {
    if (this.sort === key) this.descending = !this.descending;
    else {
      this.sort = key;
      this.descending = false;
    }
    await this.refresh();
  }

  // ---- selection -----------------------------------------------------------------------------

  select(name: string, mode: 'only' | 'toggle' | 'range' = 'only') {
    if (mode === 'toggle') {
      this.selected = this.selectedSet.has(name) ? this.selected.filter((n) => n !== name) : [...this.selected, name];
      this.anchor = name;
    } else if (mode === 'range' && this.anchor) {
      const a = this.entries.findIndex((e) => e.name === this.anchor);
      const b = this.entries.findIndex((e) => e.name === name);
      if (a >= 0 && b >= 0) this.selected = this.entries.slice(Math.min(a, b), Math.max(a, b) + 1).map((e) => e.name);
    } else {
      this.selected = [name];
      this.anchor = name;
    }
  }

  selectAll() {
    this.selected = this.entries.map((e) => e.name);
  }

  clearSelection() {
    this.selected = [];
    this.anchor = null;
  }

  locOf(name: string): Location | null {
    return this.location ? childOf(this.location, name) : null;
  }

  selectedLocations(): Location[] {
    return this.selected.map((n) => this.locOf(n)).filter((l): l is Location => l !== null);
  }

  // ---- actions -------------------------------------------------------------------------------

  async open(entry: Entry) {
    if (isDirEntry(entry)) {
      const l = this.locOf(entry.name);
      if (l) await this.navigate(l);
      return;
    }
    const l = this.locOf(entry.name);
    if (!l) return;
    if (!entry.open.allowed) {
      ui.toast(entry.open.reason ?? 'This type of file is not opened from VLINUX.', 'error', 7000);
      return;
    }
    try {
      await ipc.fsOpen(this.distro, l);
      ui.toast(`Opened “${entry.name}” in Windows`);
    } catch (e) {
      ui.toast(asFsError(e).message, 'error', 7000);
    }
  }

  async reveal(name: string | null) {
    const l = name ? this.locOf(name) : this.location;
    if (!l) return;
    try {
      await ipc.fsReveal(this.distro, l);
    } catch (e) {
      ui.toast(asFsError(e).message, 'error', 7000);
    }
  }

  /** Returns an error message to show in the prompt, or `null` on success. */
  async create(kind: 'folder' | 'file', name: string): Promise<string | null> {
    if (!this.location) return 'There is no open folder.';
    try {
      const made = await ipc.fsCreate(this.distro, this.location, name, kind);
      await this.refresh();
      const last = made.space === 'linux' ? made.path.split('/').pop() : made.rel[made.rel.length - 1];
      if (last) this.select(last);
      return null;
    } catch (e) {
      return asFsError(e).message;
    }
  }

  async rename(name: string, newName: string): Promise<string | null> {
    const l = this.locOf(name);
    if (!l) return 'Nothing to rename.';
    try {
      await ipc.fsRename(this.distro, l, newName);
      await this.refresh();
      this.select(newName);
      return null;
    } catch (e) {
      return asFsError(e).message;
    }
  }

  async deleteSelected(): Promise<string | null> {
    const targets = this.selectedLocations();
    if (!targets.length) return null;
    try {
      const j = await ipc.fsDelete(this.distro, targets, true);
      this.track(j.job_id, 'delete');
      return null;
    } catch (e) {
      return asFsError(e).message;
    }
  }

  copy(cut: boolean) {
    const sources = this.selectedLocations();
    if (!sources.length) return;
    this.clipboard = { mode: cut ? 'move' : 'copy', sources, names: [...this.selected] };
    ui.toast(`${cut ? 'Cut' : 'Copied'} ${sources.length === 1 ? `“${this.selected[0]}”` : `${sources.length} items`} — open a folder and paste`);
  }

  async paste(into: Location | null = this.location) {
    const c = this.clipboard;
    if (!c || !into) return;
    if (await this.transfer(c.sources, into, c.mode)) {
      if (c.mode === 'move') this.clipboard = null;
    }
  }

  /** Copy or move. Name clashes keep both; nothing is ever overwritten. Returns whether it started. */
  async transfer(sources: Location[], dest: Location, mode: TransferMode): Promise<boolean> {
    try {
      const j = await ipc.fsTransfer(this.distro, sources, dest, mode, 'keep_both');
      this.track(j.job_id, mode);
      return true;
    } catch (e) {
      ui.toast(asFsError(e).message, 'error', 7000);
      return false;
    }
  }

  async cancelJob(id: string) {
    await ipc.fsJobCancel(id);
  }

  dismissFinished() {
    this.finished = null;
  }

  // ---- jobs ----------------------------------------------------------------------------------

  private track(id: string, kind: JobEvent['kind']) {
    if (!this.jobs.some((j) => j.id === id)) this.jobs = [...this.jobs, { id, kind, state: 'running', files: 0, bytes: 0, current: '', skipped: 0, failed: 0 }];
  }

  private onJob(e: JobEvent) {
    this.track(e.job_id, e.kind);
    if (e.state === 'running') {
      this.jobs = this.jobs.map((j) => (j.id === e.job_id ? { ...j, files: e.progress?.files ?? j.files, bytes: e.progress?.bytes ?? j.bytes, current: e.progress?.current ?? j.current, skipped: e.progress?.skipped ?? j.skipped, failed: e.progress?.failed ?? j.failed } : j));
      return;
    }
    this.jobs = this.jobs.filter((j) => j.id !== e.job_id);
    const [ing, ed] = VERB[e.kind];
    const r = e.report;
    const problems = (r?.failed.length ?? 0) + (r?.skipped.length ?? 0);
    if (e.state === 'cancelled') ui.toast(`${ing} was cancelled${r ? ` after ${r.items_done} item${r.items_done === 1 ? '' : 's'}` : ''}. Nothing already ${ed.toLowerCase()} was undone.`);
    else if (e.state === 'failed') ui.toast(e.error?.message ?? `${ing} failed.`, 'error', 8000);
    else ui.toast(`${ed} ${r?.items_done ?? 0} item${r?.items_done === 1 ? '' : 's'}${problems ? ` — ${problems} need attention` : ''}`, problems ? 'error' : 'info');
    if (e.state === 'failed' || problems > 0) this.finished = { title: `${ing} finished with problems`, report: r, error: e.error };
    void this.refresh();
  }

  /** Is `loc` this place, or (for a shared folder opened through Linux) its Windows folder? */
  isHere(loc: Location): boolean {
    if (!this.location) return false;
    return locKey(this.location) === locKey(loc);
  }
}

export const files = new FilesStore();

export function formatJob(j: JobView): string {
  const [ing] = VERB[j.kind];
  const bits = [`${ing}…`, `${j.files} item${j.files === 1 ? '' : 's'}`];
  return bits.join(' ');
}
