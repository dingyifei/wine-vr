<script lang="ts">
  import type { DoctorRow } from "../stores/doctor.svelte";
  import { contractFixIdToAction } from "../ipc";
  import StatusIcon, { type IconKind } from "./StatusIcon.svelte";

  interface Props {
    row: DoctorRow;
    /** This row is the one standing in for "currently running" (see the store's `runningSlug`). */
    isRunning?: boolean;
    /** Disables the Fix button — set while a fix for THIS row is in flight. */
    busy?: boolean;
    /** Non-null disables the Fix button and explains why via its `title`
     * (e.g. a live session, where a fix would be refused server-side anyway). */
    disabledReason?: string | null;
    /** Present only when `row.fix` resolves to a modelled `FixAction`
     * (`contractFixIdToAction`); omit to render no Fix button at all. */
    onFix?: (fixId: string) => void;
  }

  let { row, isRunning = false, busy = false, disabledReason = null, onFix }: Props = $props();

  const spinning = $derived(row.phase === "waiting" && isRunning);
  const placeholder = $derived(row.phase === "waiting" && !isRunning);
  const showFix = $derived(!!row.fix && !!onFix && contractFixIdToAction(row.fix) !== null);

  const iconKind = $derived.by((): IconKind => {
    if (spinning) return "spinner";
    if (placeholder || row.status === "skipped" || row.status === "not_implemented") return "empty";
    if (row.status === "pass") return "ok";
    if (row.status === "warn") return "warn";
    if (row.status === "fail") return "fail";
    return "info";
  });
</script>

<div class="check-row" class:dim={row.phase === "waiting"}>
  <span class="icon"><StatusIcon kind={iconKind} /></span>
  <div class="body">
    <div class="message">{row.message}</div>
    {#if row.remedy}
      <div class="text-muted remedy">{row.remedy}</div>
    {/if}
    {#if row.detail}
      <!-- The check's own diagnostic (e.g. "read error"/"JSON parse error"): the doctor.sh-verbatim
           message can blame a cause the native evaluator never hit (A3b-3, pinned by
           checks::config::tests::malformed_json_warns and ::unreadable_session_json_warns), so detail is truthful. -->
      <div class="text-muted detail">{row.detail}</div>
    {/if}
  </div>
  {#if showFix}
    <button
      class="btn btn-secondary fix-btn"
      disabled={busy || !!disabledReason}
      title={disabledReason ?? undefined}
      onclick={() => onFix?.(row.fix!)}
    >
      {busy ? "Fixing…" : "Fix"}
    </button>
  {/if}
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
  .icon :global(svg) {
    display: block;
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
  .detail {
    font-size: 11px;
    margin-top: 1px;
    opacity: 0.75;
  }
  .fix-btn {
    flex: none;
    align-self: center;
    font-size: 12px;
    padding: 2px 10px;
  }
</style>
