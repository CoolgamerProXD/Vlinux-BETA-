<script lang="ts">
  import Settings from '../apps/Settings.svelte';
  import System from '../apps/System.svelte';
  import Setup from '../apps/Setup.svelte';
  import Terminal from '../apps/Terminal.svelte';
  import Files from '../apps/Files.svelte';
  import Distros from '../apps/Distros.svelte';

  let { appId }: { appId: string } = $props();
</script>

{#if appId === 'settings'}
  <Settings />
{:else if appId === 'terminal'}
  <Terminal />
{:else if appId === 'files'}
  <Files />
{:else if appId === 'distros'}
  <Distros />
{:else if appId === 'setup'}
  <Setup />
{:else if appId === 'system'}
  <System />
{/if}
