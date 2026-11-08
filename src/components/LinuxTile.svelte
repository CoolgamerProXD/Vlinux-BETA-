<script lang="ts">
  import { tileHue, tileLetter } from '../lib/linux-apps-types';

  let { name, src = null, size = 44 }: { name: string; src?: string | null; size?: number } = $props();
  let broken = $state(false);
  $effect(() => {
    src; // a new icon gets a fresh chance
    broken = false;
  });
</script>

<span class="tile" style="width:{size}px; height:{size}px; border-radius:{Math.round(size * 0.28)}px; --h:{tileHue(name)}" aria-hidden="true">
  {#if src && !broken}
    <img {src} alt="" width={Math.round(size * 0.72)} height={Math.round(size * 0.72)} loading="lazy" decoding="async" onerror={() => (broken = true)} />
  {:else}
    <span class="letter" style="font-size:{Math.round(size * 0.42)}px">{tileLetter(name)}</span>
  {/if}
</span>

<style>
  .tile {
    display: grid; place-items: center; flex: none; overflow: hidden;
    background: linear-gradient(160deg, hsl(var(--h) 45% 32% / 0.9), hsl(var(--h) 40% 20% / 0.9));
    border: 1px solid hsl(var(--h) 50% 60% / 0.3);
    box-shadow: inset 0 1px 0 rgb(255 255 255 / 0.12);
  }
  img { object-fit: contain; }
  .letter { font-weight: 700; color: hsl(var(--h) 80% 85%); }
</style>
