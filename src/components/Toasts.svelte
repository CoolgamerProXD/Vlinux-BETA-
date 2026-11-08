<script lang="ts">
  import { notify } from '../lib/notify.svelte';
  import Icon from './Icon.svelte';
</script>

<div class="toasts" role="status" aria-live="polite">
  {#each notify.toasts as t (t.id)}
    <div class="toast glass {t.kind}" class:error={t.kind === 'error'}>
      <Icon name={t.kind === 'error' || t.kind === 'warning' ? 'alert' : t.kind === 'success' ? 'check' : 'info'} size={16} />
      <span class="msg">{t.text}{#if t.count > 1} <span class="count mono">×{t.count}</span>{/if}</span>
      <button class="x" aria-label="Dismiss notification" onclick={() => notify.hideToast(t.id)}><Icon name="close" size={13} /></button>
    </div>
  {/each}
</div>

<style>
  .toasts { position: absolute; right: var(--gap); bottom: var(--gap); z-index: 120; display: grid; gap: 8px; max-width: min(420px, calc(100vw - 24px)); pointer-events: none; }
  .toast { display: flex; gap: 10px; align-items: flex-start; padding: 10px 10px 10px 14px; border-radius: 12px; font-size: 0.84rem; background: var(--panel-solid); animation: pop-in 240ms var(--ease) both; user-select: text; pointer-events: auto; }
  .toast :global(svg) { flex: none; margin-top: 2px; color: var(--accent); }
  .msg { flex: 1; min-width: 0; overflow-wrap: anywhere; }
  .count { color: var(--faint); font-size: 0.74rem; }
  .x { width: 22px; height: 22px; flex: none; display: grid; place-items: center; border-radius: 6px; color: var(--faint); }
  .x:hover { background: rgb(255 255 255 / 0.1); color: var(--text); }
  .x :global(svg) { margin: 0; color: inherit; }
  .toast.error { border-color: rgb(255 110 130 / 0.5); }
  .toast.error :global(svg) { color: #ff8a98; }
  .toast.warning { border-color: rgb(255 190 90 / 0.45); }
  .toast.warning :global(svg) { color: #ffc46b; }
  .toast.success :global(svg) { color: #5ff0b0; }
  .toast .x :global(svg) { color: inherit; }
</style>
