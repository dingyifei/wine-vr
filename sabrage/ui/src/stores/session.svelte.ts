// The live session: launch state, the crash-recovery reconcile pass, and the
// app-quit dialog. A plain Svelte 5 rune store, module-scoped — the same
// shape as doctor.svelte.ts/stage.svelte.ts — but this one also owns two
// standing subscriptions (the `session://status` broadcast and
// `app://quit-requested`) set up once, here, for the app's lifetime: there is
// exactly one session store per app window, so there is nothing to
// unsubscribe on.

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
  type Reconciled,
  type SessionStatus,
  type StageEvent,
  type StageOutcome,
} from "../ipc";

function createSessionStore() {
  let status = $state<SessionStatus>(IDLE_SESSION_STATUS);

  // ── launch ───────────────────────────────────────────────────────────────
  // Owned here (not by GateModal or the Session screen) because both need to
  // observe the SAME in-flight launch: GateModal renders `launchRows` live,
  // the Session screen's Stop button and status card read `launching`, and a
  // Retry from a Fatal row calls `launch` again from whichever component the
  // user is looking at.

  let launching = $state(false);
  let launchRows = $state<StageEvent[]>([]);
  /** Wall-clock timestamp of the `"launched"` event, when one has arrived —
   * `sabrage_core::events::StageEvent::Launched`'s own `startedAtUnixMs`,
   * mirrored here for a consumer that wants "when did the game come up"
   * without scanning `launchRows` itself (the Session screen currently reads
   * the equivalent value off `status.startedAtUnixMs` instead, once the
   * broadcast catches up — this is the earlier, launch-local signal). */
  let launchedAt = $state<number | null>(null);
  let lastOutcome = $state<StageOutcome | null>(null);
  /** Set only on an `invoke()` rejection — a genuine IPC-layer failure (repo
   * root unresolved, a bad argument) rather than a reported `"fatal"` row,
   * which already appears in `launchRows`. Callers that also render
   * `launchRows` (GateModal) check for a `"fatal"` row first and treat this
   * as the fallback, exactly like the non-run stage path's `invokeError`/
   * `sawFatal` pair. */
  let lastError = $state<string | null>(null);
  /** The `gameId` this store's own most recent `launch()` call was given
   * (Library's Run button passes one; the plain Session screen's launches
   * omit it, leaving this `null`). `SessionStatus` has no `gameId` field —
   * this is the one place a caller can ask "was THIS session started for
   * that Library entry" without falling back to bottle-name equality, which
   * conflates every entry sharing a bottle. Kept (not cleared) once the
   * session ends, same as `status.bottle` staying populated through
   * `"exited"` — a consumer comparing against a fresh `launch()` call's own
   * `gameId` clears the ambiguity on its own. */
  let launchedGameId = $state<string | null>(null);

  /** The one place `status` is assigned. A session that turns live while no
   * `launch()` of ours is in flight was started elsewhere (demo.sh, a
   * re-attach at app start) — it cannot belong to the entry our last
   * `launch()` was given, so forget that id rather than let a stale value
   * pin "Running" on the wrong Library row. */
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
    lastOutcome = null;
    lastError = null;
    launchedGameId = opts.gameId ?? null;
    try {
      const outcome = await ipcLaunch(opts, (ev) => {
        launchRows.push(ev);
        if (ev.kind === "launched") {
          launchedAt = ev.startedAtUnixMs;
        }
      });
      lastOutcome = outcome;
      return outcome;
    } catch (e) {
      lastError = e instanceof Error ? e.message : String(e);
      throw e;
    } finally {
      launching = false;
    }
  }

  // ── stop / detach ────────────────────────────────────────────────────────

  /**
   * Stop the session. Every known call site (the Pipeline menu, GateModal's
   * Cancel, the Session screen's Stop button) already has a `SessionStatus`
   * in view and none of them track a bottle name or a run id separately, so
   * this is the one place that turns "stop whatever session is showing" into
   * the right IPC call:
   *
   * - While `status.phase` is `"preflight"` or `"launching"` (a `runId` is
   *   set): `run_stage(Stage::Run)` still holds `OPERATION_LOCK` through that
   *   whole window (`sabrage_core::stages`'s "Lock policy for `run`") and
   *   only releases it once the wine child is up — `stopSession` would block
   *   on that very lock until this same launch finishes on its own.
   *   `cancelStage(runId)` fires the run's own cancellation token instead,
   *   needing no lock at all — the same INT path GateModal's own Cancel
   *   button already takes while launching (see that component's
   *   `cancelRun`). The resolved `StageOutcome` here is synthetic, mirroring
   *   `stop_session`'s own INT-parity value for a live session
   *   (`exitCodeEquiv: 130`) — the still-pending `launch()` call is what
   *   actually streams every resulting row, onto `launchRows`, not `onEvent`.
   * - Otherwise: `stopSession(status.bottle, …)`, as before. `onEvent` is
   *   optional here too: when a session this process is supervising is
   *   already live (past `"launching"`), `stopSession` streams nothing new
   *   anyway — the Session screen's own Stop button passes one purely to
   *   show a local progress list next to the button for the *not*-supervised
   *   case, where the `Stop` stage runs for real and does emit rows.
   */
  async function stop(onEvent?: (ev: StageEvent) => void): Promise<StageOutcome> {
    if ((status.phase === "preflight" || status.phase === "launching") && status.runId) {
      await ipcCancelStage(status.runId);
      return { stage: "run", ok: true, exitCodeEquiv: 130 };
    }
    return ipcStopSession(status.bottle ?? null, onEvent ?? (() => {}));
  }

  /** Detach from the live session — the app-quit "leave it running" answer,
   * also reachable directly from the Session screen's Detach button.
   * Refreshes `status` immediately afterward (rather than waiting for the
   * next 1 Hz `session://status` tick) since `detach_session` flips
   * `session-state.json`'s `detached` flag synchronously and the caller's
   * next render should reflect `SessionPhase::Detached` without a visible
   * lag. */
  async function detach(): Promise<void> {
    await ipcDetachSession();
    await refreshStatus();
  }

  /** Re-poll `status` outside the 1 Hz broadcast cadence — a best-effort
   * refresh; a failure (no repo root resolved) just leaves whatever `status`
   * already held, same as the mount-time seed below. */
  async function refreshStatus(): Promise<void> {
    try {
      applyStatus(await getSessionStatus());
    } catch {
      // best-effort — see doc comment above
    }
  }

  // ── reconcile ────────────────────────────────────────────────────────────

  let reconcileResult = $state<Reconciled | null>(null);
  let reconcileRows = $state<string[]>([]);

  async function reconcile(bottle: string | null): Promise<ReconcileReport> {
    const report = await ipcReconcileSession(bottle);
    reconcileResult = report.kind;
    reconcileRows = report.rows;
    return report;
  }

  // ── quit ─────────────────────────────────────────────────────────────────

  let quitRequested = $state(false);

  async function resolveQuit(choice: QuitChoice): Promise<void> {
    try {
      await ipcResolveQuit(choice);
    } finally {
      // A no-op for "stop"/"keep" in practice — both exit the app from the
      // Rust side before this line is likely to run — but harmless, and the
      // one line that actually matters for "cancel".
      quitRequested = false;
    }
  }

  // ── standing subscriptions ───────────────────────────────────────────────
  // Seed with one poll, then let the 1 Hz broadcast take over. A seed failure
  // (no repo root resolved yet) just leaves the idle default in place — the
  // broadcast corrects it the moment a root resolves, same as every other
  // best-effort mount-time fetch in this codebase.
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
    get lastOutcome() {
      return lastOutcome;
    },
    get lastError() {
      return lastError;
    },
    get launchedGameId() {
      return launchedGameId;
    },
    get reconcileResult() {
      return reconcileResult;
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
