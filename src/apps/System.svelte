<script lang="ts">
  import { host } from '../lib/host.svelte';
  import { config } from '../lib/config.svelte';
  import { runtime } from '../lib/runtime.svelte';
  import { wm } from '../lib/wm.svelte';
  import { APPS } from '../lib/apps';
  import Logo from '../components/Logo.svelte';

  const info = $derived(host.info);

  const roadmap: { id: string; name: string; state: 'done' | 'next' | 'planned' }[] = [
    { id: 'M0', name: 'Foundation — project, config, build', state: 'done' },
    { id: 'M1', name: 'Desktop shell — bar, dock, launcher, exit to Windows', state: 'done' },
    { id: 'M2', name: 'WSL detection, distro adapters, safe setup workflow, storage model', state: 'done' },
    { id: 'M3', name: 'Linux terminal (real pseudoterminal, tabs)', state: 'done' },
    { id: 'M4', name: 'File manager and shared-folder mapping', state: 'next' },
    { id: 'M5', name: 'Distribution manager', state: 'planned' },
    { id: 'M6', name: 'Polish, resource limits, installer testing', state: 'planned' },
  ];

  const r = $derived(runtime.report);
  const regular = $derived((r?.distros ?? []).filter((d) => d.kind === 'regular'));
  const openSetup = () => wm.open(APPS.find((a) => a.id === 'setup')!);
</script>

<div class="sys">
  <header>
    <Logo size={56} />
    <div>
      <h2>VLINUX</h2>
      <p>Version {info?.app_version ?? '…'} · early development build</p>
    </div>
  </header>

  <section>
    <h3>What this is</h3>
    <p>
      VLINUX is a Windows application that presents a Linux-style desktop. The desktop you are looking at is a
      <strong>Windows-rendered shell</strong>; it is not a Linux compositor and not a bare-metal installation.
      Linux itself will run through <strong>WSL2</strong>, which is virtualised.
    </p>
  </section>

  <section>
    <h3>Host</h3>
    <dl>
      <dt>Operating system</dt><dd>{info ? `${info.os_name} ${info.os_version}` : '…'}</dd>
      <dt>Architecture</dt><dd>{info?.os_arch ?? '…'}</dd>
      <dt>CPU threads</dt><dd>{info?.cpu_threads ?? '…'}</dd>
      <dt>Display mode</dt><dd>{host.fullscreen ? 'Full screen' : 'Windowed'}</dd>
      <dt>Settings file</dt><dd class="mono small">{config.path || '…'}</dd>
    </dl>
  </section>

  <section>
    <h3>Linux runtime</h3>
    <dl>
      <dt>WSL</dt><dd class:dim={!r}>{runtime.summary}</dd>
      {#if r && (r.status.kind === 'ready' || r.status.kind === 'no_distributions')}
        <dt>WSL version</dt><dd>{r.wsl_version ?? 'unknown'} (default: WSL {r.default_wsl_version ?? '?'})</dd>
        <dt>Distributions</dt><dd>{regular.length ? regular.map((d) => `${d.name} (${d.state})`).join(', ') : 'None'}</dd>
      {/if}
      <dt>Selected</dt><dd class:dim={!runtime.selected}>{runtime.selected ?? 'None — choose one in Linux Setup'}</dd>
      <dt>Linux processes</dt><dd class="dim">VLINUX starts none on its own; a distribution runs only if you start it</dd>
    </dl>
    <button class="open" onclick={openSetup}>Open Linux Setup</button>
  </section>

  <section>
    <h3>Roadmap</h3>
    <ol class="road">
      {#each roadmap as m}
        <li class={m.state}><span class="id mono">{m.id}</span><span>{m.name}</span><em>{m.state === 'done' ? 'done' : m.state === 'next' ? 'up next' : 'planned'}</em></li>
      {/each}
    </ol>
  </section>
</div>

<style>
  .sys { padding: 20px 24px 26px; display: grid; gap: 4px; }
  header { display: flex; align-items: center; gap: 16px; padding-bottom: 8px; }
  h2 { margin: 0; font-size: 1.5rem; letter-spacing: 0.18em; background: linear-gradient(120deg, var(--accent), var(--accent-2)); -webkit-background-clip: text; background-clip: text; color: transparent; }
  header p { margin: 2px 0 0; color: var(--muted); font-size: 0.86rem; }
  h3 { margin: 18px 0 8px; font-size: 0.74rem; text-transform: uppercase; letter-spacing: 0.12em; color: var(--muted); }
  section > p { margin: 0; color: var(--muted); line-height: 1.6; font-size: 0.9rem; }
  strong { color: var(--text); }
  dl { display: grid; grid-template-columns: max-content 1fr; gap: 8px 20px; margin: 0; font-size: 0.88rem; }
  dt { color: var(--faint); }
  dd { margin: 0; overflow-wrap: anywhere; }
  .dim { color: var(--muted); }
  .open { margin-top: 12px; padding: 8px 14px; border-radius: 10px; background: rgb(var(--accent-rgb) / 0.16); border: 1px solid rgb(var(--accent-rgb) / 0.4); }
  .open:hover { background: rgb(var(--accent-rgb) / 0.26); }
  .small { font-size: 0.74rem; user-select: all; }
  .road { list-style: none; margin: 0; padding: 0; display: grid; gap: 6px; }
  .road li { display: grid; grid-template-columns: 36px 1fr auto; gap: 10px; align-items: center; padding: 8px 12px; border-radius: 10px; background: rgb(255 255 255 / 0.04); font-size: 0.86rem; }
  .road .id { color: var(--faint); }
  .road em { font-style: normal; font-size: 0.72rem; color: var(--faint); }
  .road li.done { background: rgb(var(--accent-rgb) / 0.1); }
  .road li.done .id, .road li.done em { color: var(--accent); }
  .road li.next em { color: var(--accent-2); }
</style>
