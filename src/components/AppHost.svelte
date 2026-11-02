<script lang="ts">
  import Settings from '../apps/Settings.svelte';
  import System from '../apps/System.svelte';
  import Setup from '../apps/Setup.svelte';
  import Terminal from '../apps/Terminal.svelte';

  let { appId }: { appId: string } = $props();
</script>

{#if appId === 'settings'}
  <Settings />
{:else if appId === 'terminal'}
  <Terminal />
{:else if appId === 'setup'}
  <Setup />
{:else if appId === 'system'}
  <System />
{/if}
