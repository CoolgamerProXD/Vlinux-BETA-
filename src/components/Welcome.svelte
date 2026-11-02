<script lang="ts">
  import { openApp } from '../lib/open-app';
  import Icon from './Icon.svelte';

  let { ondismiss }: { ondismiss: () => void } = $props();
</script>

<section class="welcome glass" aria-labelledby="welcome-title" data-testid="welcome">
  <h2 id="welcome-title">Welcome to VLINUX</h2>
  <p class="lead">A Linux desktop for Windows, powered by WSL2. Your Linux distribution keeps running inside WSL; VLINUX gives it a desktop, a terminal and a file manager. Nothing is installed, started or changed until you ask.</p>

  <ol>
    <li>
      <div>
        <strong>Find your Linux distribution.</strong>
        <span class="muted">Open <em>Distributions</em>: it lists what WSL reports on this PC (read-only — nothing is started). Pick yours and press <em>Set as preferred</em>. That is VLINUX's own setting; it does not change the Windows default.</span>
      </div>
      <button class="primary" onclick={() => openApp('distros')} data-testid="welcome-detect"><Icon name="layers" size={15} /> Show my distributions</button>
    </li>
    <li>
      <div>
        <strong>Use it.</strong>
        <span class="muted">Open the <em>Terminal</em> (a real shell as your distribution's default user) or <em>Files</em> (your Linux files and your Windows folders).</span>
      </div>
      <span class="row">
        <button onclick={() => openApp('terminal')}><Icon name="terminal" size={15} /> Terminal</button>
        <button onclick={() => openApp('files')}><Icon name="folder" size={15} /> Files</button>
      </span>
    </li>
    <li>
      <div>
        <strong>Check that everything works on this PC.</strong>
        <span class="muted"><em>Distributions → Diagnostics</em> runs compatibility checks and gives you a report you can copy. It only creates files if you allow it.</span>
      </div>
    </li>
  </ol>

  <p class="keys"><span class="mono">Alt+F1</span> applications · <span class="mono">F11</span> full screen · <span class="mono">Ctrl+Alt+Q</span> return to Windows · <span class="mono">Alt+Shift+Q</span> power &amp; exit</p>
  <div class="foot">
    <button onclick={ondismiss} data-testid="welcome-dismiss">Don't show this again</button>
  </div>
</section>

<style>
  .welcome { position: absolute; left: 50%; top: clamp(8px, calc(50vh - 340px), 56px); transform: translateX(-50%); width: min(660px, calc(100% - 24px)); max-height: calc(100vh - 175px); overflow: auto; padding: 22px 26px; border-radius: 18px; display: grid; gap: 12px; background: var(--panel-solid); z-index: 5; }
  h2 { margin: 0; font-size: 1.35rem; }
  .lead { margin: 0; color: var(--muted); font-size: 0.9rem; line-height: 1.55; }
  ol { margin: 4px 0 0; padding: 0; list-style: none; display: grid; gap: 10px; counter-reset: step; }
  li { counter-increment: step; display: flex; justify-content: space-between; gap: 14px; align-items: center; padding: 11px 14px 11px 46px; border-radius: 12px; border: 1px solid var(--line); background: rgb(255 255 255 / 0.04); position: relative; flex-wrap: wrap; font-size: 0.86rem; line-height: 1.5; }
  li::before { content: counter(step); position: absolute; left: 14px; top: 50%; transform: translateY(-50%); width: 22px; height: 22px; border-radius: 50%; background: rgb(var(--accent-rgb) / 0.25); display: grid; place-items: center; font-size: 0.78rem; font-weight: 600; }
  li > div { display: grid; gap: 2px; flex: 1 1 280px; min-width: 0; }
  .muted { color: var(--muted); }
  .row { display: flex; gap: 8px; flex-wrap: wrap; }
  button { display: inline-flex; align-items: center; gap: 7px; padding: 7px 13px; border-radius: 10px; background: rgb(255 255 255 / 0.09); border: 1px solid var(--line); font-size: 0.84rem; color: inherit; cursor: pointer; }
  button:hover { background: rgb(255 255 255 / 0.15); }
  button.primary { background: var(--accent); color: #0a0a12; border-color: transparent; font-weight: 600; }
  button.primary:hover { filter: brightness(1.1); background: var(--accent); }
  .keys { margin: 0; font-size: 0.8rem; color: var(--muted); }
  .keys .mono { padding: 1px 6px; border-radius: 5px; background: rgb(255 255 255 / 0.08); color: inherit; }
  .foot { display: flex; justify-content: flex-end; }
</style>
