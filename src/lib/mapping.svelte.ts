import * as ipc from './ipc';
import { asFsError, type MappingReport, type MappingStatus, type Resolution } from './fs-types';

/**
 * What the selected distribution really looks like for each shared folder. Nothing here is guessed
 * from the saved preference: `report` only fills in after a check against the distribution, and until
 * then every folder reads "preferred · not checked".
 */
class MappingStore {
  report = $state<MappingReport | null>(null);
  distro = $state<string | null>(null);
  loading = $state(false);
  /** Folder currently being applied or undone. */
  busy = $state<string | null>(null);
  error = $state<string | null>(null);
  messages = $state<Record<string, { text: string; ok: boolean }>>({});
  /** The user asked at least once; later preference changes re-check automatically. */
  checkedOnce = $state(false);

  statusOf(folder: string): MappingStatus | undefined {
    return this.report?.folders.find((f) => f.folder === folder);
  }

  /** Cheap, starts nothing in Linux: the preferred state plus Windows-side facts. */
  async load(distro: string | null) {
    if (!distro) {
      this.report = null;
      this.distro = null;
      return;
    }
    if (this.distro !== distro) {
      this.report = null;
      this.checkedOnce = false;
      this.messages = {};
    }
    this.distro = distro;
    this.error = null;
    try {
      const r = await ipc.mappingStatus(distro, false);
      if (!this.report?.checked) this.report = r;
    } catch (e) {
      this.error = asFsError(e).message;
    }
  }

  /** Looks inside the distribution (starts it if it is stopped). Only on the user's request. */
  async check(distro: string) {
    this.loading = true;
    this.error = null;
    this.distro = distro;
    try {
      this.report = await ipc.mappingStatus(distro, true);
      this.checkedOnce = true;
    } catch (e) {
      this.error = asFsError(e).message;
    } finally {
      this.loading = false;
    }
  }

  private replace(s: MappingStatus) {
    if (!this.report) return;
    this.report = { ...this.report, folders: this.report.folders.map((f) => (f.folder === s.folder ? s : f)) };
  }

  async apply(distro: string, folder: string, resolution: Resolution | null = null) {
    this.busy = folder;
    try {
      const o = await ipc.mappingApply(distro, folder, resolution);
      this.replace(o.status);
      this.messages = { ...this.messages, [folder]: { text: o.message, ok: o.status.state.state === 'applied' } };
    } catch (e) {
      this.messages = { ...this.messages, [folder]: { text: asFsError(e).message, ok: false } };
      // The filesystem is unchanged on failure, but show what is really there.
      void this.check(distro);
    } finally {
      this.busy = null;
    }
  }

  async undo(distro: string, folder: string) {
    this.busy = folder;
    try {
      const o = await ipc.mappingUndo(distro, folder);
      this.replace(o.status);
      this.messages = { ...this.messages, [folder]: { text: o.message, ok: true } };
    } catch (e) {
      this.messages = { ...this.messages, [folder]: { text: asFsError(e).message, ok: false } };
    } finally {
      this.busy = null;
    }
  }
}

export const mapping = new MappingStore();
