<script lang="ts">
  import type { Wallpaper } from '../lib/types';
  import { ridge, stars } from '../lib/wallpaper-gen';

  let { id }: { id: Wallpaper } = $props();

  // Generated once at module scope of the component instance (deterministic).
  const nebulaStars = stars(11, 220);
  const duskStars = stars(23, 90, 420);
  const peakStars = stars(37, 160, 520);
  const dusk = [ridge(3, 640, 150), ridge(4, 700, 110), ridge(5, 770, 80)];
  const peaks = [ridge(8, 560, 260, 0.6), ridge(9, 650, 190, 0.58), ridge(10, 740, 120, 0.55)];
</script>

<div class="wp" aria-hidden="true">
  {#key id}
    <svg class="layer" viewBox="0 0 1600 900" preserveAspectRatio="xMidYMid slice">
      {#if id === 'nebula'}
        <defs>
          <radialGradient id="n1" cx="22%" cy="28%" r="60%"><stop offset="0" stop-color="#6b3fe0" stop-opacity=".75" /><stop offset="1" stop-color="#6b3fe0" stop-opacity="0" /></radialGradient>
          <radialGradient id="n2" cx="78%" cy="70%" r="55%"><stop offset="0" stop-color="#1fb8e6" stop-opacity=".5" /><stop offset="1" stop-color="#1fb8e6" stop-opacity="0" /></radialGradient>
          <radialGradient id="n3" cx="62%" cy="18%" r="40%"><stop offset="0" stop-color="#d049a8" stop-opacity=".4" /><stop offset="1" stop-color="#d049a8" stop-opacity="0" /></radialGradient>
          <linearGradient id="nb" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#0d0a1f" /><stop offset="1" stop-color="#05060d" /></linearGradient>
        </defs>
        <rect width="1600" height="900" fill="url(#nb)" />
        <rect width="1600" height="900" fill="url(#n1)" />
        <rect width="1600" height="900" fill="url(#n2)" />
        <rect width="1600" height="900" fill="url(#n3)" />
        {#each nebulaStars as s}<circle cx={s.x} cy={s.y} r={s.r} fill="#fff" opacity={s.o} />{/each}
      {:else if id === 'dusk'}
        <defs>
          <linearGradient id="ds" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#0c0a24" /><stop offset=".5" stop-color="#3a1f5e" /><stop offset=".78" stop-color="#c2467a" /><stop offset="1" stop-color="#ff9a5c" /></linearGradient>
          <radialGradient id="sun" cx="50%" cy="72%" r="30%"><stop offset="0" stop-color="#ffd9a0" stop-opacity=".9" /><stop offset=".35" stop-color="#ff9a5c" stop-opacity=".35" /><stop offset="1" stop-color="#ff9a5c" stop-opacity="0" /></radialGradient>
        </defs>
        <rect width="1600" height="900" fill="url(#ds)" />
        {#each duskStars as s}<circle cx={s.x} cy={s.y} r={s.r} fill="#fff" opacity={s.o * 0.8} />{/each}
        <rect width="1600" height="900" fill="url(#sun)" />
        <path d={dusk[0]} fill="#2a1840" />
        <path d={dusk[1]} fill="#1a1030" />
        <path d={dusk[2]} fill="#0d0820" />
      {:else if id === 'peaks'}
        <defs>
          <linearGradient id="ps" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#040b14" /><stop offset=".6" stop-color="#0b2a3a" /><stop offset="1" stop-color="#1b6a73" /></linearGradient>
          <radialGradient id="moon" cx="74%" cy="22%" r="14%"><stop offset="0" stop-color="#e8fbff" stop-opacity=".55" /><stop offset="1" stop-color="#7fe3ff" stop-opacity="0" /></radialGradient>
        </defs>
        <rect width="1600" height="900" fill="url(#ps)" />
        {#each peakStars as s}<circle cx={s.x} cy={s.y} r={s.r} fill="#dff8ff" opacity={s.o} />{/each}
        <rect width="1600" height="900" fill="url(#moon)" />
        <circle cx="1184" cy="198" r="34" fill="#effcff" />
        <path d={peaks[0]} fill="#0f3d4d" opacity=".85" />
        <path d={peaks[1]} fill="#0a2a38" />
        <path d={peaks[2]} fill="#051620" />
      {:else}
        <defs>
          <radialGradient id="gb" cx="50%" cy="40%" r="75%"><stop offset="0" stop-color="#16162a" /><stop offset="1" stop-color="#05050a" /></radialGradient>
          <pattern id="dots" width="32" height="32" patternUnits="userSpaceOnUse"><circle cx="16" cy="16" r="1.2" fill="#fff" opacity=".16" /></pattern>
          <radialGradient id="gm" cx="50%" cy="50%" r="50%"><stop offset="0" stop-color="#fff" /><stop offset="1" stop-color="#000" /></radialGradient>
          <mask id="fade"><rect width="1600" height="900" fill="url(#gm)" /></mask>
          <linearGradient id="gl" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="var(--accent)" stop-opacity=".5" /><stop offset="1" stop-color="var(--accent-2)" stop-opacity=".5" /></linearGradient>
        </defs>
        <rect width="1600" height="900" fill="url(#gb)" />
        <rect width="1600" height="900" fill="url(#dots)" mask="url(#fade)" />
        <path d="M560 560 L800 280 L1040 560" fill="none" stroke="url(#gl)" stroke-width="30" stroke-linecap="round" stroke-linejoin="round" opacity=".5" />
      {/if}
    </svg>
  {/key}
  <div class="vignette"></div>
</div>

<style>
  .wp { position: absolute; inset: 0; overflow: hidden; background: var(--ink); }
  .layer { position: absolute; inset: 0; width: 100%; height: 100%; animation: fade-in 700ms var(--ease); }
  .vignette {
    position: absolute; inset: 0;
    background: radial-gradient(ellipse at center, transparent 55%, rgb(0 0 0 / 0.45) 100%);
    pointer-events: none;
  }
</style>
