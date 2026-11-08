<script lang="ts">
  import { exitToWindows, quitApp } from '../lib/ipc';
  import { host } from '../lib/host.svelte';
  import { ui } from '../lib/ui.svelte';
  import { terminals } from '../lib/terminal.svelte';
  import Icon from './Icon.svelte';

  let confirmQuit = $state(false);
  let busy = $state(false);

  async function run(fn: () => Promise<unknown>, closeAfter = true) {
    if (busy) return;
    busy = true;
    try {
      await fn();
      if (closeAfter) ui.close();
    } catch (e) {
      ui.toast(String((e as Error)?.message ?? e), 'error');
      ui.close();
    } finally {
      busy = false;
    }
  }

  function key(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.stopPropagation();
      if (confirmQuit) confirmQuit = false;
      else ui.close();
    }
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="backdrop" onpointerdown={(e) => e.target === e.currentTarget && ui.close()} onkeydown={key}>
  <div class="panel glass" role="dialog" aria-modal="true" aria-label="Power and exit">
    {#if !confirmQuit}
      <h2>Session</h2>
      <div class="tiles">
        <button class="tile primary" onclick={() => run(exitToWindows)} disabled={busy}>
          <Icon name="windows" size={30} />
          <strong>Return to Windows</strong>
          <span>Leaves full screen and minimises VLINUX to the taskbar. Nothing is stopped; click VLINUX in the taskbar to come back.</span>
        </button>
        <button class="tile" onclick={() => run(() => host.toggleFullscreen())} disabled={busy}>
          <Icon name={host.fullscreen ? 'shrink' : 'expand'} size={30} />
          <strong>{host.fullscreen ? 'Leave full screen' : 'Enter full screen'}</strong>
          <span>{host.fullscreen ? 'Show VLINUX in a regular window.' : 'Fill the whole display.'}</span>
        </button>
        <button class="tile danger" onclick={() => (confirmQuit = true)} disabled={busy}>
          <Icon name="power" size={30} />
          <strong>Quit VLINUX…</strong>
          <span>Closes the VLINUX application. Asks for confirmation first.</span>
        </button>
      </div>
      <p class="hint">Windows always stays reachable: the Win key, Alt+Tab and the taskbar keep working in full screen. Shortcut: <kbd class="mono">Ctrl</kbd>+<kbd class="mono">Alt</kbd>+<kbd class="mono">Q</kbd> returns to Windows instantly.</p>
    {:else}
      <h2>Quit VLINUX?</h2>
      <p class="confirm">VLINUX will close and you will be back on the Windows desktop. Your settings are already saved.
        {#if terminals.liveCount > 0}
          {terminals.liveCount === 1 ? 'The open terminal session ends' : `The ${terminals.liveCount} open terminal sessions end`}, like closing any terminal window: programs running in them stop. Your Linux distribution itself keeps running and no files are deleted.
        {:else}
          No terminal sessions are open, and VLINUX never shuts your Linux distribution down.
        {/if}
      </p>
      <div class="actions">
        <!-- svelte-ignore a11y_autofocus -->
        <button class="btn" autofocus onclick={() => (confirmQuit = false)}>Cancel</button>
        <button class="btn danger" disabled={busy} onclick={() => run(quitApp, false)}>Quit VLINUX</button>
      </div>
    {/if}
  </div>
</div>

<style>
  .backdrop { position: absolute; inset: 0; z-index: 90; display: grid; place-items: center; padding: 12px; background: rgb(4 4 10 / 0.62); backdrop-filter: blur(10px); animation: fade-in 180ms ease both; }
  .panel { width: min(760px, 100%); max-height: 100%; overflow: auto; border-radius: 24px; padding: 22px; background: rgb(var(--panel-rgb) / calc(var(--glass-a) + 0.25)); animation: pop-in 260ms var(--ease) both; }
  h2 { margin: 0 0 14px; font-size: 0.8rem; text-transform: uppercase; letter-spacing: 0.14em; color: var(--muted); font-weight: 600; }
  .tiles { display: grid; grid-template-columns: repeat(auto-fit, minmax(190px, 1fr)); gap: 12px; }
  .tile { display: grid; align-content: start; gap: 8px; text-align: left; padding: 18px; border-radius: 18px; background: rgb(255 255 255 / 0.05); border: 1px solid var(--line); transition: all var(--dur) var(--ease); color: var(--text); }
  .tile span { color: var(--muted); font-size: 0.8rem; line-height: 1.45; }
  .tile:hover:not(:disabled) { background: rgb(255 255 255 / 0.1); transform: translateY(-3px); border-color: var(--line-strong); }
  .tile.primary { border-color: rgb(var(--accent-rgb) / 0.55); background: rgb(var(--accent-rgb) / 0.12); }
  .tile.primary :global(svg) { color: var(--accent); }
  .tile.danger:hover:not(:disabled) { border-color: rgb(255 110 130 / 0.6); background: rgb(255 110 130 / 0.12); }
  .tile.danger :global(svg) { color: #ff7a8a; }
  .hint { margin: 16px 2px 0; font-size: 0.78rem; color: var(--faint); line-height: 1.5; }
  kbd { padding: 1px 6px; border-radius: 5px; background: rgb(255 255 255 / 0.1); font-size: 0.72rem; color: var(--muted); }
  .confirm { margin: 0 0 18px; color: var(--muted); line-height: 1.55; max-width: 56ch; }
  .actions { display: flex; gap: 10px; justify-content: flex-end; }
  .btn { padding: 9px 18px; border-radius: 11px; background: rgb(255 255 255 / 0.1); border: 1px solid var(--line); }
  .btn:hover:not(:disabled) { background: rgb(255 255 255 / 0.16); }
  .btn.danger { background: rgb(255 90 110 / 0.2); border-color: rgb(255 110 130 / 0.6); color: #ffb3bd; }
  .btn.danger:hover:not(:disabled) { background: rgb(255 90 110 / 0.32); }
</style>
