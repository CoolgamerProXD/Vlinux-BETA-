<script lang="ts">
  import { clock } from '../lib/clock.svelte';
  import { config } from '../lib/config.svelte';
  import { host } from '../lib/host.svelte';
  import { ui } from '../lib/ui.svelte';
  import { wm } from '../lib/wm.svelte';
  import Icon from './Icon.svelte';
  import Logo from './Logo.svelte';

  const shell = $derived(config.value.shell);
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

    {#if focused}
      <span class="title" title={focused.title}>{focused.title}</span>
    {/if}
  </div>

  <div class="center" aria-live="off">
    <span class="time mono">{time}</span>
    {#if shell.show_date}<span class="date">{date}</span>{/if}
  </div>

  <div class="right">
    <button class="chip" aria-label="System status" title="System status" aria-expanded={ui.overlay === 'status'}
            onclick={() => ui.toggle('status')}>
      <Icon name={host.online ? 'wifi' : 'wifi-off'} size={16} />
      {#if batteryPct !== null}
        <span class="batt mono" class:charging={host.battery?.charging} title={host.battery?.charging ? 'Charging' : 'On battery'}>{batteryPct}%</span>
      {/if}
      <Icon name="activity" size={16} />
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
    .title, .date { display: none; }
  }
  @media (max-width: 640px) {
    .batt { display: none; }
    .pill { min-width: 18px; padding: 0 4px; }
    .pill.active { min-width: 26px; }
  }
</style>
