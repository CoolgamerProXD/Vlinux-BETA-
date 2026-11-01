<script lang="ts">
  let { size = 24 }: { size?: number } = $props();
  const uid = Math.random().toString(36).slice(2, 8);
</script>

<!-- VLINUX mark: a "V" chevron with a terminal-cursor tick. Original artwork. -->
<svg width={size} height={size} viewBox="0 0 512 512" aria-hidden="true">
  <defs>
    <linearGradient id="g{uid}" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="var(--accent)" />
      <stop offset="1" stop-color="var(--accent-2)" />
    </linearGradient>
  </defs>
  <path d="M116 150 L256 372 L396 150" fill="none" stroke="url(#g{uid})" stroke-width="58" stroke-linecap="round" stroke-linejoin="round" />
  <rect x="318" y="338" width="76" height="30" rx="15" fill="var(--accent-2)" />
</svg>
