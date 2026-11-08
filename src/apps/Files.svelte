<script lang="ts">
  import { onMount, tick, untrack } from 'svelte';
  import Icon from '../components/Icon.svelte';
  import type { IconName } from '../lib/icons';
  import { files, formatJob } from '../lib/files.svelte';
  import {
    childOf, formatDate, formatSize, isDirEntry, locKey, nameProblem, parentOf, parseTypedPath, suggestName,
    type Entry, type Location, type Place,
  } from '../lib/fs-types';
  import { runtime } from '../lib/runtime.svelte';
  import { openApp } from '../lib/open-app';
  import { wm } from '../lib/wm.svelte';

  const ROW = 30;
  let distroMenu = $state(false);

  // ---- lifecycle ---------------------------------------------------------------------------------
  onMount(() => {
    wm.guards.files = () =>
      files.busy
        ? 'A file operation is still running. It carries on in the background if you close this window, but you will not see its progress or be able to cancel it.'
        : null;
    void files.init();
    return () => {
      delete wm.guards.files;
      files.dispose();
    };
  });

  const installed = $derived((runtime.report?.distros ?? []).filter((d) => d.kind === 'regular'));
  const places = $derived(files.places);
  const homePath = $derived(places?.home?.location.space === 'linux' ? places.home.location.path : null);
  const head = $derived(files.head);
  const total = $derived(head?.total ?? 0);
  const where = $derived(head?.resolved.space ?? files.location?.space ?? null);

  // ---- virtual list: only the rows in view exist in the DOM; pages load as they are reached ---------
  let scroller: HTMLElement | undefined = $state();
  let scrollTop = $state(0);
  let viewH = $state(360);
  const first = $derived(Math.max(0, Math.floor(scrollTop / ROW) - 4));
  const last = $derived(Math.min(total, Math.ceil((scrollTop + viewH) / ROW) + 4));
  const visible = $derived(files.entries.slice(first, Math.min(last, files.entries.length)));
  const locationKey = $derived(files.location ? locKey(files.location) : '');

  $effect(() => {
    const l = last;
    void head?.id;
    untrack(() => void files.ensureLoaded(l + 100));
  });
  $effect(() => {
    void locationKey;
    untrack(() => {
      scrollTop = 0;
      if (scroller) scroller.scrollTop = 0;
    });
  });

  function ensureVisible(name: string) {
    const i = files.entries.findIndex((e) => e.name === name);
    if (i < 0 || !scroller) return;
    const top = i * ROW;
    if (top < scroller.scrollTop) scroller.scrollTop = top;
    else if (top + ROW > scroller.scrollTop + viewH) scroller.scrollTop = top + ROW - viewH;
  }

  // ---- path bar ----------------------------------------------------------------------------------
  let editing = $state(false);
  let typed = $state('');
  let typedError = $state<string | null>(null);
  let pathInput: HTMLInputElement | undefined = $state();
  let blurTimer: ReturnType<typeof setTimeout> | undefined;

  async function startEdit() {
    clearTimeout(blurTimer);
    typed = files.linuxPath ?? '';
    typedError = null;
    editing = true;
    await tick();
    pathInput?.focus();
    pathInput?.select();
  }
  async function commitTyped() {
    const r = parseTypedPath(typed, homePath);
    if ('error' in r) {
      typedError = r.error;
      return;
    }
    if (!files.distro) {
      typedError = 'Choose a Linux distribution first to browse its files.';
      return;
    }
    editing = false;
    await files.navigate(r);
  }

  // ---- selection and keyboard ------------------------------------------------------------------------
  function rowClick(e: MouseEvent, entry: Entry) {
    menu = null;
    files.select(entry.name, e.ctrlKey || e.metaKey ? 'toggle' : e.shiftKey ? 'range' : 'only');
  }

  function move(delta: number, extend: boolean) {
    const list = files.entries;
    if (!list.length) return;
    const cur = files.anchor ? list.findIndex((e) => e.name === files.anchor) : -1;
    const idx = Math.min(list.length - 1, Math.max(0, cur < 0 ? (delta > 0 ? 0 : list.length - 1) : cur + delta));
    const name = list[idx]!.name;
    if (extend) {
      const anchorName = files.anchor ?? name;
      const a = list.findIndex((e) => e.name === anchorName);
      files.selected = list.slice(Math.min(a, idx), Math.max(a, idx) + 1).map((x) => x.name);
      // keep the anchor where the range began
    } else files.select(name);
    ensureVisible(name);
  }

  function onKey(e: KeyboardEvent) {
    if (dialog || editing) return;
    const mod = e.ctrlKey || e.metaKey;
    const one = files.selected.length === 1 ? files.entries.find((x) => x.name === files.selected[0]) : undefined;
    if (e.altKey && e.key === 'ArrowLeft') void files.back();
    else if (e.altKey && e.key === 'ArrowRight') void files.forward();
    else if (e.altKey && e.key === 'ArrowUp') void files.up();
    else if (e.key === 'ArrowDown') move(1, e.shiftKey);
    else if (e.key === 'ArrowUp') move(-1, e.shiftKey);
    else if (e.key === 'Home') move(-1e9, e.shiftKey);
    else if (e.key === 'End') move(1e9, e.shiftKey);
    else if (e.key === 'Enter' && one) void files.open(one);
    else if (e.key === 'Backspace') void files.up();
    else if (e.key === 'F5') void files.refresh();
    else if (e.key === 'F2' && one) askRename(one);
    else if (e.key === 'Delete' && files.selected.length) askDelete();
    else if (mod && e.key.toLowerCase() === 'a') files.selectAll();
    else if (mod && e.key.toLowerCase() === 'c') files.copy(false);
    else if (mod && e.key.toLowerCase() === 'x') files.copy(true);
    else if (mod && e.key.toLowerCase() === 'v') void files.paste();
    else if (mod && e.key.toLowerCase() === 'l') void startEdit();
    else if (mod && e.shiftKey && e.key.toLowerCase() === 'n') askNew('folder');
    else if (e.key === 'Escape') {
      if (menu) menu = null;
      else files.clearSelection();
    } else return;
    e.preventDefault();
    e.stopPropagation();
  }

  // ---- context menu ------------------------------------------------------------------------------
  type MenuKind = { kind: 'entry'; entry: Entry } | { kind: 'blank' } | { kind: 'place'; place: Place };
  let root: HTMLElement | undefined = $state();
  let menu = $state<({ x: number; y: number } & MenuKind) | null>(null);

  function openMenu(e: MouseEvent, what: MenuKind) {
    e.preventDefault();
    e.stopPropagation();
    const r = root?.getBoundingClientRect();
    if (what.kind === 'entry' && !files.selectedSet.has(what.entry.name)) files.select(what.entry.name);
    const x = Math.min(e.clientX - (r?.left ?? 0), (r?.width ?? 800) - 210);
    const y = Math.min(e.clientY - (r?.top ?? 0), (r?.height ?? 500) - (what.kind === 'entry' ? 250 : 200));
    menu = { x: Math.max(4, x), y: Math.max(4, y), ...what };
  }
  function act(fn: () => void) {
    menu = null;
    fn();
  }

  // ---- dialogs -------------------------------------------------------------------------------------
  type Dialog =
    | { kind: 'prompt'; mode: 'folder' | 'file' | 'rename'; value: string; target: string; error: string | null; busy: boolean }
    | { kind: 'delete'; names: string[]; error: string | null; busy: boolean };
  let dialog = $state<Dialog | null>(null);
  let promptInput: HTMLInputElement | undefined = $state();

  async function askNew(mode: 'folder' | 'file') {
    if (!files.location || files.error) return;
    const base = mode === 'folder' ? 'New folder' : 'New file.txt';
    dialog = { kind: 'prompt', mode, value: suggestName(base, files.entries.map((e) => e.name)), target: '', error: null, busy: false };
    await tick();
    promptInput?.focus();
    promptInput?.select();
  }
  async function askRename(entry: Entry) {
    dialog = { kind: 'prompt', mode: 'rename', value: entry.name, target: entry.name, error: null, busy: false };
    await tick();
    promptInput?.focus();
    const dot = entry.name.lastIndexOf('.');
    promptInput?.setSelectionRange(0, dot > 0 && entry.kind === 'file' ? dot : entry.name.length);
  }
  function askDelete() {
    if (!files.selected.length) return;
    dialog = { kind: 'delete', names: [...files.selected], error: null, busy: false };
  }

  async function submitPrompt() {
    if (!dialog || dialog.kind !== 'prompt' || dialog.busy) return;
    const d = dialog;
    const name = d.value.trim();
    const bad = nameProblem(name);
    if (bad) {
      d.error = bad;
      return;
    }
    d.busy = true;
    const problem = d.mode === 'rename' ? (name === d.target ? null : await files.rename(d.target, name)) : await files.create(d.mode, name);
    if (problem) {
      d.error = problem;
      d.busy = false;
    } else dialog = null;
  }

  async function submitDelete() {
    if (!dialog || dialog.kind !== 'delete' || dialog.busy) return;
    const d = dialog;
    d.busy = true;
    const problem = await files.deleteSelected();
    if (problem) {
      d.error = problem;
      d.busy = false;
    } else dialog = null;
  }

  function dialogKey(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.stopPropagation();
      dialog = null;
    }
  }

  // ---- drag and drop -------------------------------------------------------------------------------
  let dropKey = $state<string | null>(null);

  function dragStart(e: DragEvent, entry: Entry) {
    if (!files.selectedSet.has(entry.name)) files.select(entry.name);
    const names = [...files.selected];
    files.drag = { sources: files.selectedLocations(), names };
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = 'copyMove';
      e.dataTransfer.setData('text/plain', names.join('\n'));
    }
  }
  function dragEnd() {
    files.drag = null;
    dropKey = null;
  }
  /** May the current drag be dropped on a folder `dest`? (Not on one of the dragged items.) */
  function canDrop(dest: Location | null, destName?: string): boolean {
    if (!files.drag || !dest) return false;
    if (destName !== undefined && files.drag.names.includes(destName)) return false;
    return true;
  }
  function dragOver(e: DragEvent, key: string, dest: Location | null, destName?: string) {
    if (!canDrop(dest, destName)) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = e.ctrlKey ? 'copy' : 'move';
    dropKey = key;
  }
  async function drop(e: DragEvent, dest: Location | null, destName?: string) {
    e.preventDefault();
    const d = files.drag;
    const ok = canDrop(dest, destName);
    dragEnd();
    if (!d || !ok || !dest) return;
    await files.transfer(d.sources, dest, e.ctrlKey ? 'copy' : 'move');
  }

  // ---- labels --------------------------------------------------------------------------------------
  function placeIcon(p: Place, fallback: IconName): IconName {
    if (p.id === 'home') return 'home';
    if (p.id === 'root') return 'drive';
    return fallback;
  }
  function rowIcon(e: Entry): IconName {
    if (e.kind === 'symlink') return 'link';
    return isDirEntry(e) ? 'folder' : 'file';
  }
  function isActivePlace(p: Place): boolean {
    if (!files.location) return false;
    if (locKey(files.location) === locKey(p.location)) return true;
    // A Linux folder that is really a shared Windows folder, browsed through the Windows view.
    return p.location.space === 'windows' && files.location.space === 'windows' && files.location.root === p.location.root && files.location.rel.length === 0;
  }
  const selectedBytes = $derived(files.entries.filter((e) => files.selectedSet.has(e.name)).reduce((n, e) => n + (e.size ?? 0), 0));
  const cutNames = $derived(files.clipboard?.mode === 'move' && files.location && files.clipboard.sources.every((s) => parentOf(s) && locKey(parentOf(s)!) === locKey(files.location!)) ? new Set(files.clipboard.names) : new Set<string>());
  const deleteWhere = $derived(
    where === 'windows' ? 'in your shared Windows folder' : `in the Linux filesystem of ${files.distro ?? 'the distribution'}`,
  );
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="files" bind:this={root} onpointerdown={() => (menu = null)} onkeydown={(e) => dialog && dialogKey(e)}>
  <div class="toolbar">
    <button class="tool" aria-label="Back" title="Back (Alt+Left)" onclick={() => files.back()} disabled={!files.canBack}><Icon name="arrow-left" size={16} /></button>
    <button class="tool" aria-label="Forward" title="Forward (Alt+Right)" onclick={() => files.forward()} disabled={!files.canForward}><Icon name="arrow-right" size={16} /></button>
    <button class="tool" aria-label="Parent folder" title="Parent folder (Alt+Up)" onclick={() => files.up()} disabled={!files.canUp}><Icon name="arrow-up" size={16} /></button>
    <button class="tool" aria-label="Refresh" title="Refresh (F5)" onclick={() => files.refresh()} disabled={!files.location}><Icon name="rotate" size={15} /></button>

    {#if editing}
      <form class="pathbar editing" onsubmit={(e) => { e.preventDefault(); void commitTyped(); }}>
        <input bind:this={pathInput} bind:value={typed} spellcheck="false" aria-label="Location" placeholder="Type a Linux path, e.g. ~/Documents or /etc"
               oninput={() => (typedError = null)} onkeydown={(e) => { if (e.key === 'Escape') { e.stopPropagation(); clearTimeout(blurTimer); editing = false; } }} onblur={() => { clearTimeout(blurTimer); blurTimer = setTimeout(() => (editing = false), 150); }} />
        {#if typedError}<small class="bad" role="alert">{typedError}</small>{/if}
      </form>
    {:else}
      <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
      <div class="pathbar" role="navigation" aria-label="Location" onclick={(e) => { if (e.target === e.currentTarget) void startEdit(); }} ondblclick={() => startEdit()}>
        {#if where}<span class="space {where}" title={where === 'windows' ? 'A shared Windows folder' : `The Linux filesystem of ${files.distro ?? ''}`}>{where === 'windows' ? 'Windows' : 'Linux'}</span>{/if}
        {#each files.crumbs as c, i (i + c.label)}
          {#if i > 0}<span class="slash" aria-hidden="true">›</span>{/if}
          <button class="crumb" class:drop={dropKey === 'crumb' + i} data-crumb={c.label} onclick={() => files.navigate(c.location)}
                  ondragover={(e) => dragOver(e, 'crumb' + i, c.location)} ondragleave={() => (dropKey = null)} ondrop={(e) => drop(e, c.location)}>{c.label}</button>
        {/each}
      </div>
    {/if}

    <button class="tool" class:on={files.showHidden} aria-pressed={files.showHidden} aria-label="Show hidden files" title="Show hidden files" data-act="hidden"
            onclick={() => files.setHidden(!files.showHidden)}><Icon name={files.showHidden ? 'eye' : 'eye-off'} size={16} /></button>
    <button class="tool wide" aria-label="New folder" title="New folder (Ctrl+Shift+N)" data-act="new-folder" onclick={() => askNew('folder')} disabled={!files.location || !!files.error}><Icon name="plus" size={15} /> <span>New folder</span></button>
  </div>

  <div class="body">
    <aside class="side" aria-label="Places">
      <div class="side-scroll">
        <div class="head">Places</div>
        {#if places?.home}
          {@render item(places.home, 'home')}
        {/if}
        {#if places?.linux_available}
          {#each places?.folders ?? [] as p (p.id)}
            {@render item(p, 'folder')}
          {/each}
        {/if}

        <div class="head">
          <span>Linux</span>
          <span class="menu-wrap">
            <button class="mini" aria-haspopup="menu" aria-expanded={distroMenu} title="Choose the distribution to browse" data-act="distro-menu"
                    onclick={() => (distroMenu = !distroMenu)} disabled={installed.length === 0}>{files.distro ?? 'none'} ▾</button>
            {#if distroMenu}
              <!-- svelte-ignore a11y_no_static_element_interactions -->
              <div class="catch" onpointerdown={() => (distroMenu = false)}></div>
              <div class="dmenu glass" role="menu" aria-label="Browse distribution">
                {#each installed as d (d.name)}
                  <button role="menuitem" class="mi" data-distro={d.name} onclick={() => { distroMenu = false; void files.setDistro(d.name); }}>
                    <strong>{d.name}</strong><span>{d.display_label}{d.name === runtime.selected ? ' · selected for VLINUX' : ''}{d.state === 'stopped' ? ' · stopped (will start)' : ''}</span>
                  </button>
                {/each}
                <div class="mnote">Browsing a distribution starts it if it is stopped. This does not change your selected or default distribution.</div>
              </div>
            {/if}
          </span>
        </div>
        {#each places?.roots ?? [] as p (p.id)}
          {@render item(p, 'drive')}
        {/each}
        {#if places && !places.linux_available}
          <div class="note" data-note="linux">
            <p>{places.linux_note ?? 'The Linux filesystem is not available.'}</p>
            <button class="btn small" onclick={() => openApp('setup')}>Open Linux Setup</button>
          </div>
        {/if}

        <div class="head">Windows folders</div>
        {#each places?.shared ?? [] as p (p.id)}
          {@render item(p, 'windows')}
        {/each}
        {#if files.placesError}<div class="note"><p class="bad">{files.placesError}</p></div>{/if}
      </div>
    </aside>

    <section class="main" aria-label="Folder contents">
      {#if head && where === 'windows' && files.location?.space === 'linux'}
        <div class="chip info" data-chip="shared"><Icon name="link" size={13} /> This Linux folder is your Windows folder — the same files, no copies.</div>
      {/if}
      {#if head?.truncated}
        <div class="chip warn">Only the first entries of this very large folder are listed.</div>
      {/if}

      <div class="cols" role="row">
        <button class="col name" onclick={() => files.setSort('name')} aria-label="Sort by name">Name {files.sort === 'name' ? (files.descending ? '↓' : '↑') : ''}</button>
        <button class="col size" onclick={() => files.setSort('size')} aria-label="Sort by size">Size {files.sort === 'size' ? (files.descending ? '↓' : '↑') : ''}</button>
        <button class="col mod" onclick={() => files.setSort('modified')} aria-label="Sort by date modified">Modified {files.sort === 'modified' ? (files.descending ? '↓' : '↑') : ''}</button>
        <span class="col type">Type</span>
      </div>

      <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
      <div class="rows" role="grid" tabindex="0" aria-label="Files" aria-rowcount={total} aria-multiselectable="true" bind:this={scroller}
           bind:clientHeight={viewH} onscroll={(e) => (scrollTop = e.currentTarget.scrollTop)} onkeydown={onKey}
           oncontextmenu={(e) => { if (e.target === e.currentTarget || (e.target as HTMLElement).classList.contains('canvas')) openMenu(e, { kind: 'blank' }); }}
           onpointerdown={(e) => { if (e.target === e.currentTarget || (e.target as HTMLElement).classList.contains('canvas')) files.clearSelection(); }}>
        {#if files.error}
          <div class="state err" role="alert" data-state="error" data-kind={files.error.kind}>
            <h3>{files.error.kind === 'permission_denied' ? 'Permission denied' : files.error.kind === 'not_found' ? 'This folder is not there' : files.error.kind === 'unavailable' ? 'Not available' : 'Can’t open this folder'}</h3>
            <p>{files.error.message}</p>
            <div class="acts">
              <button class="btn" onclick={() => files.back()} disabled={!files.canBack}>Go back</button>
              <button class="btn" onclick={() => files.up()} disabled={!files.canUp}>Go to parent folder</button>
              <button class="btn" onclick={() => files.refresh()}>Try again</button>
              {#if files.error.kind === 'unavailable'}<button class="btn primary" onclick={() => openApp('setup')}>Open Linux Setup</button>{/if}
            </div>
          </div>
        {:else if files.loading && !head}
          <div class="state" aria-busy="true"><p>Opening…</p></div>
        {:else if !files.location && files.booted && !files.loading}
          <div class="state"><h3>Nothing to show yet</h3><p>No place is open. Pick a folder in the sidebar.</p></div>
        {:else if head && total === 0}
          <div class="state" data-state="empty"><h3>This folder is empty</h3><p>{files.showHidden ? 'There are no files here.' : 'There are no visible files here. Hidden files are not shown.'}</p></div>
        {:else}
          <div class="canvas" style="height:{total * ROW}px">
            {#each visible as e, i (e.name)}
              {@const idx = first + i}
              {@const isDir = isDirEntry(e)}
              <!-- svelte-ignore a11y_click_events_have_key_events -->
              <div class="row" class:sel={files.selectedSet.has(e.name)} class:dim={e.hidden || cutNames.has(e.name)} class:drop={dropKey === 'row:' + e.name}
                   role="row" aria-selected={files.selectedSet.has(e.name)} tabindex="-1" style="top:{idx * ROW}px" data-name={e.name} data-kind={e.kind} draggable="true"
                   onclick={(ev) => rowClick(ev, e)} ondblclick={() => files.open(e)} oncontextmenu={(ev) => openMenu(ev, { kind: 'entry', entry: e })}
                   ondragstart={(ev) => dragStart(ev, e)} ondragend={dragEnd}
                   ondragover={(ev) => isDir && dragOver(ev, 'row:' + e.name, childOf(files.location!, e.name), e.name)} ondragleave={() => (dropKey = null)}
                   ondrop={(ev) => isDir && drop(ev, childOf(files.location!, e.name), e.name)}>
                <span class="cell name" title={e.link_target ? `${e.name} → ${e.link_target}` : e.name}>
                  <span class="ico" class:folder={isDir} class:link={e.kind === 'symlink'}><Icon name={rowIcon(e)} size={16} /></span>
                  <span class="nm">{e.name}</span>
                  {#if e.kind === 'symlink'}<span class="lt">→ {e.link_target ?? '?'}{e.link_is_dir === null ? ' (broken or unknown)' : ''}</span>{/if}
                  {#if e.readonly}<span class="tag">read-only</span>{/if}
                </span>
                <span class="cell size">{isDir ? '' : formatSize(e.size)}</span>
                <span class="cell mod">{formatDate(e.modified)}</span>
                <span class="cell type">{e.kind === 'symlink' ? (e.link_is_dir ? 'Link to folder' : 'Link') : e.class.label}</span>
              </div>
            {/each}
            {#if files.entries.length < total && last > files.entries.length}
              <div class="loadmore" style="top:{files.entries.length * ROW}px" aria-busy="true">Loading more…</div>
            {/if}
          </div>
        {/if}
      </div>

      {#if files.finished}
        <div class="finished" role="status" data-finished>
          <div class="fh"><strong>{files.finished.title}</strong><button class="mini" onclick={() => files.dismissFinished()} aria-label="Dismiss"><Icon name="close" size={12} /></button></div>
          {#if files.finished.error}<p class="bad">{files.finished.error.message}</p>{/if}
          <ul>
            {#each files.finished.report?.failed.slice(0, 5) ?? [] as f}<li class="bad">{f.message}</li>{/each}
            {#each files.finished.report?.skipped.slice(0, 5) ?? [] as s}<li>{s}</li>{/each}
          </ul>
          {#if (files.finished.report?.failed.length ?? 0) > 5}<small>… and {(files.finished.report?.failed.length ?? 0) - 5} more problems.</small>{/if}
        </div>
      {/if}
    </section>
  </div>

  {#if files.jobs.length}
    <div class="jobs" role="status" aria-live="polite">
      {#each files.jobs as j (j.id)}
        <div class="job" data-job={j.kind}>
          <span class="spin" aria-hidden="true"></span>
          <span class="jt">{formatJob(j)}{j.bytes ? ` · ${formatSize(j.bytes)}` : ''}{j.current ? ` · ${j.current}` : ''}</span>
          <button class="mini" onclick={() => files.cancelJob(j.id)}>Cancel</button>
        </div>
      {/each}
    </div>
  {/if}

  <div class="status" role="status">
    <span data-status="count">{total.toLocaleString()} item{total === 1 ? '' : 's'}{head && files.entries.length < total ? ` (${files.entries.length.toLocaleString()} loaded)` : ''}</span>
    {#if files.selected.length}<span class="sep">·</span><span data-status="sel">{files.selected.length} selected{selectedBytes ? ` · ${formatSize(selectedBytes)}` : ''}</span>{/if}
    {#if files.clipboard}<span class="sep">·</span><span>{files.clipboard.mode === 'move' ? 'Cut' : 'Copied'}: {files.clipboard.sources.length}</span>{/if}
    <span class="grow"></span>
    <span class="muted">{where === 'windows' ? 'Windows folder' : where === 'linux' ? `Linux · ${files.distro}` : ''}</span>
  </div>

  {#if menu}
    <div class="ctx glass" role="menu" tabindex="-1" aria-label="Actions" style="left:{menu.x}px; top:{menu.y}px" onpointerdown={(e) => e.stopPropagation()}>
      {#if menu.kind === 'entry'}
        {@const en = menu.entry}
        {@const many = files.selected.length > 1}
        <button role="menuitem" class="mi" data-cmd="open" disabled={many || (!isDirEntry(en) && !en.open.allowed)} title={!isDirEntry(en) && !en.open.allowed ? en.open.reason ?? '' : ''} onclick={() => act(() => files.open(en))}><Icon name="folder" size={14} /> Open</button>
        <button role="menuitem" class="mi" data-cmd="reveal" disabled={many} onclick={() => act(() => files.reveal(en.name))}><Icon name="external" size={14} /> Show in Windows Explorer</button>
        {#if isDirEntry(en) && files.clipboard && !many}
          <button role="menuitem" class="mi" data-cmd="paste-into" onclick={() => act(() => files.paste(childOf(files.location!, en.name)))}><Icon name="clipboard" size={14} /> Paste into folder</button>
        {/if}
        <div class="sep"></div>
        <button role="menuitem" class="mi" data-cmd="rename" disabled={many} onclick={() => act(() => askRename(en))}><Icon name="edit" size={14} /> Rename <kbd>F2</kbd></button>
        <button role="menuitem" class="mi" data-cmd="copy" onclick={() => act(() => files.copy(false))}><Icon name="copy" size={14} /> Copy <kbd>Ctrl+C</kbd></button>
        <button role="menuitem" class="mi" data-cmd="cut" onclick={() => act(() => files.copy(true))}><Icon name="scissors" size={14} /> Cut <kbd>Ctrl+X</kbd></button>
        <div class="sep"></div>
        <button role="menuitem" class="mi danger" data-cmd="delete" onclick={() => act(askDelete)}><Icon name="trash" size={14} /> Delete… <kbd>Del</kbd></button>
      {:else if menu.kind === 'blank'}
        <button role="menuitem" class="mi" data-cmd="new-folder" onclick={() => act(() => askNew('folder'))}><Icon name="plus" size={14} /> New folder</button>
        <button role="menuitem" class="mi" data-cmd="new-file" onclick={() => act(() => askNew('file'))}><Icon name="file" size={14} /> New file</button>
        <button role="menuitem" class="mi" data-cmd="paste" disabled={!files.clipboard} onclick={() => act(() => files.paste())}><Icon name="clipboard" size={14} /> Paste <kbd>Ctrl+V</kbd></button>
        <div class="sep"></div>
        <button role="menuitem" class="mi" data-cmd="refresh" onclick={() => act(() => files.refresh())}><Icon name="rotate" size={14} /> Refresh <kbd>F5</kbd></button>
        <button role="menuitem" class="mi" data-cmd="reveal-here" onclick={() => act(() => files.reveal(null))}><Icon name="external" size={14} /> Show folder in Windows Explorer</button>
      {:else}
        {@const pl = menu.place}
        <button role="menuitem" class="mi" data-cmd="open-place" onclick={() => act(() => files.navigate(pl.location))}><Icon name="folder" size={14} /> Open</button>
        {#if files.clipboard}<button role="menuitem" class="mi" data-cmd="paste-place" onclick={() => act(() => files.paste(pl.location))}><Icon name="clipboard" size={14} /> Paste here</button>{/if}
      {/if}
    </div>
  {/if}

  {#if dialog}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="backdrop" onpointerdown={(e) => e.target === e.currentTarget && (dialog = null)}>
      {#if dialog.kind === 'prompt'}
        <!-- svelte-ignore a11y_no_noninteractive_element_to_interactive_role -->
        <form class="panel glass" role="dialog" aria-modal="true" aria-label={dialog.mode === 'rename' ? 'Rename' : dialog.mode === 'folder' ? 'New folder' : 'New file'} data-dialog="prompt"
              onsubmit={(e) => { e.preventDefault(); void submitPrompt(); }}>
          <h2>{dialog.mode === 'rename' ? `Rename “${dialog.target}”` : dialog.mode === 'folder' ? 'New folder' : 'New file'}</h2>
          <input bind:this={promptInput} bind:value={dialog.value} spellcheck="false" aria-label="Name" oninput={() => { if (dialog && dialog.kind === 'prompt') dialog.error = null; }} />
          {#if dialog.error}<p class="bad" role="alert" data-dialog-error>{dialog.error}</p>{/if}
          <div class="actions">
            <button type="button" class="btn" onclick={() => (dialog = null)}>Cancel</button>
            <button type="submit" class="btn primary" data-act="confirm-prompt" disabled={dialog.busy}>{dialog.mode === 'rename' ? 'Rename' : 'Create'}</button>
          </div>
        </form>
      {:else}
        <div class="panel glass" role="alertdialog" aria-modal="true" aria-label="Confirm delete" data-dialog="delete">
          <h2>Delete {dialog.names.length === 1 ? `“${dialog.names[0]}”` : `${dialog.names.length} items`}?</h2>
          <p>This permanently deletes {dialog.names.length === 1 ? 'it' : 'them'} {deleteWhere}. There is no recycle bin, so it can’t be undone. Symbolic links are removed, never followed.</p>
          {#if dialog.names.length > 1}
            <ul>{#each dialog.names.slice(0, 6) as n}<li>{n}</li>{/each}{#if dialog.names.length > 6}<li>… and {dialog.names.length - 6} more</li>{/if}</ul>
          {/if}
          {#if dialog.error}<p class="bad" role="alert">{dialog.error}</p>{/if}
          <div class="actions">
            <!-- svelte-ignore a11y_autofocus -->
            <button class="btn" autofocus onclick={() => (dialog = null)}>Keep {dialog.names.length === 1 ? 'it' : 'them'}</button>
            <button class="btn danger" data-act="confirm-delete" disabled={dialog.busy} onclick={submitDelete}>Delete permanently</button>
          </div>
        </div>
      {/if}
    </div>
  {/if}
</div>

{#snippet item(p: Place, fallback: IconName)}
  <button class="place" class:active={isActivePlace(p)} class:drop={dropKey === 'place:' + p.id + p.location.space} data-place={p.id} data-space={p.location.space} data-linked={p.linked}
          disabled={!p.exists} title={p.exists ? [p.detail, p.warning].filter(Boolean).join(' — ') : `${p.detail ?? p.label} does not exist or can't be reached right now.`}
          onclick={() => files.navigate(p.location)} oncontextmenu={(e) => openMenu(e, { kind: 'place', place: p })}
          ondragover={(e) => dragOver(e, 'place:' + p.id + p.location.space, p.location)} ondragleave={() => (dropKey = null)} ondrop={(e) => drop(e, p.location)}>
    <Icon name={placeIcon(p, fallback === 'windows' ? 'folder' : fallback)} size={15} />
    <span class="pl">{p.label}</span>
    {#if p.linked}<span class="badge" title={p.detail ?? 'Shared with Windows'}><Icon name="link" size={11} /></span>{/if}
    {#if p.unc}<span class="badge net" title="Network location">net</span>{/if}
  </button>
{/snippet}

<style>
  .files { position: relative; display: flex; flex-direction: column; height: 100%; min-height: 0; font-size: 0.84rem; user-select: none; }
  button { color: inherit; }
  .toolbar { display: flex; align-items: center; gap: 6px; padding: 8px 10px; border-bottom: 1px solid var(--line); flex: none; }
  .tool { display: inline-flex; align-items: center; justify-content: center; gap: 6px; height: 30px; min-width: 30px; padding: 0 8px; border-radius: 9px; background: transparent; border: 1px solid transparent; }
  .tool:hover:not(:disabled) { background: rgb(255 255 255 / 0.1); }
  .tool:disabled { opacity: 0.35; cursor: default; }
  .tool.on { background: rgb(var(--accent-rgb) / 0.2); border-color: rgb(var(--accent-rgb) / 0.45); }
  .tool.wide { background: rgb(255 255 255 / 0.07); border-color: var(--line); }
  .pathbar { flex: 1; min-width: 0; display: flex; align-items: center; gap: 2px; height: 30px; padding: 0 8px; border-radius: 9px; background: rgb(0 0 0 / 0.28); border: 1px solid var(--line); overflow: hidden; white-space: nowrap; }
  .pathbar.editing { padding: 0; position: relative; }
  .pathbar input { flex: 1; height: 100%; padding: 0 10px; background: transparent; border: 0; outline: none; color: var(--text); font: inherit; font-family: var(--font-mono); font-size: 0.8rem; user-select: text; }
  .pathbar small { position: absolute; top: 34px; left: 0; z-index: 4; padding: 4px 8px; border-radius: 8px; background: rgb(40 16 22 / 0.97); }
  .space { padding: 1px 8px; margin-right: 4px; border-radius: 999px; font-size: 0.68rem; letter-spacing: 0.06em; text-transform: uppercase; border: 1px solid var(--line-strong); color: var(--muted); }
  .space.windows { color: #8fd0ff; border-color: rgb(143 208 255 / 0.4); }
  .space.linux { color: #ffd37a; border-color: rgb(255 211 122 / 0.4); }
  .slash { color: var(--faint); padding: 0 1px; }
  .crumb { padding: 2px 6px; border-radius: 6px; background: transparent; border: 0; max-width: 200px; overflow: hidden; text-overflow: ellipsis; }
  .crumb:hover, .crumb.drop { background: rgb(255 255 255 / 0.12); }
  .crumb.drop { outline: 1px solid var(--accent); }

  .body { flex: 1; min-height: 0; display: flex; }
  .side { width: 212px; flex: none; border-right: 1px solid var(--line); background: rgb(0 0 0 / 0.14); min-height: 0; }
  .side-scroll { height: 100%; overflow-y: auto; padding: 6px 6px 12px; scrollbar-width: thin; }
  .head { display: flex; align-items: center; justify-content: space-between; gap: 6px; padding: 12px 8px 4px; font-size: 0.66rem; text-transform: uppercase; letter-spacing: 0.13em; color: var(--faint); }
  .place { width: 100%; display: flex; align-items: center; gap: 9px; padding: 6px 8px; border-radius: 9px; text-align: left; background: transparent; border: 1px solid transparent; }
  .place:hover:not(:disabled) { background: rgb(255 255 255 / 0.08); }
  .place.active { background: rgb(var(--accent-rgb) / 0.2); border-color: rgb(var(--accent-rgb) / 0.4); }
  .place.drop { outline: 1px solid var(--accent); background: rgb(var(--accent-rgb) / 0.25); }
  .place:disabled { opacity: 0.4; cursor: not-allowed; }
  .pl { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .badge { display: inline-flex; align-items: center; color: #8fd0ff; }
  .badge.net { font-size: 0.62rem; text-transform: uppercase; letter-spacing: 0.08em; padding: 0 5px; border-radius: 999px; border: 1px solid rgb(255 180 84 / 0.5); color: #ffc87a; }
  .note { margin: 4px 4px 0; padding: 8px 10px; border-radius: 10px; background: rgb(255 180 84 / 0.08); border: 1px solid rgb(255 180 84 / 0.3); }
  .note p { margin: 0 0 8px; color: var(--muted); font-size: 0.76rem; line-height: 1.45; user-select: text; }
  .menu-wrap { position: relative; text-transform: none; letter-spacing: 0; }
  .mini { padding: 2px 8px; border-radius: 7px; background: rgb(255 255 255 / 0.08); border: 1px solid var(--line); font-size: 0.72rem; max-width: 120px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .mini:hover:not(:disabled) { background: rgb(255 255 255 / 0.15); }
  .mini:disabled { opacity: 0.4; }
  .catch { position: fixed; inset: 0; z-index: 5; }
  .dmenu { position: absolute; z-index: 6; top: 26px; left: -4px; width: 260px; padding: 6px; border-radius: 12px; background: rgb(var(--panel-rgb) / 0.98); box-shadow: 0 18px 50px rgb(0 0 0 / 0.5); display: grid; gap: 2px; }
  .dmenu .mi { display: grid; gap: 1px; padding: 7px 9px; }
  .dmenu .mi span { color: var(--muted); font-size: 0.72rem; }
  .mnote { padding: 6px 8px 2px; color: var(--faint); font-size: 0.7rem; line-height: 1.4; }

  .main { flex: 1; min-width: 0; display: flex; flex-direction: column; position: relative; container-type: inline-size; }
  .chip { margin: 8px 12px 0; padding: 6px 10px; border-radius: 9px; font-size: 0.76rem; display: flex; align-items: center; gap: 7px; }
  .chip.info { background: rgb(143 208 255 / 0.1); border: 1px solid rgb(143 208 255 / 0.3); color: #bfe3ff; }
  .chip.warn { background: rgb(255 180 84 / 0.1); border: 1px solid rgb(255 180 84 / 0.35); color: #ffd7a0; }
  .cols { display: grid; grid-template-columns: minmax(0, 1fr) 84px 148px 124px; gap: 0; padding: 0 14px; margin-top: 8px; border-bottom: 1px solid var(--line); flex: none; color: var(--faint); font-size: 0.72rem; text-transform: uppercase; letter-spacing: 0.08em; }
  .col { text-align: left; padding: 6px 4px; background: transparent; border: 0; font: inherit; letter-spacing: inherit; text-transform: inherit; color: inherit; }
  button.col:hover { color: var(--text); }
  .col.size { text-align: right; padding-right: 14px; }
  .rows { flex: 1; min-height: 0; overflow-y: auto; position: relative; outline: none; scrollbar-width: thin; scrollbar-color: rgb(255 255 255 / 0.2) transparent; }
  .rows:focus-visible { box-shadow: inset 0 0 0 1px rgb(var(--accent-rgb) / 0.5); }
  .canvas { position: relative; width: 100%; }
  .row { position: absolute; left: 0; right: 0; height: 30px; display: grid; grid-template-columns: minmax(0, 1fr) 84px 148px 124px; align-items: center; padding: 0 14px; cursor: default; }
  .row:hover { background: rgb(255 255 255 / 0.05); }
  .row.sel { background: rgb(var(--accent-rgb) / 0.22); }
  .row.dim { opacity: 0.55; }
  .row.drop { outline: 1px solid var(--accent); background: rgb(var(--accent-rgb) / 0.28); }
  .cell { padding: 0 4px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--muted); }
  .cell.name { display: flex; align-items: center; gap: 9px; color: var(--text); }
  .cell.size { text-align: right; padding-right: 14px; font-variant-numeric: tabular-nums; }
  .ico { display: inline-flex; flex: none; color: var(--muted); }
  .ico.folder { color: #ffd37a; }
  .ico.link { color: #8fd0ff; }
  .nm { flex: 0 0 auto; max-width: 62%; overflow: hidden; text-overflow: ellipsis; }
  .lt { flex: 1 1 0; min-width: 0; color: var(--faint); font-size: 0.74rem; overflow: hidden; text-overflow: ellipsis; }
  .tag { flex: none; font-size: 0.62rem; padding: 0 6px; border-radius: 999px; border: 1px solid var(--line-strong); color: var(--faint); }
  .loadmore { position: absolute; left: 0; right: 0; padding: 6px 18px; color: var(--faint); font-size: 0.78rem; }
  @container (max-width: 560px) {
    .cols, .row { grid-template-columns: minmax(0, 1fr) 76px 128px; }
    .col.type, .cell.type { display: none; }
  }
  .state { display: grid; place-content: center; justify-items: center; text-align: center; gap: 6px; padding: 40px 24px; height: 100%; color: var(--muted); }
  .state h3 { margin: 0; color: var(--text); font-size: 1rem; }
  .state p { margin: 0; max-width: 440px; line-height: 1.5; user-select: text; }
  .state.err h3 { color: #ffb3bd; }
  .acts { display: flex; gap: 8px; flex-wrap: wrap; justify-content: center; margin-top: 10px; }
  .btn { display: inline-flex; align-items: center; gap: 8px; padding: 6px 14px; border-radius: 10px; background: rgb(255 255 255 / 0.09); border: 1px solid var(--line); font-size: 0.82rem; }
  .btn.small { padding: 3px 10px; font-size: 0.74rem; }
  .btn:hover:not(:disabled) { background: rgb(255 255 255 / 0.15); }
  .btn:disabled { opacity: 0.45; cursor: not-allowed; }
  .btn.primary { background: var(--accent); color: #0a0a12; border-color: transparent; font-weight: 600; }
  .btn.danger { background: #ff6b7d; color: #1a0508; border-color: transparent; font-weight: 600; }
  .bad { color: #ffb3bd; }
  .finished { position: absolute; right: 12px; bottom: 10px; width: min(380px, calc(100% - 24px)); padding: 10px 12px; border-radius: 12px; background: rgb(var(--panel-rgb) / 0.97); border: 1px solid rgb(255 107 125 / 0.45); box-shadow: var(--shadow); }
  .finished .fh { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
  .finished ul { margin: 6px 0 0; padding-left: 18px; color: var(--muted); font-size: 0.76rem; line-height: 1.5; user-select: text; }
  .finished small { color: var(--faint); }

  .jobs { display: grid; gap: 4px; padding: 6px 12px; border-top: 1px solid var(--line); background: rgb(var(--accent-rgb) / 0.08); flex: none; }
  .job { display: flex; align-items: center; gap: 10px; font-size: 0.78rem; }
  .jt { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .spin { width: 12px; height: 12px; border-radius: 50%; border: 2px solid rgb(255 255 255 / 0.2); border-top-color: var(--accent); animation: spin 0.8s linear infinite; flex: none; }
  @keyframes spin { to { transform: rotate(360deg); } }
  .status { display: flex; align-items: center; gap: 8px; padding: 5px 12px; border-top: 1px solid var(--line); background: rgb(255 255 255 / 0.03); color: var(--muted); font-size: 0.74rem; white-space: nowrap; overflow: hidden; flex: none; }
  .status .sep { color: var(--faint); }
  .status .muted { color: var(--faint); overflow: hidden; text-overflow: ellipsis; }
  .status .grow { flex: 1; }

  .ctx { position: absolute; z-index: 20; min-width: 210px; padding: 6px; border-radius: 12px; background: rgb(var(--panel-rgb) / 0.98); box-shadow: 0 18px 50px rgb(0 0 0 / 0.55); display: grid; gap: 1px; }
  .mi { display: flex; align-items: center; gap: 9px; padding: 7px 10px; border-radius: 8px; text-align: left; background: transparent; border: 0; font: inherit; }
  .mi:hover:not(:disabled) { background: rgb(255 255 255 / 0.1); }
  .mi:disabled { opacity: 0.4; cursor: not-allowed; }
  .mi.danger { color: #ff9aa8; }
  .mi kbd { margin-left: auto; padding-left: 14px; color: var(--faint); font: inherit; font-size: 0.7rem; }
  .ctx .sep { height: 1px; margin: 4px 6px; background: var(--line); }

  .backdrop { position: absolute; inset: 0; z-index: 30; display: grid; place-items: center; background: rgb(4 4 10 / 0.55); padding: 16px; }
  .panel { width: min(430px, 100%); padding: 20px; border-radius: 16px; background: rgb(var(--panel2-rgb) / 0.98); border: 1px solid var(--line-strong); box-shadow: var(--shadow); display: grid; gap: 12px; }
  .panel h2 { margin: 0; font-size: 1.02rem; overflow-wrap: anywhere; }
  .panel p { margin: 0; color: var(--muted); line-height: 1.5; font-size: 0.84rem; }
  .panel ul { margin: 0; padding-left: 18px; color: var(--muted); font-size: 0.82rem; max-height: 120px; overflow: auto; }
  .panel input { height: 34px; padding: 0 10px; border-radius: 9px; background: rgb(0 0 0 / 0.32); border: 1px solid var(--line-strong); color: var(--text); font: inherit; outline: none; user-select: text; }
  .panel input:focus { border-color: var(--accent); }
  .actions { display: flex; justify-content: flex-end; gap: 8px; }
</style>
