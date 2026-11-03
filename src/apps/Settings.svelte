<script lang="ts">
  import { untrack } from 'svelte';
  import { config } from '../lib/config.svelte';
  import { mapping } from '../lib/mapping.svelte';
  import { mappingBadge, type Resolution } from '../lib/fs-types';
  import { openFilesAt } from '../lib/open-files';
  import { runtime } from '../lib/runtime.svelte';
  import { host } from '../lib/host.svelte';
  import { storageOverview, storageValidatePath } from '../lib/ipc';
  import type { StorageOverview, Verdict } from '../lib/runtime-types';
  import { defaultStorage, FOLDER_IDS, type FolderId } from '../lib/types';
  import { ui } from '../lib/ui.svelte';
  import { ACCENTS, DOCK_POSITIONS, WALLPAPERS, type Accent, type Wallpaper } from '../lib/types';
  import { ACCENT_COLORS } from '../lib/theme';
  import Icon from '../components/Icon.svelte';
  import WslConfigPanel from '../components/WslConfigPanel.svelte';
  import { distros } from '../lib/distros.svelte';
  import type { IconName } from '../lib/icons';

  type Section = 'appearance' | 'desktop' | 'startup' | 'folders' | 'wsl' | 'shortcuts' | 'storage';
  const sections: { id: Section; label: string; icon: IconName }[] = [
    { id: 'appearance', label: 'Appearance', icon: 'palette' },
    { id: 'desktop', label: 'Desktop & dock', icon: 'grid' },
    { id: 'startup', label: 'Startup & display', icon: 'monitor' },
    { id: 'folders', label: 'Linux folders', icon: 'folder' },
    { id: 'wsl', label: 'WSL configuration', icon: 'layers' },
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
    ['Ctrl + Shift + T / W', 'Terminal: new tab / close tab'],
    ['Ctrl + Shift + C / V', 'Terminal: copy selection / paste (Ctrl + C is the shell\'s interrupt)'],
  ];

  // ---- WSL configuration: cached discovery only; reading a distribution's file never starts it ----
  let wslChosen = $state<string | null>(null);
  const wslNames = $derived(distros.cards.filter((x) => x.kind === 'regular').map((x) => x.name));
  const wslPick = $derived(wslChosen && wslNames.includes(wslChosen) ? wslChosen : (distros.current?.name ?? wslNames[0] ?? null));
  $effect(() => {
    if (current === 'wsl' && !distros.overview) untrack(() => void distros.refresh(false));
  });

  // ---- Linux folders: saved preferences, and (separately) what a check of the distribution found ----
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

  const distro = $derived(runtime.selectedDistro?.name ?? null);
  let chosen = $state<Record<string, Resolution | null>>({});
  let confirming = $state<string | null>(null);

  $effect(() => {
    if (current === 'folders') {
      const d = distro;
      untrack(() => {
        void loadOverview();
        if (!runtime.report) void runtime.refresh(false);
        void mapping.load(d);
      });
    }
  });

  async function setPref(mutate: (c: typeof config.value) => void) {
    config.update(mutate);
    await config.flush();
    await loadOverview();
    // A changed preference never changes the distribution by itself. If the user has already
    // checked it, look again so the badges keep telling the truth about what is really linked.
    if (distro && mapping.checkedOnce) await mapping.check(distro);
  }

  async function applyFolder(id: string, resolution: Resolution | null) {
    if (!distro) return;
    confirming = null;
    await mapping.apply(distro, id, resolution);
    chosen[id] = null;
  }

  function homeFolder(linuxPath: string): string {
    const home = mapping.report?.home;
    return home ? linuxPath.replace(/^~/, home) : linuxPath;
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
      await host.toggleFullscreen();
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
      <div class="notice" role="note"><Icon name="info" size={16} /> {overview?.notes[0] ?? 'A folder is only connected after it has been checked and applied for a distribution.'}</div>
      <div class="mapbar" data-mapbar>
        <span class="muted">Distribution: <strong>{distro ?? 'none selected'}</strong></span>
        <button class="btn" data-act="check-mappings" disabled={!distro || mapping.loading} onclick={() => distro && mapping.check(distro)}><Icon name="rotate" size={15} /> {mapping.loading ? 'Checking…' : 'Check status'}</button>
        {#if distro}<button class="btn" onclick={() => openFilesAt(distro)}><Icon name="folder" size={15} /> Open Files</button>{/if}
      </div>
      <p class="muted small">{distro ? `“Check status” looks inside ${distro} (and starts it if it is stopped). It changes nothing.` : 'Choose a distribution in Linux Setup to check and apply these folders.'}</p>
      {#if mapping.error}<div class="notice err" role="alert"><Icon name="alert" size={16} /> {mapping.error}</div>{/if}
      {#if overviewError}<div class="notice err" role="alert"><Icon name="alert" size={16} /> {overviewError}</div>{/if}
      {#if overview}
        {#each overview.folders as f (f.id)}
          {@const id = f.id as FolderId}
          {@const st = mapping.statusOf(f.id)}
          {@const badge = mappingBadge(st, f.mode)}
          {@const checkedReport = mapping.report?.checked === true}
          <div class="folder" data-folder={f.id}>
            <label class="row nopad">
              <span><strong>{f.label}</strong> <code class="mono dim">{f.linux_path}</code>
                <span class="badge {badge.tone}" data-badge={badge.tone}>{badge.text}</span>
                <small class:bad={f.verdict?.kind === 'unsupported'}>{f.points_to}</small></span>
              <input type="checkbox" role="switch" aria-label="Share {f.label} with Windows" checked={f.mode === 'shared'}
                     onchange={(e) => setPref((x) => (x.storage[id].mode = e.currentTarget.checked ? 'shared' : 'private'))} />
            </label>

            <div class="mapstate" data-mapstate={checkedReport && st ? st.state.state : 'unchecked'}>
              {#if !distro}
                <small>No distribution is selected, so nothing can be linked yet.</small>
              {:else if !checkedReport || !st}
                <small>Preferred only — VLINUX hasn’t looked inside {distro} yet. Use “Check status”.</small>
              {:else if st.state.state === 'applied'}
                <small class="okc">{st.linux_path} is a link to <code class="mono">{st.state.target}</code> — files appear on both sides immediately.</small>
                <div class="macts">
                  <button class="btn" onclick={() => openFilesAt(distro, { space: 'linux', path: homeFolder(st.linux_path) })}>Open in Files</button>
                  <button class="btn" data-act="undo" disabled={mapping.busy === f.id} onclick={() => distro && mapping.undo(distro, f.id)}>Undo link</button>
                </div>
              {:else if st.state.state === 'linked_but_private'}
                <small class="warnc">Preference is Linux-private, but {st.linux_path} is still linked to <code class="mono">{st.state.target}</code>. Nothing was removed.</small>
                <div class="macts"><button class="btn" data-act="undo" disabled={mapping.busy === f.id} onclick={() => distro && mapping.undo(distro, f.id)}>Undo link</button></div>
              {:else if st.state.state === 'ready'}
                <small>{st.state.note}</small>
                {#if confirming === f.id}
                  {@const replacing = st.state.needs}
                  <div class="confirm" role="group" aria-label="Confirm linking {f.label}">
                    {#if replacing}
                      <p>{st.linux_path} is an empty folder. Confirming replaces <strong>only that empty folder</strong> with a link to your Windows folder. It can be undone.</p>
                    {:else}
                      <p>Confirming creates <strong>one new symbolic link</strong> inside {distro}: <code class="mono">{st.linux_path}</code> will point to <code class="mono">{f.points_to}</code>. Nothing else changes — no file is copied, moved, renamed or deleted, and nothing happens on the Windows side. <strong>Undo link</strong> removes only this link.</p>
                    {/if}
                    <div class="macts">
                      <button class="btn danger" data-act="confirm-apply" disabled={mapping.busy === f.id} onclick={() => applyFolder(f.id, st.state.state === 'ready' ? st.state.needs : null)}>{replacing ? 'Yes, replace the empty folder' : 'Yes, create the link'}</button>
                      <button class="btn" onclick={() => (confirming = null)}>Cancel</button>
                    </div>
                  </div>
                {:else}
                  <div class="macts">
                    <button class="btn primary" data-act="apply" disabled={mapping.busy === f.id} onclick={() => (confirming = f.id)}>Review and apply…</button>
                  </div>
                {/if}
              {:else if st.state.state === 'conflict'}
                {@const c1 = st.state}
                <div class="conflict" role="group" aria-label="{f.label} needs your decision">
                  <strong>{st.linux_path} can’t be linked automatically</strong>
                  <p>{c1.detail}</p>
                  {#if c1.entries.length}
                    <ul class="ents">{#each c1.entries as en}<li><code class="mono">{en}</code></li>{/each}{#if c1.more}<li>… and {c1.more} more</li>{/if}</ul>
                  {/if}
                  {#if c1.offers.length}
                    <div class="offers" role="radiogroup" aria-label="Choose what to do">
                      {#each c1.offers as o}
                        <label class="offer" class:sel={chosen[f.id] === o.resolution}>
                          <input type="radio" name="res-{f.id}" value={o.resolution} checked={chosen[f.id] === o.resolution} onchange={() => (chosen[f.id] = o.resolution)} />
                          <span><strong>{o.label}</strong><small>{o.description}</small></span>
                        </label>
                      {/each}
                    </div>
                    <div class="macts">
                      <button class="btn primary" data-act="apply-resolution" disabled={!chosen[f.id] || mapping.busy === f.id} onclick={() => applyFolder(f.id, chosen[f.id] ?? null)}>Confirm and apply</button>
                    </div>
                  {:else}
                    <p class="muted small">There is no safe automatic option. Nothing was changed. Move or rename it yourself in Files, then check again.</p>
                    <div class="macts"><button class="btn" onclick={() => openFilesAt(distro, { space: 'linux', path: homeFolder(st.linux_path).replace(/\/[^/]+$/, '') || '/' })}>Open in Files</button></div>
                  {/if}
                </div>
              {:else if st.state.state === 'unavailable'}
                <small class="bad">Can’t be linked: {st.state.reason}</small>
              {:else if st.state.state === 'failed'}
                <small class="bad">Failed: {st.state.message}</small>
              {:else if st.state.state === 'private'}
                <small>Linux-private inside {distro}. Nothing is shared.</small>
              {/if}
              {#if mapping.messages[f.id]}<small class:okc={mapping.messages[f.id]?.ok} class:bad={!mapping.messages[f.id]?.ok} role="status" data-mapmsg>{mapping.messages[f.id]?.text}</small>{/if}
            </div>
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
        
        <h3>Defaults</h3>
        <button class="btn" onclick={restoreFolders}><Icon name="rotate" size={15} /> Restore folder defaults</button>
        <p class="muted small">Only changes these preferences. No file or folder is touched.</p>
      {:else if !overviewError}
        <p class="muted" aria-busy="true">Resolving where Windows keeps your folders…</p>
      {/if}
    {:else if current === 'wsl'}
      <h3>WSL configuration</h3>
      <p class="muted">How each distribution behaves (automount, interoperability, default user) and how WSL is limited on this PC. Nothing is written until you review a change and confirm it, a backup is taken first, and nothing is restarted for you.</p>
      {#if wslNames.length === 0}
        <div class="dm-card"><p class="muted" data-testid="wsl-section-empty">{distros.loading ? 'Looking at WSL…' : 'No distribution to configure. Open Distributions and press Refresh, or set one up in Linux Setup.'}</p></div>
      {:else}
        <div class="dm-card dm-row" style="margin: 10px 0">
          <label for="wsl-distro">Distribution</label>
          <select id="wsl-distro" class="dm-select" value={wslPick} onchange={(e) => (wslChosen = e.currentTarget.value)}>
            {#each wslNames as n}<option value={n}>{n}{n === distros.overview?.preferred_name ? ' (preferred)' : ''}</option>{/each}
          </select>
        </div>
        <WslConfigPanel distro={wslPick} windows />
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
  .mapbar { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; margin: 8px 0 2px; }
  .badge { display: inline-block; margin-left: 8px; padding: 1px 9px; border-radius: 999px; font-size: 0.7rem; border: 1px solid var(--line-strong); vertical-align: 1px; white-space: nowrap; }
  .badge.ok { color: #7ff0b4; border-color: rgb(61 220 151 / 0.5); background: rgb(61 220 151 / 0.1); }
  .badge.info { color: #a8d8ff; border-color: rgb(143 208 255 / 0.45); }
  .badge.warn { color: #ffd08a; border-color: rgb(255 180 84 / 0.5); background: rgb(255 180 84 / 0.08); }
  .badge.bad { color: #ffb3bd; border-color: rgb(255 110 130 / 0.5); background: rgb(255 90 110 / 0.08); }
  .badge.idle { color: var(--muted); }
  .mapstate { display: grid; gap: 6px; padding: 4px 0 6px; font-size: 0.82rem; }
  .mapstate small { display: block; margin: 0; color: var(--muted); line-height: 1.45; }
  .mapstate small.okc, small.okc { color: #7ff0b4; }
  .mapstate small.warnc { color: #ffd08a; }
  .macts { display: flex; gap: 8px; flex-wrap: wrap; }
  .btn.primary { background: var(--accent); color: #0a0a12; border-color: transparent; font-weight: 600; }
  .conflict, .confirm { display: grid; gap: 8px; padding: 12px 14px; border-radius: 12px; background: rgb(255 180 84 / 0.08); border: 1px solid rgb(255 180 84 / 0.4); }
  .conflict p, .confirm p { margin: 0; color: var(--muted); line-height: 1.5; }
  .ents { margin: 0; padding-left: 18px; max-height: 110px; overflow: auto; color: var(--muted); font-size: 0.78rem; }
  .offers { display: grid; gap: 6px; }
  .offer { display: flex; gap: 10px; align-items: flex-start; padding: 8px 10px; border-radius: 10px; border: 1px solid var(--line); background: rgb(255 255 255 / 0.04); cursor: pointer; }
  .offer.sel { border-color: var(--accent); background: rgb(var(--accent-rgb) / 0.14); }
  .offer small { display: block; margin-top: 2px; }
</style>
