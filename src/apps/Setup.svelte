<script lang="ts">
  import { onMount } from 'svelte';
  import { runtime } from '../lib/runtime.svelte';
  import { openTerminalIn } from '../lib/open-terminal';
  import { statusHeadline, type Distro } from '../lib/runtime-types';
  import Icon from '../components/Icon.svelte';

  let approve = $state(false);
  let showCatalog = $state(false);
  let confirmStop = $state<string | null>(null);
  let dismissedOffer = $state(false);

  const r = $derived(runtime.report);
  const status = $derived(r?.status ?? null);
  const regular = $derived((r?.distros ?? []).filter((d) => d.kind === 'regular'));
  const internal = $derived((r?.distros ?? []).filter((d) => d.kind === 'internal'));
  const canInstallDistro = $derived(status?.kind === 'ready' || status?.kind === 'no_distributions');
  const offer = $derived(
    status?.kind === 'ready' && runtime.suggested && !runtime.selected && !dismissedOffer && !runtime.job ? runtime.suggested : null,
  );

  onMount(() => {
    if (!runtime.report) void runtime.refresh(false);
  });

  // A fresh review always starts unapproved.
  $effect(() => {
    void runtime.plan?.plan_id;
    approve = false;
  });

  function toggleCatalog() {
    showCatalog = !showCatalog;
    if (showCatalog && !runtime.catalog) void runtime.loadCatalog(false);
  }

  const stateLabel = (d: Distro) => ({ running: 'Running', stopped: 'Stopped', installing: 'Installing', uninstalling: 'Uninstalling', converting: 'Converting', unknown: 'State unknown' })[d.state];
  const when = (t: number) => new Date(t * 1000).toLocaleTimeString();
</script>

<div class="setup">
  <header>
    <div>
      <h2>Linux Setup</h2>
      <p>Choose the Linux distribution VLINUX should use. Nothing is installed, started or changed unless you ask for it.</p>
    </div>
    <button class="btn" onclick={() => runtime.refresh(true)} disabled={runtime.loading} aria-label="Check again">
      <Icon name="rotate" size={15} /> {runtime.loading ? 'Checking…' : 'Check again'}
    </button>
  </header>

  {#if runtime.error}
    <div class="notice err" role="alert"><Icon name="alert" size={16} /> {runtime.error}</div>
  {/if}
  {#each r?.warnings ?? [] as w}
    <div class="notice" class:sim={w.startsWith('SIMULATED')} class:warn={!w.startsWith('SIMULATED')}><Icon name="info" size={16} /> {w}</div>
  {/each}

  {#if !r}
    <div class="card" aria-busy="true"><p class="muted">{runtime.loading ? 'Checking WSL… this is read-only and does not start any distribution.' : 'Not checked yet.'}</p></div>
  {:else if status}
    <section class="card status" data-status={status.kind} aria-label="WSL status">
      <div class="head">
        <span class="dot" class:ok={status.kind === 'ready'} class:bad={status.kind === 'error' || status.kind === 'feature_problem' || status.kind === 'unsupported_host'}></span>
        <h3>{statusHeadline(status)}</h3>
      </div>

      {#if status.kind === 'unsupported_host'}
        <p>{status.reason}</p>
      {:else if status.kind === 'not_installed'}
        <p>The Windows Subsystem for Linux (WSL) is not installed on this PC. VLINUX can install it with Windows' official installer, but only after you review and approve it. Windows may ask for administrator permission and a restart.</p>
        <button class="btn primary" onclick={() => runtime.review('wsl_platform', null)} disabled={runtime.planLoading || runtime.installing}>Review WSL setup…</button>
      {:else if status.kind === 'restart_required'}
        <p>Windows has a restart pending (usually after enabling WSL). Save your work and restart Windows, then reopen VLINUX. VLINUX will not restart your PC for you.</p>
      {:else if status.kind === 'feature_problem'}
        <p>{status.hint}</p>
        {#if status.code}<p class="muted">Error code <code class="mono">{status.code}</code></p>{/if}
        <button class="btn" onclick={() => runtime.review('wsl_platform', null)} disabled={runtime.planLoading || runtime.installing}>Review WSL repair…</button>
      {:else if status.kind === 'error'}
        <p>{status.message}</p>
      {:else if status.kind === 'no_distributions'}
        <p>WSL works, but no Linux distribution is registered. Pick one from the official list below to install it. VLINUX does not assume which distribution you want.</p>
      {/if}

      {#if status.kind === 'ready' || status.kind === 'no_distributions'}
        <dl class="facts">
          <dt>WSL</dt><dd>{r.wsl_version ?? 'version unknown'}</dd>
          <dt>Default version</dt><dd>WSL {r.default_wsl_version ?? '?'}</dd>
          <dt>WSL 2</dt><dd>{r.wsl2_available === true ? 'Available' : r.wsl2_available === false ? 'Not available' : 'Not confirmed (shown once a WSL 2 distribution runs)'}</dd>
          {#if r.kernel_version}<dt>Kernel</dt><dd class="mono">{r.kernel_version}</dd>{/if}
        </dl>
      {/if}
      <p class="muted small">Checked {when(r.detected_at)} · read-only check; no distribution was started to produce this.</p>
    </section>

    {#if offer}
      <section class="card offer" aria-label="Existing distribution found">
        <h3>Use your existing {offer.display_label}?</h3>
        <p>VLINUX found <strong>{offer.display_label}</strong> already installed as <code class="mono">{offer.name}</code>{offer.adapter.package_manager !== 'unknown' ? ` (${offer.adapter.package_manager_name})` : ''}. You can use it as it is — VLINUX will not reinstall, reset, rename or modify it.</p>
        <div class="acts">
          <button class="btn primary" onclick={() => runtime.select(offer.name)}>Use {offer.name}</button>
          <button class="btn" onclick={() => (dismissedOffer = true)}>Not now</button>
        </div>
      </section>
    {/if}

    {#if regular.length || internal.length}
      <section aria-label="Installed distributions">
        <h3 class="sec">Installed distributions</h3>
        <ul class="distros">
          {#each regular as d (d.name)}
            <li class="distro" data-name={d.name}>
              <div class="main">
                <div class="title">
                  <strong>{d.display_label}</strong>
                  <code class="mono dim">{d.name}</code>
                </div>
                <div class="badges">
                  <span class="badge" class:inst={d.origin === 'installed_by_vlinux'}>{d.origin === 'installed_by_vlinux' ? 'Installed by VLINUX' : 'Existing'}</span>
                  <span class="badge" class:run={d.state === 'running'}>{stateLabel(d)}</span>
                  {#if runtime.selected === d.name}<span class="badge sel">Selected</span>{/if}
                  {#if d.is_default}<span class="badge">Default in WSL</span>{/if}
                  {#if d.wsl_version !== 2}<span class="badge warn" title="VLINUX targets WSL 2. This distribution uses WSL {d.wsl_version}.">WSL {d.wsl_version}</span>{/if}
                  {#if d.adapter.package_manager !== 'unknown'}<span class="badge">{d.adapter.package_manager_name}</span>{:else}<span class="badge" title="No VLINUX adapter for this distribution yet">Generic</span>{/if}
                </div>
                {#if d.identity_source === 'registered_name'}
                  <small class="muted">Identified from its name only; VLINUX does not start a distribution just to look inside it.</small>
                {/if}
              </div>
              <div class="row-acts">
                <button class="btn" onclick={() => openTerminalIn(d.name)} disabled={d.state === 'installing' || d.state === 'uninstalling' || d.state === 'converting'} title="Open a terminal in {d.name} (starts it if it is stopped)">
                  <Icon name="terminal" size={15} /> Open terminal
                </button>
                {#if runtime.selected === d.name}
                  <button class="btn" onclick={() => runtime.select(null)}>Stop using</button>
                {:else}
                  <button class="btn" onclick={() => runtime.select(d.name)} disabled={d.wsl_version !== 2} title={d.wsl_version !== 2 ? 'VLINUX needs a WSL 2 distribution' : ''}>Use with VLINUX</button>
                {/if}
                {#if d.state === 'stopped'}
                  <button class="btn" onclick={() => runtime.start(d.name)} disabled={runtime.busyDistro !== null}>{runtime.busyDistro === d.name ? 'Starting…' : 'Start'}</button>
                {:else if d.state === 'running'}
                  {#if confirmStop === d.name}
                    <span class="confirm">Stop {d.name}? Running programs in it end.
                      <button class="btn danger" onclick={() => { const n = d.name; confirmStop = null; void runtime.stop(n); }}>Stop it</button>
                      <button class="btn" onclick={() => (confirmStop = null)}>Keep running</button>
                    </span>
                  {:else}
                    <button class="btn" onclick={() => (confirmStop = d.name)} disabled={runtime.busyDistro !== null}>{runtime.busyDistro === d.name ? 'Stopping…' : 'Stop…'}</button>
                  {/if}
                {/if}
              </div>
            </li>
          {/each}
          {#each internal as d (d.name)}
            <li class="distro internal" data-name={d.name}>
              <div class="main">
                <div class="title"><strong>{d.name}</strong></div>
                <div class="badges"><span class="badge">Managed by another application</span><span class="badge">{stateLabel(d)}</span></div>
                <small class="muted">VLINUX never queries, starts, stops or selects this one.</small>
              </div>
            </li>
          {/each}
        </ul>
      </section>
    {/if}

    {#if canInstallDistro}
      <section aria-label="Install a distribution">
        <h3 class="sec">Install a distribution <span class="opt">optional</span></h3>
        <p class="muted">{regular.length ? 'You already have a distribution, so you do not need another. ' : ''}Installing uses Microsoft's official WSL mechanism and official images. It always asks for your approval first.</p>
        <button class="btn" onclick={toggleCatalog} aria-expanded={showCatalog}>{showCatalog ? 'Hide official list' : 'Show official list…'}</button>
        {#if showCatalog}
          {#if runtime.catalogLoading}
            <p class="muted" aria-busy="true">Fetching the official list (needs internet)…</p>
          {:else if runtime.catalog?.error}
            <div class="notice warn"><Icon name="alert" size={16} /> {runtime.catalog.error} <button class="btn" onclick={() => runtime.loadCatalog(true)}>Try again</button></div>
          {:else if runtime.catalog}
            <ul class="catalog">
              {#each runtime.catalog.entries as e (e.wsl_name)}
                <li data-name={e.wsl_name}>
                  <span><strong>{e.friendly_name}</strong> <code class="mono dim">{e.wsl_name}</code></span>
                  <span class="cat-acts">
                    {#if e.already_installed}
                      <span class="badge">Installed</span>
                    {:else if !e.supported}
                      <span class="badge" title="VLINUX has no adapter for this distribution yet">Not supported yet</span>
                    {:else}
                      <button class="btn" onclick={() => runtime.review('distribution', e.wsl_name)} disabled={runtime.planLoading || runtime.installing}>Review install…</button>
                    {/if}
                  </span>
                </li>
              {/each}
            </ul>
          {/if}
        {/if}
      </section>
    {/if}
  {/if}

  {#if runtime.plan}
    {@const p = runtime.plan}
    <section class="card review" aria-label="Review installation">
      <h3>Review: {p.display_name}</h3>
      <ol class="steps">
        {#each p.steps as s}<li>{s}</li>{/each}
      </ol>
      <dl class="facts">
        <dt>Command</dt><dd><code class="mono">{p.command_preview}</code></dd>
        <dt>Administrator</dt><dd>{p.requires_admin === 'may_prompt' ? 'Windows may show a UAC prompt. VLINUX itself does not run elevated.' : 'Not required'}</dd>
        <dt>Restart</dt><dd>{p.restart_possible ? 'Windows may ask you to restart afterwards. VLINUX will not restart your PC.' : 'Not expected'}</dd>
        <dt>Internet</dt><dd>{p.needs_internet ? 'Required' : 'Not required'}</dd>
        <dt>Left untouched</dt><dd>{p.existing_untouched.length ? p.existing_untouched.join(', ') : 'No existing distributions'}</dd>
      </dl>
      {#if p.blockers.length}
        <div class="notice err" role="alert"><Icon name="alert" size={16} /><ul>{#each p.blockers as b}<li>{b}</li>{/each}</ul></div>
      {/if}
      <label class="consent">
        <input type="checkbox" bind:checked={approve} disabled={p.blockers.length > 0} />
        <span>I have read this and approve running the command above.</span>
      </label>
      <div class="acts">
        <button class="btn" onclick={() => runtime.discardPlan()}>Cancel</button>
        <button class="btn primary" disabled={!approve || p.blockers.length > 0 || runtime.installing} onclick={() => runtime.startPlan(approve)}>
          {p.kind === 'wsl_platform' ? 'Install WSL' : `Install ${p.display_name}`}
        </button>
      </div>
    </section>
  {/if}

  {#if runtime.job}
    {@const j = runtime.job}
    <section class="card job" aria-label="Installation progress" role="status" aria-live="polite" data-outcome={j.outcome?.kind ?? 'running'}>
      <h3>{j.outcome ? 'Installation finished' : `Installing ${j.title}`}</h3>
      {#if !j.outcome}
        <progress max="100" value={j.percent ?? undefined} aria-label="Installation progress"></progress>
        <p class="muted">{j.message}</p>
        <div class="acts">
          <button class="btn" onclick={() => runtime.cancelJob()} disabled={j.cancelRequested || j.id === ''}>{j.cancelRequested ? 'Cancelling…' : 'Cancel installation'}</button>
        </div>
      {:else if j.outcome.kind === 'succeeded'}
        <p>{j.finalMessage}</p>
        <p class="muted">It now appears in the list above. The first time you open it, the distribution may ask you to create a user account.</p>
      {:else if j.outcome.kind === 'restart_required'}
        <p><strong>Restart Windows to finish.</strong> {j.outcome.message}</p>
      {:else if j.outcome.kind === 'failed'}
        <p class="bad-text">{j.outcome.message}</p>
        <p>{j.outcome.hint}</p>
        {#if j.finalMessage && j.finalMessage !== j.outcome.message}<p class="muted">{j.finalMessage}</p>{/if}
        {#if j.outcome.exit_code !== null}<p class="muted small">Exit code {j.outcome.exit_code}</p>{/if}
      {:else if j.outcome.kind === 'cancelled'}
        <p>{j.finalMessage}</p>
        <p class="muted">Existing distributions were not changed.</p>
      {:else}
        <p>{j.finalMessage}</p>
      {/if}
      {#if j.outcome}
        <div class="acts"><button class="btn" onclick={() => runtime.dismissJob()}>Dismiss</button></div>
      {/if}
    </section>
  {/if}
</div>

<style>
  .setup { padding: 18px 22px 26px; display: grid; gap: 14px; align-content: start; overflow: auto; height: 100%; }
  header { display: flex; justify-content: space-between; gap: 16px; align-items: flex-start; }
  h2 { margin: 0; font-size: 1.25rem; }
  header p { margin: 4px 0 0; color: var(--muted); font-size: 0.84rem; line-height: 1.5; max-width: 60ch; }
  h3 { margin: 0; font-size: 1rem; }
  h3.sec { font-size: 0.74rem; text-transform: uppercase; letter-spacing: 0.12em; color: var(--muted); font-weight: 600; margin: 6px 0 8px; }
  .opt { text-transform: none; letter-spacing: 0; color: var(--faint); font-weight: 400; margin-left: 6px; }
  p { margin: 8px 0 0; line-height: 1.55; font-size: 0.88rem; }
  .muted { color: var(--muted); }
  .small { font-size: 0.76rem; }
  .dim { color: var(--faint); }
  .bad-text { color: #ffb3bd; }

  .card { padding: 14px 16px; border-radius: 14px; border: 1px solid var(--line); background: rgb(255 255 255 / 0.04); }
  .card.offer { border-color: rgb(var(--accent-rgb) / 0.55); background: rgb(var(--accent-rgb) / 0.1); }
  .card.review { border-color: var(--line-strong); }
  .head { display: flex; align-items: center; gap: 10px; }
  .dot { width: 10px; height: 10px; border-radius: 50%; background: var(--faint); flex: none; }
  .dot.ok { background: #3ddc97; }
  .dot.bad { background: #ff6b81; }
  .facts { display: grid; grid-template-columns: max-content 1fr; gap: 6px 18px; margin: 10px 0 0; font-size: 0.84rem; }
  .facts dt { color: var(--faint); }
  .facts dd { margin: 0; overflow-wrap: anywhere; }
  .steps { margin: 10px 0 0; padding-left: 20px; display: grid; gap: 6px; font-size: 0.86rem; line-height: 1.5; }

  .distros, .catalog { list-style: none; margin: 0; padding: 0; display: grid; gap: 8px; }
  .distro { display: flex; justify-content: space-between; gap: 14px; align-items: center; padding: 12px 14px; border-radius: 12px; border: 1px solid var(--line); background: rgb(255 255 255 / 0.04); flex-wrap: wrap; }
  .distro.internal { opacity: 0.75; }
  .main { display: grid; gap: 6px; min-width: 0; }
  .title { display: flex; gap: 10px; align-items: baseline; flex-wrap: wrap; }
  .badges { display: flex; gap: 6px; flex-wrap: wrap; }
  .badge { padding: 2px 9px; border-radius: 999px; font-size: 0.7rem; background: rgb(255 255 255 / 0.08); border: 1px solid var(--line); color: var(--muted); }
  .badge.inst { background: rgb(var(--accent-2-rgb, 94 224 255) / 0.15); color: var(--accent-2); }
  .badge.run { color: #3ddc97; border-color: rgb(61 220 151 / 0.4); }
  .badge.sel { background: var(--accent); color: #0a0a12; border-color: transparent; font-weight: 600; }
  .badge.warn { color: #ffb454; border-color: rgb(255 180 84 / 0.5); }
  .row-acts { display: flex; gap: 8px; flex-wrap: wrap; align-items: center; }
  .confirm { display: inline-flex; gap: 8px; align-items: center; flex-wrap: wrap; font-size: 0.8rem; color: #ffb3bd; }
  .catalog li { display: flex; justify-content: space-between; align-items: center; gap: 12px; padding: 9px 12px; border-radius: 10px; background: rgb(255 255 255 / 0.04); font-size: 0.86rem; }
  .catalog { margin-top: 10px; }

  .consent { display: flex; gap: 10px; align-items: center; margin-top: 14px; font-size: 0.88rem; cursor: pointer; }
  .consent input { width: 16px; height: 16px; accent-color: var(--accent); }
  progress { width: 100%; height: 8px; accent-color: var(--accent); margin-top: 10px; }

  .acts { display: flex; gap: 8px; justify-content: flex-end; margin-top: 12px; flex-wrap: wrap; }
  .btn { display: inline-flex; align-items: center; gap: 8px; padding: 7px 13px; border-radius: 10px; background: rgb(255 255 255 / 0.09); border: 1px solid var(--line); font-size: 0.84rem; }
  .btn:hover:not(:disabled) { background: rgb(255 255 255 / 0.15); }
  .btn:disabled { opacity: 0.45; cursor: not-allowed; }
  .btn.primary { background: var(--accent); color: #0a0a12; border-color: transparent; font-weight: 600; }
  .btn.primary:hover:not(:disabled) { filter: brightness(1.1); background: var(--accent); }
  .btn.danger { background: rgb(255 90 110 / 0.2); border-color: rgb(255 110 130 / 0.6); color: #ffb3bd; }

  .notice { display: block; padding: 10px 14px; border-radius: 12px; font-size: 0.82rem; line-height: 1.45; }
  .notice :global(svg) { vertical-align: -3px; margin-right: 6px; }
  .notice ul { margin: 0; padding-left: 16px; display: inline-block; vertical-align: top; }
  .notice.warn { background: rgb(255 180 84 / 0.12); border: 1px solid rgb(255 180 84 / 0.4); }
  .notice.err { background: rgb(255 90 110 / 0.12); border: 1px solid rgb(255 110 130 / 0.5); }
  .notice.sim { background: rgb(94 224 255 / 0.1); border: 1px dashed rgb(94 224 255 / 0.5); }
  code { overflow-wrap: anywhere; }
</style>
