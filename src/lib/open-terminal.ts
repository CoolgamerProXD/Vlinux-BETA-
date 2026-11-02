import { appById } from './apps';
import { terminals } from './terminal.svelte';
import { wm } from './wm.svelte';

/**
 * Open the Terminal window and a session in `distro` (or the selected distribution when `null`).
 * Used by buttons outside the Terminal app, e.g. in Linux Setup.
 */
export function openTerminalIn(distro: string | null = null) {
  const app = appById('terminal');
  if (!app) return;
  terminals.launchRequest = distro ?? '';
  wm.open(app);
}
