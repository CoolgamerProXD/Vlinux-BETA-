<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { clock } from './lib/clock.svelte';
  import { config } from './lib/config.svelte';
  import { host } from './lib/host.svelte';
  import { exitToWindows, setFullscreen } from './lib/ipc';
  import { applyTheme } from './lib/theme';
  import { ui } from './lib/ui.svelte';
  import { wm } from './lib/wm.svelte';
  import AppHost from './components/AppHost.svelte';
  import CloseConfirm from './components/CloseConfirm.svelte';
  import Dock from './components/Dock.svelte';
  import Launcher from './components/Launcher.svelte';
  import PowerMenu from './components/PowerMenu.svelte';
  import StatusPanel from './components/StatusPanel.svelte';
  import Toasts from './components/Toasts.svelte';
  import TopBar from './components/TopBar.svelte';
  import Wallpaper from './components/Wallpaper.svelte';
  import WindowFrame from './components/WindowFrame.svelte';
  import Welcome from './components/Welcome.svelte';

  let ready = $state(false);

  // First-run guidance. Pure UI state (kept in the WebView's own storage, not in config.toml): it is
  // shown while the user has not chosen a distribution and has not dismissed it.
  const WELCOME_KEY = 'vlinux.welcome.dismissed';
  let welcomeDismissed = $state(false);
  try {
    welcomeDismissed = localStorage.getItem(WELCOME_KEY) === '1';
  } catch {
    /* storage unavailable: show the guidance */
  }
  const showWelcome = $derived(ready && !welcomeDismissed && !config.value.runtime.selected_distro && wm.visible.length === 0);
  function dismissWelcome() {
    welcomeDismissed = true;
    try {
      localStorage.setItem(WELCOME_KEY, '1');
    } catch {
      /* ignore */
    }
  }

  onMount(() => {
    void (async () => {
      try {
        await Promise.all([config.load(), host.init()]);
      } catch (e) {
        // Never leave the user with an invisible shell: show it with defaults and say what happened.
        ui.toast(`Start-up problem: ${String((e as Error)?.message ?? e)}`, 'error', 9000);
      }
      ready = true;
      performance.mark('vlinux-ready');
    })();
    return () => {
      host.dispose();
      clock.stop();
    };
  });

  $effect(() => applyTheme(config.value));
  // Effects depend only on the config value; the side effects themselves are untracked.
  $effect(() => {
    const perSecond = config.value.shell.show_seconds;
    untrack(() => clock.start(perSecond));
  });
  $effect(() => {
    const count = config.value.shell.workspace_count;
    untrack(() => wm.limitWorkspaces(count));
  });

  let work: HTMLElement | undefined = $state();
  $effect(() => {
    if (!work) return;
    const ro = new ResizeObserver(([e]) => {
      if (e) wm.setBounds(Math.round(e.contentRect.width), Math.round(e.contentRect.height));
    });
    ro.observe(work);
    return () => ro.disconnect();
  });

  async function guarded(fn: () => Promise<unknown>) {
    try {
      await fn();
    } catch (e) {
      ui.toast(String((e as Error)?.message ?? e), 'error');
    }
  }

  function onkey(e: KeyboardEvent) {
    // Ctrl+Alt+Q: always-available, immediate way back to Windows (nothing is lost).
    if (e.ctrlKey && e.altKey && !e.shiftKey && e.code === 'KeyQ') {
      e.preventDefault();
      void guarded(exitToWindows);
      return;
    }
    if (e.code === 'F11') {
      e.preventDefault();
      void guarded(() => setFullscreen(!host.fullscreen));
      return;
    }
    if (e.code === 'Escape' && ui.overlay) {
      ui.close();
      return;
    }
    if (!e.altKey || e.ctrlKey || e.metaKey) return;
    if (e.code === 'F1') {
      e.preventDefault();
      ui.toggle('launcher');
    } else if (e.shiftKey && e.code === 'KeyQ') {
      e.preventDefault();
      ui.toggle('power');
    } else if (!e.shiftKey && e.code === 'KeyW') {
      e.preventDefault();
      wm.closeFocused();
    } else if (!e.shiftKey && /^Digit[1-9]$/.test(e.code)) {
      e.preventDefault();
      wm.switchWorkspace(Number(e.code.slice(5)) - 1, config.value.shell.workspace_count);
    }
  }
</script>

<svelte:window onkeydown={onkey} />

<div class="shell" class:ready data-dock={config.value.dock.position}>
  <Wallpaper id={config.value.appearance.wallpaper} />
  <TopBar />
  <div class="body">
    <Dock />
    <main class="work" bind:this={work} aria-label="Desktop">
      {#each wm.windows as win (win.id)}
        <WindowFrame {win}><AppHost appId={win.appId} /></WindowFrame>
      {/each}
      {#if showWelcome}
        <Welcome ondismiss={dismissWelcome} />
      {:else if wm.visible.length === 0}
        <div class="hint" aria-hidden="true">
          <span class="mono">Alt+F1</span> applications · <span class="mono">Alt+Shift+Q</span> power &amp; exit
        </div>
      {/if}
    </main>
  </div>

  {#if ui.overlay === 'launcher'}<Launcher />{/if}
  {#if ui.overlay === 'status'}<StatusPanel />{/if}
  {#if ui.overlay === 'power'}<PowerMenu />{/if}
  <CloseConfirm />
  <Toasts />
</div>


<style>
  .shell { position: absolute; inset: 0; display: flex; flex-direction: column; opacity: 0; transition: opacity 400ms ease; }
  .shell.ready { opacity: 1; }
  .body { flex: 1; min-height: 0; display: flex; flex-direction: column-reverse; align-items: stretch; padding: var(--gap); gap: var(--gap); position: relative; }
  .body :global(.dock) { align-self: center; }
  [data-dock='left'] .body { flex-direction: row; align-items: center; }
  [data-dock='left'] .body :global(.dock) { align-self: center; }
  .work { position: relative; flex: 1; min-width: 0; min-height: 0; align-self: stretch; }
  .hint { position: absolute; inset: auto 0 12px 0; text-align: center; color: rgb(255 255 255 / 0.35); font-size: 0.8rem; pointer-events: none; }
  .hint .mono { padding: 1px 6px; border-radius: 5px; background: rgb(255 255 255 / 0.08); }
</style>
