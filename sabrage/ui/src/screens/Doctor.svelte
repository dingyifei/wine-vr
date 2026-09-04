<script lang="ts">
  // The Doctor screen — drives `doctorStore` (every check pass goes through
  // `runChecks()`) and owns bottle selection plus Fix-in-flight state.
  // Whole-stage fixes open the app-root GateModal via `stageStore.openGate`, the
  // same singleton StagesPanel drives; `sessionStore` blocks mutation while live.
  import { onMount } from "svelte";
  import { cap, errMsg, titleCase } from "../lib/text";
  import BottleSelect from "../components/BottleSelect.svelte";
  import CheckRow from "../components/CheckRow.svelte";
  import { doctorStore } from "../stores/doctor.svelte";
  import { sessionStore } from "../stores/session.svelte";
  import { stageStore } from "../stores/stage.svelte";
  import {
    applyFix,
    blocksMutation,
    cancelStage,
    contractFixIdToAction,
    FIX_META,
    type FixAction,
  } from "../ipc";

  /** A Doctor run recent enough that re-navigating here shouldn't re-fire a
   * full check pass — the "Run checks" button remains the forced refresh. */
  const AUTORUN_STALE_MS = 60_000;

  interface Props {
    /** Bumped by App.svelte on every Pipeline ▸ Run Doctor (⌘D) firing;
     * mirrors Session's `launchRequest`. Each new value forces a fresh pass,
     * even when Doctor is already the open screen or its last run is still
     * within `AUTORUN_STALE_MS`. */
    doctorRequest?: number;
  }
  let { doctorRequest = 0 }: Props = $props();

  interface FixError {
    slug: string;
    message: string;
    remedy: string | null;
    fix: FixAction | null;
  }

  let selectedBottle = $state("");
  let fixBusySlug = $state<string | null>(null);
  /** The in-flight fix's run id, off the first `StageEvent` `applyFix`
   * streams (`fixes::apply` emits it before waiting for the operation lock —
   * see that function's doc comment) — a fix queued behind another Sabrage
   * operation can run for minutes with nothing to cancel otherwise. */
  let fixRunId = $state<string | null>(null);
  let fixCancelling = $state(false);
  let fixError = $state<FixError | null>(null);
  let confirmFix = $state<{ slug: string; action: FixAction } | null>(null);
  /**
   * Per-slug notice from the last fix run. Keyed so it survives the
   * `runChecks()` repaint `runFix` fires in its `finally`: a fix can emit a
   * `warn` or resolve `changed: false` while the check still reports the same
   * status. Cleared when a new run starts (`dismissFixNotice`) or on dismiss. */
  let fixNotices = $state<Record<string, string>>({});

  /** Drops slug's notice, if any. Called by the row's Dismiss control and by
   * `runFix` before a new attempt, so a notice from a previous attempt never
   * survives into a fresh one; it re-appears only if the new run warns or
   * resolves unchanged. */
  function dismissFixNotice(slug: string) {
    if (!(slug in fixNotices)) return;
    const { [slug]: _dropped, ...rest } = fixNotices;
    fixNotices = rest;
  }

  let lastHandledDoctorRequest = $state(0);
  let doctorRequestNotice = $state<string | null>(null);
  /** True once this mount's `onMount` has made its freshness-based autorun
   * decision. The request-replay effect below waits on it so it cannot read
   * `doctorStore.running` or start a pass before that decision is made
   * (mirrors Session's `bottlePrefillDone`). */
  let doctorAutorunDecided = $state(false);

  function pickDefaultBottle(bottles: string[], preferred: string | null): string {
    if (preferred && bottles.includes(preferred)) return preferred;
    return bottles.includes("Steam") ? "Steam" : (bottles[0] ?? "");
  }

  onMount(async () => {
    await doctorStore.loadBottles();
    selectedBottle = pickDefaultBottle(doctorStore.bottles, doctorStore.defaultBottle);
    const fresh =
      doctorStore.hasRun &&
      doctorStore.lastRunAtMs != null &&
      Date.now() - doctorStore.lastRunAtMs < AUTORUN_STALE_MS;
    if (!fresh) void runChecks();
    // Set only after the decision above: `runChecks()`, if it fired, already
    // set `doctorStore.running` synchronously before its first await, so the
    // request-replay effect can never start a second concurrent pass.
    doctorAutorunDecided = true;
  });

  // A menu-triggered Run Doctor (⌘D) always forces a fresh pass, bypassing
  // `AUTORUN_STALE_MS` — including when Doctor is already the open screen, where
  // nothing remounts to re-run `onMount`'s pass. Waits on `doctorAutorunDecided`.
  $effect(() => {
    const reqId = doctorRequest;
    if (reqId === lastHandledDoctorRequest) return;
    if (!doctorAutorunDecided) return;
    lastHandledDoctorRequest = reqId;
    if (doctorStore.running) {
      doctorRequestNotice = "Checks are already running.";
      return;
    }
    doctorRequestNotice = null;
    void runChecks();
  });

  /** May Doctor mutate the machine right now? The backend refuses every Fix
   * while a session is live (`fixes::apply` -> `deny_if_session_live`); this
   * only disables the button early so the user learns why before clicking. */
  const sessionLive = $derived(blocksMutation(sessionStore.status.phase));

  /** Whether a pipeline stage is already running — `stageStore.running`, not
   * `stageStore.gate`, which reads `null` once the modal is Hidden. Whole-stage
   * fixes share that one GateModal (`stageStore.openGate`), so a second would
   * queue behind the first and display mislabelled when the modal reopens. */
  const stageRunning = $derived(stageStore.running);

  async function runChecks() {
    await doctorStore.run({ bottle: selectedBottle || null });
  }

  /** Handles a CheckRow's Fix button: resolves the contract fix id to a
   * `FixAction` and dispatches it. An unrecognised id is ignored. */
  function handleFixRequest(slug: string, fixId: string) {
    const action = contractFixIdToAction(fixId);
    if (!action) return;
    dispatchFix(slug, action);
  }

  /** Dispatches an already-resolved `FixAction` — shared with the error
   * banner's retry button, whose action comes off a `Fatal` event rather than
   * a contract id string. */
  function dispatchFix(slug: string, action: FixAction) {
    if (sessionLive) {
      fixError = {
        slug,
        message: "Refusing to run while a session is live — stop the session first.",
        remedy: null,
        fix: null,
      };
      return;
    }
    const meta = FIX_META[action];
    if (meta.stage) {
      if (stageRunning) {
        fixError = {
          slug,
          message: "A pipeline stage is already running — wait for it to finish, then retry.",
          remedy: null,
          fix: null,
        };
        return;
      }
      stageStore.openGate({
        stage: meta.stage,
        bottle: selectedBottle || null,
        onFinished: () => void runChecks(),
      });
      return;
    }
    if (meta.destructive) {
      confirmFix = { slug, action };
      return;
    }
    void runFix(slug, action);
  }

  interface FatalInfo {
    message: string;
    remedy: string | null;
    fix: FixAction | null;
  }

  async function runFix(slug: string, action: FixAction) {
    fixBusySlug = slug;
    fixRunId = null;
    fixCancelling = false;
    fixError = null;
    confirmFix = null;
    dismissFixNotice(slug);
    // A failing fix emits a `Fatal` event on this stream (message + remedy +
    // follow-up fix id) before `applyFix` rejects; the catch below reports
    // that, not the bare rejection message.
    let fatal: FatalInfo | null = null;
    // A fix can warn without failing outright, and the check it is meant to
    // fix may keep reporting the same status, so these warnings are the only
    // signal that something is still wrong.
    const warnings: string[] = [];
    try {
      const report = await applyFix(action, { bottle: selectedBottle || null }, true, (ev) => {
        if (fixRunId === null) fixRunId = ev.runId;
        if (ev.kind === "fatal") {
          fatal = { message: ev.message, remedy: ev.remedy, fix: ev.fix };
        } else if (ev.kind === "line" && ev.severity === "warn") {
          warnings.push(ev.text);
        }
      });
      if (warnings.length > 0) {
        fixNotices = { ...fixNotices, [slug]: warnings.join(" ") };
      } else if (!report.changed) {
        fixNotices = { ...fixNotices, [slug]: report.description };
      }
    } catch (e) {
      // TypeScript narrows `fatal` to the initializer's `null` here — it cannot
      // prove the callback above ran — so the cast restores the declared union;
      // the truthiness check below still does the branching.
      const captured = fatal as FatalInfo | null;
      fixError = captured
        ? { slug, message: captured.message, remedy: captured.remedy, fix: captured.fix }
        : { slug, message: errMsg(e), remedy: null, fix: null };
      if (warnings.length > 0) {
        fixNotices = { ...fixNotices, [slug]: warnings.join(" ") };
      }
    } finally {
      fixBusySlug = null;
      fixRunId = null;
      fixCancelling = false;
      void runChecks();
    }
  }

  async function cancelRunningFix() {
    if (!fixRunId || fixCancelling) return;
    fixCancelling = true;
    try {
      await cancelStage(fixRunId);
    } finally {
      fixCancelling = false;
    }
  }

  const summaryText = $derived.by(() => {
    // A rerun that rejected before reporting every slug must not keep reading
    // "Running checks…": that text otherwise survives once `running` flips
    // false, with nothing else in the header saying doctor failed.
    if (doctorStore.error && !doctorStore.running) return "Doctor failed to complete";
    const s = doctorStore.summary;
    if (!s) {
      const done = doctorStore.rows.filter((r) => r.phase === "done").length;
      return done > 0 ? `${done} checked so far` : "Running checks…";
    }
    if (s.failCount === 0 && s.warnCount === 0) return `All ${s.total} checks passed`;
    if (s.failCount === 0) {
      return `All ${s.total} checks passed · ${s.warnCount} warning${s.warnCount === 1 ? "" : "s"}`;
    }
    return `${s.failCount} check${s.failCount === 1 ? "" : "s"} failed, ${s.warnCount} warning${
      s.warnCount === 1 ? "" : "s"
    } — remedies inline`;
  });
</script>

<div class="screen-header">
  <div class="header-top">
    <h3>Doctor</h3>
    <div class="header-actions">
      <span class="text-muted summary-text">{summaryText}</span>
      <button class="btn btn-primary" onclick={runChecks} disabled={doctorStore.running}>
        {doctorStore.running ? "Running…" : "Run checks"}
      </button>
    </div>
  </div>
  {#if doctorRequestNotice}
    <p class="text-muted doctor-request-notice">{doctorRequestNotice}</p>
  {/if}
  <div class="bottle-row">
    <label class="text-muted" for="doctor-bottle">Bottle</label>
    <BottleSelect
      id="doctor-bottle"
      class="input bottle-select"
      bottles={doctorStore.bottles}
      bottlesLoaded={doctorStore.bottlesLoaded}
      bind:value={selectedBottle}
      disabled={doctorStore.running}
    />
  </div>
</div>

<div class="screen-body">
  {#if doctorStore.error}
    <div class="blueprint error-card">
      <i class="corner tl"></i><i class="corner tr"></i><i class="corner bl"></i><i class="corner br"></i>
      <h6>Could not run doctor</h6>
      <p class="text-muted">{doctorStore.error}</p>
    </div>
  {/if}
  {#if doctorStore.rows.length === 0}
    {#if doctorStore.running}
      <p class="text-muted">Running checks…</p>
    {:else if !doctorStore.error}
      <p class="text-muted">Doctor checks have not run yet.</p>
    {/if}
  {:else}
    <div class="rows">
      {#each doctorStore.rows as row, i (row.slug)}
        {#if i === 0 || row.group !== doctorStore.rows[i - 1].group}
          <h6 class="group-header">{titleCase(row.group)}</h6>
        {/if}
        <CheckRow
          {row}
          isRunning={row.slug === doctorStore.runningSlug}
          busy={fixBusySlug === row.slug}
          disabledReason={sessionLive
            ? "Refusing to run while a session is live — stop the session first."
            : stageRunning
              ? "A pipeline stage is already running — wait for it to finish."
              : null}
          onFix={(fixId) => handleFixRequest(row.slug, fixId)}
        />
        {#if fixNotices[row.slug]}
          <div class="fix-notice" role="status">
            <span>{fixNotices[row.slug]}</span>
            <button
              class="btn btn-ghost fix-notice-dismiss"
              onclick={() => dismissFixNotice(row.slug)}
            >
              Dismiss
            </button>
          </div>
        {/if}
      {/each}
    </div>
  {/if}
</div>

{#if fixBusySlug && fixRunId}
  <div class="fix-error-banner">
    <div class="text-muted">
      Applying fix{fixCancelling ? " — cancelling…" : "…"} it can be cancelled while it waits or runs.
    </div>
    <button class="btn btn-ghost fix-error-retry" onclick={cancelRunningFix} disabled={fixCancelling}>
      Cancel
    </button>
  </div>
{/if}

{#if fixError}
  <div class="fix-error-banner">
    <div class="text-muted">Fix failed: {fixError.message}</div>
    {#if fixError.remedy}<div class="fix-error-remedy">{fixError.remedy}</div>{/if}
    {#if fixError.fix}
      <button
        class="btn btn-ghost fix-error-retry"
        onclick={() => dispatchFix(fixError!.slug, fixError!.fix!)}
      >
        {FIX_META[fixError.fix].stage ? `Open ${cap(FIX_META[fixError.fix].stage!)}` : "Try suggested fix"}
      </button>
    {/if}
  </div>
{/if}

{#if confirmFix}
  <div class="confirm-backdrop">
    <div class="dialog blueprint elev-md confirm-dialog">
      <i class="corner tl"></i><i class="corner tr"></i><i class="corner bl"></i><i class="corner br"></i>
      <div class="dialog-title">{FIX_META[confirmFix.action].title}</div>
      <p class="dialog-body">
        {FIX_META[confirmFix.action].consequence ?? "This cannot be undone."} Continue?
      </p>
      <div class="dialog-actions">
        <button class="btn btn-secondary" onclick={() => (confirmFix = null)}>Cancel</button>
        <button class="btn btn-primary" onclick={() => runFix(confirmFix!.slug, confirmFix!.action)}>
          Yes, continue
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .screen-header {
    padding: 18px 28px 14px;
    border-bottom: 1px solid var(--color-divider);
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .header-top {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
  }
  .header-top h3 {
    margin: 0;
  }
  .header-actions {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .summary-text {
    font-size: 12px;
  }
  .doctor-request-notice {
    font-size: 12px;
    margin: 0;
  }
  .bottle-row {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .bottle-row label {
    font-size: 12px;
  }
  .bottle-row :global(.bottle-select) {
    width: auto;
    min-height: 30px;
    padding: 3px 8px;
    font-size: 13px;
  }
  .screen-body {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 16px 28px 24px;
  }
  .group-header {
    margin: 18px 0 4px;
    color: var(--color-accent-700);
  }
  .rows > .group-header:first-child {
    margin-top: 0;
  }
  /* Advisory tone, deliberately distinct from `.fix-error-banner`'s Fatal
     treatment below: a fix notice is not a failed fix, so it matches the
     `warn` StatusIcon rather than the error banner's accent color. */
  .fix-notice {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    margin: -2px 0 8px 26px;
    padding: 4px 10px;
    font-size: 11.5px;
    color: var(--color-neutral-800);
    background: color-mix(in srgb, var(--color-neutral-600) 12%, transparent);
    border: 1px solid color-mix(in srgb, var(--color-neutral-600) 30%, transparent);
  }
  .fix-notice-dismiss {
    flex: none;
    font-size: 11px;
    padding: 2px 8px;
  }
  .error-card {
    padding: 18px 20px;
    max-width: 460px;
  }
  .error-card h6 {
    color: var(--color-accent-900);
    margin-bottom: 6px;
  }
  .error-card p {
    font-size: 13px;
    margin: 0;
  }
  .fix-error-banner {
    position: fixed;
    left: 50%;
    bottom: 20px;
    transform: translateX(-50%);
    background: var(--color-bg);
    border: 1px solid var(--color-divider);
    padding: 8px 14px;
    font-size: 12.5px;
    color: var(--color-accent-900);
    box-shadow: var(--shadow-md);
    z-index: 45;
    display: flex;
    flex-direction: column;
    gap: 4px;
    max-width: 460px;
  }
  .fix-error-remedy {
    font-size: 11.5px;
    color: color-mix(in srgb, var(--color-text) 60%, transparent);
  }
  .fix-error-retry {
    align-self: flex-start;
    font-size: 11.5px;
    padding: 2px 8px;
    margin-top: 2px;
  }
  .confirm-backdrop {
    position: fixed;
    inset: 0;
    z-index: 45;
    display: grid;
    place-items: center;
    background: color-mix(in srgb, var(--color-neutral-900) 45%, transparent);
  }
  .confirm-dialog {
    width: 380px;
    max-width: calc(100vw - 32px);
    background: var(--color-bg);
  }
  .confirm-dialog .dialog-body {
    margin: 0;
  }
</style>
