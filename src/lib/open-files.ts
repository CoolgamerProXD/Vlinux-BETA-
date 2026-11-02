import { appById } from './apps';
import { files } from './files.svelte';
import type { Location } from './fs-types';
import { wm } from './wm.svelte';

/** Open (or focus) Files, optionally at a place in `distro`. */
export function openFilesAt(distro: string | null = null, location: Location | null = null) {
  const app = appById('files');
  if (!app) return;
  if (files.booted && wm.byApp('files')) {
    if (distro !== null && distro !== files.distro) void files.setDistro(distro).then(() => location && files.navigate(location));
    else if (location) void files.navigate(location);
  } else {
    files.pending = { distro, location };
  }
  wm.open(app);
}
