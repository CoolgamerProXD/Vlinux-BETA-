<script lang="ts">
  import { ICONS, type IconName } from '../lib/icons';
  let { name, size = 18, stroke = 1.7 }: { name: IconName; size?: number; stroke?: number } = $props();
</script>

<svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width={stroke}
     stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false">
  <path d={ICONS[name]} />
</svg>
