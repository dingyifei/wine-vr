<script lang="ts">
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
    /** Bumped by App.svelte on every Pipeline ▸ Run Doctor menu firing
     * (⌘D) — mirrors Session's `launchRequest`. Unlike a plain navigation,
     * this must always trigger a fresh pass: whether Doctor is already the
     * open screen (`navigate` alone is then a no-op, so nothing would ever
     * remount and re-fire `onMount`'s pass) or was just re-navigated to with
     * `AUTORUN_STALE_MS` still fresh (which `onMount` would otherwise skip),
     * an explicit command must never be silently swallowed. */
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
   * Per-slug notice from the LAST fix run against that slug, keyed so it
   * survives the `runChecks()` repaint every `runFix` triggers in its
   * `finally` — a fix can emit a `warn` line (e.g. `RemoveAdbForwards` when
   * `adb forward --list` itself failed to query) or resolve `changed: false`
   * with an explanatory description while the *check* it is meant to fix
   * still reports the same (possibly falsely green) status; without this the
   * warning is visible for one tick and then overwritten by the very repaint
   * that was supposed to reflect it. Cleared the instant a new fix run
   * starts for that slug (`dismissFixNotice`, called from `runFix`'s top) —
   * a stale notice must never linger behind a fresh attempt — or when the
   * user dismisses it explicitly via the row's Dismiss control. */
  let fixNotices = $state<Record<string, string>>({});

  /** Drops slug's notice, if any — the row's Dismiss control calls this
   * directly, and `runFix` calls it before doing anything else so a notice
   * from a previous attempt never survives into a fresh one still in
   * flight (it only re-appears if the new run itself warns or resolves
   * unchanged). */
  function dismissFixNotice(slug: string) {
    if (!(slug in fixNotices)) return;
    const { [slug]: _dropped, ...rest } = fixNotices;
    fixNotices = rest;
  }

  let lastHandledDoctorRequest = $state(0);
  let doctorRequestNotice = $state<string | null>(null);
  /** Set once this mount's own `onMount` has made its freshness-based
   * autorun decision — the request-replay effect below must not act before
   * then (mirrors Session's `bottlePrefillDone`): reading `doctorStore.running`
   * or calling `runChecks()` earlier could race the very decision it needs
   * to respect (see the effect's doc comment for the ordering this relies
   * on). */
  let doctorAutorunDecided = $state(false);

  function pickDefaultBottle(bottles: string[], preferred: string | null): string {
    if (preferred && bottles.includes(preferred)) return preferred;
    return bottles.includes("Steam") ? "Steam" : (bottles[0] ?? "");
  }

  onMount(async () => {
    await doctorStore.loadBottles();
    selectedBottle = pickDefaultBottle(doctorStore.bottles, doctorStore.defaultBottle);
    // Skip the automatic pass when the last run is still fresh (e.g. flipping
    // to another screen and back) — "Run checks" is always available as the
    // explicit forced refresh.
    const fresh =
      doctorStore.hasRun &&
      doctorStore.lastRunAtMs != null &&
      Date.now() - doctorStore.lastRunAtMs < AUTORUN_STALE_MS;
    if (!fresh) void runChecks();
    // Only now — `runChecks()` above, if it fired, has already
    // (synchronously, before its own first `await`) set `doctorStore.running`
    // — does the request-replay effect become eligible to act, so it can
    // never start a second concurrent pass on top of this decision.
    doctorAutorunDecided = true;
  });

  // A menu-triggered Run Doctor (⌘D) always forces a fresh pass, bypassing
  // `AUTORUN_STALE_MS` — both when Doctor was already the open screen
  // (`navigate("doctor")` alone is a no-op then, so nothing above ever
  // remounts to re-run `onMount`'s pass) and when it wasn't but the just-
  // remounted screen's cache is still fresh. See `doctorAutorunDecided`'s
  // doc comment for why this waits on it.
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

  /** May Doctor mutate the machine right now? Every Fix is refused by the
   * backend while a session is live (`fixes::apply` -> `deny_if_session_live`)
   * — this only disables the button early so the user learns why before
   * clicking, instead of after a failed IPC round trip pulls a live session's
   * active adb forwards or clobbers a running install. */
  const sessionLive = $derived(blocksMutation(sessionStore.status.phase));

  /** A whole-stage fix opens the shared GateModal (`stageStore.openGate`),
   * the same singleton StagesPanel's Run/Dry-run drive — if one is already
   * running (possibly Hidden, so `stageStore.gate` reads `null` again) a
   * second whole-stage Fix here would silently queue behind it and only
   * display once the modal reopens, mislabelled. See the GateModal fix this
   * pairs with. */
  const stageRunning = $derived(stageStore.running);

  async function runChecks() {
    await doctorStore.run({ bottle: selectedBottle || null });
  }

  /** Handles a CheckRow's Fix button. Whole-stage fixes (`run-setup` /
   * `run-build` / `run-install`) open the shared GateModal so the run streams
   * exactly like a Pipeline-panel invocation; the rest run in place via
   * `fix()` and re-run doctor afterward — a destructive one confirms first. */
  function handleFixRequest(slug: string, fixId: string) {
    const action = contractFixIdToAction(fixId);
    if (!action) return;
    dispatchFix(slug, action);
  }

  /** The part of `handleFixRequest` that only needs an already-resolved
   * `FixAction` — shared with the error banner's "try the suggested fix"
   * button, whose `FixAction` comes straight off a `Fatal` event rather than
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
    // A failing fix announces itself as a `Fatal` event on this same stream
    // (message + structured remedy + a possible follow-up fix id) before the
    // `applyFix` promise ever rejects — capture it so the error state below
    // can show the same detail the GateModal shows for a stage's own Fatal,
    // instead of only the rejected promise's bare message.
    let fatal: FatalInfo | null = null;
    // A fix can warn without failing outright (e.g. `RemoveAdbForwards` when
    // `adb forward --list` itself couldn't be queried) — the check it is
    // meant to fix may keep reporting the same status regardless, so this is
    // the only place that ever learns something is still wrong.
    const warnings: string[] = [];
    try {
      const report = await applyFix(action, { bottle: selectedBottle || null }, true, (ev) => {
        // Every `StageEvent` carries `runId` — the first one to arrive is
        // this fix's own (see `applyFix`'s doc comment), captured once so a
        // Cancel affordance can target it even while still queued behind
        // another Sabrage operation.
        if (fixRunId === null) fixRunId = ev.runId;
        if (ev.kind === "fatal") {
          fatal = { message: ev.message, remedy: ev.remedy, fix: ev.fix };
        } else if (ev.kind === "line" && ev.severity === "warn") {
          warnings.push(ev.text);
        }
      });
      // `dismissFixNotice` above already cleared any stale notice for this
      // slug, so — unlike the `catch` branch below — there's no third case
      // to handle here: a clean, changed result simply leaves it cleared.
      if (warnings.length > 0) {
        fixNotices = { ...fixNotices, [slug]: warnings.join(" ") };
      } else if (!report.changed) {
        fixNotices = { ...fixNotices, [slug]: report.description };
      }
    } catch (e) {
      // `fatal` is only ever reassigned inside the callback above, so
      // TypeScript's control-flow narrowing — which can't prove that
      // callback ran before this line — types every read of it here as the
      // initializer's literal `null`, not the declared union. The cast back
      // to the declared type is what the runtime already knows to be true;
      // the truthiness check below still does the actual branching.
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
    // A rerun that rejected before reporting every slug must not read as
    // "Running checks…" forever — that text otherwise survives verbatim once
    // `running` flips false, with nothing else in the header saying doctor
    // actually failed.
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
  /* Warning styling, deliberately distinct from `.fix-error-banner`'s Fatal
     treatment below — a stale-adb-forwards notice is advisory, not a failed
     fix, so it uses the same neutral tone as the `warn` StatusIcon rather
     than the error banner's accent color. */
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
