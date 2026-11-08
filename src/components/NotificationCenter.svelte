<script lang="ts">
  import { onDestroy } from 'svelte';
  import { notify, type Notice } from '../lib/notify.svelte';
  import { ui } from '../lib/ui.svelte';
  import Icon from './Icon.svelte';

  const list = $derived([...notify.items].reverse());

  // Anything shown here has been seen. Unread markers stay visible while the panel is open and
  // clear when it closes.
  onDestroy(() => notify.markAllRead());

  function ago(at: number): string {
    const s = Math.max(0, Math.round((Date.now() - at) / 1000));
    if (s < 10) return 'just now';
    if (s < 60) return `${s} s ago`;
    if (s < 3600) return `${Math.round(s / 60)} min ago`;
    return new Date(at).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' });
  }
  const icon = (n: Notice) => (n.kind === 'error' || n.kind === 'warning' ? 'alert' : n.kind === 'success' ? 'check' : 'info');
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="backdrop" onpointerdown={(e) => e.target === e.currentTarget && ui.close()}>
  <div class="panel glass" role="dialog" aria-label="Notifications">
    <header>
      <h2>Notifications</h2>
      {#if list.length}<button class="link" onclick={() => notify.clear()}>Clear all</button>{/if}
    </header>
    {#if list.length}
      <ul>
        {#each list as n (n.id)}
          <li class={n.kind} class:unread={!n.read}>
            <span class="ic"><Icon name={icon(n)} size={16} /></span>
            <div class="body">
              <p>{n.text}{#if n.count > 1} <span class="count mono">×{n.count}</span>{/if}</p>
              {#if n.details}<details><summary>Details</summary><pre>{n.details}</pre></details>{/if}
              <span class="meta"><span class="src">{n.source}</span> · {ago(n.at)}</span>
            </div>
            <button class="x" aria-label="Remove notification" onclick={() => notify.remove(n.id)}><Icon name="close" size={13} /></button>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="empty">Nothing to show. Messages about files, distributions, apps and settings appear here.</p>
    {/if}
  </div>
</div>

<style>
  .backdrop { position: absolute; inset: 0; z-index: 85; }
  .panel {
    position: absolute; top: calc(var(--bar-h) + var(--gap) * 2); right: var(--gap);
    width: min(400px, calc(100vw - 24px)); max-height: min(560px, calc(100vh - 110px)); overflow: auto;
    border-radius: 18px; padding: 12px; background: rgb(var(--panel-rgb) / calc(var(--glass-a) + 0.25));
    animation: drop-in 200ms var(--ease) both;
  }
  header { display: flex; align-items: center; justify-content: space-between; padding: 2px 4px 8px; }
  h2 { margin: 0; font-size: 0.95rem; }
  .link { color: var(--muted); font-size: 0.8rem; padding: 3px 8px; border-radius: 7px; }
  .link:hover { background: rgb(255 255 255 / 0.1); color: var(--text); }
  ul { list-style: none; margin: 0; padding: 0; display: grid; gap: 6px; }
  li { display: flex; gap: 10px; padding: 9px 8px 9px 10px; border-radius: 12px; background: rgb(255 255 255 / 0.04); border: 1px solid transparent; }
  li.unread { border-color: rgb(var(--accent-rgb) / 0.35); background: rgb(var(--accent-rgb) / 0.07); }
  li.error .ic { color: #ff8a98; }
  li.warning .ic { color: #ffc46b; }
  li.success .ic { color: #5ff0b0; }
  .ic { color: var(--accent); margin-top: 2px; display: grid; }
  .body { flex: 1; min-width: 0; display: grid; gap: 3px; }
  p { margin: 0; font-size: 0.84rem; overflow-wrap: anywhere; user-select: text; }
  .count { color: var(--faint); font-size: 0.74rem; }
  .meta { color: var(--faint); font-size: 0.74rem; }
  .src { text-transform: capitalize; }
  details summary { cursor: pointer; color: var(--muted); font-size: 0.78rem; }
  pre { margin: 4px 0 0; white-space: pre-wrap; overflow-wrap: anywhere; font-size: 0.74rem; color: var(--muted); user-select: text; font-family: var(--font-mono); }
  .x { width: 22px; height: 22px; flex: none; display: grid; place-items: center; border-radius: 6px; color: var(--faint); }
  .x:hover { background: rgb(255 255 255 / 0.1); color: var(--text); }
  .empty { color: var(--muted); text-align: center; padding: 26px 10px; }
</style>
