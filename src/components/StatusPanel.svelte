<script lang="ts">
  import { host } from '../lib/host.svelte';
  import { APPS } from '../lib/apps';
  import { ui } from '../lib/ui.svelte';
  import { wm } from '../lib/wm.svelte';
  import Icon from './Icon.svelte';

  const info = $derived(host.info);
  const batt = $derived(host.battery ? `${Math.round(host.battery.level * 100)}%${host.battery.charging ? ' (charging)' : ''}` : 'Not available');

  function details() {
    wm.open(APPS.find((a) => a.id === 'system')!);
    ui.close();
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="backdrop" onpointerdown={(e) => e.target === e.currentTarget && ui.close()} onkeydown={(e) => e.key === 'Escape' && ui.close()}>
  <div class="panel glass" role="dialog" aria-label="System status">
    <h2>System status</h2>
    <dl>
      <dt>Host</dt><dd>{info ? `${info.os_name} ${info.os_version}` : '…'}</dd>
      <dt>CPU threads</dt><dd>{info?.cpu_threads ?? '…'}</dd>
      <dt>Network</dt><dd class:bad={!host.online}>{host.online ? 'Online' : 'Offline'}</dd>
      <dt>Battery</dt><dd>{batt}</dd>
      <dt>Display</dt><dd>{host.fullscreen ? 'Full screen' : 'Windowed'}</dd>
      <dt>WSL2</dt><dd class="dim">Not checked yet — detection arrives in M2</dd>
      <dt>Linux</dt><dd class="dim">Not running (VLINUX does not start Linux yet)</dd>
    </dl>
    <button class="more" onclick={details}><Icon name="activity" size={16} /> Open System details</button>
  </div>
</div>

<style>
  .backdrop { position: absolute; inset: 0; z-index: 70; }
  .panel {
    position: absolute; top: calc(var(--bar-h) + var(--gap) * 2); right: var(--gap);
    width: min(340px, calc(100vw - 2 * var(--gap))); border-radius: 18px; padding: 16px;
    background: rgb(16 16 28 / calc(var(--glass-a) + 0.25)); animation: drop-in 220ms var(--ease) both;
  }
  h2 { margin: 0 0 12px; font-size: 0.74rem; text-transform: uppercase; letter-spacing: 0.14em; color: var(--muted); }
  dl { display: grid; grid-template-columns: auto 1fr; gap: 8px 14px; margin: 0; font-size: 0.86rem; }
  dt { color: var(--faint); }
  dd { margin: 0; text-align: right; overflow-wrap: anywhere; }
  .dim { color: var(--muted); }
  .bad { color: #ff8a98; }
  .more { margin-top: 14px; width: 100%; display: flex; gap: 8px; align-items: center; justify-content: center; padding: 9px; border-radius: 11px; background: rgb(var(--accent-rgb) / 0.16); border: 1px solid rgb(var(--accent-rgb) / 0.4); }
  .more:hover { background: rgb(var(--accent-rgb) / 0.26); }
</style>
