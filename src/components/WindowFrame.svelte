<script lang="ts">
  import type { Snippet } from 'svelte';
  import { wm, type WinState } from '../lib/wm.svelte';
  import { appById } from '../lib/apps';
  import Icon from './Icon.svelte';

  let { win, children }: { win: WinState; children: Snippet } = $props();

  const app = $derived(appById(win.appId));
  const focused = $derived(wm.focusedId === win.id);
  const hidden = $derived(win.workspace !== wm.workspace || win.minimized);

  type Drag = { kind: 'move' | 'size'; px: number; py: number; x: number; y: number; w: number; h: number };
  let drag: Drag | null = null;

  function start(kind: Drag['kind'], e: PointerEvent) {
    wm.focus(win.id);
    if (e.button !== 0 || win.maximized) return;
    drag = { kind, px: e.clientX, py: e.clientY, x: win.x, y: win.y, w: win.w, h: win.h };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    e.preventDefault();
  }
  function moveEv(e: PointerEvent) {
    if (!drag) return;
    const dx = e.clientX - drag.px;
    const dy = e.clientY - drag.py;
    if (drag.kind === 'move') wm.move(win.id, drag.x + dx, drag.y + dy);
    else wm.resize(win.id, drag.w + dx, drag.h + dy);
  }
  function end() {
    drag = null;
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions, a11y_no_noninteractive_element_to_interactive_role -->
<section class="win glass" class:focused class:maximized={win.maximized} class:hidden
         style="--x:{win.x}px; --y:{win.y}px; --w:{win.w}px; --h:{win.h}px; z-index:{win.z}"
         onpointerdown={() => wm.focus(win.id)} role="dialog" aria-label={win.title} aria-hidden={hidden}>
  <header onpointerdown={(e) => start('move', e)} onpointermove={moveEv} onpointerup={end} onpointercancel={end}
          ondblclick={() => wm.toggleMaximize(win.id)}>
    <span class="ico">{#if app}<Icon name={app.icon} size={15} />{/if}</span>
    <span class="name">{win.title}</span>
    <div class="ctl" onpointerdown={(e) => e.stopPropagation()} ondblclick={(e) => e.stopPropagation()}>
      <button aria-label="Minimise" title="Minimise" onclick={() => wm.minimize(win.id)}><Icon name="minus" size={14} /></button>
      <button aria-label={win.maximized ? 'Restore' : 'Maximise'} title={win.maximized ? 'Restore' : 'Maximise'} onclick={() => wm.toggleMaximize(win.id)}><Icon name={win.maximized ? 'shrink' : 'square'} size={13} /></button>
      <button class="x" aria-label="Close" title="Close (Alt+W)" onclick={() => wm.close(win.id)}><Icon name="close" size={14} /></button>
    </div>
  </header>
  <div class="content">{@render children()}</div>
  {#if !win.maximized}
    <div class="grip" aria-hidden="true" onpointerdown={(e) => start('size', e)} onpointermove={moveEv} onpointerup={end} onpointercancel={end}></div>
  {/if}
</section>

<style>
  @keyframes win-in { from { opacity: 0; scale: 0.97; } to { opacity: 1; scale: 1; } }
  .win {
    position: absolute; left: 0; top: 0; width: var(--w); height: var(--h);
    transform: translate3d(var(--x), var(--y), 0);
    border-radius: 16px; display: flex; flex-direction: column; overflow: hidden;
    background: rgb(14 14 24 / calc(var(--glass-a) + 0.3));
    animation: win-in 260ms var(--ease) both;
    transition: border-color var(--dur), box-shadow var(--dur);
  }
  .win.focused { border-color: rgb(var(--accent-rgb) / 0.45); box-shadow: var(--shadow), 0 0 0 1px rgb(var(--accent-rgb) / 0.18); }
  .win:not(.focused) { filter: brightness(0.92); }
  .win.maximized { inset: 0; width: auto; height: auto; transform: none; border-radius: 14px; }
  .win.hidden { display: none; }

  header { display: flex; align-items: center; gap: 8px; height: 38px; padding: 0 8px 0 12px; flex: none; touch-action: none; border-bottom: 1px solid var(--line); background: rgb(255 255 255 / 0.03); }
  .ico { color: var(--accent); display: grid; }
  .name { flex: 1; font-size: 0.84rem; font-weight: 600; color: var(--muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .focused .name { color: var(--text); }
  .ctl { display: flex; gap: 2px; }
  .ctl button { width: 28px; height: 26px; border-radius: 8px; display: grid; place-items: center; color: var(--muted); transition: background var(--dur), color var(--dur); }
  .ctl button:hover { background: rgb(255 255 255 / 0.12); color: var(--text); }
  .ctl .x:hover { background: rgb(255 80 100 / 0.8); color: #fff; }
  .content { flex: 1; min-height: 0; overflow: auto; user-select: text; }
  .grip { position: absolute; right: 0; bottom: 0; width: 18px; height: 18px; cursor: nwse-resize; touch-action: none; }
  .grip::after { content: ''; position: absolute; right: 5px; bottom: 5px; width: 7px; height: 7px; border-right: 2px solid var(--faint); border-bottom: 2px solid var(--faint); border-radius: 0 0 3px 0; }
</style>
