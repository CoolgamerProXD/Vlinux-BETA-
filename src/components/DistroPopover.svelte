<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { config } from '../lib/config.svelte';
  import { distros } from '../lib/distros.svelte';
  import { shownState } from '../lib/distros-view';
  import { openApp } from '../lib/open-app';
  import { ui } from '../lib/ui.svelte';
  import Icon from './Icon.svelte';

  // Reads the Distribution Manager's cached overview. Listeners exist only while this panel is open
  // (attach/detach), nothing is polled, and nothing is started unless the user presses Start.
  onMount(() => void distros.attach());
  onDestroy(() => distros.detach());

  const selected = $derived(config.value.runtime.selected_distro);
  const regular = $derived(distros.cards.filter((c) => c.kind === 'regular'));
  const card = $derived(selected ? distros.cards.find((c) => c.name === selected) ?? null : null);
  const shown = $derived(card ? shownState(card, distros.pending[card.name]) : null);

  function go(id: string) {
    openApp(id);
    ui.close();
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="backdrop" onpointerdown={(e) => e.target === e.currentTarget && ui.close()}>
  <div class="panel glass" role="dialog" aria-label="Linux distribution">
    <header>
      <h2>Linux distribution</h2>
      <button class="x" aria-label="Close" onclick={() => ui.close()}><Icon name="close" size={14} /></button>
    </header>

    {#if !selected}
      <p class="msg">No distribution is selected. VLINUX never picks one for you.</p>
      <button class="btn primary" onclick={() => go('setup')}>Choose in Linux Setup</button>
    {:else}
      <div class="now">
        <span class="dot {shown?.tone ?? 'idle'}" aria-hidden="true"></span>
        <div class="who">
          <strong>{card?.os_pretty ?? card?.display_name ?? selected}</strong>
          <span class="sub mono">{selected}</span>
        </div>
        <span class="state {shown?.tone ?? 'idle'}">{shown ? shown.label : distros.loading || !distros.overview ? 'Checking…' : 'Not found'}</span>
      </div>

      {#if !card && distros.overview}
        <p class="msg warn">“{selected}” is not registered with WSL any more. Nothing was changed; open Distributions to see what is installed.</p>
      {/if}
      {#if distros.error}<p class="msg warn">{distros.error}</p>{/if}
      {#if card?.failure}<p class="msg warn">{card.failure}</p>{/if}
      {#if distros.notes[selected]}<p class="note">{distros.notes[selected]}</p>{/if}

      <div class="actions">
        {#if card && shown && (shown.key === 'stopped' || shown.key === 'detected' || shown.key === 'starting')}
          <button class="btn primary" disabled={!!distros.pending[selected]} onclick={() => distros.start(selected)}>
            {distros.pending[selected] === 'start' ? 'Starting…' : 'Start'}
          </button>
        {/if}
        <button class="btn" onclick={() => go('terminal')}>Terminal</button>
        <button class="btn" onclick={() => go('files')}>Files</button>
        <button class="btn" onclick={() => go('distros')}>Manage…</button>
      </div>
    {/if}

    {#if regular.length > 1}
      <h3>Use another distribution</h3>
      <ul>
        {#each regular as c (c.name)}
          {@const s = shownState(c, distros.pending[c.name])}
          <li>
            <button class="row" aria-current={c.name === selected ? 'true' : undefined} disabled={c.name === selected} onclick={() => distros.setPreferred(c.name)}>
              <span class="dot {s.tone}" aria-hidden="true"></span>
              <span class="nm">{c.display_name}</span>
              <span class="st">{s.label}</span>
              {#if c.name === selected}<Icon name="check" size={14} />{/if}
            </button>
          </li>
        {/each}
      </ul>
    {/if}

    <p class="foot">VLINUX never changes the Windows default distribution{distros.overview?.windows_default ? ` (currently “${distros.overview.windows_default}”)` : ''}.</p>
  </div>
</div>

<style>
  .backdrop { position: absolute; inset: 0; z-index: 85; }
  .panel { position: absolute; top: calc(var(--bar-h) + var(--gap) * 2); right: var(--gap); width: min(360px, calc(100vw - 24px)); max-height: calc(100vh - 110px); overflow: auto; border-radius: 18px; padding: 14px; background: rgb(var(--panel-rgb) / calc(var(--glass-a) + 0.25)); animation: drop-in 200ms var(--ease) both; display: grid; gap: 10px; }
  header { display: flex; align-items: center; justify-content: space-between; }
  h2 { margin: 0; font-size: 0.95rem; }
  h3 { margin: 4px 0 0; font-size: 0.72rem; text-transform: uppercase; letter-spacing: 0.08em; color: var(--faint); }
  .x { width: 26px; height: 26px; display: grid; place-items: center; border-radius: 8px; color: var(--muted); }
  .x:hover { background: rgb(255 255 255 / 0.1); color: var(--text); }
  .now { display: flex; align-items: center; gap: 10px; padding: 10px 12px; border-radius: 14px; background: rgb(255 255 255 / 0.05); }
  .who { display: grid; min-width: 0; flex: 1; }
  .who strong { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sub { color: var(--faint); font-size: 0.74rem; }
  .dot { width: 9px; height: 9px; border-radius: 50%; background: var(--faint); flex: none; }
  .dot.good { background: #5ff0b0; box-shadow: 0 0 8px #5ff0b0; }
  .dot.busy { background: #ffc46b; }
  .dot.bad { background: #ff7a8a; }
  .dot.warn { background: #ffc46b; }
  .state { font-size: 0.78rem; color: var(--muted); }
  .state.good { color: #5ff0b0; }
  .state.bad { color: #ff9aa8; }
  .msg { margin: 0; color: var(--muted); font-size: 0.84rem; }
  .msg.warn { color: #ffc46b; }
  .note { margin: 0; color: var(--faint); font-size: 0.78rem; }
  .actions { display: flex; flex-wrap: wrap; gap: 6px; }
  .btn { padding: 6px 12px; border-radius: 9px; background: rgb(255 255 255 / 0.1); border: 1px solid var(--line-strong); font-size: 0.82rem; }
  .btn:hover:not(:disabled) { background: rgb(255 255 255 / 0.16); }
  .btn.primary { background: linear-gradient(120deg, var(--accent), var(--accent-2)); color: #0a0a12; border: 0; font-weight: 600; }
  .btn:disabled { opacity: 0.6; cursor: progress; }
  ul { list-style: none; margin: 0; padding: 0; display: grid; gap: 2px; }
  .row { display: flex; align-items: center; gap: 9px; width: 100%; padding: 7px 8px; border-radius: 9px; text-align: left; font-size: 0.84rem; }
  .row:hover:not(:disabled) { background: rgb(255 255 255 / 0.08); }
  .row:disabled { cursor: default; }
  .row[aria-current='true'] { background: rgb(var(--accent-rgb) / 0.12); }
  .nm { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .st { color: var(--faint); font-size: 0.76rem; }
  .foot { margin: 0; color: var(--faint); font-size: 0.74rem; }
</style>
