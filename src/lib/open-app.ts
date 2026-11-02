import { appById } from './apps';
import { wm } from './wm.svelte';

/** Open (or focus) a shell app by id. */
export function openApp(id: string) {
  const app = appById(id);
  if (app) wm.open(app);
}
