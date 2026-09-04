// The live session: launch state, the crash-recovery reconcile pass, and the
// app-quit dialog. A plain Svelte 5 rune store, module-scoped — the same
// shape as doctor.svelte.ts/stage.svelte.ts — but this one also owns two
// standing subscriptions (the `session://status` broadcast and
// `app://quit-requested`) set up once, here, for the app's lifetime: there is
// exactly one session store per app window, so there is nothing to
// unsubscribe on.

import { errMsg } from "../lib/text";
import {
  cancelStage as ipcCancelStage,
  detachSession as ipcDetachSession,
  getSessionStatus,
  IDLE_SESSION_STATUS,
  isLivePhase,
  launch as ipcLaunch,
  onQuitRequested,
  onSessionStatus,
  reconcileSession as ipcReconcileSession,
  resolveQuit as ipcResolveQuit,
  stopSession as ipcStopSession,
  type LaunchOpts,
  type QuitChoice,
  type ReconcileReport,
  type SessionStatus,
  type StageEvent,
  type StageOutcome,
} from "../ipc";

function createSessionStore() {
  let status = $state<SessionStatus>(IDLE_SESSION_STATUS);

  // Launch state lives here because both GateModal and the Session screen
  // observe the same in-flight launch: GateModal renders `launchRows` live,
  // the Session screen reads `launching`, and Retry calls `launch` from
  // either component.

  let launching = $state(false);
  let launchRows = $state<StageEvent[]>([]);
  /** Wall-clock timestamp of this launch's `"launched"` event; null from the
   * start of `launch()` until one arrives. The launch-local twin of
   * `status.startedAtUnixMs`, available before the `session://status`
   * broadcast catches up. */
  let launchedAt = $state<number | null>(null);
  /** The `"launched"`/`"fatal"`/`"stageStarted"` rows of this launch, captured
   * as they arrive so consumers read an O(1) field instead of re-scanning
   * `launchRows` on every event. */
  let launchedEv = $state<Extract<StageEvent, { kind: "launched" }> | null>(null);
  let fatalEv = $state<Extract<StageEvent, { kind: "fatal" }> | null>(null);
  let startedEv = $state<Extract<StageEvent, { kind: "stageStarted" }> | null>(null);
  let lastOutcome = $state<StageOutcome | null>(null);
  /** Set only on an `invoke()` rejection — an IPC-layer failure rather than a
   * reported `"fatal"` row, which appears in `launchRows`. Consumers that
   * render `launchRows` check for a `"fatal"` row first and use this as the
   * fallback. */
  let lastError = $state<string | null>(null);
  /** The `gameId` given to this store's most recent `launch()`, or null when
   * omitted. `SessionStatus` has no `gameId`, so this is the only way to
   * match a session to a Library entry without bottle-name equality, which
   * conflates entries sharing a bottle. Not cleared when the session ends. */
  let launchedGameId = $state<string | null>(null);

  /** The one place `status` is assigned. Clears `launchedGameId` when a
   * session turns live with no `launch()` of ours in flight: that session was
   * started elsewhere, so a stale id would pin "Running" on the wrong Library
   * row. */
  function applyStatus(next: SessionStatus): void {
    if (!launching && isLivePhase(next.phase) && !isLivePhase(status.phase)) {
      launchedGameId = null;
    }
    status = next;
  }

  async function launch(opts: LaunchOpts): Promise<StageOutcome> {
    launching = true;
    launchRows = [];
    launchedAt = null;
    launchedEv = null;
    fatalEv = null;
    startedEv = null;
    lastOutcome = null;
    lastError = null;
    launchedGameId = opts.gameId ?? null;
    try {
      const outcome = await ipcLaunch(opts, (ev) => {
        launchRows.push(ev);
        if (ev.kind === "launched") {
          launchedAt = ev.startedAtUnixMs;
          launchedEv = ev;
        } else if (ev.kind === "fatal") {
          fatalEv = ev;
        } else if (ev.kind === "stageStarted") {
          startedEv = ev;
        }
      });
      lastOutcome = outcome;
      return outcome;
    } catch (e) {
      lastError = errMsg(e);
      throw e;
    } finally {
      launching = false;
    }
  }


  /**
   * Stop whatever session `status` is showing.
   *
   * During `"preflight"`/`"launching"` with a `runId`, `stopSession` would
   * block on `run_stage`'s `OPERATION_LOCK`; `cancelStage(runId)` bypasses it.
   * See `sabrage_core::session::tests::stop_plan_decides_from_the_status_alone`.
   */
  async function stop(onEvent?: (ev: StageEvent) => void): Promise<StageOutcome> {
    if ((status.phase === "preflight" || status.phase === "launching") && status.runId) {
      await ipcCancelStage(status.runId);
      return { stage: "run", ok: true, exitCodeEquiv: 130 };
    }
    return ipcStopSession(status.bottle ?? null, onEvent ?? (() => {}));
  }

  /** Detach from the live session, leaving it running. Refreshes `status`
   * before returning so the next render shows `SessionPhase::Detached`
   * without waiting for the 1 Hz `session://status` tick. */
  async function detach(): Promise<void> {
    await ipcDetachSession();
    await refreshStatus();
  }

  /** Re-poll `status` outside the 1 Hz broadcast. Best-effort: a failure
   * leaves the previous `status` in place. */
  async function refreshStatus(): Promise<void> {
    try {
      applyStatus(await getSessionStatus());
    } catch {
      // best-effort: keep the previous status.
    }
  }

  // `report.kind` is not kept: only `rows` (the banner text) has a consumer.

  let reconcileRows = $state<string[]>([]);

  async function reconcile(bottle: string | null): Promise<ReconcileReport> {
    const report = await ipcReconcileSession(bottle);
    reconcileRows = report.rows;
    return report;
  }


  let quitRequested = $state(false);

  async function resolveQuit(choice: QuitChoice): Promise<void> {
    try {
      await ipcResolveQuit(choice);
    } finally {
      // Matters only for "cancel"; for "stop"/"keep" the Rust side normally
      // exits the app before this line runs.
      quitRequested = false;
    }
  }

  // Seed with one poll, then let the 1 Hz broadcast take over. A seed failure
  // (no repo root resolved yet) leaves the idle default in place; the
  // broadcast corrects it once a root resolves.
  void getSessionStatus()
    .then((s) => {
      applyStatus(s);
    })
    .catch(() => {});
  void onSessionStatus((s) => {
    applyStatus(s);
  });
  void onQuitRequested(() => {
    quitRequested = true;
  });

  return {
    get status() {
      return status;
    },
    get launching() {
      return launching;
    },
    get launchRows() {
      return launchRows;
    },
    get launchedAt() {
      return launchedAt;
    },
    /** This launch's own `"launched"` row, once it arrived — O(1). */
    get launchedEv() {
      return launchedEv;
    },
    /** This launch's own `"fatal"` row, once it arrived — O(1). */
    get fatalEv() {
      return fatalEv;
    },
    /** This launch's own `"stageStarted"` row, once it arrived — O(1). */
    get startedEv() {
      return startedEv;
    },
    get lastOutcome() {
      return lastOutcome;
    },
    get lastError() {
      return lastError;
    },
    get launchedGameId() {
      return launchedGameId;
    },
    get reconcileRows() {
      return reconcileRows;
    },
    get quitRequested() {
      return quitRequested;
    },
    launch,
    stop,
    detach,
    refreshStatus,
    reconcile,
    resolveQuit,
  };
}

/** Module-singleton store — one app window, one session. */
export const sessionStore = createSessionStore();
