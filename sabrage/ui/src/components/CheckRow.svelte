<script lang="ts">
  import type { DoctorRow } from "../stores/doctor.svelte";

  interface Props {
    row: DoctorRow;
    /** This row is the one standing in for "currently running" (see the store's `runningSlug`). */
    isRunning?: boolean;
  }

  let { row, isRunning = false }: Props = $props();

  const spinning = $derived(row.phase === "waiting" && isRunning);
  const placeholder = $derived(row.phase === "waiting" && !isRunning);
</script>

<div class="check-row" class:dim={row.phase === "waiting"}>
  <span class="icon">
    {#if spinning}
      <span class="spinner" aria-label="running"></span>
    {:else if placeholder || row.status === "skipped" || row.status === "not_implemented"}
      <span class="empty-square" aria-hidden="true"></span>
    {:else if row.status === "pass"}
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="var(--color-accent-700)" stroke-width="2">
        <polyline points="20 6 9 17 4 12"></polyline>
      </svg>
    {:else if row.status === "warn"}
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="var(--color-neutral-600)" stroke-width="1.5">
        <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"></path>
        <line x1="12" y1="9" x2="12" y2="13"></line>
        <line x1="12" y1="17" x2="12.01" y2="17"></line>
      </svg>
    {:else if row.status === "fail"}
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="var(--color-accent-900)" stroke-width="2.5">
        <line x1="18" y1="6" x2="6" y2="18"></line>
        <line x1="6" y1="6" x2="18" y2="18"></line>
      </svg>
    {:else}
      <!-- info -->
      <span class="info-dot" aria-hidden="true"></span>
    {/if}
  </span>
  <div class="body">
    <div class="message">{row.message}</div>
    {#if row.remedy}
      <div class="text-muted remedy">{row.remedy}</div>
    {/if}
  </div>
</div>

<style>
  .check-row {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 6px 8px;
    border-bottom: 1px solid color-mix(in srgb, var(--color-text) 7%, transparent);
    transition: opacity 0.15s ease;
  }
  .check-row.dim {
    opacity: 0.35;
  }
  .icon {
    width: 16px;
    flex: none;
    padding-top: 2px;
    display: block;
  }
  .icon svg {
    display: block;
  }
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
  .body {
    flex: 1;
    min-width: 0;
  }
  .message {
    font-size: 13px;
  }
  .remedy {
    font-size: 11.5px;
    margin-top: 1px;
  }
</style>
