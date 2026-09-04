<script lang="ts">
  // Presentational only: callers own `bottles`/`bottlesLoaded` and pass them in; no owned state.
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
