<script lang="ts">
  import * as ipc from '../lib/ipc';
  import { isTauri } from '../lib/ipc';
  import { openTerminalIn } from '../lib/open-terminal';
  import { capabilityText, parsePackageNames, MAX_PACKAGES } from '../lib/distros-view';
  import type { DistroCard, PackageHit, PackagePlan } from '../lib/distros-types';
  import '../lib/distros.css';

  let { card }: { card: DistroCard } = $props();

  let names = $state('');
  let query = $state('');
  let plan = $state<PackagePlan | null>(null);
  let hits = $state<PackageHit[] | null>(null);
  let error = $state<string | null>(null);
  let working = $state(false);
  let copied = $state(false);

  const caps = $derived(card.packages);
  const capRows = $derived([
    { label: 'Detect', c: caps.detect },
    { label: 'Version', c: caps.version },
    { label: 'Search', c: caps.search },
    { label: 'Install', c: caps.install },
    { label: 'Update', c: caps.update },
  ]);
  const parsed = $derived(parsePackageNames(names));
  const running = $derived(card.wsl_state === 'running');

  // Switching distribution clears everything that belonged to the previous one.
  let forName = $state('');
  $effect(() => {
    if (card.name !== forName) {
      forName = card.name;
      plan = null;
      hits = null;
      error = null;
      names = '';
      query = '';
    }
  });

  async function review(op: 'install' | 'update') {
    working = true;
    error = null;
    plan = null;
    try {
      plan = await ipc.distroPkgPlan(card.name, op, op === 'install' ? parsed : []);
    } catch (e) {
      error = String(e);
    } finally {
      working = false;
    }
  }

  async function search() {
    working = true;
    error = null;
    hits = null;
    try {
      hits = await ipc.distroPkgSearch(card.name, query.trim());
    } catch (e) {
      error = String(e);
    } finally {
      working = false;
    }
  }

  async function copy() {
    if (!plan) return;
    try {
      await navigator.clipboard.writeText(plan.commands.join('\n'));
      copied = true;
      setTimeout(() => (copied = false), 1800);
    } catch {
      copied = false;
    }
  }
</script>

<div class="pk" data-testid="packages-panel">
  <section class="dm-card">
    <h3>Package manager <span class="dm-faint">· {card.name}</span></h3>
    {#if caps.availability.state === 'not_checked'}
      <div class="dm-notice info" data-testid="pkg-not-inspected">The package manager has not been looked for yet. VLINUX never assumes it from the distribution's name — inspect the distribution first (Overview tab).</div>
    {:else if caps.availability.state === 'absent'}
      <div class="dm-notice warn" data-testid="pkg-unsupported">No package manager VLINUX supports (APT, pacman or DNF) was found in this distribution, so package features are off for it.</div>
    {:else}
      <p class="dm-muted dm-small"><strong>{caps.manager_name}</strong>{caps.availability.version ? ` ${caps.availability.version}` : ''} was found inside the distribution.</p>
    {/if}
    <table class="dm-table" aria-label="Package capabilities">
      <tbody>
        {#each capRows as r (r.label)}
          <tr data-cap={r.label.toLowerCase()} data-support={r.c.support}><th scope="row">{r.label}</th><td>{capabilityText(r.c)}</td><td class="dm-muted">{r.c.note}</td></tr>
        {/each}
      </tbody>
    </table>
    <p class="dm-muted dm-small">VLINUX does not install or upgrade packages itself: that needs administrator rights inside the distribution, and VLINUX never escalates privileges. It shows you the exact command; you run it in the Terminal and type your password there.</p>
  </section>

  {#if caps.search.support === 'available'}
    <section class="dm-card">
      <h4 class="dm-h">Search (read-only)</h4>
      <form class="dm-row" onsubmit={(e) => { e.preventDefault(); void search(); }}>
        <input class="dm-input" bind:value={query} placeholder="package name" maxlength="64" aria-label="Search for a package" spellcheck="false" disabled={!running} />
        <button class="dm-btn" type="submit" disabled={working || !running || query.trim() === ''} data-testid="pkg-search">Search</button>
        {#if !running}<span class="dm-muted dm-small">“{card.name}” is stopped. Searching never starts it — start it first.</span>{/if}
      </form>
      {#if hits}
        {#if hits.length === 0}<p class="dm-muted dm-small">No packages matched.</p>{:else}
          <ul class="hits" data-testid="pkg-hits">{#each hits as h}<li><strong>{h.name}</strong>{h.version ? ` ${h.version}` : ''} <span class="dm-muted">— {h.description}</span></li>{/each}</ul>
        {/if}
      {/if}
    </section>
  {/if}

  {#if caps.install.support === 'plan_only'}
    <section class="dm-card">
      <h4 class="dm-h">Prepare a command</h4>
      <form class="dm-row" onsubmit={(e) => { e.preventDefault(); void review('install'); }}>
        <input class="dm-input" bind:value={names} placeholder="packages to install, e.g. neovim git" aria-label="Packages to install" spellcheck="false" style="flex: 1; min-width: 14ch" />
        <button class="dm-btn" type="submit" disabled={working || parsed.length === 0} data-testid="pkg-review-install">Review install</button>
        <button class="dm-btn" type="button" disabled={working} onclick={() => review('update')} data-testid="pkg-review-update">Review update</button>
      </form>
      <p class="dm-muted dm-small">Up to {MAX_PACKAGES} package names (letters, digits and <code>+ . _ -</code>). Nothing runs when you press these buttons.</p>
    </section>
  {/if}

  {#if error}<div class="dm-notice err" role="alert" data-testid="pkg-error">{error}</div>{/if}

  {#if plan}
    <section class="dm-card" aria-label="Command to review" data-testid="pkg-plan">
      <h4 class="dm-h">{plan.op === 'install' ? 'Install' : 'Update'} with {plan.manager_name} — not run</h4>
      {#if plan.packages.length}<p class="dm-small">Packages: <strong>{plan.packages.join(', ')}</strong></p>{/if}
      <pre class="dm-code" data-testid="pkg-commands">{plan.commands.join('\n')}</pre>
      {#each plan.notes as n}<p class="dm-muted dm-small">{n}</p>{/each}
      <div class="dm-row" style="margin-top: 8px">
        <button class="dm-btn small" onclick={copy}>{copied ? 'Copied' : 'Copy command'}</button>
        <button class="dm-btn small primary" onclick={() => openTerminalIn(card.name)} data-testid="pkg-open-terminal">Open terminal in “{card.name}”</button>
      </div>
    </section>
  {/if}
  {#if !isTauri()}<div class="dm-notice sim">SIMULATED: the browser preview shows example search results; nothing real is queried.</div>{/if}
</div>

<style>
  .pk { display: grid; gap: 12px; align-content: start; }
  h3 { margin: 0; font-size: 0.98rem; }
  p { margin: 6px 0 0; line-height: 1.5; }
  .hits { list-style: none; margin: 8px 0 0; padding: 0; display: grid; gap: 4px; font-size: 0.84rem; }
  .dm-table { margin-top: 8px; }
  code { overflow-wrap: anywhere; }
</style>
