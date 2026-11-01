<script lang="ts">
  import Settings from '../apps/Settings.svelte';
  import System from '../apps/System.svelte';

  let { appId }: { appId: string } = $props();
</script>

{#if appId === 'settings'}
  <Settings />
{:else if appId === 'system'}
  <System />
{/if}
