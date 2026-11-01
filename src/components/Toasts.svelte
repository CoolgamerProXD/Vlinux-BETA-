<script lang="ts">
  import { ui } from '../lib/ui.svelte';
  import Icon from './Icon.svelte';
</script>

<div class="toasts" role="status" aria-live="polite">
  {#each ui.toasts as t (t.id)}
    <div class="toast glass" class:error={t.kind === 'error'}>
      <Icon name={t.kind === 'error' ? 'alert' : 'info'} size={16} />
      <span>{t.text}</span>
    </div>
  {/each}
</div>

<style>
  .toasts { position: absolute; right: var(--gap); bottom: var(--gap); z-index: 120; display: grid; gap: 8px; max-width: min(420px, calc(100vw - 24px)); pointer-events: none; }
  .toast { display: flex; gap: 10px; align-items: flex-start; padding: 10px 14px; border-radius: 12px; font-size: 0.84rem; background: var(--panel-solid); animation: pop-in 240ms var(--ease) both; user-select: text; }
  .toast :global(svg) { flex: none; margin-top: 2px; color: var(--accent); }
  .toast.error { border-color: rgb(255 110 130 / 0.5); }
  .toast.error :global(svg) { color: #ff8a98; }
</style>
