<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { distros, type DistroTab } from '../lib/distros.svelte';
  import { isTauri } from '../lib/ipc';
  import { openTerminalIn } from '../lib/open-terminal';
  import { openFilesAt } from '../lib/open-files';
  import { actionOf, cardFacts, emptyState, inspectedText, preferredNotice, shownState } from '../lib/distros-view';
  import type { ActionId, DistroCard } from '../lib/distros-types';
  import Icon from '../components/Icon.svelte';
  import PackagesPanel from '../components/PackagesPanel.svelte';
  import WslConfigPanel from '../components/WslConfigPanel.svelte';
  import DiagnosticsPanel from '../components/DiagnosticsPanel.svelte';
  import '../lib/distros.css';

  onMount(() => void distros.attach());
  onDestroy(() => distros.detach());

  const o = $derived(distros.overview);
  const empty = $derived(emptyState(o, distros.error, distros.loading));
  const notice = $derived(preferredNotice(o));
  const cur = $derived(distros.current);
  const tabs: { id: DistroTab; label: string }[] = [
    { id: 'overview', label: 'Overview' },
    { id: 'packages', label: 'Packages' },
    { id: 'config', label: 'Configuration' },
    { id: 'diagnostics', label: 'Diagnostics' },
  ];

  function reason(c: DistroCard, id: ActionId): string | undefined {
    if (distros.pending[c.name] && id !== 'refresh') return 'An operation on this distribution is in progress.';
    const a = actionOf(c, id);
    return a.enabled ? undefined : (a.reason ?? 'Not available.');
  }
  const can = (c: DistroCard, id: ActionId) => !distros.pending[c.name] && actionOf(c, id).enabled;
  const showTime = (t: number) => new Date(t * 1000).toLocaleString();
</script>

<div class="dm" data-testid="distros-app">
  <header>
    <div>
      <h2>Distributions</h2>
      <p>Every WSL distribution on this PC. VLINUX only looks, starts or stops a distribution when you press a button here — it never unregisters, resets, deletes or re-installs one, and it never changes the Windows default.</p>
    </div>
    <button class="dm-btn" onclick={() => distros.refresh(true)} disabled={distros.loading} data-testid="distros-refresh">
      <Icon name="rotate" size={15} /> {distros.loading ? 'Checking…' : 'Refresh'}
    </button>
  </header>

  {#if !isTauri()}
    <div class="dm-notice sim" data-testid="distros-sim-banner">SIMULATED: this is the browser preview. The distributions, states and results below are simulated and nothing real is read or changed. Use <code>?wsl=multi</code> to see several distributions.</div>
  {/if}

  {#if notice}
    <div class="dm-notice" class:warn={notice.tone === 'warn'} class:info={notice.tone === 'info'} role={notice.tone === 'warn' ? 'alert' : 'note'} data-testid="preferred-notice"><Icon name={notice.tone === 'warn' ? 'alert' : 'info'} size={15} /> {notice.text}</div>
  {/if}
  {#if distros.error && o}<div class="dm-notice err" role="alert">{distros.error}</div>{/if}

  {#if empty}
    <div class="dm-card empty" data-kind={empty.kind} data-testid="distros-empty" aria-busy={empty.kind === 'loading'}>
      <h3>{empty.title}</h3>
      <p class="dm-muted">{empty.detail}</p>
    </div>
  {:else if o}
    <div class="summary" data-testid="distros-summary">
      <span>Preferred for VLINUX: <strong data-testid="pref-name">{o.preferred_name ?? 'none chosen'}</strong></span>
      <span class="dm-faint">·</span>
      <span>Windows default: <strong data-testid="win-default-name">{o.windows_default ?? 'none'}</strong></span>
      <span class="dm-muted dm-small">These are separate settings. Choosing a preferred distribution never changes the Windows default, and never changes terminals that are already open.</span>
    </div>

    <ul class="cards" aria-label="Distributions">
      {#each o.cards as c (c.name)}
        {@const st = shownState(c, distros.pending[c.name])}
        {@const tone = st.tone}
        <li class="dm-card card" class:selected={cur?.name === c.name} class:internal={c.kind === 'internal'} data-testid="distro-card" data-distro={c.name} data-state={st.key} data-preferred={c.is_preferred}>
          <div class="top">
            <button class="name" onclick={() => distros.select(c.name)} aria-pressed={cur?.name === c.name} title="Show details below">
              <strong>{c.display_name}</strong>
              <span class="dm-faint">{c.name}</span>
            </button>
            <div class="badges">
              <span class="dm-badge" class:good={tone === 'good'} class:busy={tone === 'busy'} class:warn={tone === 'warn'} class:bad={tone === 'bad'} data-testid="state-badge">{st.label}</span>
              {#if c.is_preferred}<span class="dm-badge pref">Preferred</span>{/if}
              {#if c.is_windows_default}<span class="dm-badge win" title="Set in Windows with wsl --set-default. VLINUX does not change it.">Windows default</span>{/if}
              {#if c.kind === 'internal'}<span class="dm-badge">Used by another app</span>{/if}
            </div>
          </div>

          <dl class="dm-facts">
            {#each cardFacts(c) as f}
              <dt>{f.label}</dt><dd class:unknown={!f.known}>{f.value}</dd>
            {/each}
            <dt>Last inspected</dt><dd class:unknown={c.inspected_at === null} title={c.inspected_at ? showTime(c.inspected_at) : undefined}>{inspectedText(c.inspected_at)}</dd>
          </dl>

          {#if c.failure}<div class="dm-notice err" role="alert">{c.failure}</div>{/if}
          {#if distros.notes[c.name]}<div class="dm-small dm-muted" role="status" data-testid="card-note">{distros.notes[c.name]}</div>{/if}
          {#each c.package_notes as n}<div class="dm-small dm-muted">{n}</div>{/each}

          {#if distros.stopPrompt === c.name}
            <div class="dm-confirm" role="alertdialog" aria-label={`Stop ${c.name}`} data-testid="stop-confirm">
              <span><strong>Stop “{c.name}”?</strong> Anything running inside it — including open terminal sessions and unsaved work in Linux programs — will be interrupted. Only this distribution stops; other distributions and Windows are not affected.</span>
              <span class="dm-row">
                <button class="dm-btn danger" onclick={() => distros.confirmStop(c.name)} data-testid="stop-confirm-yes">Stop “{c.name}”</button>
                <button class="dm-btn" onclick={() => (distros.stopPrompt = null)} data-testid="stop-confirm-no">Cancel</button>
              </span>
            </div>
          {/if}
          {#if distros.startPrompt === c.name}
            <div class="dm-confirm soft" role="alertdialog" aria-label={`Start ${c.name} to inspect`} data-testid="inspect-start-confirm">
              <span><strong>“{c.name}” is stopped.</strong> Inspecting it means starting it first. Nothing inside it is changed, and VLINUX will tell you it was started.</span>
              <span class="dm-row">
                <button class="dm-btn primary" onclick={() => distros.inspect(c.name, true)} data-testid="inspect-start-yes">Start and inspect</button>
                <button class="dm-btn" onclick={() => (distros.startPrompt = null)}>Cancel</button>
              </span>
            </div>
          {/if}

          <div class="acts">
            <button class="dm-btn small" data-action="open_terminal" disabled={!can(c, 'open_terminal')} title={reason(c, 'open_terminal')} onclick={() => openTerminalIn(c.name)}><Icon name="terminal" size={14} /> Open terminal</button>
            <button class="dm-btn small" data-action="open_files" disabled={!can(c, 'open_files')} title={reason(c, 'open_files')} onclick={() => openFilesAt(c.name)}><Icon name="folder" size={14} /> Open Files</button>
            <button class="dm-btn small" data-action="start" disabled={!can(c, 'start')} title={reason(c, 'start')} onclick={() => distros.start(c.name)}>{st.key === 'starting' ? 'Starting…' : 'Start'}</button>
            <button class="dm-btn small" data-action="inspect" disabled={!can(c, 'inspect')} title={reason(c, 'inspect')} onclick={() => distros.inspect(c.name, false)}>{st.key === 'inspecting' ? 'Inspecting…' : c.inspected_at ? 'Inspect again' : 'Inspect'}</button>
            {#if st.key === 'inspecting'}<button class="dm-btn small" onclick={() => distros.cancelInspect(c.name)}>Cancel inspection</button>{/if}
            <button class="dm-btn small" data-action="set_preferred" disabled={!can(c, 'set_preferred')} title={reason(c, 'set_preferred')} onclick={() => distros.setPreferred(c.name)}>Set as preferred</button>
            <button class="dm-btn small" data-action="open_config" disabled={!can(c, 'open_config')} title={reason(c, 'open_config')} onclick={() => distros.select(c.name, 'config')}>Configuration</button>
            <button class="dm-btn small" data-action="view_diagnostics" disabled={!can(c, 'view_diagnostics')} title={reason(c, 'view_diagnostics')} onclick={() => distros.select(c.name, 'diagnostics')}>Diagnostics</button>
            <button class="dm-btn small" data-action="stop" disabled={!can(c, 'stop')} title={reason(c, 'stop') ?? 'Stop only this distribution (asks first)'} onclick={() => distros.requestStop(c.name)}>Stop…</button>
          </div>
          {#if c.is_preferred}
            <div class="dm-row"><button class="dm-btn small" onclick={() => distros.setPreferred(null)} data-testid="clear-preferred">Clear preferred</button></div>
          {/if}
        </li>
      {/each}
    </ul>

    {#if cur}
      <section class="detail" aria-label={`Details for ${cur.name}`} data-testid="distro-detail" data-distro={cur.name}>
        <div class="tabs" role="tablist" aria-label="Distribution details">
          {#each tabs as t}
            <button role="tab" id={`dm-tab-${t.id}`} aria-selected={distros.tab === t.id} aria-controls="dm-tabpanel" class:on={distros.tab === t.id} onclick={() => (distros.tab = t.id)} data-tab={t.id}>{t.label}</button>
          {/each}
          <span class="dm-faint dm-small who">{cur.name}</span>
        </div>
        <div id="dm-tabpanel" role="tabpanel" aria-labelledby={`dm-tab-${distros.tab}`}>
          {#if distros.tab === 'overview'}
            <div class="dm-card" data-testid="tab-overview">
              <h3>About this distribution</h3>
              <dl class="dm-facts">
                <dt>Registered as</dt><dd>{cur.name}</dd>
                <dt>Identified from</dt><dd>{cur.identity_source === 'os_release' ? 'its own /etc/os-release (read by inspection)' : 'its registered name only (not inspected yet, so this is a first guess)'}</dd>
                <dt>Origin</dt><dd>{cur.origin === 'installed_by_vlinux' ? 'Installed through VLINUX (with your approval)' : 'Already on this PC'}</dd>
                <dt>Package manager</dt><dd>{cur.packages.availability.state === 'present' ? `${cur.packages.manager_name}${cur.packages.availability.version ? ` ${cur.packages.availability.version}` : ''}` : cur.packages.availability.state === 'absent' ? 'None VLINUX supports was found' : 'Not inspected yet'}</dd>
              </dl>
              <p class="dm-muted dm-small">Inspection is read-only: it reads <code>/etc/os-release</code>, the kernel, the default user and which package managers exist. The result is cached; press Inspect to read it again. A stopped distribution is started only if you allow it, and VLINUX says so afterwards.</p>
              <div class="dm-row">
                <button class="dm-btn" onclick={() => distros.inspect(cur.name, false)} disabled={!can(cur, 'inspect')} title={reason(cur, 'inspect')}>{cur.inspected_at ? 'Inspect again' : 'Inspect'}</button>
              </div>
            </div>
          {:else if distros.tab === 'packages'}
            <PackagesPanel card={cur} />
          {:else if distros.tab === 'config'}
            <WslConfigPanel distro={cur.name} windows />
          {:else}
            <DiagnosticsPanel distro={cur.name} />
          {/if}
        </div>
      </section>
    {/if}
  {/if}
</div>

<style>
  .dm { padding: 18px 22px 26px; display: grid; gap: 14px; align-content: start; overflow: auto; height: 100%; }
  header { display: flex; justify-content: space-between; gap: 16px; align-items: flex-start; }
  h2 { margin: 0; font-size: 1.25rem; }
  h3 { margin: 0; font-size: 0.98rem; }
  header p { margin: 4px 0 0; color: var(--muted); font-size: 0.84rem; line-height: 1.5; max-width: 70ch; }
  p { margin: 6px 0 0; line-height: 1.5; font-size: 0.86rem; }
  .summary { display: flex; flex-wrap: wrap; gap: 6px 10px; align-items: baseline; font-size: 0.86rem; }
  .cards { list-style: none; margin: 0; padding: 0; display: grid; grid-template-columns: repeat(auto-fill, minmax(340px, 1fr)); gap: 12px; }
  .card { display: grid; gap: 10px; align-content: start; }
  .top { display: flex; justify-content: space-between; gap: 10px; align-items: flex-start; flex-wrap: wrap; }
  .name { display: grid; text-align: left; gap: 1px; background: none; border: 0; padding: 0; color: inherit; cursor: pointer; font-size: 0.98rem; }
  .name .dm-faint { font-size: 0.74rem; }
  .badges { display: flex; gap: 6px; flex-wrap: wrap; justify-content: flex-end; }
  .acts { display: flex; gap: 6px; flex-wrap: wrap; }
  .detail { display: grid; gap: 10px; }
  .tabs { display: flex; gap: 4px; align-items: center; border-bottom: 1px solid var(--line); flex-wrap: wrap; }
  .tabs button { padding: 8px 14px; background: none; border: 0; border-bottom: 2px solid transparent; color: var(--muted); font-size: 0.86rem; cursor: pointer; }
  .tabs button:hover { color: inherit; }
  .tabs button.on { color: inherit; border-bottom-color: var(--accent); }
  .tabs .who { margin-left: auto; }
  .empty { text-align: center; padding: 28px; }
  code { overflow-wrap: anywhere; }
  .dm-notice :global(svg) { vertical-align: -3px; margin-right: 6px; }
</style>
