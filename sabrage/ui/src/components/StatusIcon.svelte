<script lang="ts" module>
  export type IconKind = "ok" | "warn" | "fail" | "info" | "empty" | "spinner" | "lock";
</script>

<script lang="ts">
  // The status-row icon set, previously hand-duplicated (byte-identical SVGs,
  // same stroke widths/colors) across CheckRow.svelte's `pass/warn/fail`
  // dispatch and GateModal.svelte's `lineIcon`/`checkIcon` snippets.
  interface Props {
    kind: IconKind;
    /** GateModal's rows render 13px icons; CheckRow's render 14px. The lock
     * icon (admin-note) has always been 15px regardless of `size`. */
    size?: number;
  }
  let { kind, size = 14 }: Props = $props();
</script>

{#if kind === "ok"}
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="var(--color-accent-700)" stroke-width="2"
    ><polyline points="20 6 9 17 4 12"></polyline></svg
  >
{:else if kind === "warn"}
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="var(--color-neutral-600)" stroke-width="1.5"
    ><path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"
    ></path><line x1="12" y1="9" x2="12" y2="13"></line><line x1="12" y1="17" x2="12.01" y2="17"></line></svg
  >
{:else if kind === "fail"}
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="var(--color-accent-900)" stroke-width="2.5"
    ><line x1="18" y1="6" x2="6" y2="18"></line><line x1="6" y1="6" x2="18" y2="18"></line></svg
  >
{:else if kind === "lock"}
  <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="var(--color-accent-700)" stroke-width="1.5"
    ><rect x="3" y="11" width="18" height="10" rx="2"></rect><path d="M7 11V7a5 5 0 0 1 10 0v4"></path></svg
  >
{:else if kind === "empty"}
  <span class="empty-square" aria-hidden="true"></span>
{:else if kind === "spinner"}
  <span class="spinner" aria-label="running"></span>
{:else}
  <!-- info -->
  <span class="info-dot" aria-hidden="true"></span>
{/if}

<style>
  .empty-square {
    display: block;
    width: 11px;
    height: 11px;
    margin: 2px;
    border: 1px solid var(--color-divider);
  }
  .info-dot {
    display: block;
    width: 6px;
    height: 6px;
    margin: 6px;
    border-radius: 50%;
    background: var(--color-neutral-500);
  }
  .spinner {
    display: block;
    width: 11px;
    height: 11px;
    margin: 2px;
    border: 1.5px solid var(--color-accent-300);
    border-top-color: var(--color-accent-700);
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
