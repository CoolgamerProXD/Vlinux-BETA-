<script lang="ts">
  import { untrack } from 'svelte';
  import * as ipc from '../lib/ipc';
  import { isTauri } from '../lib/ipc';
  import { distros } from '../lib/distros.svelte';
  import { draftToChanges } from '../lib/distros-view';
  import type { ApplyOutcome, BackupInfo, ConfPlan, ConfView, SettingChange, SettingValue, WindowsConfig } from '../lib/distros-types';
  import '../lib/distros.css';

  let { distro, windows = false }: { distro: string | null; windows?: boolean } = $props();

  type Read = { kind: 'view'; view: ConfView; wsl_version: number } | { kind: 'needs_start'; distro: string } | null;

  let read = $state<Read>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let draft = $state<Record<string, string | null | undefined>>({});
  let plan = $state<ConfPlan | null>(null);
  let planChanges = $state<SettingChange[]>([]);
  let reviewed = $state(false);
  let working = $state(false);
  let outcome = $state<ApplyOutcome | null>(null);
  let backups = $state<BackupInfo[]>([]);
  let restoreId = $state<string | null>(null);
  let restoreForce = $state(false);
  let restoreMsg = $state<string | null>(null);
  let win = $state<WindowsConfig | null>(null);
  let winError = $state<string | null>(null);
  let copied = $state(false);
  let token = 0;

  const view = $derived(read?.kind === 'view' ? read.view : null);
  const changes = $derived(view ? draftToChanges(view, draft) : []);
  const busyOp = $derived(distro ? (distros.cards.find((c) => c.name === distro)?.busy ?? null) : null);

  async function load() {
    const mine = ++token;
    plan = null;
    outcome = null;
    draft = {};
    error = null;
    restoreMsg = null;
    if (!distro) {
      read = null;
      return;
    }
    loading = true;
    try {
      const r = await ipc.wslconfRead(distro);
      if (mine !== token) return;
      read = r;
      backups = await ipc.wslconfBackups(distro);
    } catch (e) {
      if (mine === token) {
        read = null;
        error = String(e);
      }
    } finally {
      if (mine === token) loading = false;
    }
  }

  // Reads when the distribution changes (opening the panel is the user's action). It never starts
  // a distribution: a stopped one is reported as such.
  $effect(() => {
    void distro;
    untrack(() => void load());
  });

  $effect(() => {
    if (!windows) return;
    untrack(() => {
      ipc.wslconfigWindows().then((w) => (win = w)).catch((e) => (winError = String(e)));
    });
  });

  async function startAndRead() {
    if (!distro) return;
    await distros.start(distro);
    await load();
  }

  function setDraft(s: SettingValue, v: string) {
    draft = { ...draft, [s.id]: v === '' ? null : v };
    plan = null;
    reviewed = false;
    outcome = null;
  }
  const shown = (s: SettingValue) => (s.id in draft ? (draft[s.id] ?? '') : (s.value ?? ''));

  async function review() {
    if (!distro || !view) return;
    error = null;
    outcome = null;
    working = true;
    try {
      planChanges = changes;
      plan = await ipc.wslconfPreview(distro, planChanges);
      reviewed = false;
    } catch (e) {
      plan = null;
      error = String(e);
    } finally {
      working = false;
    }
  }

  async function apply() {
    if (!distro || !plan || !reviewed) return;
    working = true;
    error = null;
    try {
      outcome = await ipc.wslconfApply(distro, planChanges, plan.based_on, true);
      if (outcome.kind === 'applied') {
        plan = null;
        reviewed = false;
        draft = {};
        read = await ipc.wslconfRead(distro);
        backups = await ipc.wslconfBackups(distro);
      }
    } catch (e) {
      error = String(e);
    } finally {
      working = false;
    }
  }

  async function restore(id: string, force: boolean) {
    if (!distro) return;
    working = true;
    restoreMsg = null;
    try {
      const r = await ipc.wslconfRestore(distro, id, force, true);
      outcome = r;
      restoreId = null;
      restoreForce = false;
      read = await ipc.wslconfRead(distro);
      backups = await ipc.wslconfBackups(distro);
    } catch (e) {
      restoreMsg = String(e);
      restoreForce = restoreMsg.includes('force the restore');
    } finally {
      working = false;
    }
  }

  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      copied = true;
      setTimeout(() => (copied = false), 1800);
    } catch {
      copied = false;
    }
  }

  const when = (t: number) => new Date(t * 1000).toLocaleString();
</script>

<div class="panel" data-testid="wslconf-panel">
  {#if !distro}
    <p class="dm-muted">Choose a distribution to see its configuration.</p>
  {:else}
    <section class="dm-card" aria-label="Distribution configuration">
      <div class="dm-row" style="justify-content: space-between">
        <div>
          <h3>Distribution configuration <span class="dm-faint">· {distro}</span></h3>
          <p class="dm-muted dm-small">
            Managed by the <strong>distribution</strong>: <code>/etc/wsl.conf</code> inside it. VLINUX reads it first, shows every change before saving, and keeps a backup you can restore.
          </p>
        </div>
        <button class="dm-btn small" onclick={() => load()} disabled={loading}>{loading ? 'Reading…' : 'Re-read'}</button>
      </div>

      {#if loading && !read}
        <p class="dm-muted" aria-busy="true">Reading the file…</p>
      {/if}
      {#if error}<div class="dm-notice err" role="alert">{error}</div>{/if}

      {#if read?.kind === 'needs_start'}
        <div class="dm-confirm soft" data-testid="conf-needs-start">
          <span>“{distro}” is stopped. Reading or changing its configuration needs it to be running, and VLINUX does not start a distribution unless you ask.</span>
          <div class="dm-row"><button class="dm-btn" onclick={startAndRead} disabled={busyOp !== null}>Start “{distro}” and read</button></div>
        </div>
      {/if}

      {#if view}
        {#if !view.exists}<div class="dm-notice info">There is no <code>/etc/wsl.conf</code> in this distribution, so WSL's defaults apply. Saving a change creates the file.</div>{/if}
        {#each view.notes.filter((n) => view && (view.exists || !n.includes('no /etc/wsl.conf'))) as n}<div class="dm-notice info">{n}</div>{/each}
        {#if view.malformed.length}
          <div class="dm-notice err" role="alert">The file has lines VLINUX cannot understand (line {view.malformed.join(', ')}). Editing is switched off so nothing is lost: fix the file by hand first.</div>
        {/if}

        <div class="fields">
          {#each view.settings as s (s.id)}
            <div class="dm-field">
              <label for={`wc-${s.id}`}>{s.label}</label>
              {#if s.kind === 'bool'}
                <select id={`wc-${s.id}`} class="dm-select" value={shown(s)} disabled={!view.editable} onchange={(e) => setDraft(s, e.currentTarget.value)}>
                  <option value="">Not set (WSL default)</option>
                  <option value="true">On (true)</option>
                  <option value="false">Off (false)</option>
                </select>
              {:else}
                <input id={`wc-${s.id}`} class="dm-input" value={shown(s)} disabled={!view.editable} placeholder={s.kind === 'path' ? '/mnt/' : 'user name'} spellcheck="false" autocomplete="off"
                  oninput={(e) => setDraft(s, e.currentTarget.value)} />
              {/if}
              <span class="help">{s.help} <span class="dm-faint">{s.default_note}</span> · currently: <strong>{s.value ?? 'not set'}</strong></span>
            </div>
          {/each}
        </div>
        {#if view.other_keys.length}
          <p class="dm-muted dm-small">Other settings in the file ({view.other_keys.join(', ')}) and all comments are kept exactly as they are.</p>
        {/if}
        <div class="dm-row">
          <button class="dm-btn primary" onclick={review} disabled={!view.editable || changes.length === 0 || working || busyOp !== null} data-testid="conf-review">Review changes</button>
          {#if changes.length === 0}<span class="dm-muted dm-small">Change a value above to review it.</span>{:else}<span class="dm-muted dm-small">{changes.length} change{changes.length === 1 ? '' : 's'} not saved yet.</span>{/if}
        </div>
      {/if}

      {#if plan}
        <div class="review" data-testid="conf-plan">
          <h4>Review before saving</h4>
          <ul>{#each plan.summary as s}<li>{s}</li>{/each}</ul>
          <div class="dm-diff" role="region" aria-label="Changes to /etc/wsl.conf">
            {#each plan.diff as l}<div class={l.kind}>{l.kind === 'add' ? '+ ' : l.kind === 'remove' ? '− ' : '  '}{l.text}</div>{/each}
          </div>
          {#each plan.warnings as w}<div class="dm-notice warn">{w}</div>{/each}
          <div class="dm-notice info">{plan.restart_note}</div>
          <p class="dm-muted dm-small">A backup of the current file is saved first{plan.creates_file ? ' (the file does not exist yet, so the backup records that)' : ''}. VLINUX needs no administrator rights; if this distribution does not let it write the file, nothing is changed and you are shown how to do it yourself.</p>
          <label class="dm-check"><input type="checkbox" bind:checked={reviewed} /> <span>I have reviewed this change and want it saved to <code>/etc/wsl.conf</code> in “{distro}”.</span></label>
          <div class="dm-row">
            <button class="dm-btn primary" disabled={!reviewed || working} onclick={apply} data-testid="conf-apply">{working ? 'Saving…' : 'Save change'}</button>
            <button class="dm-btn" onclick={() => { plan = null; reviewed = false; }}>Discard</button>
          </div>
        </div>
      {/if}

      {#if outcome?.kind === 'applied'}
        <div class="dm-notice good" role="status" data-testid="conf-applied">
          <strong>{outcome.verified_on_disk ? 'Saved and read back from disk.' : 'Saved.'}</strong> {outcome.message}
          <br /><strong>Not yet in effect:</strong> VLINUX cannot see what the distribution is using, and it will not restart it for you.
        </div>
      {:else if outcome?.kind === 'not_writable'}
        <div class="dm-notice warn" role="alert" data-testid="conf-not-writable">
          <strong>Nothing was changed.</strong> {outcome.reason}
          <p>{outcome.instructions}</p>
          <pre class="dm-code">{outcome.proposed}</pre>
          <div class="dm-row"><button class="dm-btn small" onclick={() => copy(outcome && outcome.kind === 'not_writable' ? outcome.proposed : '')}>{copied ? 'Copied' : 'Copy proposed content'}</button></div>
        </div>
      {/if}
    </section>

    {#if backups.length}
      <section class="dm-card" aria-label="Backups">
        <h3>Backups</h3>
        <p class="dm-muted dm-small">Taken before every save or restore, stored in VLINUX's own data (not in the distribution). Restoring writes the old content back — and first backs up the current file, so a restore can be undone too.</p>
        {#if restoreMsg}<div class="dm-notice err" role="alert">{restoreMsg}</div>{/if}
        <ul class="bk">
          {#each backups as b (b.id)}
            <li data-testid="backup-row">
              <span>{when(b.taken_at)} · {b.existed ? `${b.bytes} bytes` : 'the file did not exist'}{b.applied ? '' : ' · safety copy'}{b.restored_at ? ` · restored ${when(b.restored_at)}` : ''}</span>
              {#if restoreId === b.id}
                <span class="dm-confirm">
                  <span>{restoreForce ? 'The file was changed by something other than VLINUX. Restoring anyway discards those changes.' : `Restore this version of /etc/wsl.conf${b.existed ? '' : ' (which means removing the file)'}?`} It takes effect only after the distribution restarts.</span>
                  <span class="dm-row">
                    <button class="dm-btn small danger" disabled={working} onclick={() => restore(b.id, restoreForce)}>{restoreForce ? 'Restore anyway' : 'Restore'}</button>
                    <button class="dm-btn small" onclick={() => { restoreId = null; restoreForce = false; restoreMsg = null; }}>Cancel</button>
                  </span>
                </span>
              {:else}
                <button class="dm-btn small" disabled={working || busyOp !== null || read?.kind !== 'view'} onclick={() => { restoreId = b.id; restoreMsg = null; }}>Restore…</button>
              {/if}
            </li>
          {/each}
        </ul>
      </section>
    {/if}
  {/if}

  {#if windows}
    <section class="dm-card" aria-label="Windows WSL configuration" data-testid="winconf">
      <h3>Windows-wide WSL settings <span class="dm-faint">· read-only</span></h3>
      <p class="dm-muted dm-small">Managed by <strong>Windows</strong> (<code>{win?.display_path ?? '%USERPROFILE%\\.wslconfig'}</code>), shared by every distribution. VLINUX only reads it; changing it needs the whole WSL VM to restart, which VLINUX never does.</p>
      {#if winError}<div class="dm-notice err">{winError}</div>{/if}
      {#if !win && !winError}<p class="dm-muted" aria-busy="true">Reading…</p>{/if}
      {#if win}
        {#if win.read_error}<div class="dm-notice err">{win.read_error}</div>{/if}
        {#if !win.exists}<div class="dm-notice info">There is no <code>.wslconfig</code>; WSL's defaults apply.</div>{/if}
        {#if win.malformed.length}<div class="dm-notice warn">Some lines could not be understood (line {win.malformed.join(', ')}).</div>{/if}
        <table class="dm-table">
          <tbody>
            {#each win.resources as r}
              <tr><th scope="row">{r.label}</th><td>{r.value ?? 'not set'}{#if !r.value && r.note} <span class="dm-faint">— {r.note}</span>{/if}</td></tr>
            {/each}
            {#if win.host_processors}<tr><th scope="row">Logical processors on this PC</th><td>{win.host_processors}</td></tr>{/if}
          </tbody>
        </table>
        {#each win.notes as n}<p class="dm-muted dm-small" class:sim={n.startsWith('SIMULATED')}>{n}</p>{/each}
      {/if}
    </section>
  {/if}
  {#if !isTauri()}
    <div class="dm-notice sim">SIMULATED: this is the browser preview. Configuration, backups and the Windows file shown here are simulated in memory; nothing real is read or written.</div>
  {/if}
</div>

<style>
  .panel { display: grid; gap: 12px; align-content: start; }
  h3 { margin: 0; font-size: 0.98rem; }
  h4 { margin: 0; font-size: 0.88rem; }
  p { margin: 6px 0 0; line-height: 1.5; }
  .fields { display: grid; gap: 12px; margin-top: 12px; }
  .review { margin-top: 14px; display: grid; gap: 10px; padding: 12px; border-radius: 12px; border: 1px solid var(--line-strong, var(--line)); background: rgb(255 255 255 / 0.03); }
  .review ul { margin: 0; padding-left: 18px; font-size: 0.84rem; }
  .bk { list-style: none; margin: 10px 0 0; padding: 0; display: grid; gap: 8px; }
  .bk li { display: flex; justify-content: space-between; gap: 12px; align-items: center; flex-wrap: wrap; font-size: 0.82rem; padding: 7px 10px; border-radius: 10px; background: rgb(255 255 255 / 0.04); }
  .sim { color: #5ee0ff; }
  code { overflow-wrap: anywhere; }
</style>
