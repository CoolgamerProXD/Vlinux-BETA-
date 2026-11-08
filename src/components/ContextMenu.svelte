<script lang="ts">
  import { tick } from 'svelte';
  import { ctx } from '../lib/ctx.svelte';

  let el: HTMLDivElement | undefined = $state();
  let pos = $state({ x: 0, y: 0 });

  $effect(() => {
    const m = ctx.menu;
    if (!m) return;
    pos = { x: m.x, y: m.y };
    void tick().then(() => {
      if (!el) return;
      // Keep it on screen.
      const r = el.getBoundingClientRect();
      pos = {
        x: Math.max(4, Math.min(m.x, window.innerWidth - r.width - 4)),
        y: Math.max(4, Math.min(m.y, window.innerHeight - r.height - 4)),
      };
      el.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus();
    });
  });

  function buttons(): HTMLButtonElement[] {
    return el ? [...el.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')] : [];
  }

  function key(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.stopPropagation();
      ctx.close();
    } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      const list = buttons();
      if (!list.length) return;
      const i = list.indexOf(document.activeElement as HTMLButtonElement);
      const next = e.key === 'ArrowDown' ? (i + 1) % list.length : (i - 1 + list.length) % list.length;
      list[next]?.focus();
    } else if (e.key === 'Tab') {
      e.preventDefault();
    }
  }

  function run(action?: () => void) {
    ctx.close();
    action?.();
  }
</script>

<svelte:window onpointerdown={(e) => ctx.menu && !el?.contains(e.target as Node) && ctx.close()} onblur={() => ctx.close()} />

{#if ctx.menu}
  {@const m = ctx.menu}
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div class="menu glass" bind:this={el} role="menu" aria-label={m.label} style="left:{pos.x}px; top:{pos.y}px" onkeydown={key}
       oncontextmenu={(e) => e.preventDefault()} tabindex="-1">
    {#each m.items as it, i (i)}
      {#if it.separator}
        <hr />
      {:else}
        <button role={it.checked !== undefined ? 'menuitemcheckbox' : 'menuitem'} aria-checked={it.checked} disabled={it.disabled} class:danger={it.danger}
                onclick={() => run(it.action)}>
          <span class="check" aria-hidden="true">{it.checked ? '✓' : ''}</span>
          <span class="lbl">{it.label}</span>
          {#if it.hint}<span class="hint mono">{it.hint}</span>{/if}
        </button>
      {/if}
    {/each}
  </div>
{/if}

<style>
  .menu {
    position: fixed; z-index: 140; min-width: 210px; max-width: 320px; padding: 6px; border-radius: 12px;
    background: rgb(var(--panel2-rgb) / 0.98); display: grid; gap: 1px; animation: pop-in 140ms var(--ease) both;
  }
  button { display: flex; align-items: center; gap: 8px; width: 100%; padding: 6px 10px 6px 6px; border-radius: 8px; text-align: left; font-size: 0.84rem; }
  button:hover:not(:disabled), button:focus-visible { background: rgb(var(--accent-rgb) / 0.2); outline: none; }
  button:disabled { opacity: 0.45; cursor: not-allowed; }
  button.danger { color: #ff8a98; }
  .check { width: 14px; text-align: center; color: var(--accent); }
  .lbl { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .hint { color: var(--faint); font-size: 0.72rem; }
  hr { border: 0; border-top: 1px solid var(--line); margin: 4px 2px; }
</style>
