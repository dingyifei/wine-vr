<script lang="ts">
  /**
   * The single gate modal, mounted once at the app root. Owns the transcript
   * for one pipeline operation (rows, console, progress, outcome, fix state)
   * and drives `stageStore`: reads `.gate` via the `request` prop and holds
   * `.setRunning` for setup/build/install/stop runs, which it starts via
   * `runStage`. In run mode it starts nothing — `sessionStore.launch(...)` was
   * already called by the opener, and this component only renders
   * `sessionStore.launchRows` (the same store Session.svelte reads, so the
   * transcript survives closing the modal and mid-launch navigation).
   */
  import { onMount } from "svelte";
  import { cap, errMsg } from "../lib/text";
  import StatusIcon from "./StatusIcon.svelte";
  import {
    runStage,
    cancelStage,
    applyFix,
    onStageQueued,
    FIX_META,
    type CheckStatus,
    type StageEvent,
    type StageOutcome,
    type Severity,
    type FixAction,
  } from "../ipc";
  import { stageStore, type GateRequest } from "../stores/stage.svelte";
  import { sessionStore } from "../stores/session.svelte";
  import type { Screen } from "../types";

  interface Props {
    /** `null` renders nothing. Set via `stageStore.openGate(...)`. */
    request: GateRequest | null;
    onClose: () => void;
    /** Wired from App.svelte — the run-mode "Open Session" button navigates
     * here once a `launched` row arrives. Only used for `stage === "run"`. */
    onNavigate?: (screen: Screen) => void;
  }
  let { request, onClose, onNavigate }: Props = $props();

  // Extracted from the imported union so the payload shape cannot drift from
  // events.rs by hand.
  type CheckEv = Extract<StageEvent, { kind: "check" }>;

  type Row =
    | { kind: "section"; title: string }
    | { kind: "line"; step: string | null; severity: Severity; text: string; remedy: string | null }
    | { kind: "text"; text: string }
    | { kind: "autoFixed"; step: string; description: string }
    | { kind: "needsAdmin"; step: string; reason: string }
    | { kind: "fatal"; message: string; remedy: string | null; fix: FixAction | null }
    | { kind: "check"; outcome: CheckEv["outcome"] }
    | { kind: "launched"; pid: number; logPath: string };

  /** One `StageEvent` -> the row it renders as, or `null` for the four kinds
   * that drive other UI (progress bar, console pane, runId, finished banner)
   * instead of a row of their own. */
  function toRow(ev: StageEvent): Row | null {
    switch (ev.kind) {
      case "section":
        return { kind: "section", title: ev.title };
      case "line":
        return { kind: "line", step: ev.step, severity: ev.severity, text: ev.text, remedy: ev.remedy };
      case "text":
        return { kind: "text", text: ev.text };
      case "autoFixed":
        return { kind: "autoFixed", step: ev.step, description: ev.description };
      case "needsAdmin":
        return { kind: "needsAdmin", step: ev.step, reason: ev.reason };
      case "fatal":
        return { kind: "fatal", message: ev.message, remedy: ev.remedy, fix: ev.fix };
      case "check":
        return { kind: "check", outcome: ev.outcome };
      case "launched":
        return { kind: "launched", pid: ev.pid, logPath: ev.logPath };
      default:
        return null;
    }
  }


  let runId = $state<string | null>(null);
  /** This request's runId announced by `stage://queued` - arrives (if at all)
   * while the run is still waiting on `OPERATION_LOCK`, before its own
   * `stageStarted` row exists, so Cancel has a target during that wait. */
  let queuedRunId = $state<string | null>(null);
  let rows = $state<Row[]>([]);
  let consoleLines = $state<string[]>([]);
  let consoleOpen = $state(false);
  let latestProgress = $state<{ label: string; current: number; total: number | null } | null>(null);
  let running = $state(false);
  let finished = $state<StageOutcome | null>(null);
  let invokeError = $state<string | null>(null);
  // Set when a `fatal` event already rendered the condition as a row: the
  // rejected promise that follows carries the same text verbatim, so showing
  // it again duplicates it. Invoke-layer rejections never set it.
  let sawFatal = $state(false);
  let confirmFix = $state<FixAction | null>(null);
  let fixBusy = $state(false);
  /** The in-flight fix's run id, from `applyFix`'s first `StageEvent`.
   * Emitted before the operation lock, so a queued fix has a `cancelStage`
   * target. Distinct from `runId` above, the stage's own run. */
  let fixRunId = $state<string | null>(null);
  let fixCancelling = $state(false);
  let fixError = $state<string | null>(null);
  /** Run-mode-only echo of a successful non-destructive fix's description —
   * the non-run path shows the same text as an inline `ok` row instead
   * (`rows` isn't rendered in run mode). */
  let fixNotice = $state<string | null>(null);

  let rowsEl: HTMLDivElement | null = $state(null);
  let consoleEl: HTMLPreElement | null = $state(null);
  /**
   * The request actually driving `rows`/`runId`/`sessionStore.launchRows`.
   * The `request` prop reflects `stageStore.gate`, which a new `openGate(...)`
   * can replace while this one is still running and merely hidden. The template
   * renders `displayRequest`, never `request`, so a queued replacement cannot
   * relabel the in-flight operation's title, rows, or Cancel target.
   */
  let activeRequest = $state<GateRequest | null>(null);
  /** Falls back to `request` only when nothing has started yet (first paint
   * of a fresh gate, before this effect has run) — otherwise always the
   * in-flight/just-finished operation. */
  const displayRequest = $derived(activeRequest ?? request);
  const isRunMode = $derived(displayRequest?.stage === "run");

  // Fresh `openGate(...)` calls always hand in a new object, so identity
  // comparison detects "a new run was requested". Run mode never calls
  // `start()`: `sessionStore.launch(...)` already ran and this component only
  // observes. A replacement arriving while `running` is deliberately deferred.
  $effect(() => {
    const r = request;
    if (r && r !== activeRequest && !running) {
      activeRequest = r;
      if (r.stage !== "run") {
        void start(r);
      } else {
        resetRunModeLocals();
      }
    }
    if (!r && !running) {
      activeRequest = null;
    }
  });

  $effect(() => {
    void rows.length;
    if (rowsEl) rowsEl.scrollTop = rowsEl.scrollHeight;
  });

  $effect(() => {
    void consoleLines.length;
    if (consoleOpen && consoleEl) consoleEl.scrollTop = consoleEl.scrollHeight;
  });

  // One modal instance, mounted once at the app root, and every opener
  // (Doctor, StagesPanel, Session) routes through it: with the process-wide
  // operation lock allowing at most one stage running or waiting, a
  // `stage://queued` event always belongs to whichever request is open.
  onMount(() => {
    let unlisten: (() => void) | undefined;
    void onStageQueued((q) => {
      if (displayRequest && q.stage === displayRequest.stage) {
        queuedRunId = q.runId;
      }
    }).then((fn) => {
      unlisten = fn;
    });
    return () => unlisten?.();
  });

  function reset() {
    runId = null;
    queuedRunId = null;
    rows = [];
    consoleLines = [];
    consoleOpen = false;
    latestProgress = null;
    finished = null;
    invokeError = null;
    sawFatal = false;
    confirmFix = null;
    fixError = null;
    fixRunId = null;
    fixCancelling = false;
  }

  function resetRunModeLocals() {
    queuedRunId = null;
    confirmFix = null;
    fixError = null;
    fixRunId = null;
    fixCancelling = false;
    fixNotice = null;
    autoCloseFired = false;
    if (autoCloseTimer) {
      clearTimeout(autoCloseTimer);
      autoCloseTimer = null;
    }
  }

  async function start(req: GateRequest) {
    reset();
    running = true;
    // Mirrored on the store so other openers (StagesPanel's Run/Dry-run,
    // Doctor's whole-stage Fix) disable themselves instead of racing a second
    // `openGate(...)` against this in-flight one.
    stageStore.setRunning(true);
    try {
      finished = await runStage(
        req.stage,
        { bottle: req.bottle, bsDir: req.bsDir, dryRun: req.dryRun },
        handleEvent,
      );
    } catch (e) {
      if (!sawFatal) invokeError = errMsg(e);
    } finally {
      running = false;
      stageStore.setRunning(false);
      req.onFinished?.();
    }
  }

  function pushConsole(chunk: string) {
    consoleLines.push(chunk);
    if (consoleLines.length > 2000) {
      consoleLines = consoleLines.slice(consoleLines.length - 2000);
    }
  }

  function handleEvent(ev: StageEvent) {
    switch (ev.kind) {
      case "stageStarted":
        runId = ev.runId;
        return;
      case "output":
        pushConsole(ev.chunk);
        return;
      case "progress":
        latestProgress = { label: ev.label, current: ev.current, total: ev.total };
        return;
      case "stageFinished":
        latestProgress = null;
        return;
      case "fatal":
        sawFatal = true;
        break;
      default:
        break;
    }
    const row = toRow(ev);
    if (row) rows.push(row);
  }

  async function cancel() {
    const id = runId ?? queuedRunId;
    if (id) await cancelStage(id);
  }

  function requestFix(action: FixAction) {
    const meta = FIX_META[action];
    if (meta.stage) {
      // A whole-stage remedy (e.g. "run setup" after a build failure): reopen
      // this same modal against that stage instead of a second dialog.
      stageStore.openGate({ stage: meta.stage, bottle: displayRequest?.bottle, bsDir: displayRequest?.bsDir });
      return;
    }
    if (meta.destructive) {
      confirmFix = action;
    } else {
      void doApplyFix(action);
    }
  }

  async function doApplyFix(action: FixAction) {
    confirmFix = null;
    fixBusy = true;
    fixError = null;
    fixNotice = null;
    fixRunId = null;
    fixCancelling = false;
    try {
      const report = await applyFix(
        action,
        { bottle: displayRequest?.bottle, bsDir: displayRequest?.bsDir },
        true,
        (ev) => {
          // The first event of this fix's own stream carries its run id;
          // capture it once so Cancel targets the fix, not the stage's `runId`.
          if (fixRunId === null) fixRunId = ev.runId;
          handleEvent(ev);
        },
      );
      if (isRunMode) {
        fixNotice = report.description;
      } else {
        rows.push({ kind: "line", step: null, severity: "ok", text: report.description, remedy: null });
      }
      if (report.changed && activeRequest && activeRequest.stage !== "run") {
        void start(activeRequest);
      }
    } catch (e) {
      fixError = errMsg(e);
    } finally {
      fixBusy = false;
      fixRunId = null;
      fixCancelling = false;
    }
  }

  async function cancelFix() {
    if (!fixRunId || fixCancelling) return;
    fixCancelling = true;
    try {
      await cancelStage(fixRunId);
    } finally {
      fixCancelling = false;
    }
  }


  const runRows = $derived.by((): Row[] => {
    if (!isRunMode) return [];
    const out: Row[] = [];
    for (const ev of sessionStore.launchRows) {
      const row = toRow(ev);
      if (row) out.push(row);
    }
    return out;
  });
  // `sessionStore.launchedEv`/`fatalEv`/`startedEv` are O(1) fields the store
  // captures in its own `launch()` callback, so these `$derived`s cost no scan
  // over `launchRows`. See that store's doc comments.
  const runLaunchedEv = $derived(isRunMode ? sessionStore.launchedEv : undefined);
  const runFatalEv = $derived(isRunMode ? sessionStore.fatalEv : undefined);
  // The launch's own `runId`, the id `cancelStage` takes.
  // Never `sessionStore.stop()` in this window: run holds `OPERATION_LOCK`
  // from `stageStarted` through `launched` (sabrage-core `stages`, "Lock
  // policy for `run`"), so a stop would block until this launch finishes.
  const runStartedEv = $derived(isRunMode ? sessionStore.startedEv : undefined);
  // A dry run (or any launch that settles with neither a `launched` nor a
  // `fatal` row) still needs a terminal state once the invocation resolves.
  const runDone = $derived(isRunMode && !sessionStore.launching && !runLaunchedEv && !runFatalEv);

  let cancelling = $state(false);
  async function cancelRun() {
    cancelling = true;
    try {
      if (runLaunchedEv) {
        // Unreachable while the template swaps Cancel for "Open Session" past
        // `launched`; kept as the correct fallback, since past Launched the run
        // itself is live, not a stage waiting on the lock.
        await sessionStore.stop();
      } else if (runStartedEv) {
        await cancelStage(runStartedEv.runId);
      } else if (queuedRunId) {
        // Queued behind another operation: no `stageStarted` row exists yet,
        // but `stage://queued` already announced this run's id.
        await cancelStage(queuedRunId);
      }
      // else: no run id has arrived and the run hasn't launched - nothing safe
      // to cancel; the button's `disabled` guard keeps this unreachable.
    } finally {
      cancelling = false;
    }
  }

  function retryLaunch() {
    if (displayRequest?.launch) void sessionStore.launch(displayRequest.launch);
  }

  let autoCloseFired = false;
  let autoCloseTimer: ReturnType<typeof setTimeout> | null = null;

  function openSessionNow() {
    if (autoCloseTimer) {
      clearTimeout(autoCloseTimer);
      autoCloseTimer = null;
    }
    onNavigate?.("session");
    onClose();
  }

  $effect(() => {
    if (isRunMode && runLaunchedEv && !autoCloseFired) {
      autoCloseFired = true;
      autoCloseTimer = setTimeout(() => {
        autoCloseTimer = null;
        openSessionNow();
      }, 1500);
    }
  });
</script>

{#snippet okIcon()}
  <StatusIcon kind="ok" size={13} />
{/snippet}
{#snippet failIcon()}
  <StatusIcon kind="fail" size={13} />
{/snippet}
{#snippet lockIcon()}
  <StatusIcon kind="lock" />
{/snippet}
{#snippet lineIcon(severity: Severity)}
  <StatusIcon kind={severity} size={13} />
{/snippet}
{#snippet checkIcon(status: CheckStatus)}
  <StatusIcon
    kind={status === "pass"
      ? "ok"
      : status === "warn" || status === "fail" || status === "info"
        ? status
        : "empty"}
    size={13}
  />
{/snippet}

{#snippet renderRow(row: Row)}
  {#if row.kind === "section"}
    <h6 class="gate-section">-- {row.title}</h6>
  {:else if row.kind === "line"}
    <div class="gate-row">
      <span class="icon">{@render lineIcon(row.severity)}</span>
      <div class="body">
        <div class="text" class:muted={row.severity === "info"}>{row.text}</div>
        {#if row.remedy}<div class="text-muted remedy">{row.remedy}</div>{/if}
      </div>
    </div>
  {:else if row.kind === "text"}
    <pre class="gate-text">{row.text.length ? row.text : " "}</pre>
  {:else if row.kind === "autoFixed"}
    <div class="gate-row">
      <span class="icon">{@render okIcon()}</span>
      <div class="body"><div class="text">{row.description}</div></div>
    </div>
  {:else if row.kind === "needsAdmin"}
    <div class="admin-note">
      {@render lockIcon()}
      <!-- `reason` (privilege.rs's `needs_admin_reason`) already names the
           mechanism picked and why - macOS authorization dialog or sudo in the
           launching terminal - so no static prompt text here: under sudo,
           reachable from `cargo tauri dev`, it would be wrong. -->
      <div>{row.reason}</div>
    </div>
  {:else if row.kind === "check"}
    <div class="gate-row" class:dim={row.outcome.status === "skipped" || row.outcome.status === "not_implemented"}>
      <span class="icon">{@render checkIcon(row.outcome.status)}</span>
      <div class="body">
        <div class="text">{row.outcome.message}</div>
        {#if row.outcome.remedy}<div class="text-muted remedy">{row.outcome.remedy}</div>{/if}
      </div>
    </div>
  {:else if row.kind === "launched"}
    <div class="gate-row">
      <span class="icon">{@render okIcon()}</span>
      <div class="body"><div class="text">Beat Saber launched — pid {row.pid} — log {row.logPath}</div></div>
    </div>
  {:else if row.kind === "fatal"}
    <div class="gate-row fatal-row">
      <span class="icon">{@render failIcon()}</span>
      <div class="body">
        <div class="text">{row.message}</div>
        {#if row.remedy}<div class="text-muted remedy">{row.remedy}</div>{/if}
      </div>
      {#if row.fix}
        <button
          class="btn btn-primary gate-fix-btn"
          title={FIX_META[row.fix].title}
          disabled={fixBusy}
          onclick={() => requestFix(row.fix!)}
        >
          {FIX_META[row.fix].stage ? `Open ${cap(FIX_META[row.fix].stage!)}` : "Fix"}
        </button>
      {/if}
    </div>
  {/if}
{/snippet}

{#if request}
  <div class="gate-backdrop">
    <div class="dialog blueprint elev-lg gate-dialog">
      <i class="corner tl"></i><i class="corner tr"></i><i class="corner bl"></i><i class="corner br"></i>
      <div>
        <div class="card-kicker">Pipeline</div>
        <div class="dialog-title gate-title">
          <span>{cap(displayRequest!.stage)}</span>
          {#if latestProgress}
            <span class="text-muted gate-progress">{latestProgress.label}</span>
          {/if}
        </div>
      </div>

      {#if latestProgress}
        <div class="progress-track">
          <div
            class="progress-fill"
            class:indeterminate={latestProgress.total == null}
            style={latestProgress.total != null
              ? `width:${Math.min(100, (latestProgress.current / latestProgress.total) * 100)}%`
              : undefined}
          ></div>
        </div>
      {/if}

      {#if isRunMode}
        <div class="gate-rows" bind:this={rowsEl}>
          {#if runRows.length === 0 && sessionStore.launching}
            <p class="text-muted gate-empty">Starting…</p>
          {/if}
          {#each runRows as row, i (i)}
            {@render renderRow(row)}
          {/each}
        </div>
      {:else}
        <div class="gate-rows" bind:this={rowsEl}>
          {#if rows.length === 0 && running}
            <p class="text-muted gate-empty">Starting…</p>
          {/if}
          {#each rows as row, i (i)}
            {@render renderRow(row)}
          {/each}
        </div>
      {/if}

      {#if confirmFix}
        <div class="confirm-inline">
          <div class="text-muted">
            {FIX_META[confirmFix].title} — {FIX_META[confirmFix].consequence ?? "this cannot be undone."} Continue?
          </div>
          <div class="confirm-actions">
            <button class="btn btn-secondary" onclick={() => (confirmFix = null)}>Cancel</button>
            <button class="btn btn-primary" onclick={() => doApplyFix(confirmFix!)}>Yes, continue</button>
          </div>
        </div>
      {/if}
      {#if fixBusy && fixRunId}
        <div class="confirm-inline">
          <div class="text-muted">
            Applying fix{fixCancelling ? " — cancelling…" : "…"} it can be cancelled while it waits or runs.
          </div>
          <div class="confirm-actions">
            <button class="btn btn-secondary" onclick={cancelFix} disabled={fixCancelling}>Cancel</button>
          </div>
        </div>
      {/if}
      {#if fixError}
        <div class="text-muted fix-error">Fix failed: {fixError}</div>
      {/if}
      {#if isRunMode && fixNotice}
        <div class="text-muted gate-outcome">{fixNotice}</div>
      {/if}

      {#if !isRunMode && consoleLines.length > 0}
        <button class="btn btn-ghost console-toggle" onclick={() => (consoleOpen = !consoleOpen)}>
          {consoleOpen ? "Hide" : "Show"} console output ({consoleLines.length} lines)
        </button>
        {#if consoleOpen}
          <pre class="console" bind:this={consoleEl}>{consoleLines.join("\n")}</pre>
        {/if}
      {/if}

      {#if !isRunMode && invokeError}
        <div class="gate-row fatal-row">
          <span class="icon">{@render failIcon()}</span>
          <div class="body"><div class="text">{invokeError}</div></div>
        </div>
      {/if}
      {#if isRunMode && sessionStore.lastError && !runFatalEv}
        <div class="gate-row fatal-row">
          <span class="icon">{@render failIcon()}</span>
          <div class="body"><div class="text">{sessionStore.lastError}</div></div>
        </div>
      {/if}

      {#if !isRunMode && finished}
        <div class="text-muted gate-outcome">
          {finished.ok
            ? `${cap(finished.stage)} completed.`
            : `${cap(finished.stage)} failed (exit ${finished.exitCodeEquiv}).`}
        </div>
      {/if}
      {#if isRunMode && runDone && sessionStore.lastOutcome}
        <div class="text-muted gate-outcome">
          {sessionStore.lastOutcome.ok
            ? "Run completed."
            : `Run failed (exit ${sessionStore.lastOutcome.exitCodeEquiv}).`}
        </div>
      {/if}

      <div class="dialog-actions">
        {#if isRunMode}
          {#if runLaunchedEv}
            <button class="btn btn-primary" onclick={openSessionNow}>Open Session</button>
          {:else if runFatalEv || runDone}
            {#if runFatalEv && displayRequest?.launch}
              <button class="btn btn-secondary" onclick={retryLaunch}>Retry</button>
            {/if}
            <button class="btn btn-primary" onclick={onClose}>Close</button>
          {:else}
            <button class="btn btn-ghost" onclick={onClose}>Hide</button>
            <button
              class="btn btn-secondary"
              onclick={cancelRun}
              disabled={cancelling || (!runStartedEv && !runLaunchedEv && !queuedRunId)}
            >
              {cancelling ? "Stopping…" : "Cancel"}
            </button>
          {/if}
        {:else if running}
          <button class="btn btn-ghost" onclick={onClose}>Hide</button>
          <button class="btn btn-secondary" onclick={cancel} disabled={!runId && !queuedRunId}>Cancel</button>
        {:else}
          <button class="btn btn-primary" onclick={onClose}>Close</button>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .gate-backdrop {
    position: fixed;
    inset: 0;
    z-index: 40;
    display: grid;
    place-items: center;
    background: color-mix(in srgb, var(--color-neutral-900) 45%, transparent);
  }
  .gate-dialog {
    width: 600px;
    max-width: calc(100vw - 32px);
    max-height: 86vh;
    overflow: auto;
    background: var(--color-bg);
  }
  .gate-title {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 12px;
  }
  .gate-progress {
    font-size: 12px;
    font-family: var(--font-body);
    font-weight: 400;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .progress-track {
    height: 3px;
    background: var(--color-divider);
    overflow: hidden;
  }
  .progress-fill {
    height: 100%;
    background: var(--color-accent);
    transition: width 0.2s ease;
  }
  .progress-fill.indeterminate {
    width: 30%;
    animation: gate-indeterminate 1.1s ease-in-out infinite;
  }
  @keyframes gate-indeterminate {
    0% {
      transform: translateX(-100%);
    }
    100% {
      transform: translateX(333%);
    }
  }
  .gate-rows {
    display: flex;
    flex-direction: column;
    max-height: 320px;
    overflow-y: auto;
  }
  .gate-empty {
    padding: 10px 4px;
    margin: 0;
  }
  .gate-section {
    margin: 10px 0 2px;
    color: var(--color-accent-700);
    font-size: 11.5px;
  }
  .gate-text {
    margin: 1px 4px;
    font-family: ui-monospace, Menlo, monospace;
    font-size: 12px;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .gate-row {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 5px 4px;
    border-bottom: 1px solid color-mix(in srgb, var(--color-text) 6%, transparent);
    transition: opacity 0.15s ease;
  }
  .gate-row.dim {
    opacity: 0.4;
  }
  .gate-row .icon {
    width: 16px;
    flex: none;
    padding-top: 2px;
    display: block;
  }
  .gate-row .body {
    flex: 1;
    min-width: 0;
  }
  .gate-row .text {
    font-size: 13px;
  }
  .gate-row .text.muted {
    color: color-mix(in srgb, var(--color-text) 60%, transparent);
  }
  .gate-row .remedy {
    font-size: 11.5px;
    margin-top: 1px;
  }
  .fatal-row {
    background: color-mix(in srgb, var(--color-accent-900) 8%, transparent);
  }
  .gate-fix-btn {
    flex: none;
    align-self: center;
    padding: 2px 14px;
    font-size: 12px;
  }
  .admin-note {
    display: flex;
    gap: 10px;
    align-items: center;
    border: 1px solid var(--color-divider);
    padding: 8px 12px;
    font-size: 12.5px;
    background: color-mix(in srgb, var(--color-accent) 8%, transparent);
  }
  .confirm-inline {
    border: 1px solid var(--color-divider);
    padding: 10px 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    font-size: 13px;
  }
  .confirm-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .fix-error {
    color: var(--color-accent-900);
    font-size: 12px;
  }
  .console-toggle {
    align-self: flex-start;
    font-size: 12px;
    padding: 2px 4px;
  }
  .console {
    margin: 0;
    max-height: 180px;
    overflow: auto;
    background: var(--color-surface);
    border: 1px solid var(--color-divider);
    padding: 8px 10px;
    font-family: ui-monospace, Menlo, monospace;
    font-size: 11px;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .gate-outcome {
    font-size: 12px;
  }
</style>
