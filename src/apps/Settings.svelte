<script lang="ts">
  import { config } from '../lib/config.svelte';
  import { host } from '../lib/host.svelte';
  import { setFullscreen, storageOverview, storageValidatePath } from '../lib/ipc';
  import type { StorageOverview, Verdict } from '../lib/runtime-types';
  import { defaultStorage, FOLDER_IDS, type FolderId } from '../lib/types';
  import { ui } from '../lib/ui.svelte';
  import { ACCENTS, DOCK_POSITIONS, WALLPAPERS, type Accent, type Wallpaper } from '../lib/types';
  import { ACCENT_COLORS } from '../lib/theme';
  import Icon from '../components/Icon.svelte';
  import type { IconName } from '../lib/icons';

  type Section = 'appearance' | 'desktop' | 'startup' | 'folders' | 'shortcuts' | 'storage';
  const sections: { id: Section; label: string; icon: IconName }[] = [
    { id: 'appearance', label: 'Appearance', icon: 'palette' },
    { id: 'desktop', label: 'Desktop & dock', icon: 'grid' },
    { id: 'startup', label: 'Startup & display', icon: 'monitor' },
    { id: 'folders', label: 'Linux folders', icon: 'folder' },
    { id: 'shortcuts', label: 'Shortcuts', icon: 'keyboard' },
    { id: 'storage', label: 'Settings file', icon: 'info' },
  ];
  let current = $state<Section>('appearance');
  let confirmReset = $state(false);

  const c = $derived(config.value);
  const WP_LABEL: Record<Wallpaper, string> = { nebula: 'Nebula', dusk: 'Dusk', peaks: 'Peaks', grid: 'Mono' };
  const WP_PREVIEW: Record<Wallpaper, string> = {
    nebula: 'radial-gradient(circle at 25% 30%, #6b3fe0, transparent 60%), radial-gradient(circle at 80% 75%, #1fb8e6aa, transparent 55%), #0a0818',
    dusk: 'linear-gradient(#150f33, #5a2a70 55%, #d9507a 80%, #ff9a5c)',
    peaks: 'linear-gradient(#050d18, #0b2f40 60%, #1b6a73)',
    grid: 'radial-gradient(circle at 50% 40%, #1c1c34, #05050a)',
  };

  const shortcuts: [string, string][] = [
    ['Alt + F1', 'Open / close the application launcher'],
    ['Alt + 1 … 9', 'Switch workspace'],
    ['Alt + W', 'Close the focused window'],
    ['F11', 'Toggle full screen'],
    ['Alt + Shift + Q', 'Open the power / exit menu'],
    ['Ctrl + Alt + Q', 'Return to Windows immediately'],
    ['Esc', 'Close the open menu'],
  ];

  // ---- Linux folders (saved preferences; applied from M4 on) ----
  let overview = $state<StorageOverview | null>(null);
  let overviewError = $state<string | null>(null);
  let draft = $state<Record<string, string>>({});
  let checked = $state<Record<string, Verdict | null>>({});

  async function loadOverview() {
    overviewError = null;
    try {
      overview = await storageOverview();
    } catch (e) {
      overviewError = String(e);
    }
  }

  $effect(() => {
    if (current === 'folders') void loadOverview();
  });

  async function setPref(mutate: (c: typeof config.value) => void) {
    config.update(mutate);
    await config.flush();
    await loadOverview();
  }

  async function checkPath(id: string) {
    const text = (draft[id] ?? '').trim();
    try {
      checked[id] = await storageValidatePath(text);
    } catch (e) {
      checked[id] = { kind: 'unsupported', message: String(e) };
    }
  }

  async function useCustom(id: FolderId) {
    const text = (draft[id] ?? '').trim();
    await setPref((x) => (x.storage[id].windows_path = text));
    draft[id] = '';
    checked[id] = null;
  }

  async function restoreFolders() {
    await setPref((x) => (x.storage = defaultStorage()));
    draft = {};
    checked = {};
    ui.toast('Folder preferences restored to defaults');
  }

  async function fullscreenNow() {
    try {
      await setFullscreen(!host.fullscreen);
    } catch (e) {
      ui.toast(`Could not change full-screen: ${String(e)}`, 'error');
    }
  }
</script>

<div class="settings">
  <nav aria-label="Settings sections">
    {#each sections as s}
      <button class:active={current === s.id} aria-current={current === s.id ? 'page' : undefined} onclick={() => (current = s.id)}>
        <Icon name={s.icon} size={16} /> <span>{s.label}</span>
      </button>
    {/each}
  </nav>

  <div class="page">
    {#if config.warning}
      <div class="notice warn" role="alert"><Icon name="alert" size={16} /> {config.warning}</div>
    {/if}
    {#if config.saveError}
      <div class="notice err" role="alert"><Icon name="alert" size={16} /> Settings could not be saved: {config.saveError}</div>
    {/if}

    {#if current === 'appearance'}
      <h3>Wallpaper</h3>
      <div class="wps" role="radiogroup" aria-label="Wallpaper">
        {#each WALLPAPERS as w}
          <button role="radio" aria-checked={c.appearance.wallpaper === w} class="wp" class:sel={c.appearance.wallpaper === w}
                  onclick={() => config.update((x) => (x.appearance.wallpaper = w))}>
            <span class="prev" style="background:{WP_PREVIEW[w]}"></span>
            <span>{WP_LABEL[w]}</span>
          </button>
        {/each}
      </div>

      <h3>Accent colour</h3>
      <div class="swatches" role="radiogroup" aria-label="Accent colour">
        {#each ACCENTS as a}
          <button role="radio" aria-checked={c.appearance.accent === a} aria-label={a} title={a} class="sw" class:sel={c.appearance.accent === a}
                  style="--s:{ACCENT_COLORS[a as Accent].main}" onclick={() => config.update((x) => (x.appearance.accent = a))}>
            {#if c.appearance.accent === a}<Icon name="check" size={14} stroke={2.4} />{/if}
          </button>
        {/each}
      </div>

      <h3>Glass transparency <span class="val mono">{c.appearance.glass}%</span></h3>
      <input type="range" min="0" max="100" step="1" value={c.appearance.glass} aria-label="Glass transparency"
             oninput={(e) => config.update((x) => (x.appearance.glass = +e.currentTarget.value))} />

      <label class="row">
        <span><strong>Animations</strong><small>Subtle motion for windows, menus and the dock. Also follows your Windows “reduce motion” setting.</small></span>
        <input type="checkbox" role="switch" checked={c.appearance.animations} onchange={(e) => config.update((x) => (x.appearance.animations = e.currentTarget.checked))} />
      </label>
    {:else if current === 'desktop'}
      <h3>Dock position</h3>
      <div class="seg" role="radiogroup" aria-label="Dock position">
        {#each DOCK_POSITIONS as p}
          <button role="radio" aria-checked={c.dock.position === p} class:sel={c.dock.position === p} onclick={() => config.update((x) => (x.dock.position = p))}>{p === 'bottom' ? 'Bottom' : 'Left'}</button>
        {/each}
      </div>

      <h3>Dock icon size <span class="val mono">{c.dock.icon_size}px</span></h3>
      <input type="range" min="32" max="72" step="2" value={c.dock.icon_size} aria-label="Dock icon size"
             oninput={(e) => config.update((x) => (x.dock.icon_size = +e.currentTarget.value))} />

      <h3>Workspaces <span class="val mono">{c.shell.workspace_count}</span></h3>
      <input type="range" min="2" max="9" step="1" value={c.shell.workspace_count} aria-label="Number of workspaces"
             oninput={(e) => config.update((x) => (x.shell.workspace_count = +e.currentTarget.value))} />

      <h3>Clock</h3>
      <label class="row"><span><strong>24-hour clock</strong></span>
        <input type="checkbox" role="switch" checked={c.shell.clock_24h} onchange={(e) => config.update((x) => (x.shell.clock_24h = e.currentTarget.checked))} /></label>
      <label class="row"><span><strong>Show seconds</strong><small>Makes the clock update every second instead of every minute.</small></span>
        <input type="checkbox" role="switch" checked={c.shell.show_seconds} onchange={(e) => config.update((x) => (x.shell.show_seconds = e.currentTarget.checked))} /></label>
      <label class="row"><span><strong>Show date</strong></span>
        <input type="checkbox" role="switch" checked={c.shell.show_date} onchange={(e) => config.update((x) => (x.shell.show_date = e.currentTarget.checked))} /></label>
    {:else if current === 'startup'}
      <h3>Display</h3>
      <div class="row">
        <span><strong>Full screen now</strong><small>Currently {host.fullscreen ? 'full screen' : 'windowed'}. F11 also toggles this.</small></span>
        <button class="btn" onclick={fullscreenNow}>{host.fullscreen ? 'Leave full screen' : 'Enter full screen'}</button>
      </div>
      <label class="row"><span><strong>Start in full screen</strong><small>VLINUX opens full screen. Windows stays reachable (Win key, Alt+Tab, taskbar).</small></span>
        <input type="checkbox" role="switch" checked={c.shell.start_fullscreen} onchange={(e) => config.update((x) => (x.shell.start_fullscreen = e.currentTarget.checked))} /></label>
      <h3>Linux runtime</h3>
      <p class="muted">Selected distribution: <strong>{config.value.runtime.selected_distro ?? 'none'}</strong>. Choose it in Linux Setup. VLINUX never starts Linux at launch and never changes WSL's own default distribution. WSL memory/CPU limits and auto-start options will appear here with full distribution management (M5), not before they do something.</p>
    {:else if current === 'folders'}
      <h3>Shared personal folders</h3>
      <p class="muted">Your Linux home can point straight at your real Windows folders — the same files, no copies, no sync. System files, package data and caches always stay private to Linux.</p>
      <div class="notice warn" role="note"><Icon name="info" size={16} /> These are saved preferences. They are not applied to any distribution yet — that arrives with the file manager (M4), and an existing Linux folder will never be replaced without asking.</div>
      {#if overviewError}<div class="notice err" role="alert"><Icon name="alert" size={16} /> {overviewError}</div>{/if}
      {#if overview}
        {#each overview.folders as f (f.id)}
          {@const id = f.id as FolderId}
          <div class="folder" data-folder={f.id}>
            <label class="row nopad">
              <span><strong>{f.label}</strong> <code class="mono dim">{f.linux_path}</code>
                <small class:bad={f.verdict?.kind === 'unsupported'}>{f.points_to}</small></span>
              <input type="checkbox" role="switch" aria-label="Share {f.label} with Windows" checked={f.mode === 'shared'}
                     onchange={(e) => setPref((x) => (x.storage[id].mode = e.currentTarget.checked ? 'shared' : 'private'))} />
            </label>
            {#if f.mode === 'shared'}
              <div class="custom">
                {#if f.custom_windows_path}
                  <span class="muted">Custom location: <code class="mono">{f.custom_windows_path}</code></span>
                  <button class="btn" onclick={() => setPref((x) => (x.storage[id].windows_path = null))}>Use the Windows default</button>
                {:else}
                  <span class="muted">Windows default{f.default_windows_path ? '' : ' (not reported by Windows)'}: <code class="mono">{f.default_windows_path ?? '—'}</code></span>
                {/if}
                <div class="pathrow">
                  <input class="text" type="text" spellcheck="false" placeholder="Choose another Windows folder, e.g. D:\Media\{f.label}" aria-label="Alternative Windows folder for {f.label}"
                         bind:value={draft[f.id]} oninput={() => (checked[f.id] = null)} />
                  <button class="btn" onclick={() => checkPath(f.id)} disabled={!(draft[f.id] ?? '').trim()}>Check</button>
                  <button class="btn" onclick={() => useCustom(id)} disabled={!checked[f.id] || checked[f.id]?.kind === 'unsupported'}>Use this folder</button>
                </div>
                {#if checked[f.id]}
                  {@const v = checked[f.id]!}
                  <small class:bad={v.kind === 'unsupported'} class:warnc={v.kind === 'warning'}>
                    {v.kind === 'unsupported' ? `Cannot be shared: ${v.message}` : v.kind === 'warning' ? `Usable, with a caveat: ${v.message}` : `Looks good — Linux would see it at ${v.wsl_path}`}
                  </small>
                {/if}
              </div>
            {/if}
          </div>
        {/each}

        <h3>Development projects</h3>
        <div class="seg" role="radiogroup" aria-label="Development projects location">
          <button role="radio" aria-checked={c.storage.projects === 'linux_private'} class:sel={c.storage.projects === 'linux_private'} onclick={() => setPref((x) => (x.storage.projects = 'linux_private'))}>Inside Linux (recommended)</button>
          <button role="radio" aria-checked={c.storage.projects === 'shared'} class:sel={c.storage.projects === 'shared'} onclick={() => setPref((x) => (x.storage.projects = 'shared'))}>On Windows</button>
        </div>
        <p class="muted">{overview.notes[1]}</p>
        <p class="muted small">{overview.notes[2]}</p>
        {#if overview.notes[3]}<p class="muted small">{overview.notes[3]}</p>{/if}

        <h3>Defaults</h3>
        <button class="btn" onclick={restoreFolders}><Icon name="rotate" size={15} /> Restore folder defaults</button>
        <p class="muted small">Only changes these preferences. No file or folder is touched.</p>
      {:else if !overviewError}
        <p class="muted" aria-busy="true">Resolving where Windows keeps your folders…</p>
      {/if}
    {:else if current === 'shortcuts'}
      <h3>Keyboard shortcuts</h3>
      <dl class="keys">
        {#each shortcuts as [k, d]}
          <dt><kbd class="mono">{k}</kbd></dt><dd>{d}</dd>
        {/each}
      </dl>
    {:else}
      <h3>Where settings are stored</h3>
      <p class="muted">Your VLINUX settings are a small TOML file kept separate from every Linux distribution. Uninstalling VLINUX never touches your distributions or their files.</p>
      <code class="path mono">{config.path || '…'}</code>
      <h3>Reset</h3>
      {#if !confirmReset}
        <button class="btn" onclick={() => (confirmReset = true)}><Icon name="rotate" size={15} /> Reset VLINUX settings…</button>
      {:else}
        <div class="notice warn">
          This resets only the VLINUX appearance and desktop settings to their defaults. It does not touch any Linux distribution or file.
          <div class="acts">
            <button class="btn" onclick={() => (confirmReset = false)}>Cancel</button>
            <button class="btn danger" onclick={() => { config.reset(); confirmReset = false; ui.toast('Settings reset to defaults'); }}>Reset settings</button>
          </div>
        </div>
      {/if}
    {/if}
  </div>
</div>

<style>
  .settings { display: grid; grid-template-columns: 190px 1fr; height: 100%; min-height: 0; }
  nav { display: flex; flex-direction: column; gap: 2px; padding: 12px 8px; border-right: 1px solid var(--line); background: rgb(0 0 0 / 0.15); }
  nav button { display: flex; align-items: center; gap: 10px; padding: 9px 12px; border-radius: 10px; color: var(--muted); text-align: left; transition: background var(--dur), color var(--dur); }
  nav button:hover { background: rgb(255 255 255 / 0.06); color: var(--text); }
  nav button.active { background: rgb(var(--accent-rgb) / 0.18); color: var(--text); }
  nav button.active :global(svg) { color: var(--accent); }
  .page { padding: 6px 22px 22px; overflow: auto; }
  h3 { margin: 20px 0 10px; font-size: 0.74rem; text-transform: uppercase; letter-spacing: 0.12em; color: var(--muted); font-weight: 600; display: flex; justify-content: space-between; }
  .val { color: var(--accent); letter-spacing: 0; text-transform: none; }
  .muted { color: var(--muted); line-height: 1.55; margin: 0 0 10px; font-size: 0.88rem; }

  .wps { display: grid; grid-template-columns: repeat(auto-fill, minmax(120px, 1fr)); gap: 10px; }
  .wp { display: grid; gap: 6px; padding: 6px; border-radius: 12px; border: 1px solid var(--line); font-size: 0.82rem; color: var(--muted); transition: all var(--dur); }
  .wp:hover { border-color: var(--line-strong); }
  .wp.sel { border-color: var(--accent); color: var(--text); box-shadow: 0 0 0 3px rgb(var(--accent-rgb) / 0.2); }
  .prev { aspect-ratio: 16 / 10; border-radius: 8px; border: 1px solid var(--line); }

  .swatches { display: flex; gap: 10px; }
  .sw { width: 34px; height: 34px; border-radius: 50%; display: grid; place-items: center; background: var(--s); color: #0a0a12; border: 2px solid transparent; outline-offset: 3px; transition: transform var(--dur) var(--ease); }
  .sw:hover { transform: scale(1.1); }
  .sw.sel { border-color: #fff; }

  input[type='range'] { width: 100%; accent-color: var(--accent); }

  .row { display: flex; justify-content: space-between; align-items: center; gap: 16px; padding: 12px 0; border-bottom: 1px solid var(--line); cursor: pointer; }
  .row strong { display: block; font-weight: 600; font-size: 0.9rem; }
  .row small { display: block; color: var(--muted); margin-top: 3px; line-height: 1.4; font-size: 0.78rem; max-width: 46ch; }

  input[role='switch'] { appearance: none; flex: none; width: 40px; height: 22px; border-radius: 999px; background: rgb(255 255 255 / 0.14); position: relative; cursor: pointer; transition: background var(--dur); }
  input[role='switch']::after { content: ''; position: absolute; top: 3px; left: 3px; width: 16px; height: 16px; border-radius: 50%; background: #fff; transition: transform var(--dur) var(--ease); }
  input[role='switch']:checked { background: var(--accent); }
  input[role='switch']:checked::after { transform: translateX(18px); }

  .seg { display: inline-flex; padding: 3px; border-radius: 12px; background: rgb(255 255 255 / 0.06); border: 1px solid var(--line); }
  .seg button { padding: 7px 18px; border-radius: 9px; color: var(--muted); }
  .seg button.sel { background: var(--accent); color: #0a0a12; font-weight: 600; }

  .btn { display: inline-flex; align-items: center; gap: 8px; padding: 8px 14px; border-radius: 10px; background: rgb(255 255 255 / 0.09); border: 1px solid var(--line); }
  .btn:hover { background: rgb(255 255 255 / 0.15); }
  .btn.danger { background: rgb(255 90 110 / 0.2); border-color: rgb(255 110 130 / 0.6); color: #ffb3bd; }

  .folder { padding: 4px 0 10px; border-bottom: 1px solid var(--line); }
  .row.nopad { border-bottom: 0; padding-bottom: 6px; }
  .folder small { display: block; margin-top: 3px; font-size: 0.78rem; color: var(--muted); overflow-wrap: anywhere; }
  .folder small.bad, small.bad { color: #ffb3bd; }
  small.warnc { color: #ffb454; }
  .custom { display: grid; gap: 8px; padding: 2px 0 4px; font-size: 0.82rem; }
  .custom .btn { justify-self: start; }
  .pathrow { display: flex; gap: 8px; flex-wrap: wrap; }
  .text { flex: 1 1 220px; min-width: 0; padding: 7px 10px; border-radius: 10px; background: rgb(0 0 0 / 0.3); border: 1px solid var(--line); color: var(--text); }
  .btn:disabled { opacity: 0.45; cursor: not-allowed; }
  .small { font-size: 0.76rem; }
  .keys { display: grid; grid-template-columns: auto 1fr; gap: 10px 18px; margin: 0; align-items: center; }
  .keys dd { margin: 0; color: var(--muted); }
  kbd { padding: 3px 8px; border-radius: 7px; background: rgb(255 255 255 / 0.1); border: 1px solid var(--line); font-size: 0.78rem; white-space: nowrap; }

  .path { display: block; padding: 10px 12px; border-radius: 10px; background: rgb(0 0 0 / 0.35); border: 1px solid var(--line); font-size: 0.78rem; overflow-wrap: anywhere; user-select: all; }

  .notice { display: block; padding: 10px 14px; border-radius: 12px; margin: 10px 0; font-size: 0.84rem; line-height: 1.45; }
  .notice.warn { background: rgb(255 180 84 / 0.12); border: 1px solid rgb(255 180 84 / 0.4); }
  .notice.err { background: rgb(255 90 110 / 0.12); border: 1px solid rgb(255 110 130 / 0.5); }
  .notice :global(svg) { vertical-align: -3px; margin-right: 6px; }
  .acts { display: flex; gap: 8px; width: 100%; justify-content: flex-end; margin-top: 6px; }

  @media (max-width: 720px) {
    .settings { grid-template-columns: 1fr; grid-template-rows: auto 1fr; }
    nav { flex-direction: row; overflow-x: auto; border-right: 0; border-bottom: 1px solid var(--line); }
    nav span { display: none; }
  }
</style>
