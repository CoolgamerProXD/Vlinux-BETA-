<script lang="ts">
  import type { AppDef } from '../lib/apps';
  import Icon from './Icon.svelte';
  let { app, size = 48 }: { app: AppDef; size?: number } = $props();
</script>

<span class="tile" class:planned={app.status === 'planned'}
      style="--c:{app.color}; width:{size}px; height:{size}px; border-radius:{Math.round(size * 0.3)}px">
  <Icon name={app.icon} size={Math.round(size * 0.5)} stroke={1.6} />
</span>

<style>
  .tile {
    display: grid; place-items: center; flex: none;
    color: var(--c);
    background:
      radial-gradient(120% 120% at 20% 10%, color-mix(in srgb, var(--c) 32%, transparent), transparent 60%),
      linear-gradient(160deg, rgb(255 255 255 / 0.07), rgb(255 255 255 / 0.02)),
      rgb(var(--win-rgb) / 0.85);
    border: 1px solid color-mix(in srgb, var(--c) 35%, rgb(255 255 255 / 0.08));
    box-shadow: inset 0 1px 0 rgb(255 255 255 / 0.12), 0 6px 16px rgb(0 0 0 / 0.35);
  }
  .planned { filter: saturate(0.25); opacity: 0.55; }
</style>
