<script lang="ts">
  import { clock } from '../lib/clock.svelte';
  import { config } from '../lib/config.svelte';
  import { appById } from '../lib/apps';
  import { ctx, type MenuItem } from '../lib/ctx.svelte';
  import { distros } from '../lib/distros.svelte';
  import { shownState } from '../lib/distros-view';
  import { host } from '../lib/host.svelte';
  import { notify } from '../lib/notify.svelte';
  import { ui } from '../lib/ui.svelte';
  import { wm, type WinState } from '../lib/wm.svelte';
  import { windowMenu } from '../lib/menus';
  import Icon from './Icon.svelte';
  import Logo from './Logo.svelte';

  const shell = $derived(config.value.shell);
  const bar = $derived(config.value.topbar);
  const distroName = $derived(config.value.runtime.selected_distro);
  // The state dot only shows what the Distribution Manager already knows; the bar never asks WSL.
  const distroCard = $derived(distroName ? distros.cards.find((c) => c.name === distroName) ?? null : null);
  const distroTone = $derived(distroCard ? shownState(distroCard, distros.pending[distroCard.name]).tone : 'idle');
  const distroLabel = $derived(
    distroCard ? shownState(distroCard, distros.pending[distroCard.name]).label : distroName ? 'state unknown until checked' : 'none selected',
  );
  const here = $derived(wm.ordered.filter((w) => w.workspace === wm.workspace));

  function winClick(w: WinState) {
    if (wm.focusedId === w.id && !w.minimized) wm.minimize(w.id);
    else wm.reveal(w.id);
  }
  function winMenu(e: MouseEvent | KeyboardEvent, w: WinState) {
    ctx.openFrom(e, windowMenu(w, shell.workspace_count), w.title);
  }
  const time = $derived(
    clock.now.toLocaleTimeString(undefined, {
      hour: '2-digit',
      minute: '2-digit',
      second: shell.show_seconds ? '2-digit' : undefined,
      hour12: !shell.clock_24h,
    }),
  );
  const date = $derived(clock.now.toLocaleDateString(undefined, { weekday: 'short', day: 'numeric', month: 'short' }));
  const focused = $derived(wm.windows.find((w) => w.id === wm.focusedId));
  const batteryPct = $derived(host.battery ? Math.round(host.battery.level * 100) : null);

  async function toggleFullscreen() {
    try {
      await host.toggleFullscreen();
    } catch (e) {
      ui.toast(`Could not change full-screen: ${String(e)}`, 'error');
    }
  }
</script>

<header class="bar glass">
  <div class="left">
    <button class="logo" aria-label="Open application launcher" title="Applications (Alt+F1)"
            aria-expanded={ui.overlay === 'launcher'} onclick={() => ui.toggle('launcher')}>
      <Logo size={22} />
    </button>

    {#if bar.show_workspaces}
    <nav class="ws" aria-label="Workspaces">
      {#each { length: shell.workspace_count } as _, i}
        <button class="pill" class:active={wm.workspace === i} class:occupied={wm.occupied(i)}
                aria-label={`Workspace ${i + 1}${wm.occupied(i) ? ' (has windows)' : ''}`}
                aria-current={wm.workspace === i ? 'true' : undefined}
                title={`Workspace ${i + 1} (Alt+${i + 1})`}
                onclick={() => wm.switchWorkspace(i, shell.workspace_count)}>
          <span>{i + 1}</span>
        </button>
      {/each}
    </nav>
    {/if}

    {#if bar.show_windows && here.length}
      <ul class="wins" aria-label="Windows on this workspace">
        {#each here as w (w.id)}
          {@const app = appById(w.appId)}
          <li>
            <button class="win" class:active={wm.focusedId === w.id && !w.minimized} class:min={w.minimized}
                    aria-label={`${w.title}${w.minimized ? ' (minimised)' : ''}`} aria-pressed={wm.focusedId === w.id && !w.minimized}
                    title={`${w.title}${w.minimized ? ' — minimised' : ''}`}
                    onclick={() => winClick(w)} oncontextmenu={(e) => winMenu(e, w)}
                    onkeydown={(e) => (e.key === 'ContextMenu' || (e.shiftKey && e.key === 'F10')) && winMenu(e, w)}>
              {#if app}<Icon name={app.icon} size={15} />{/if}
            </button>
          </li>
        {/each}
      </ul>
    {/if}

    {#if bar.show_title && focused}
      <span class="title" title={focused.title}>{focused.title}</span>
    {/if}
  </div>

  <div class="center" aria-live="off">
    <span class="time mono">{time}</span>
    {#if shell.show_date}<span class="date">{date}</span>{/if}
  </div>

  <div class="right">
    <button class="chip distro" aria-label={`Linux distribution: ${distroName ?? 'none selected'}, ${distroLabel}`} title="Linux distribution"
            aria-expanded={ui.overlay === 'distro'} onclick={() => ui.toggle('distro')}>
      <span class="sdot {distroTone}" aria-hidden="true"></span>
      <span class="dname">{distroName ?? 'No distro'}</span>
    </button>
    <button class="chip" aria-label="System status" title="System status" aria-expanded={ui.overlay === 'status'}
            onclick={() => ui.toggle('status')}>
      <Icon name={host.online ? 'wifi' : 'wifi-off'} size={16} />
      {#if batteryPct !== null && bar.show_battery}
        <span class="batt mono" class:charging={host.battery?.charging} title={host.battery?.charging ? 'Charging' : 'On battery'}>{batteryPct}%</span>
      {/if}
      <Icon name="activity" size={16} />
    </button>
    <button class="chip icon bell" aria-label={notify.unread ? `Notifications, ${notify.unread} unread` : 'Notifications'} title="Notifications (Alt+Shift+N)"
            aria-expanded={ui.overlay === 'notifications'} onclick={() => ui.toggle('notifications')}>
      <Icon name="bell" size={16} />
      {#if notify.unread}<span class="badge mono" class:err={notify.unreadErrors > 0}>{notify.unread > 9 ? '9+' : notify.unread}</span>{/if}
    </button>
    <button class="chip icon" aria-label={host.fullscreen ? 'Leave full screen' : 'Enter full screen'}
            title={host.fullscreen ? 'Leave full screen (F11)' : 'Enter full screen (F11)'} onclick={toggleFullscreen}>
      <Icon name={host.fullscreen ? 'shrink' : 'expand'} size={16} />
    </button>
    <button class="chip icon power" aria-label="Power and exit menu" title="Power / return to Windows (Alt+Shift+Q)"
            aria-expanded={ui.overlay === 'power'} onclick={() => ui.toggle('power')}>
      <Icon name="power" size={16} />
    </button>
  </div>
</header>

<style>
  .bar {
    height: var(--bar-h);
    margin: var(--gap) var(--gap) 0;
    border-radius: 14px;
    display: grid;
    grid-template-columns: 1fr auto 1fr;
    align-items: center;
    padding: 0 6px;
    font-size: 0.86rem;
    position: relative;
    z-index: 50;
    animation: drop-in 500ms var(--ease) both;
  }
  .left, .right { display: flex; align-items: center; gap: 6px; min-width: 0; }
  .right { justify-content: flex-end; }

  .logo { width: 30px; height: 30px; display: grid; place-items: center; border-radius: 9px; transition: background var(--dur); }
  .logo:hover, .logo[aria-expanded='true'] { background: rgb(255 255 255 / 0.1); }

  .ws { display: flex; gap: 4px; padding: 0 4px; }
  .pill {
    min-width: 22px; height: 22px; padding: 0 6px; border-radius: 999px;
    display: grid; place-items: center;
    font-family: var(--font-mono); font-size: 0.72rem; color: var(--faint);
    background: rgb(255 255 255 / 0.04);
    transition: all var(--dur) var(--ease);
  }
  .pill:hover { color: var(--text); background: rgb(255 255 255 / 0.1); }
  .pill.occupied { color: var(--muted); }
  .pill.active {
    min-width: 34px; color: #0a0a12; font-weight: 600;
    background: linear-gradient(120deg, var(--accent), var(--accent-2));
    box-shadow: 0 0 14px rgb(var(--accent-rgb) / 0.55);
  }

  .wins { list-style: none; margin: 0; padding: 0 2px; display: flex; gap: 2px; min-width: 0; }
  .win { width: 28px; height: 26px; border-radius: 8px; display: grid; place-items: center; color: var(--muted); position: relative; transition: background var(--dur), color var(--dur); }
  .win:hover { background: rgb(255 255 255 / 0.1); color: var(--text); }
  .win.min { opacity: 0.55; }
  .win.active { color: var(--text); background: rgb(var(--accent-rgb) / 0.2); }
  .win.active::after { content: ''; position: absolute; left: 8px; right: 8px; bottom: 1px; height: 2px; border-radius: 2px; background: var(--accent); }

  .distro { max-width: 170px; }
  .dname { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 0.8rem; }
  .sdot { width: 8px; height: 8px; border-radius: 50%; background: var(--faint); flex: none; }
  .sdot.good { background: #5ff0b0; box-shadow: 0 0 7px #5ff0b0; }
  .sdot.busy, .sdot.warn { background: #ffc46b; }
  .sdot.bad { background: #ff7a8a; }
  .bell { position: relative; }
  .badge { position: absolute; top: -2px; right: -2px; min-width: 16px; height: 16px; padding: 0 4px; border-radius: 8px; font-size: 0.62rem; display: grid; place-items: center; background: var(--accent); color: #0a0a12; font-weight: 700; }
  .badge.err { background: #ff7a8a; }

  .title { color: var(--muted); padding-left: 8px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 22ch; }

  .center { display: flex; align-items: baseline; gap: 10px; }
  .time { font-weight: 600; letter-spacing: 0.02em; font-size: 0.95rem; }
  .date { color: var(--muted); }

  .chip {
    height: 28px; padding: 0 9px; border-radius: 9px; display: flex; align-items: center; gap: 7px; color: var(--muted);
    transition: background var(--dur), color var(--dur);
  }
  .chip.icon { width: 30px; padding: 0; justify-content: center; }
  .chip:hover, .chip[aria-expanded='true'] { background: rgb(255 255 255 / 0.1); color: var(--text); }
  .power:hover { color: #ff7a8a; }
  .batt { font-size: 0.74rem; }
  .batt.charging { color: var(--accent); }

  @media (max-width: 900px) {
    .title, .date, .dname { display: none; }
  }
  @media (max-width: 640px) {
    .batt { display: none; }
    .pill { min-width: 18px; padding: 0 4px; }
    .pill.active { min-width: 26px; }
  }
</style>
