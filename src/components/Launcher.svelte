<script lang="ts">
  import { untrack } from 'svelte';
  import { APPS } from '../lib/apps';
  import { config } from '../lib/config.svelte';
  import { distros } from '../lib/distros.svelte';
  import { linuxApps } from '../lib/linux-apps.svelte';
  import { describeApp, rankApps, matchScore, type LinuxApp } from '../lib/linux-apps-types';
  import { notify } from '../lib/notify.svelte';
  import { openApp } from '../lib/open-app';
  import { ui } from '../lib/ui.svelte';
  import { wm } from '../lib/wm.svelte';
  import AppTile from './AppTile.svelte';
  import Icon from './Icon.svelte';
  import LinuxTile from './LinuxTile.svelte';

  let query = $state('');
  let input: HTMLInputElement | undefined = $state();
  let sel = $state(0);

  const distro = $derived(config.value.runtime.selected_distro);
  const q = $derived(query.trim());

  const shell = $derived(
    APPS.filter((a) => !q || matchScore(q, { name: a.name, generic_name: null, comment: a.description, categories: [], keywords: [] }) > 0)
      .sort((a, b) => Number(b.status === 'available') - Number(a.status === 'available')),
  );

  const ranked = $derived(rankApps(linuxApps.apps, q));
  const recent = $derived(
    q ? [] : linuxApps.recents.map((id) => linuxApps.apps.find((a) => a.id === id)).filter((a): a is LinuxApp => !!a && a.launchable),
  );
  const recentIds = $derived(new Set(recent.map((a) => a.id)));
  const rest = $derived(ranked.filter((a) => !recentIds.has(a.id)));

  type Entry = { key: string; kind: 'shell'; id: string } | { key: string; kind: 'linux'; app: LinuxApp };
  // The order the keyboard walks through: shell apps, recents, then every Linux app.
  const entries = $derived<Entry[]>([
    ...shell.filter((a) => a.status === 'available').map((a): Entry => ({ key: `s:${a.id}`, kind: 'shell', id: a.id })),
    ...[...recent, ...rest].filter((a) => a.launchable).map((a): Entry => ({ key: `l:${a.id}`, kind: 'linux', app: a })),
  ]);

  $effect(() => {
    input?.focus();
  });
  // Load (or refresh when stale) once, when the launcher opens or the selection changes.
  $effect(() => {
    const d = distro;
    untrack(() => void linuxApps.load(d));
  });
  // Icons are fetched only for what is listed, in one batch.
  $effect(() => {
    const ids = [...recent, ...rest].slice(0, 64).map((a) => a.id);
    if (ids.length) untrack(() => void linuxApps.loadIcons(ids));
  });
  $effect(() => {
    q; // a new query starts at the first result
    sel = 0;
  });
  $effect(() => {
    document.getElementById(`opt-${sel}`)?.scrollIntoView({ block: 'nearest' });
  });

  const idx = (key: string) => entries.findIndex((e) => e.key === key);

  function launchShell(id: string) {
    const app = APPS.find((a) => a.id === id);
    if (!app || app.status !== 'available') return;
    wm.open(app);
    ui.close();
  }
  async function launchLinux(app: LinuxApp) {
    if (!app.launchable) {
      notify.info(app.reason ?? `${app.name} can't be launched from here.`, 'launcher');
      return;
    }
    ui.close();
    await linuxApps.launch(app);
  }
  function activate(e: Entry | undefined) {
    if (!e) return;
    if (e.kind === 'shell') launchShell(e.id);
    else void launchLinux(e.app);
  }

  async function startDistro() {
    if (!distro) return;
    await distros.start(distro);
    await linuxApps.load(distro, true);
  }

  function key(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.stopPropagation();
      ui.close();
    } else if (e.key === 'Enter') {
      e.preventDefault();
      activate(entries[sel]);
    } else if (e.key === 'ArrowDown') {
      e.preventDefault();
      sel = Math.min(entries.length - 1, sel + 1);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      sel = Math.max(0, sel - 1);
    }
  }

  const distroName = $derived(distro ?? '');
  const startPending = $derived(!!distro && !!distros.pending[distro]);
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="backdrop" onpointerdown={(e) => e.target === e.currentTarget && ui.close()} onkeydown={key}>
  <div class="panel glass" role="dialog" aria-modal="true" aria-label="Application launcher">
    <label class="search">
      <Icon name="search" size={18} />
      <input bind:this={input} bind:value={query} placeholder="Search applications…" spellcheck="false" autocomplete="off"
             role="combobox" aria-expanded="true" aria-controls="launcher-list" aria-autocomplete="list"
             aria-activedescendant={entries.length ? `opt-${sel}` : undefined} aria-label="Search applications" />
      <kbd class="mono">Esc</kbd>
    </label>

    <div class="scroll" id="launcher-list" role="listbox" aria-label="Applications">
      {#if shell.length}
        <h3>VLINUX</h3>
        <ul class="grid">
          {#each shell as a (a.id)}
            {@const i = idx(`s:${a.id}`)}
            <li>
              <button class="app" id={i >= 0 ? `opt-${i}` : undefined} role="option" aria-selected={i === sel} class:sel={i === sel && i >= 0}
                      aria-disabled={a.status === 'planned'} onclick={() => launchShell(a.id)} onpointermove={() => i >= 0 && (sel = i)}>
                <AppTile app={a} size={44} />
                <span class="meta">
                  <span class="name">{a.name}{#if a.status === 'planned'}<span class="badge mono">{a.milestone}</span>{/if}</span>
                  <span class="desc">{a.status === 'planned' ? `Not available yet — planned for ${a.milestone}` : a.description}</span>
                </span>
              </button>
            </li>
          {/each}
        </ul>
      {/if}

      <h3 class="lin">
        <span>Linux applications{#if distroName} <span class="where">· {distroName}</span>{/if}</span>
        {#if distro}
          <button class="refresh" aria-label="Refresh Linux applications" title="Read the application list again" disabled={linuxApps.phase === 'loading'}
                  onclick={() => linuxApps.load(distro, true)}><Icon name="rotate" size={14} /></button>
        {/if}
      </h3>

      {#if !distro}
        <div class="state">
          <p>No Linux distribution is selected. Choose one in Linux Setup to see its applications here.</p>
          <button class="btn" onclick={() => { openApp('setup'); ui.close(); }}>Open Linux Setup</button>
        </div>
      {:else if linuxApps.phase === 'loading' && !linuxApps.report}
        <div class="state" aria-busy="true"><p>Reading the applications of “{distro}”…</p>
          <ul class="skeleton" aria-hidden="true">{#each [0, 1, 2, 3] as n (n)}<li></li>{/each}</ul>
        </div>
      {:else if linuxApps.phase === 'stopped'}
        <div class="state">
          <p>{linuxApps.message ?? `“${distro}” is not running.`}</p>
          <button class="btn primary" disabled={startPending} onclick={startDistro}>{startPending ? 'Starting…' : `Start ${distro}`}</button>
        </div>
      {:else if linuxApps.phase === 'unavailable' || linuxApps.phase === 'error'}
        <div class="state err" role="alert">
          <p>{linuxApps.error ?? linuxApps.message ?? 'The applications could not be read.'}</p>
          <button class="btn" onclick={() => linuxApps.load(distro, true)}>Try again</button>
        </div>
      {:else if linuxApps.phase === 'ready' || linuxApps.report}
        {#if linuxApps.message && !linuxApps.apps.length}
          <div class="state"><p>{linuxApps.message}</p></div>
        {:else}
          {#if recent.length}
            <h4>Recent</h4>
            <ul class="grid">
              {#each recent as a (a.id)}
                {@const i = idx(`l:${a.id}`)}
                <li>
                  <button class="app" id={`opt-${i}`} role="option" aria-selected={i === sel} class:sel={i === sel} onclick={() => launchLinux(a)} onpointermove={() => (sel = i)}>
                    <LinuxTile name={a.name} src={linuxApps.icons[a.id]} size={44} />
                    <span class="meta"><span class="name">{a.name}</span><span class="desc">{describeApp(a, linuxApps.apps)}</span></span>
                  </button>
                </li>
              {/each}
            </ul>
            {#if rest.length}<h4>All applications</h4>{/if}
          {/if}
          {#if rest.length}
            <ul class="grid">
              {#each rest as a (a.id)}
                {@const i = a.launchable ? idx(`l:${a.id}`) : -1}
                <li>
                  <button class="app" id={i >= 0 ? `opt-${i}` : undefined} role="option" aria-selected={i === sel && i >= 0} class:sel={i === sel && i >= 0}
                          aria-disabled={!a.launchable} title={a.reason ?? undefined} onclick={() => launchLinux(a)} onpointermove={() => i >= 0 && (sel = i)}>
                    <LinuxTile name={a.name} src={linuxApps.icons[a.id]} size={44} />
                    <span class="meta">
                      <span class="name">{a.name}{#if linuxApps.launching[a.id]}<span class="badge">starting…</span>{/if}</span>
                      <span class="desc">{a.launchable ? describeApp(a, linuxApps.apps) : a.reason}</span>
                    </span>
                  </button>
                </li>
              {/each}
            </ul>
          {:else if q && !recent.length}
            <div class="state"><p>No Linux application matches “{query}”.</p></div>
          {/if}
          {#each linuxApps.report?.warnings ?? [] as w (w)}<p class="warn">{w}</p>{/each}
        {/if}
      {/if}

      {#if q && !shell.length && !linuxApps.apps.length && linuxApps.phase !== 'loading' && !(!distro)}
        <p class="empty">Nothing matches “{query}”.</p>
      {/if}
    </div>

    <footer>
      <span><kbd class="mono">↑</kbd><kbd class="mono">↓</kbd> choose · <kbd class="mono">Enter</kbd> open</span>
      <span>Linux apps open as separate Windows windows (WSLg), not inside VLINUX.</span>
    </footer>
  </div>
</div>

<style>
  .backdrop {
    position: absolute; inset: 0; z-index: 80; display: grid; place-items: start center;
    padding-top: clamp(40px, 9vh, 120px);
    background: rgb(4 4 10 / 0.5); backdrop-filter: blur(6px); animation: fade-in 180ms ease both;
  }
  .panel {
    width: min(760px, calc(100vw - 24px)); max-height: min(640px, calc(100vh - 80px)); display: flex; flex-direction: column;
    border-radius: 22px; padding: 14px; background: rgb(var(--panel-rgb) / calc(var(--glass-a) + 0.2));
    animation: pop-in 260ms var(--ease) both;
  }
  .search {
    display: flex; align-items: center; gap: 10px; padding: 0 14px; height: 48px; border-radius: 14px; flex: none;
    background: rgb(255 255 255 / 0.06); border: 1px solid var(--line); color: var(--muted); cursor: text;
  }
  .search:focus-within { border-color: rgb(var(--accent-rgb) / 0.7); box-shadow: 0 0 0 3px rgb(var(--accent-rgb) / 0.18); }
  input { flex: 1; background: none; border: 0; outline: 0; color: var(--text); font-size: 1.05rem; user-select: text; }
  kbd { font-size: 0.7rem; padding: 2px 6px; border-radius: 6px; background: rgb(255 255 255 / 0.08); color: var(--faint); margin: 0 2px; }

  .scroll { overflow: auto; margin-top: 4px; min-height: 120px; padding-right: 2px; }
  h3 { margin: 14px 4px 6px; font-size: 0.74rem; font-weight: 600; letter-spacing: 0.08em; text-transform: uppercase; color: var(--faint); display: flex; align-items: center; justify-content: space-between; }
  h3 .where { text-transform: none; letter-spacing: 0; color: var(--muted); }
  h4 { margin: 10px 4px 4px; font-size: 0.72rem; font-weight: 600; color: var(--faint); }
  .refresh { width: 26px; height: 24px; display: grid; place-items: center; border-radius: 7px; color: var(--muted); }
  .refresh:hover:not(:disabled) { background: rgb(255 255 255 / 0.1); color: var(--text); }
  .refresh:disabled { opacity: 0.4; cursor: progress; }

  .grid { list-style: none; margin: 0; padding: 0; display: grid; gap: 4px; grid-template-columns: repeat(auto-fill, minmax(280px, 1fr)); }
  .app { display: flex; align-items: center; gap: 12px; width: 100%; padding: 8px 10px; border-radius: 14px; text-align: left; transition: background var(--dur); }
  .app:not([aria-disabled='true']):hover, .app.sel { background: rgb(255 255 255 / 0.09); }
  .app.sel { box-shadow: inset 0 0 0 1px rgb(var(--accent-rgb) / 0.5); }
  .app[aria-disabled='true'] { cursor: not-allowed; }
  .app[aria-disabled='true'] .name, .app[aria-disabled='true'] .desc { opacity: 0.6; }
  .meta { display: grid; gap: 2px; min-width: 0; }
  .name { font-weight: 600; display: flex; align-items: center; gap: 8px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .badge { font-size: 0.62rem; padding: 1px 6px; border-radius: 6px; background: rgb(255 255 255 / 0.12); color: var(--muted); font-weight: 500; }
  .desc { color: var(--muted); font-size: 0.8rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  .state { display: grid; gap: 10px; justify-items: start; padding: 12px 10px; color: var(--muted); font-size: 0.88rem; }
  .state p { margin: 0; }
  .state.err p { color: #ff9aa8; }
  .btn { padding: 6px 14px; border-radius: 9px; background: rgb(255 255 255 / 0.1); border: 1px solid var(--line-strong); font-size: 0.84rem; color: var(--text); }
  .btn:hover:not(:disabled) { background: rgb(255 255 255 / 0.16); }
  .btn.primary { background: linear-gradient(120deg, var(--accent), var(--accent-2)); color: #0a0a12; border: 0; font-weight: 600; }
  .btn:disabled { opacity: 0.6; cursor: progress; }
  .skeleton { list-style: none; margin: 0; padding: 0; display: grid; gap: 6px; width: 100%; }
  .skeleton li { height: 44px; border-radius: 12px; background: linear-gradient(90deg, rgb(255 255 255 / 0.04), rgb(255 255 255 / 0.09), rgb(255 255 255 / 0.04)); background-size: 200% 100%; animation: shimmer 1.4s linear infinite; }
  @keyframes shimmer { from { background-position: 200% 0; } to { background-position: -200% 0; } }
  .warn { margin: 8px 4px 0; color: var(--faint); font-size: 0.76rem; }
  .empty { color: var(--muted); text-align: center; padding: 20px 0; }
  footer { margin: 10px 4px 0; padding-top: 10px; border-top: 1px solid var(--line); color: var(--faint); font-size: 0.76rem; display: flex; gap: 12px; justify-content: space-between; flex-wrap: wrap; flex: none; }
</style>
