<script lang="ts">
  // The bottle `<select>` (or its "none found" fallback) repeated, byte for
  // byte, across Session/Settings/EditGame/Doctor/StagesPanel. Every caller
  // still owns its own `bottles`/`bottlesLoaded` source (`bottlesStore` or
  // `doctorStore`) and passes them in — this component only renders.
  interface Props {
    id: string;
    bottles: string[];
    bottlesLoaded: boolean;
    value: string;
    disabled?: boolean;
    /** Settings' "— none —" placeholder option; every other caller omits it. */
    includeNone?: boolean;
    onchange?: () => void;
    class?: string;
  }
  let {
    id,
    bottles,
    bottlesLoaded,
    value = $bindable(),
    disabled = false,
    includeNone = false,
    onchange,
    class: klass = "input",
  }: Props = $props();
</script>

{#if bottlesLoaded && bottles.length === 0}
  <span class="text-muted">none found — create one in the CrossOver UI</span>
{:else}
  <select {id} class={klass} bind:value {disabled} {onchange}>
    {#if includeNone}
      <option value="">— none —</option>
    {/if}
    {#each bottles as b (b)}
      <option value={b}>{b}</option>
    {/each}
  </select>
{/if}
