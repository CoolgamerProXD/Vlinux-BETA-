<script lang="ts">
  import { appById } from '../lib/apps';
  import { wm } from '../lib/wm.svelte';
  import Icon from './Icon.svelte';

  const items = $derived(
    (wm.switcher?.order ?? []).map((id) => wm.windows.find((w) => w.id === id)).filter((w): w is NonNullable<typeof w> => !!w),
  );
  const index = $derived(wm.switcher?.index ?? 0);
</script>

{#if wm.switcher}
  <div class="backdrop" role="presentation">
    <ul class="switcher glass" role="listbox" aria-label="Switch window">
      {#each items as w, i (w.id)}
        {@const app = appById(w.appId)}
        <li role="option" aria-selected={i === index} class:sel={i === index}>
          <button tabindex="-1" onpointerdown={(e) => { e.preventDefault(); wm.switcher!.index = i; wm.cycleCommit(); }}>
            <span class="ico" style="--c:{app?.color ?? 'var(--accent)'}">{#if app}<Icon name={app.icon} size={26} />{/if}</span>
            <span class="t">{w.title}</span>
            {#if w.minimized}<span class="min">minimised</span>{/if}
          </button>
        </li>
      {/each}
    </ul>
  </div>
{/if}

<style>
  .backdrop { position: absolute; inset: 0; z-index: 110; display: grid; place-items: center; pointer-events: none; }
  .switcher { list-style: none; margin: 0; padding: 10px; display: flex; gap: 8px; border-radius: 20px; max-width: calc(100vw - 24px); overflow: hidden; pointer-events: auto; animation: pop-in 120ms var(--ease) both; background: rgb(var(--panel-rgb) / calc(var(--glass-a) + 0.2)); }
  button { display: grid; justify-items: center; gap: 6px; width: 112px; padding: 12px 8px 10px; border-radius: 14px; border: 1px solid transparent; }
  li.sel button { background: rgb(var(--accent-rgb) / 0.18); border-color: rgb(var(--accent-rgb) / 0.6); }
  .ico { color: var(--c); }
  .t { font-size: 0.8rem; max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .min { font-size: 0.66rem; color: var(--faint); }
</style>
