<script lang="ts">
  import { distros } from '../lib/distros.svelte';
  import { isTauri } from '../lib/ipc';
  import { DIAG_CATEGORIES, EVIDENCE_LABEL, OUTCOME_LABEL, groupResults, reportHeadline, runPlanText } from '../lib/distros-view';
  import type { DiagCategory } from '../lib/distros-types';
  import '../lib/distros.css';

  let { distro }: { distro: string | null } = $props();

  let allowStart = $state(false);
  let allowWrite = $state(false);
  let cats = $state<Record<DiagCategory, boolean>>({ runtime: true, terminal: true, filesystem: true, shared_folders: true, desktop: true });
  let copied = $state(false);

  const simulated = !isTauri();
  const plan = $derived(runPlanText(allowStart, allowWrite, simulated));
  const card = $derived(distros.cards.find((c) => c.name === distro) ?? null);
  const shownResults = $derived(distros.diagRunning ? distros.diagProgress : (distros.diagReport?.results ?? []));
  const groups = $derived(groupResults(shownResults));
  const selectedCats = $derived(DIAG_CATEGORIES.filter((c) => cats[c.id]).map((c) => c.id));

  function run() {
    const all = selectedCats.length === DIAG_CATEGORIES.length;
    void distros.runDiagnostics(distro, allowStart, allowWrite, all ? null : selectedCats);
  }

  async function copy() {
    const t = distros.diagReport?.text;
    if (!t) return;
    try {
      await navigator.clipboard.writeText(t);
      copied = true;
      setTimeout(() => (copied = false), 1800);
    } catch {
      copied = false;
    }
  }
</script>

<div class="diag" data-testid="diag-panel">
  <section class="dm-card">
    <h3>Compatibility diagnostics {#if distro}<span class="dm-faint">· {distro}</span>{/if}</h3>
    <p class="dm-muted dm-small">Checks that the Terminal, Files and shared folders really work with this distribution. Each result says what was checked, whether anything was changed, and what to do about a problem. Checks that did not run are shown as such — never as a pass.</p>

    {#if simulated}
      <div class="dm-notice sim" data-testid="diag-mode-banner">SIMULATED: this is the browser preview. These diagnostics contact no real WSL, so they say nothing about your PC. Real results appear only in the desktop app.</div>
    {:else}
      <div class="dm-notice info" data-testid="diag-mode-banner">Real run: checks run against your actual WSL and this distribution.</div>
    {/if}

    <fieldset class="cats">
      <legend class="dm-h">Categories</legend>
      {#each DIAG_CATEGORIES as c}
        <label class="dm-check"><input type="checkbox" bind:checked={cats[c.id]} /> <span>{c.label}</span></label>
      {/each}
    </fieldset>

    <div class="perm">
      <label class="dm-check">
        <input type="checkbox" bind:checked={allowWrite} data-testid="diag-allow-write" />
        <span><strong>Allow creating temporary files.</strong> Needed for the terminal, create/rename/copy/move, link, and file-name checks. Almost everything is created in one temporary folder inside the distribution (<code>/tmp/vlinux-diag-…</code>). The home-folder test creates one hidden, uniquely named folder in your Linux home (<code>~/.vlinux-validate-…</code>), works only inside it and removes only it. Nothing else is touched.</span>
      </label>
      <label class="dm-check">
        <input type="checkbox" bind:checked={allowStart} data-testid="diag-allow-start" />
        <span><strong>Allow starting “{distro ?? 'the distribution'}” if it is stopped.</strong> Without this, checks that need a running distribution are marked “Not tested”.</span>
      </label>
    </div>

    <div class="dm-notice info" aria-label="What this run will do" data-testid="diag-plan">
      <strong>This run:</strong>
      <ul>{#each plan as l}<li>{l}</li>{/each}</ul>
    </div>

    <div class="dm-row">
      <button class="dm-btn primary" onclick={run} disabled={distros.diagRunning || !distro || selectedCats.length === 0 || (card?.busy ?? null) !== null} data-testid="diag-run">
        {distros.diagRunning ? 'Running…' : 'Run diagnostics'}
      </button>
      {#if distros.diagRunning}<button class="dm-btn" onclick={() => distros.cancelDiagnostics()} data-testid="diag-cancel">Cancel</button>{/if}
      {#if !distro}<span class="dm-muted dm-small">Choose a distribution first.</span>{/if}
    </div>
  </section>

  {#if distros.diagError}<div class="dm-notice err" role="alert">{distros.diagError}</div>{/if}

  {#if distros.diagRunning && shownResults.length === 0}
    <p class="dm-muted" aria-busy="true">Starting…</p>
  {/if}

  {#if distros.diagReport || shownResults.length}
    <section class="dm-card" aria-label="Diagnostics results" data-testid="diag-results">
      {#if distros.diagReport && !distros.diagRunning}
        {@const r = distros.diagReport}
        <div class="dm-notice" class:sim={r.mode === 'simulated'} class:info={r.mode !== 'simulated'} data-testid="diag-headline">{reportHeadline(r)}{r.cancelled ? ' · cancelled: the rest was not run' : ''}</div>
      {:else}
        <p class="dm-muted" aria-live="polite">Running… {shownResults.length} check{shownResults.length === 1 ? '' : 's'} done.</p>
      {/if}

      {#each groups as g (g.category)}
        <h4 class="dm-h cat">{g.label}</h4>
        <ul class="rows">
          {#each g.rows as r (r.id)}
            <li class="res" data-outcome={r.outcome} data-evidence={r.evidence} data-check={r.id}>
              <div class="line">
                <span class="dm-badge" class:good={r.outcome === 'passed'} class:bad={r.outcome === 'failed'} class:warn={r.outcome === 'unsupported' || r.outcome === 'skipped'}>{OUTCOME_LABEL[r.outcome]}</span>
                <strong>{r.title}</strong>
                <span class="dm-badge" class:sim={r.evidence === 'simulated'}>{EVIDENCE_LABEL[r.evidence]}</span>
                {#if r.modified}<span class="dm-badge warn" title="This check created something (inside its own temporary folder).">Modified something</span>{/if}
              </div>
              <div class="dm-muted dm-small">Checked: {r.checked}</div>
              <div class="dm-small">{r.message}</div>
              {#if r.guidance}<div class="dm-small guid">What to do: {r.guidance}</div>{/if}
            </li>
          {/each}
        </ul>
      {/each}

      {#if distros.diagReport && !distros.diagRunning}
        <div class="dm-row" style="margin-top: 12px">
          <button class="dm-btn" onclick={copy} data-testid="diag-copy">{copied ? 'Copied' : 'Copy report'}</button>
          <span class="dm-muted dm-small">The report hides user names and holds no passwords or file contents.</span>
        </div>
        <details>
          <summary class="dm-small">Show report text</summary>
          <pre class="dm-code" data-testid="diag-text">{distros.diagReport.text}</pre>
        </details>
      {/if}
    </section>
  {/if}
</div>

<style>
  .diag { display: grid; gap: 12px; align-content: start; }
  h3 { margin: 0; font-size: 0.98rem; }
  p { margin: 6px 0 0; line-height: 1.5; }
  .cats { display: flex; gap: 6px 16px; flex-wrap: wrap; border: 0; padding: 0; margin: 12px 0 0; }
  .cats legend { padding: 0; margin-bottom: 6px; }
  .perm { display: grid; gap: 10px; margin: 12px 0; }
  .dm-notice ul { margin: 4px 0 0; padding-left: 18px; }
  .rows { list-style: none; padding: 0; margin: 6px 0 0; display: grid; gap: 8px; }
  .res { display: grid; gap: 3px; padding: 8px 10px; border-radius: 10px; background: rgb(255 255 255 / 0.04); border-left: 3px solid var(--line); }
  .res[data-outcome='passed'] { border-left-color: #3ddc97; }
  .res[data-outcome='failed'] { border-left-color: #ff6b81; }
  .res[data-outcome='unsupported'], .res[data-outcome='skipped'] { border-left-color: #ffb454; }
  .line { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; font-size: 0.86rem; }
  .guid { color: #ffd9a0; }
  .cat { margin-top: 12px; }
  details { margin-top: 8px; }
  summary { cursor: pointer; color: var(--muted); }
  code { overflow-wrap: anywhere; }
</style>
