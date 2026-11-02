<script lang="ts">
  import '@xterm/xterm/css/xterm.css';
  import { onMount, tick, untrack } from 'svelte';
  import Icon from '../components/Icon.svelte';
  import { config } from '../lib/config.svelte';
  import { runtime } from '../lib/runtime.svelte';
  import { statusHeadline } from '../lib/runtime-types';
  import { terminals, type TermSession } from '../lib/terminal.svelte';
  import type { TerminalShortcut } from '../lib/terminal-types';
  import { openApp } from '../lib/open-app';
  import { wm } from '../lib/wm.svelte';

  const report = $derived(runtime.report);
  const status = $derived(report?.status ?? null);
  const installed = $derived((report?.distros ?? []).filter((d) => d.kind === 'regular'));
  const ready = $derived(status?.kind === 'ready');
  const selectedName = $derived(runtime.selected);
  const selectedDistro = $derived(runtime.selectedDistro);
  const sessions = $derived(terminals.sessions);
  const active = $derived(terminals.active);

  let booted = $state(false);
  let autoTried = false;
  let menuOpen = $state(false);
  let stage: HTMLElement | undefined = $state();

  // ---- start-up: look at WSL (cached, read-only), then open the first session ----------------
  onMount(() => {
    wm.guards.terminal = () => {
      const n = terminals.liveCount;
      if (n === 0) return null;
      return n === 1
        ? 'One terminal session is still running. Closing the window ends it, like closing any terminal: programs running in it stop. Your Linux distribution keeps running.'
        : `${n} terminal sessions are still running. Closing the window ends them, like closing any terminal: programs running in them stop. Your Linux distribution keeps running.`;
    };
    terminals.onShortcut = onShortcut;
    void (async () => {
      if (!runtime.report) await runtime.refresh(false);
      booted = true;
    })();
    return () => {
      delete wm.guards.terminal;
      terminals.onShortcut = null;
      // Closing the Terminal window closes its sessions (and only those processes).
      terminals.closeAll();
    };
  });

  $effect(() => {
    if (!booted) return;
    const req = terminals.launchRequest;
    const rdy = ready;
    const sel = selectedDistro;
    untrack(() => {
      if (req !== null) {
        terminals.launchRequest = null;
        autoTried = true;
        if (rdy) void terminals.open(req === '' ? null : req);
      } else if (!autoTried && terminals.sessions.length === 0) {
        autoTried = true;
        // Only the user's own selection opens by itself; nothing is guessed or installed.
        if (rdy && sel) void terminals.open(null);
      }
    });
  });

  function onShortcut(s: TerminalShortcut) {
    if (s === 'new-tab') void newDefaultTab();
    else if (s === 'close-tab' && terminals.activeId) void terminals.close(terminals.activeId);
  }

  async function newDefaultTab() {
    if (!ready) return;
    if (selectedDistro) await terminals.open(null);
    else menuOpen = true;
  }

  async function openIn(name: string) {
    menuOpen = false;
    await terminals.open(name);
  }

  // ---- DOM: attach xterm, keep it fitted, keep the theme current ---------------------------------
  function attach(node: HTMLElement, s: TermSession) {
    if (s.term.element) node.appendChild(s.term.element);
    else s.term.open(node);
    s.attached = true;
    // Variable fonts load lazily; measure again when ready so the grid matches the real glyphs.
    void document.fonts?.ready.then(fitSoon);
    fitSoon();
  }

  let raf = 0;
  function fitSoon() {
    cancelAnimationFrame(raf);
    raf = requestAnimationFrame(() => {
      const s = terminals.active;
      if (!s || !stage || stage.clientWidth === 0 || stage.clientHeight === 0) return;
      try {
        s.fit.fit();
      } catch {
        /* not measurable yet */
      }
    });
  }

  $effect(() => {
    if (!stage) return;
    const ro = new ResizeObserver(() => fitSoon());
    ro.observe(stage);
    return () => {
      ro.disconnect();
      cancelAnimationFrame(raf);
    };
  });

  $effect(() => {
    void terminals.activeId;
    untrack(() => {
      fitSoon();
      void tick().then(() => terminals.focus());
    });
  });

  // Keyboard focus follows the window focus.
  const windowFocused = $derived(wm.byApp('terminal')?.id === wm.focusedId);
  $effect(() => {
    if (windowFocused) untrack(() => void tick().then(() => terminals.focus()));
  });

  // Accent colour changes restyle open terminals.
  $effect(() => {
    void config.value.appearance.accent;
    // The theme sets the CSS variables in its own effect; read them one frame later.
    untrack(() => void requestAnimationFrame(() => terminals.restyle()));
  });

  // ---- labels ------------------------------------------------------------------------------
  function tabLabel(s: TermSession): string {
    const same = sessions.filter((x) => x.info.distro === s.info.distro);
    return same.length > 1 ? `${s.info.distro} · ${same.indexOf(s) + 1}` : s.info.distro;
  }
  function stateWord(s: TermSession): string {
    return s.state.kind === 'running' ? 'running' : s.state.kind === 'exited' ? 'ended' : 'failed';
  }
  const activeDistro = $derived(active ? installed.find((d) => d.name === active.info.distro) ?? null : null);
</script>

<div class="term" class:has={sessions.length > 0}>
  {#if sessions.length > 0 || ready}
    <div class="bar">
      <div class="tabs" role="tablist" aria-label="Terminal sessions">
        {#each sessions as s (s.info.id)}
          <div class="tab" class:on={s.info.id === terminals.activeId} data-session={s.info.id}>
            <button class="tab-main" role="tab" aria-selected={s.info.id === terminals.activeId}
                    title="{s.info.label} ({s.info.distro}) — {stateWord(s)}" onclick={() => terminals.activate(s.info.id)}>
              <span class="dot {s.state.kind}" aria-label={stateWord(s)}></span>
              <span class="name">{tabLabel(s)}</span>
            </button>
            <button class="tab-x" aria-label="Close {tabLabel(s)}" title="Close session (Ctrl+Shift+W)" onclick={() => terminals.close(s.info.id)}>
              <Icon name="close" size={12} />
            </button>
          </div>
        {/each}
        <div class="newgroup">
          <button class="icon-btn" aria-label="New terminal" title={selectedDistro ? `New terminal in ${selectedDistro.name} (Ctrl+Shift+T)` : 'New terminal'}
                  onclick={newDefaultTab} disabled={!ready || terminals.opening}>
            <Icon name="plus" size={15} />
          </button>
          <button class="icon-btn" aria-label="Choose a distribution for a new terminal" title="Open in another distribution"
                  aria-expanded={menuOpen} onclick={() => (menuOpen = !menuOpen)} disabled={!ready || installed.length === 0}>
            <Icon name="chevron-down" size={14} />
          </button>
          {#if menuOpen}
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <div class="menu-catch" onpointerdown={() => (menuOpen = false)}></div>
            <div class="menu glass" role="menu" aria-label="Open a terminal in">
              <div class="menu-title">Open a terminal in…</div>
              {#each installed as d (d.name)}
                <button role="menuitem" class="menu-item" data-distro={d.name} onclick={() => openIn(d.name)}
                        disabled={d.state === 'installing' || d.state === 'uninstalling' || d.state === 'converting'}>
                  <strong>{d.name}</strong>
                  <span>{d.display_label}{d.name === selectedName ? ' · selected for VLINUX' : ''}{d.state === 'stopped' ? ' · stopped (will start)' : ''}</span>
                </button>
              {/each}
              <div class="menu-note">Choosing here does not change your selected or default distribution.</div>
            </div>
          {/if}
        </div>
      </div>
      <div class="tools">
        {#if active}
          <span class="distro-badge" title="This session runs in {active.info.label}">
            <Icon name="terminal" size={13} />
            <strong>{active.info.distro}</strong>
            <span class="muted">{active.info.label}</span>
          </span>
        {/if}
        <button class="icon-btn text" onclick={() => terminals.clear()} disabled={!active} title="Clear the screen and scrollback">Clear</button>
      </div>
    </div>
  {/if}

  <div class="stage" bind:this={stage} class:empty={sessions.length === 0}>
    {#each sessions as s (s.info.id)}
      <div class="pane" class:active={s.info.id === terminals.activeId} data-session={s.info.id}
           use:attach={s} aria-label="Terminal in {s.info.distro}"></div>
    {/each}

    {#if active && active.state.kind !== 'running'}
      <div class="ended" role="status">
        <span>
          {#if active.state.kind === 'exited'}
            The shell in <strong>{active.info.distro}</strong> ended{active.state.code === null ? '' : ` (exit code ${active.state.code})`}.
          {:else}
            This session failed: {active.state.message}
          {/if}
        </span>
        <button class="btn primary" onclick={() => terminals.restart(active.info.id)} disabled={terminals.opening}>Restart</button>
        <button class="btn" onclick={() => terminals.close(active.info.id)}>Close tab</button>
      </div>
    {/if}

    {#if sessions.length === 0}
      <div class="gate">
        {#if !booted || runtime.loading || (!report && !runtime.error)}
          <div class="card"><h2>Checking WSL…</h2><p>A read-only look at which distributions are installed. Nothing is started.</p></div>
        {:else if !report}
          <div class="card warn">
            <h2>Could not check WSL</h2>
            <p>{runtime.error}</p>
            <div class="acts"><button class="btn" onclick={() => runtime.refresh(true)}>Try again</button></div>
          </div>
        {:else if status?.kind === 'no_distributions'}
          <div class="card" data-gate="no-distributions">
            <h2>No Linux distribution is installed</h2>
            <p>The terminal runs a real shell inside a Linux distribution on WSL. WSL is ready, but no distribution is installed yet. VLINUX never installs one without asking.</p>
            <div class="acts"><button class="btn primary" onclick={() => openApp('setup')}>Open Linux Setup</button></div>
          </div>
        {:else if status && status.kind !== 'ready'}
          <div class="card warn" data-gate="wsl-unavailable">
            <h2>{statusHeadline(status)}</h2>
            <p>
              {#if status.kind === 'feature_problem'}{status.hint}
              {:else if status.kind === 'unsupported_host'}{status.reason}
              {:else if status.kind === 'error'}{status.message}
              {:else if status.kind === 'restart_required'}Windows has to restart before WSL can be used. Nothing was restarted for you.
              {:else}The terminal needs WSL, which is not available on this PC yet.{/if}
            </p>
            <div class="acts">
              <button class="btn primary" onclick={() => openApp('setup')}>Open Linux Setup</button>
              <button class="btn" onclick={() => runtime.refresh(true)}>Check again</button>
            </div>
          </div>
        {:else if selectedName && !selectedDistro}
          <div class="card warn" data-gate="selected-missing">
            <h2>“{selectedName}” is no longer available</h2>
            <p>The distribution you selected for VLINUX is not installed any more (it may have been removed or renamed). VLINUX did not change anything. Choose another one:</p>
            <div class="choices">
              {#each installed as d (d.name)}
                <button class="choice" onclick={() => openIn(d.name)}><strong>{d.name}</strong><span>{d.display_label}</span></button>
              {/each}
            </div>
            <div class="acts"><button class="btn primary" onclick={() => openApp('setup')}>Open Linux Setup</button></div>
          </div>
        {:else if !selectedDistro}
          <div class="card" data-gate="choose">
            <h2>Choose a distribution</h2>
            <p>No distribution is selected for VLINUX yet. Open a terminal in one of your installed distributions — this does not change your selection or WSL's default.</p>
            <div class="choices">
              {#each installed as d (d.name)}
                <button class="choice" data-distro={d.name} onclick={() => openIn(d.name)}><strong>{d.name}</strong><span>{d.display_label}{d.state === 'stopped' ? ' · stopped (will start)' : ''}</span></button>
              {/each}
            </div>
            <div class="acts"><button class="btn" onclick={() => openApp('setup')}>Open Linux Setup to select one</button></div>
          </div>
        {:else}
          <div class="card" data-gate="idle">
            <h2>{terminals.opening ? 'Opening…' : 'No open terminals'}</h2>
            <p>
              {#if terminals.error}<span class="err">{terminals.error}</span>{:else}Open a shell in <strong>{selectedDistro.name}</strong> ({selectedDistro.display_label}).{/if}
            </p>
            <div class="acts">
              <button class="btn primary" onclick={() => terminals.open(null)} disabled={terminals.opening}>Open terminal in {selectedDistro.name}</button>
            </div>
          </div>
        {/if}
      </div>
    {/if}
  </div>

  {#if active}
    <div class="status" role="status">
      <span class="dot {active.state.kind}"></span>
      <span>{stateWord(active)}</span>
      <span class="sep">·</span>
      <span title="Registered WSL name">{active.info.distro}</span>
      <span class="muted">{active.info.label}</span>
      {#if activeDistro && activeDistro.adapter.package_manager !== 'unknown'}<span class="sep">·</span><span>{activeDistro.adapter.package_manager_name}</span>{/if}
      {#if activeDistro}<span class="sep">·</span><span>WSL {activeDistro.wsl_version}</span>{/if}
      <span class="sep">·</span><span>{active.info.shell}</span>
      <span class="grow"></span>
      <span class="mono">{active.cols}×{active.rows}</span>
    </div>
  {/if}
</div>

<style>
  .term { height: 100%; min-height: 0; display: flex; flex-direction: column; background: #0b0b14; }
  .bar { display: flex; align-items: stretch; gap: 8px; padding: 6px 8px 0; background: rgb(255 255 255 / 0.035); border-bottom: 1px solid var(--line); min-height: 38px; }
  .tabs { display: flex; align-items: flex-end; gap: 2px; min-width: 0; flex: 1; overflow-x: auto; scrollbar-width: none; position: relative; }
  .tabs::-webkit-scrollbar { display: none; }
  .tab { display: flex; align-items: center; border-radius: 9px 9px 0 0; background: transparent; border: 1px solid transparent; border-bottom: 0; max-width: 220px; flex: 0 1 auto; min-width: 0; }
  .tab:hover { background: rgb(255 255 255 / 0.05); }
  .tab.on { background: #0b0b14; border-color: var(--line); box-shadow: inset 0 2px 0 var(--accent); }
  .tab-main { display: flex; align-items: center; gap: 8px; padding: 7px 4px 7px 12px; min-width: 0; color: var(--muted); font-size: 0.82rem; }
  .tab.on .tab-main { color: var(--text); }
  .name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tab-x { display: grid; place-items: center; width: 22px; height: 22px; margin-right: 6px; border-radius: 6px; color: var(--faint); }
  .tab-x:hover { background: rgb(255 255 255 / 0.12); color: var(--text); }
  .dot { width: 8px; height: 8px; border-radius: 50%; flex: none; background: #8a8aa5; }
  .dot.running { background: #5ff0b0; box-shadow: 0 0 6px rgb(95 240 176 / 0.7); }
  .dot.exited { background: #8a8aa5; }
  .dot.failed { background: #ff6b81; }
  .newgroup { display: flex; align-items: center; gap: 0; padding: 0 0 4px 4px; position: relative; flex: none; }
  .icon-btn { display: grid; place-items: center; min-width: 26px; height: 26px; padding: 0 6px; border-radius: 8px; color: var(--muted); }
  .icon-btn:hover:not(:disabled) { background: rgb(255 255 255 / 0.1); color: var(--text); }
  .icon-btn:disabled { opacity: 0.4; cursor: not-allowed; }
  .icon-btn.text { font-size: 0.78rem; border: 1px solid var(--line); padding: 0 10px; }
  .tools { display: flex; align-items: center; gap: 8px; padding-bottom: 4px; flex: none; }
  .distro-badge { display: inline-flex; align-items: center; gap: 6px; padding: 3px 10px; border-radius: 999px; background: rgb(var(--accent-rgb) / 0.14); border: 1px solid rgb(var(--accent-rgb) / 0.35); font-size: 0.76rem; max-width: 260px; white-space: nowrap; overflow: hidden; }
  .distro-badge .muted { color: var(--muted); overflow: hidden; text-overflow: ellipsis; }
  .menu-catch { position: fixed; inset: 0; z-index: 5; }
  .menu { position: absolute; z-index: 6; top: 30px; left: 0; min-width: 280px; max-width: 340px; padding: 8px; border-radius: 14px; background: rgb(16 16 28 / 0.97); display: grid; gap: 2px; box-shadow: 0 18px 50px rgb(0 0 0 / 0.5); }
  .menu-title { padding: 4px 8px 6px; font-size: 0.7rem; text-transform: uppercase; letter-spacing: 0.12em; color: var(--faint); }
  .menu-item { display: grid; gap: 1px; text-align: left; padding: 8px 10px; border-radius: 9px; }
  .menu-item:hover:not(:disabled) { background: rgb(255 255 255 / 0.09); }
  .menu-item:disabled { opacity: 0.45; }
  .menu-item span { color: var(--muted); font-size: 0.76rem; }
  .menu-note { padding: 6px 8px 2px; color: var(--faint); font-size: 0.72rem; line-height: 1.4; }

  .stage { position: relative; flex: 1; min-height: 0; overflow: hidden; background: #0b0b14; }
  .pane { position: absolute; inset: 0; display: none; }
  .pane.active { display: block; }
  .pane :global(.xterm) { height: 100%; padding: 8px 6px 4px 12px; }
  .pane :global(.xterm-viewport) { background-color: transparent !important; scrollbar-width: thin; scrollbar-color: rgb(255 255 255 / 0.2) transparent; }

  .ended { position: absolute; left: 12px; right: 12px; bottom: 12px; display: flex; gap: 10px; align-items: center; flex-wrap: wrap; padding: 10px 12px; border-radius: 12px; background: rgb(16 16 28 / 0.94); border: 1px solid var(--line-strong); font-size: 0.84rem; backdrop-filter: blur(8px); }
  .ended span { flex: 1; min-width: 180px; color: var(--muted); }
  .ended strong { color: var(--text); }

  .gate { position: absolute; inset: 0; display: grid; place-items: center; padding: 18px; overflow: auto; }
  .card { width: min(560px, 100%); padding: 22px; border-radius: 18px; background: rgb(255 255 255 / 0.05); border: 1px solid var(--line); }
  .card.warn { border-color: rgb(255 180 84 / 0.45); background: rgb(255 180 84 / 0.08); }
  .card h2 { margin: 0 0 8px; font-size: 1.05rem; }
  .card p { margin: 0 0 16px; color: var(--muted); line-height: 1.55; font-size: 0.88rem; }
  .card .err { color: #ffb3bd; }
  .acts { display: flex; gap: 10px; flex-wrap: wrap; }
  .choices { display: grid; gap: 8px; margin: 0 0 16px; }
  .choice { display: grid; gap: 1px; text-align: left; padding: 10px 14px; border-radius: 12px; background: rgb(255 255 255 / 0.06); border: 1px solid var(--line); }
  .choice:hover { background: rgb(255 255 255 / 0.12); border-color: var(--line-strong); }
  .choice span { color: var(--muted); font-size: 0.78rem; }
  .btn { display: inline-flex; align-items: center; gap: 8px; padding: 7px 14px; border-radius: 10px; background: rgb(255 255 255 / 0.09); border: 1px solid var(--line); font-size: 0.84rem; }
  .btn:hover:not(:disabled) { background: rgb(255 255 255 / 0.15); }
  .btn:disabled { opacity: 0.45; cursor: not-allowed; }
  .btn.primary { background: var(--accent); color: #0a0a12; border-color: transparent; font-weight: 600; }
  .btn.primary:hover:not(:disabled) { filter: brightness(1.1); background: var(--accent); }

  .status { display: flex; align-items: center; gap: 8px; padding: 5px 12px; border-top: 1px solid var(--line); background: rgb(255 255 255 / 0.03); color: var(--muted); font-size: 0.74rem; white-space: nowrap; overflow: hidden; }
  .status .sep { color: var(--faint); }
  .status .muted { color: var(--faint); overflow: hidden; text-overflow: ellipsis; }
  .status .grow { flex: 1; }
</style>
