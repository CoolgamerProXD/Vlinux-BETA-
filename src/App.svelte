<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { clock } from './lib/clock.svelte';
  import { config } from './lib/config.svelte';
  import { host } from './lib/host.svelte';
  import { exitToWindows } from './lib/ipc';
  import { applyTheme } from './lib/theme';
  import { ctx } from './lib/ctx.svelte';
  import { linuxApps } from './lib/linux-apps.svelte';
  import { desktopMenu } from './lib/menus';
  import { notify } from './lib/notify.svelte';
  import { appById } from './lib/apps';
  import { desktopAction } from './lib/shortcuts';
  import { ui } from './lib/ui.svelte';
  import { wm } from './lib/wm.svelte';
  import AppHost from './components/AppHost.svelte';
  import CloseConfirm from './components/CloseConfirm.svelte';
  import ContextMenu from './components/ContextMenu.svelte';
  import DistroPopover from './components/DistroPopover.svelte';
  import NotificationCenter from './components/NotificationCenter.svelte';
  import ShortcutOverlay from './components/ShortcutOverlay.svelte';
  import WindowSwitcher from './components/WindowSwitcher.svelte';
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
      if (config.warning) notify.push({ kind: 'warning', source: 'settings', text: config.warning, ms: 12000 });
      void linuxApps.attach();
      // Start-up app: only apps that never start a Linux distribution are offered in Settings.
      const first = config.value.startup.open_app;
      const app = first === 'none' ? undefined : appById(first);
      if (app && wm.windows.length === 0) wm.open(app);
    })();
    // Nothing needs to tick while the window is hidden or minimised.
    const vis = () => {
      document.documentElement.classList.toggle('paused', document.hidden);
      if (document.hidden) clock.stop();
      else clock.start(config.value.shell.show_seconds);
    };
    document.addEventListener('visibilitychange', vis);
    return () => {
      document.removeEventListener('visibilitychange', vis);
      host.dispose();
      linuxApps.detach();
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
    if (e.code === 'Escape') {
      if (wm.switcher) {
        e.preventDefault();
        wm.cycleCancel();
        return;
      }
      if (ctx.menu) {
        ctx.close();
        return;
      }
      if (ui.overlay) ui.close();
      return;
    }
    const act = desktopAction(e);
    if (!act) return;
    e.preventDefault();
    const count = config.value.shell.workspace_count;
    switch (act.type) {
      case 'return-to-windows':
        void guarded(exitToWindows);
        break;
      case 'fullscreen':
        void guarded(() => host.toggleFullscreen());
        break;
      case 'launcher':
        ui.toggle('launcher');
        break;
      case 'shortcuts':
        ui.toggle('shortcuts');
        break;
      case 'notifications':
        ui.toggle('notifications');
        break;
      case 'power':
        ui.toggle('power');
        break;
      case 'close-window':
        wm.closeFocused();
        break;
      case 'workspace':
        wm.switchWorkspace(act.n, count);
        break;
      case 'move-to-workspace':
        if (wm.focusedId !== null) wm.moveToWorkspace(wm.focusedId, act.n, count);
        break;
      case 'snap':
        if (wm.focusedId !== null) wm.snap(wm.focusedId, act.target);
        break;
      case 'lower':
        if (wm.focusedId !== null) wm.lower(wm.focusedId);
        break;
      case 'cycle':
        if (!ui.overlay) wm.cycleStep(act.dir);
        break;
    }
  }

  // Releasing Alt confirms the window switcher; losing focus cancels it (Alt may never be seen released).
  function onkeyup(e: KeyboardEvent) {
    if (e.key === 'Alt' && wm.switcher) wm.cycleCommit();
  }
  const onblur = () => wm.switcher && wm.cycleCancel();

  function deskMenu(e: MouseEvent) {
    if (e.target !== e.currentTarget && !(e.target as HTMLElement).classList.contains('hint')) return;
    ctx.openFrom(e, desktopMenu(), 'Desktop');
  }
</script>

<svelte:window onkeydown={onkey} onkeyup={onkeyup} onblur={onblur} />

<div class="shell" class:ready data-dock={config.value.dock.position}>
  <Wallpaper id={config.value.appearance.wallpaper} />
  <TopBar />
  <div class="body">
    <Dock />
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <main class="work" bind:this={work} aria-label="Desktop" oncontextmenu={deskMenu}>
      {#each wm.windows as win (win.id)}
        <WindowFrame {win}><AppHost appId={win.appId} /></WindowFrame>
      {/each}
      {#if showWelcome}
        <Welcome ondismiss={dismissWelcome} />
      {:else if wm.visible.length === 0}
        <div class="hint" aria-hidden="true">
          <span class="mono">Alt+F1</span> applications · <span class="mono">Alt+Shift+F1</span> shortcuts · <span class="mono">Alt+Shift+Q</span> power &amp; exit
        </div>
      {/if}
    </main>
  </div>

  {#if ui.overlay === 'launcher'}<Launcher />{/if}
  {#if ui.overlay === 'status'}<StatusPanel />{/if}
  {#if ui.overlay === 'power'}<PowerMenu />{/if}
  {#if ui.overlay === 'notifications'}<NotificationCenter />{/if}
  {#if ui.overlay === 'distro'}<DistroPopover />{/if}
  {#if ui.overlay === 'shortcuts'}<ShortcutOverlay />{/if}
  <WindowSwitcher />
  <CloseConfirm />
  <ContextMenu />
  <Toasts />
</div>


<style>
  .shell { position: absolute; inset: 0; display: flex; flex-direction: column; opacity: 0; transition: opacity 400ms ease; }
  .shell.ready { opacity: 1; }
  .body { flex: 1; min-height: 0; display: flex; flex-direction: column-reverse; align-items: stretch; padding: var(--gap); gap: var(--gap); position: relative; }
  .body :global(.dock) { align-self: center; }
  [data-dock='left'] .body { flex-direction: row; align-items: center; }
  [data-dock='right'] .body { flex-direction: row-reverse; align-items: center; }
  [data-dock='left'] .body :global(.dock), [data-dock='right'] .body :global(.dock) { align-self: center; }
  .work { position: relative; flex: 1; min-width: 0; min-height: 0; align-self: stretch; }
  .hint { position: absolute; inset: auto 0 12px 0; text-align: center; color: rgb(255 255 255 / 0.35); font-size: 0.8rem; pointer-events: none; }
  .hint .mono { padding: 1px 6px; border-radius: 5px; background: rgb(255 255 255 / 0.08); }
</style>
