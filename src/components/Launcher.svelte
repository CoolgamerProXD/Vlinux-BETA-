<script lang="ts">
  import { APPS } from '../lib/apps';
  import { ui } from '../lib/ui.svelte';
  import { wm } from '../lib/wm.svelte';
  import AppTile from './AppTile.svelte';
  import Icon from './Icon.svelte';

  let query = $state('');
  let input: HTMLInputElement | undefined = $state();

  const matches = $derived(
    APPS.filter((a) => {
      const q = query.trim().toLowerCase();
      return !q || a.name.toLowerCase().includes(q) || a.description.toLowerCase().includes(q);
    }).sort((a, b) => Number(b.status === 'available') - Number(a.status === 'available')),
  );

  $effect(() => {
    input?.focus();
  });

  function launch(id: string) {
    const app = APPS.find((a) => a.id === id);
    if (!app || app.status !== 'available') return;
    wm.open(app);
    ui.close();
  }

  function key(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.stopPropagation();
      ui.close();
    } else if (e.key === 'Enter') {
      const first = matches.find((a) => a.status === 'available');
      if (first) launch(first.id);
    }
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="backdrop" onpointerdown={(e) => e.target === e.currentTarget && ui.close()} onkeydown={key}>
  <div class="panel glass" role="dialog" aria-modal="true" aria-label="Application launcher">
    <label class="search">
      <Icon name="search" size={18} />
      <input bind:this={input} bind:value={query} placeholder="Search applications…" spellcheck="false" autocomplete="off" aria-label="Search applications" />
      <kbd class="mono">Esc</kbd>
    </label>

    {#if matches.length}
      <ul class="grid">
        {#each matches as a (a.id)}
          <li>
            <button class="app" aria-disabled={a.status === 'planned'} onclick={() => launch(a.id)}>
              <AppTile app={a} size={52} />
              <span class="meta">
                <span class="name">{a.name}
                  {#if a.status === 'planned'}<span class="badge mono">{a.milestone}</span>{/if}
                </span>
                <span class="desc">{a.status === 'planned' ? `Not available yet — planned for ${a.milestone}` : a.description}</span>
              </span>
            </button>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="empty">No applications match “{query}”.</p>
    {/if}

    <footer>
      Linux applications will appear here once a distribution is set up (WSL setup arrives in M2).
    </footer>
  </div>
</div>

<style>
  .backdrop {
    position: absolute; inset: 0; z-index: 80; display: grid; place-items: start center;
    padding-top: clamp(56px, 12vh, 140px);
    background: rgb(4 4 10 / 0.5); backdrop-filter: blur(6px); animation: fade-in 180ms ease both;
  }
  .panel {
    width: min(720px, calc(100vw - 24px)); max-height: min(560px, calc(100vh - 100px)); overflow: auto;
    border-radius: 22px; padding: 14px; background: rgb(16 16 28 / calc(var(--glass-a) + 0.2));
    animation: pop-in 260ms var(--ease) both;
  }
  .search {
    display: flex; align-items: center; gap: 10px; padding: 0 14px; height: 48px; border-radius: 14px;
    background: rgb(255 255 255 / 0.06); border: 1px solid var(--line); color: var(--muted);
    cursor: text;
  }
  .search:focus-within { border-color: rgb(var(--accent-rgb) / 0.7); box-shadow: 0 0 0 3px rgb(var(--accent-rgb) / 0.18); }
  input { flex: 1; background: none; border: 0; outline: 0; color: var(--text); font-size: 1.05rem; user-select: text; }
  kbd { font-size: 0.7rem; padding: 2px 6px; border-radius: 6px; background: rgb(255 255 255 / 0.08); color: var(--faint); }

  .grid { list-style: none; margin: 12px 0 0; padding: 0; display: grid; gap: 6px; grid-template-columns: repeat(auto-fill, minmax(260px, 1fr)); }
  .app { display: flex; align-items: center; gap: 12px; width: 100%; padding: 10px; border-radius: 14px; text-align: left; transition: background var(--dur); }
  .app:not([aria-disabled='true']):hover { background: rgb(255 255 255 / 0.08); }
  .app[aria-disabled='true'] .name, .app[aria-disabled='true'] .desc { opacity: 0.55; }
  .meta { display: grid; gap: 2px; min-width: 0; }
  .name { font-weight: 600; display: flex; align-items: center; gap: 8px; }
  .badge { font-size: 0.62rem; padding: 1px 6px; border-radius: 6px; background: rgb(255 255 255 / 0.12); color: var(--muted); font-weight: 500; }
  .desc { color: var(--muted); font-size: 0.82rem; }
  .empty { color: var(--muted); text-align: center; padding: 28px 0; }
  footer { margin: 12px 4px 2px; padding-top: 10px; border-top: 1px solid var(--line); color: var(--faint); font-size: 0.78rem; }
</style>
