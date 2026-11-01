<script lang="ts">
  import { APPS, type AppDef } from '../lib/apps';
  import { config } from '../lib/config.svelte';
  import { ui } from '../lib/ui.svelte';
  import { wm } from '../lib/wm.svelte';
  import AppTile from './AppTile.svelte';
  import Icon from './Icon.svelte';

  const items = APPS.filter((a) => a.dock);
  const size = $derived(config.value.dock.icon_size);
  const vertical = $derived(config.value.dock.position === 'left');

  const label = (a: AppDef) => (a.status === 'planned' ? `${a.name} — arrives in milestone ${a.milestone}` : a.name);

  function click(a: AppDef) {
    if (a.status === 'planned') return;
    wm.activate(a);
  }
</script>

<aside class="dock glass" class:vertical aria-label="Dock">
  <button class="item launcher" aria-label="Applications" onclick={() => ui.toggle('launcher')}>
    <span class="launch-tile" style="width:{size}px;height:{size}px;border-radius:{Math.round(size * 0.3)}px">
      <Icon name="grid" size={Math.round(size * 0.5)} />
    </span>
    <span class="tip">Applications</span>
  </button>
  <span class="sep"></span>
  {#each items as a (a.id)}
    {@const win = wm.byApp(a.id)}
    <button class="item" class:running={!!win} class:focused={win && wm.focusedId === win.id}
            aria-label={label(a)} aria-disabled={a.status === 'planned'} onclick={() => click(a)}>
      <AppTile app={a} {size} />
      {#if a.status === 'planned'}<span class="soon mono">{a.milestone}</span>{/if}
      <span class="dot"></span>
      <span class="tip">{label(a)}</span>
    </button>
  {/each}
</aside>

<style>
  .dock {
    display: flex; align-items: center; gap: 10px;
    padding: 8px 12px;
    border-radius: 22px;
    position: relative;
    z-index: 40;
    animation: pop-in 600ms var(--ease) 150ms both;
  }
  .dock.vertical { flex-direction: column; padding: 12px 8px; }
  .sep { width: 1px; align-self: stretch; background: var(--line-strong); margin: 4px 0; }
  .vertical .sep { width: auto; height: 1px; align-self: stretch; margin: 0 4px; }

  .item { position: relative; display: grid; place-items: center; padding-bottom: 8px; transition: transform var(--dur) var(--ease); }
  .vertical .item { padding-bottom: 0; padding-right: 8px; }
  .item:not([aria-disabled='true']):hover { transform: translateY(-5px) scale(1.08); }
  .vertical .item:not([aria-disabled='true']):hover { transform: translateX(5px) scale(1.08); }
  .item:not([aria-disabled='true']):active { transform: scale(0.95); }

  .launch-tile {
    display: grid; place-items: center; color: #0a0a12;
    background: linear-gradient(135deg, var(--accent), var(--accent-2));
    box-shadow: 0 6px 18px rgb(var(--accent-rgb) / 0.4), inset 0 1px 0 rgb(255 255 255 / 0.4);
  }

  .dot {
    position: absolute; bottom: 0; left: 50%; width: 5px; height: 5px; border-radius: 50%;
    background: var(--accent); transform: translateX(-50%) scale(0); transition: transform var(--dur) var(--ease);
    box-shadow: 0 0 8px var(--accent);
  }
  .vertical .dot { bottom: 50%; left: auto; right: 0; transform: translateY(50%) scale(0); }
  .running .dot { transform: translateX(-50%) scale(1); }
  .vertical .running .dot { transform: translateY(50%) scale(1); }
  .focused .dot { width: 14px; border-radius: 3px; }
  .vertical .focused .dot { width: 5px; height: 14px; }

  .soon {
    position: absolute; top: -4px; right: -6px; font-size: 0.6rem; padding: 1px 5px; border-radius: 6px;
    background: rgb(255 255 255 / 0.14); color: var(--muted);
  }

  .tip {
    position: absolute; bottom: calc(100% + 10px); left: 50%; transform: translateX(-50%) translateY(4px);
    background: var(--panel-solid); border: 1px solid var(--line-strong); border-radius: 8px;
    padding: 4px 9px; font-size: 0.78rem; white-space: nowrap; pointer-events: none; opacity: 0;
    transition: opacity var(--dur), transform var(--dur) var(--ease); z-index: 5;
  }
  .vertical .tip { bottom: auto; left: calc(100% + 10px); top: 50%; transform: translateY(-50%) translateX(-4px); }
  .item:hover .tip, .item:focus-visible .tip { opacity: 1; transform: translateX(-50%); }
  .vertical .item:hover .tip, .vertical .item:focus-visible .tip { transform: translateY(-50%); }

  @media (max-width: 640px) {
    .dock { gap: 6px; padding: 6px 8px; }
  }
</style>
