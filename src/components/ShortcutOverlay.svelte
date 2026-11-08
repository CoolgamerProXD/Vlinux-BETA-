<script lang="ts">
  import { SHORTCUTS, SHORTCUT_GROUPS } from '../lib/shortcuts';
  import { ui } from '../lib/ui.svelte';
  import Icon from './Icon.svelte';

  let closeBtn: HTMLButtonElement | undefined = $state();
  $effect(() => closeBtn?.focus());
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="backdrop" onpointerdown={(e) => e.target === e.currentTarget && ui.close()}>
  <div class="panel glass" role="dialog" aria-modal="true" aria-label="Keyboard shortcuts">
    <header>
      <h2>Keyboard shortcuts</h2>
      <button bind:this={closeBtn} class="x" aria-label="Close shortcut list" onclick={() => ui.close()}><Icon name="close" size={16} /></button>
    </header>
    <div class="cols">
      {#each SHORTCUT_GROUPS as g (g)}
        <section>
          <h3>{g}</h3>
          <dl>
            {#each SHORTCUTS.filter((s) => s.group === g) as s (s.keys)}
              <dt><kbd class="mono">{s.keys}</kbd></dt><dd>{s.label}</dd>
            {/each}
          </dl>
        </section>
      {/each}
    </div>
    <p class="note">Alt + Tab is reserved by Windows, so VLINUX uses Alt + ` to switch windows.</p>
  </div>
</div>

<style>
  .backdrop { position: absolute; inset: 0; z-index: 90; display: grid; place-items: center; padding: 12px; background: rgb(4 4 10 / 0.6); backdrop-filter: blur(8px); animation: fade-in 160ms ease both; }
  .panel { width: min(860px, 100%); max-height: 100%; overflow: auto; border-radius: 22px; padding: 18px 22px; background: rgb(var(--panel-rgb) / calc(var(--glass-a) + 0.25)); animation: pop-in 220ms var(--ease) both; }
  header { display: flex; justify-content: space-between; align-items: center; }
  h2 { margin: 0; font-size: 1.05rem; }
  .x { width: 30px; height: 30px; display: grid; place-items: center; border-radius: 9px; color: var(--muted); }
  .x:hover { background: rgb(255 255 255 / 0.1); color: var(--text); }
  .cols { display: grid; grid-template-columns: repeat(auto-fit, minmax(300px, 1fr)); gap: 6px 28px; margin-top: 8px; }
  h3 { margin: 14px 0 6px; font-size: 0.72rem; text-transform: uppercase; letter-spacing: 0.08em; color: var(--faint); }
  dl { margin: 0; display: grid; grid-template-columns: auto 1fr; gap: 6px 14px; align-items: baseline; font-size: 0.84rem; }
  dd { margin: 0; color: var(--muted); }
  kbd { font-size: 0.74rem; padding: 2px 7px; border-radius: 6px; background: rgb(255 255 255 / 0.09); border: 1px solid var(--line); white-space: nowrap; }
  .note { margin: 14px 0 0; color: var(--faint); font-size: 0.78rem; }
</style>
