<script lang="ts">
  import { wm } from '../lib/wm.svelte';

  const p = $derived(wm.pendingClose);

  function key(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.stopPropagation();
      wm.pendingClose = null;
    }
  }
</script>

{#if p}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="backdrop" onpointerdown={(e) => e.target === e.currentTarget && (wm.pendingClose = null)} onkeydown={key}>
    <div class="panel glass" role="alertdialog" aria-modal="true" aria-label={`Close ${p.title}?`}>
      <h2>Close {p.title}?</h2>
      <p>{p.message}</p>
      <div class="actions">
        <!-- svelte-ignore a11y_autofocus -->
        <button class="btn" autofocus onclick={() => (wm.pendingClose = null)}>Keep it open</button>
        <button class="btn danger" onclick={() => wm.closeNow(p.id)}>Close {p.title}</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .backdrop { position: absolute; inset: 0; z-index: 95; display: grid; place-items: center; padding: 12px; background: rgb(4 4 10 / 0.55); backdrop-filter: blur(8px); animation: fade-in 160ms ease both; }
  .panel { width: min(480px, 100%); border-radius: 22px; padding: 22px; background: rgb(var(--panel-rgb) / calc(var(--glass-a) + 0.25)); animation: pop-in 240ms var(--ease) both; }
  h2 { margin: 0 0 10px; font-size: 1.05rem; }
  p { margin: 0 0 18px; color: var(--muted); line-height: 1.55; }
  .actions { display: flex; gap: 10px; justify-content: flex-end; flex-wrap: wrap; }
  .btn { padding: 9px 18px; border-radius: 11px; background: rgb(255 255 255 / 0.1); border: 1px solid var(--line); }
  .btn:hover { background: rgb(255 255 255 / 0.16); }
  .btn.danger { background: rgb(255 90 110 / 0.2); border-color: rgb(255 110 130 / 0.6); color: #ffb3bd; }
  .btn.danger:hover { background: rgb(255 90 110 / 0.32); }
</style>
