//! `demo.sh run` — launch Beat Saber through the bridge.
//!
//! Reference: `scripts/demo/run.sh`. Unlike the other four stages
//! this one is a state machine: preflight and prepare, then the guarded region
//! (`guarded`) that the shell's traps cover, then exactly one `teardown`.
//!
//! Preflight and Prepare mutations are permanent and never unwound — the
//! `cxbottle.conf` backend fix, the helper restage, the adb forward
//! create/clear, the Goldberg swap — because run.sh installs its traps after
//! all of them (parity decision 17). Only the audio device and the dashboard
//! are guarded. A normal exit leaves the bottle's wineserver alive, as run.sh's
//! EXIT trap does; only the INT/TERM path calls `stop_wine`.
//!
//! [`run`] takes `Option<OperationGuard>` and drops it as soon as the wine
//! child is up (see [`crate::stages`]'s "Lock policy for `run`"); `None` means
//! the caller owns the lock. Every teardown runs against `teardown_ctx`, a
//! fresh token and a fresh executor, because [`crate::executor::RealExecutor`]
//! refuses to mutate once its token has fired and the cancellation teardown
//! still has to run `wineserver -k`, restore audio and close the dashboard.
//!
//! run.sh keeps no session record at all, so everything built on
//! `session-state.json` is Sabrage-only: the live-session refusal is declared
//! in PARITY.md § Session (detach / reconcile), "A recorded **Live** session",
//! and detach in that same section's "Cmd-Q on a live session" row. The
//! teardown token and the early lock release are Sabrage-only concurrency
//! constructs with no ledger row of their own.
//!
//! See tests::{a_normal_exit_prints_the_blank_line_then_the_status,
//! the_cancelled_path_announces_itself_stops_wine_and_exits_130,
//! detaching_marks_the_state_leaves_the_guards_and_keeps_the_file}.

pub mod actions;
pub mod guards;
pub mod preflight;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::error::{Result, SabrageError};
use crate::events::{step, RunId, StageEvent};
use crate::executor::{DetachedChild, Executor, RealExecutor};
use crate::paths::Bottle;
use crate::process::{self, ProcInfo};
use crate::session::state::{self, SessionState, WiredForward};
use crate::session::{self, reconcile::Reconciled, LiveSessionHandle, RunPhaseInfo, SessionPhase};
use crate::stages::{require_bottle, OperationGuard, StageCtx, STOP_WINESERVER_WAIT};

use guards::{AudioGuard, DashboardGuard};

/// How long the cancellation teardown waits for the wine child after
/// `wineserver -k` before giving up on reaping it.
///
/// The game dies *with* its wineserver, so this is a generous bound on an
/// event that normally lands in well under a second.
const WINE_EXIT_WAIT: Duration = Duration::from_secs(10);

/// The two config facts the preflight reads out of `oxrsys-runtime.toml`, and
/// which later steps branch on.
///
/// Both are the raw strings from the file — unrecognized values are *not*
/// normalized away, because run.sh warns about them by name and then treats
/// them as the default (`encoder_process` unknown → warn, then behave as
/// `auto`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightFacts {
    /// `protocol` — `"alvr"` (the demo path) or `"oxrsys"` (legacy USB).
    /// Gates audio routing, the dashboard, and the adb-reverse branch.
    pub protocol: String,
    /// `encoder_process` — `"auto"` (the default when the key is absent),
    /// `"native"`, or `"inproc"`. Decides whether the arm64 helper is
    /// required.
    pub encoder_process: String,
}

/// Execute the stage; returns **wine's exit status**, which `./demo.sh run`
/// exits with verbatim (`rc=$?; … exit $rc`).
///
/// `lock` is the operation guard to release at the launch boundary, or `None`
/// when the caller holds the lock itself. Releasing it is not optional: a
/// session lasts hours, and `stop` must stay reachable throughout.
pub async fn run(ctx: &StageCtx, lock: Option<OperationGuard>) -> Result<i32> {
    // Before anything else, and with lib.sh's die text.
    let bottle = require_bottle(ctx)?.clone();

    // Nothing else can report `Preflight`/`Launching`/`Stopping` this early —
    // there is no live handle yet and no state file — and the RAII guard makes
    // every early return end in a cleared slot rather than a phase that outlives
    // its launch.
    // tests::run_publishes_preflight_and_clears_it_when_the_preflight_fails
    let mut phase = RunPhaseScope::new(ctx.run_id, &bottle.name);

    // The live-session gate (A8-1): asked before this launch’s own `Preflight`
    // (cannot self-block through `session_block_at`’s run-phase arm) and before
    // `reconcile`, which cannot see the `runtime_status.json` only `./demo.sh run` produces.
    // tests::a_shell_started_session_refuses_the_launch_before_anything_permanent
    if let Some(block) = launch_block(ctx) {
        let owner = block.bottle.as_deref().unwrap_or(&bottle.name);
        return Err(refuse_launch(
            ctx,
            "a session is already running",
            &block.reason,
            owner,
        ));
    }

    // Sabrage-only, and deliberately *before* anything permanent — the
    // preflight's two auto-fixes included — because PARITY.md § Session
    // (detach / reconcile), "A recorded **Live** session" promises that a
    // launch refused for a live session changed nothing.
    phase.publish(SessionPhase::Preflight);
    let reconciled = session::reconcile::reconcile(ctx).await?;
    if let Reconciled::Live { state } = &reconciled {
        return Err(already_running(ctx, &bottle, state));
    }
    // A record that is not ours to *touch* is not ours to launch over either
    // (A9-1 / A9-8): falling through reaches `wineserver_reset`, which takes
    // down the very session the classification exists to protect. `silent:
    // false` is the whole of "somebody else's" — the one silent shape is this
    // process's own in-flight launch.
    // tests::a_record_another_live_front_end_owns_refuses_the_launch
    if let Reconciled::Busy {
        state,
        reason,
        silent: false,
    } = &reconciled
    {
        return Err(refuse_launch(
            ctx,
            "refusing to launch over the previous session record",
            reason,
            &state.bottle,
        ));
    }
    // A record that survived reconciliation still names guards nobody could
    // release — an output device nothing could switch back to, `--wired`
    // forwards `adb` would not remove. Carry them forward instead of letting
    // this launch overwrite the record that describes them (see [`Carried`]).
    let carried = match &reconciled {
        Reconciled::Dead { state, .. } | Reconciled::IdentityMismatch { state, .. } => {
            carry_forward(state)
        }
        // `NoSession`, `Busy` and `Live` — the last two never reach here, the
        // launch having already refused.
        _ => Carried::default(),
    };
    checkpoint(ctx)?;

    let facts = preflight::run(ctx).await?;
    checkpoint(ctx)?;

    // Everything from here to the guards is permanent: run.sh installs no
    // trap until after all of it.
    let state_path = ctx.paths.session_state_path();
    let mut sess = SessionState::new(
        ctx.run_id,
        &bottle.name,
        &ctx.bs_dir,
        PathBuf::new(),
        session::now_unix_ms(),
    );
    sess.prev_audio_output = carried.audio;
    sess.wired_forwards = carried.forwards;

    // Recorded as each forward is made (session::state's invariant table, row
    // 3), inside `adb_forward_hygiene` itself: a `--wired` launch that never
    // reaches teardown still leaves the forwards on disk for the next
    // reconcile to remove.
    actions::adb_forward_hygiene(ctx, &mut sess, &state_path).await?;
    // A carried-forward forward and one this launch just made can name the
    // same `(serial, port)`; recording it twice only buys a second
    // `adb forward --remove` that finds nothing. The next save (the guards',
    // or the launch record's) writes the deduplicated list.
    dedup_forwards(&mut sess.wired_forwards);
    checkpoint(ctx)?;
    actions::wineserver_reset(ctx, &bottle).await?;
    checkpoint(ctx)?;
    actions::goldberg_stage(ctx).await?;
    checkpoint(ctx)?;

    // Everything above was `Preflight` by [`SessionPhase::Preflight`]'s own
    // definition ("checks, wineserver reset, Goldberg"); the guards and the
    // spawn are `Launching`. Once the wine child is up, `guarded` clears the
    // slot and the live handle carries the phase from there.
    phase.publish(SessionPhase::Launching);
    let mut held = Guards::default();
    let outcome = guarded(
        ctx,
        &bottle,
        &facts,
        &mut sess,
        &state_path,
        &mut held,
        lock,
    )
    .await;
    teardown(
        ctx,
        &bottle,
        &mut held,
        &mut sess,
        &state_path,
        outcome,
        &mut phase,
    )
    .await
}

/// `Err(`[`SabrageError::Cancelled`]`)` the moment `ctx.cancel` has fired.
///
/// Called between every preparation action, the way `stop` checkpoints between
/// its steps: the actions themselves spawn children that notice cancellation,
/// but `goldberg_stage` on an already-staged install performs no child at all
/// and would otherwise run straight into the launch.
fn checkpoint(ctx: &StageCtx) -> Result<()> {
    if ctx.cancel.is_cancelled() {
        return Err(SabrageError::Cancelled);
    }
    Ok(())
}

/// The output device a reconciled record still has pending, if any.
///
/// [`session::reconcile::finish_record`](crate::session::reconcile) keeps
/// `session-state.json` when a guard could not be released, and carrying the
/// name forward lets the retry that record was kept for actually happen
/// (r1:A9-2; tests::a_kept_records_forwards_travel_into_the_next_launch).
/// A record whose restore succeeded has `audio_restored` set and carries nothing.
fn unfinished_audio_restore(state: &SessionState) -> Option<String> {
    // The kept-record condition, asked of the state rather than of the
    // outcome enum: a restore that succeeded sets the flag, and that record is
    // already gone.
    if state.guards.audio_restored {
        return None;
    }
    state.prev_audio_output.clone()
}

/// Everything a reconciled-but-kept record hands to the launch that replaces
/// it.
///
/// The record is the *only* description of a guard nothing could release, and
/// the next launch overwrites it (`SessionState::new` + the first save). Every
/// outstanding guard therefore has to travel into the new record, or the
/// retry the record was kept for can never happen (A9-2).
#[derive(Debug, Default, PartialEq, Eq)]
struct Carried {
    /// The output device whose restore never finished — see
    /// [`unfinished_audio_restore`].
    audio: Option<String>,
    /// `--wired` forwards still on a device because `adb forward --remove`
    /// failed. Without these the exact `(serial, port)` pairs are lost, and a
    /// stale forward is what silently breaks the next WiFi run.
    forwards: Vec<WiredForward>,
}

/// [`Carried`] out of one reconciled record.
fn carry_forward(state: &SessionState) -> Carried {
    Carried {
        audio: unfinished_audio_restore(state),
        // Same shape as the audio flag: a removal that succeeded set
        // `forwards_cleared` (and emptied the list), and carries nothing.
        forwards: if state.guards.forwards_cleared {
            Vec::new()
        } else {
            state.wired_forwards.clone()
        },
    }
}

/// Drop repeated `(serial, port)` pairs, keeping the first of each.
fn dedup_forwards(forwards: &mut Vec<WiredForward>) {
    let mut seen: Vec<(String, u16)> = Vec::new();
    forwards.retain(|f| {
        let key = (f.serial.clone(), f.port);
        if seen.contains(&key) {
            return false;
        }
        seen.push(key);
        true
    });
}

/// [`session::live_session_block`] as the *launch* asks it: every signal
/// except the record on disk, which is [`session::reconcile`]'s to read.
///
/// The empty state path is how that is said. Nothing is lost by it — a `Live`
/// record refuses through [`already_running`] and every untouchable one
/// through the `Busy` arm — and leaving the record out of this gate preserves
/// the one behaviour `reconcile` documents by name: an unreadable record
/// warns and is explained, it does not block every future launch.
fn launch_block(ctx: &StageCtx) -> Option<session::SessionBlock> {
    session::session_block_at(
        Path::new(""),
        &ctx.paths.oxr_appsup.join("runtime_status.json"),
    )
}

/// A launch refusal in the shape [`already_running`] established: what is in
/// the way, why, and both routes out of it.
fn refuse_launch(ctx: &StageCtx, headline: &str, reason: &str, bottle: &str) -> SabrageError {
    ctx.fatal(
        format!(
            "{headline} — {reason} — stop it first: Stop in Sabrage, or \
             ./demo.sh stop --bottle {bottle}"
        ),
        None,
    )
}

/// Publishes [`session::RunPhaseInfo`] for the three phases only this stage
/// can know about, and guarantees the slot is emptied however [`run`] leaves.
///
/// Without it [`crate::session::watcher::SessionMonitor::snapshot`] has nothing
/// to report between the stage starting and the wine child being published, so
/// a launch reads as “No session” for its entire preflight. Every publication
/// names its `run_id` and bottle — without them the Session screen offers a
/// Stop that takes the operation lock then dies on “bottle name required” —
/// and `Drop` clears the slot only while it belongs to this run, since `run`
/// releases the lock at the launch boundary and a detached or cancelled run
/// can still be unwinding while the next launch publishes its own `Preflight`.
/// [`finalize_exited`](Self::finalize_exited) is the one publication meant to
/// outlive `run`, so the Session screen can say “Exited (code N)”.
///
/// See tests::{the_scope_publishes_identity_and_drop_clears_only_its_own_run,
/// a_normal_teardown_reports_stopping_then_a_surviving_exited_code}.
struct RunPhaseScope {
    run_id: RunId,
    bottle: String,
    finalized: bool,
}

impl RunPhaseScope {
    fn new(run_id: RunId, bottle: &str) -> RunPhaseScope {
        RunPhaseScope {
            run_id,
            bottle: bottle.to_string(),
            finalized: false,
        }
    }

    /// Publish `phase` with this run's identity. Never carries an exit code —
    /// only [`finalize_exited`](Self::finalize_exited) does.
    fn publish(&self, phase: SessionPhase) {
        session::publish_run_phase(Some(RunPhaseInfo {
            phase,
            run_id: self.run_id,
            bottle: self.bottle.clone(),
            exit_code: None,
        }));
    }

    /// Publish the terminal `Exited(rc)` **and stop `Drop` from taking it
    /// back**: this is the one phase that survives [`run`] returning.
    fn finalize_exited(&mut self, rc: i32) {
        session::publish_run_phase(Some(RunPhaseInfo {
            phase: SessionPhase::Exited,
            run_id: self.run_id,
            bottle: self.bottle.clone(),
            exit_code: Some(rc),
        }));
        self.finalized = true;
    }
}

impl Drop for RunPhaseScope {
    fn drop(&mut self) {
        if !self.finalized {
            session::clear_run_phase(self.run_id);
        }
    }
}

/// The guards this run holds, released in run.sh's trap order.
#[derive(Default)]
struct Guards {
    audio: Option<AudioGuard>,
    dashboard: Option<DashboardGuard>,
}

impl Guards {
    /// `stop_dashboard; stop_helper; restore_audio` — run.sh's EXIT/INT/TERM
    /// trap body, in that order.
    ///
    /// Every guard is attempted, and the first error is *reported* rather than
    /// short-circuited: the shell's trap body is three unconditional commands,
    /// and a `?` on the dashboard's `session-state.json` write would otherwise
    /// leave the audio device on BlackHole — the one mutation the user can
    /// actually hear.
    async fn release(&mut self, ctx: &StageCtx, sess: &mut SessionState) -> Result<()> {
        let mut failure: Option<SabrageError> = None;
        if let Some(d) = self.dashboard.take() {
            if let Err(e) = d.release(ctx, sess).await {
                failure.get_or_insert(e);
            }
        }
        reap_helper(ctx).await;
        if let Some(a) = self.audio.take() {
            if let Err(e) = a.release(ctx, sess).await {
                failure.get_or_insert(e);
            }
        }
        match failure {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// Detach: forget both guards without undoing either.
    fn disarm(&mut self) {
        if let Some(d) = self.dashboard.take() {
            d.disarm();
        }
        if let Some(a) = self.audio.take() {
            a.disarm();
        }
    }
}

/// How supervision ended.
enum Supervised {
    Exited(i32),
    Cancelled,
    Detached,
}

/// What [`teardown`] has to unwind.
enum Reason {
    /// Wine exited on its own; `rc` is its status.
    Normal { rc: i32, log: PathBuf },
    /// The INT path. `child` is `Some` only when a wine child was actually up.
    Cancelled { child: Option<DetachedChild> },
    /// The detach token fired: leave everything running.
    Detached { log: PathBuf },
    /// `--dry-run`: nothing was launched, nothing supervised.
    DryRun,
    /// Anything else went wrong; the guards still have to come off.
    Failed(SabrageError),
}

/// The undoable half of run.sh's launch: from its trap installation to
/// `wait $WINE_PID`.
///
/// Everything this function does is undoable, and everything above it is not.
/// It never tears anything down itself — `teardown` owns every exit path, so
/// there is exactly one place the guards come off.
#[allow(clippy::too_many_arguments)]
async fn guarded(
    ctx: &StageCtx,
    bottle: &Bottle,
    facts: &PreflightFacts,
    sess: &mut SessionState,
    state_path: &Path,
    held: &mut Guards,
    lock: Option<OperationGuard>,
) -> Result<Reason> {
    // In two halves on purpose (A8-3): the guard is installed HERE, before
    // `apply_switch` runs the child that can come back `Cancelled` with
    // CoreAudio already changed — so the switch unwinds through `teardown`,
    // which sets `guards.audio_restored` and saves, rather than through `Drop`,
    // which restores the device but can record neither.
    held.audio = Some(AudioGuard::arm(ctx, facts, sess).await?);
    if let Some(audio) = held.audio.as_mut() {
        audio.apply_switch(ctx, sess).await?;
    }
    if ctx.cancel.is_cancelled() {
        return Ok(Reason::Cancelled { child: None });
    }
    held.dashboard = Some(DashboardGuard::acquire(ctx, facts, sess).await?);
    if ctx.cancel.is_cancelled() {
        return Ok(Reason::Cancelled { child: None });
    }
    actions::adb_reverse_cleanup(ctx, facts).await?;
    if ctx.cancel.is_cancelled() {
        return Ok(Reason::Cancelled { child: None });
    }

    let Some((child, log)) = actions::launch_wine(ctx, bottle).await? else {
        // `--dry-run`: the launch was planned, not performed. No session, no
        // supervision, and the inert guards still come off in `teardown`.
        return Ok(Reason::DryRun);
    };

    let identity = child.identity.clone();
    sess.wine = Some(identity.clone());
    sess.log_path = log.clone();

    // The game becomes reachable HERE: `spawn_detached` sets
    // `kill_on_drop(false)`, so from the spawn until this line a running Beat
    // Saber is reachable by *nothing* — no live handle, no record on disk.
    // Publishing the handle is infallible and in-process, so it goes first and
    // everything fallible follows it.
    let detach = CancellationToken::new();
    session::set_live_session(LiveSessionHandle {
        run_id: ctx.run_id,
        bottle: bottle.name.clone(),
        identity: identity.clone(),
        log_path: log.clone(),
        started_at_unix_ms: sess.started_at_unix_ms,
        // The INT path — `stop_session()` and app-quit fire this one.
        cancel: ctx.cancel.clone(),
        // Walk away and leave the guards in place — a different thing entirely.
        detach: detach.clone(),
    });

    // …and with the handle up, the run stage has nothing left to say: the
    // live session carries the phase (and a richer identity — pid, log path)
    // from here until teardown publishes `Stopping`. Run-id guarded, so a
    // previous run still unwinding cannot clear this one's slot instead.
    session::clear_run_phase(ctx.run_id);

    // Only now the disk record — and only best effort (see the function).
    record_launched_session(ctx, state_path, sess).await;

    ctx.emit(StageEvent::Launched {
        run_id: ctx.run_id,
        pid: identity.pid,
        start_time: identity.start_time,
        log_path: log.display().to_string(),
        started_at_unix_ms: sess.started_at_unix_ms,
    });

    // A session lasts hours; holding the operation lock through it would block
    // `stop` exactly when the user reaches for it. See `crate::stages`'s "Lock
    // policy for `run`". `None` means the caller owns it and it is not ours.
    drop(lock);

    // run.sh's `wait $WINE_PID`.
    let mut proc = child.child;
    // `biased` on purpose: an unbiased `select!` picks at random among ready
    // branches, and a Stop losing that coin toss to a Detach would disarm the
    // guards and leave the game running while `stop_session` reports success.
    // Stop is terminal here — cancel is checked first and the detach arm
    // re-checks the token below — so a Stop that fired at ANY point wins.
    let how = tokio::select! {
        biased;
        _ = ctx.cancel.cancelled() => Supervised::Cancelled,
        status = proc.wait() => Supervised::Exited(
            status.map(process::exit_code_of).unwrap_or(1),
        ),
        _ = detach.cancelled() => Supervised::Detached,
    };
    let cancelled = |identity: ProcInfo, proc: tokio::process::Child| Reason::Cancelled {
        child: Some(DetachedChild {
            identity,
            child: proc,
        }),
    };
    Ok(match how {
        Supervised::Exited(rc) => Reason::Normal { rc, log },
        Supervised::Cancelled => cancelled(identity, proc),
        // Dropping the handle does not touch the process: `spawn_detached` sets
        // `kill_on_drop(false)` precisely so this is a clean walk-away — unless
        // a Stop is already in flight, in which case this is that Stop.
        Supervised::Detached if ctx.cancel.is_cancelled() => cancelled(identity, proc),
        Supervised::Detached => Reason::Detached { log },
    })
}

/// Write `session-state.json` for a launch that is **already running and
/// already published** — deliberately best effort.
///
/// The file exists for a *later* process: a reconcile after a crash, a second
/// Sabrage, `./demo.sh stop`. Propagating a write failure would unwind out of
/// `guarded` with Beat Saber running and no way to stop it from Sabrage, so
/// this warns and carries on supervising.
///
/// See tests::a_failed_state_write_warns_instead_of_orphaning_the_running_game.
async fn record_launched_session(ctx: &StageCtx, state_path: &Path, sess: &SessionState) {
    if let Err(e) = state::save(&*ctx.executor, state_path, sess).await {
        ctx.step(step::RUN_LAUNCH).warn(format!(
            "could not record the session in {} ({e}) — Stop still works here, \
             but a Sabrage restart will not find this session",
            state_path.display()
        ));
    }
}

/// The one place the guards come off, for every exit path.
async fn teardown(
    ctx: &StageCtx,
    bottle: &Bottle,
    held: &mut Guards,
    sess: &mut SessionState,
    state_path: &Path,
    outcome: Result<Reason>,
    phase: &mut RunPhaseScope,
) -> Result<i32> {
    // A cancelled run's own executor refuses to act; teardown needs one that
    // does not. See this module's header.
    let tctx = teardown_ctx(ctx);

    let reason = match outcome {
        Ok(r) => r,
        // A cancellation raised from inside the guarded region is the same
        // event the token would have signalled — run.sh's trap fires either
        // way, wine child or not.
        Err(SabrageError::Cancelled) => Reason::Cancelled { child: None },
        Err(e) => Reason::Failed(e),
    };

    // Teardown is a phase of its own and has to be visible *before*
    // `clear_live_session` runs, which is why published `Stopping` outranks a
    // live handle in `snapshot()`'s precedence table. Not `Detached` (that
    // phase is derived from the state file the arm leaves behind) and not
    // `DryRun`, where nothing ran.
    // tests::a_normal_teardown_reports_stopping_then_a_surviving_exited_code
    if matches!(
        reason,
        Reason::Normal { .. } | Reason::Cancelled { .. } | Reason::Failed(_)
    ) {
        phase.publish(SessionPhase::Stopping);
    }

    match reason {
        Reason::Detached { log } => {
            // Leak the guards on purpose: the dashboard stays open, the audio
            // device stays on BlackHole, and `session-state.json` keeps
            // describing both so a later reconcile can finish the job. The
            // record goes down FIRST — a write that fails keeps the guards
            // armed, because disarming before the write left the device on
            // BlackHole with nothing on disk naming it.
            // tests::a_detach_that_cannot_write_its_record_keeps_the_guards_armed
            sess.detached = true;
            match state::save(&*tctx.executor, state_path, sess).await {
                Ok(()) => held.disarm(),
                Err(e) => tctx.step(step::RUN_TEARDOWN).warn(format!(
                    "could not record the detached session in {} ({e}) — the guards will be \
                     released instead of left in place",
                    state_path.display()
                )),
            }
            // Detach is the one way supervision ends without a teardown, so
            // the announcement belongs to `step::RUN_SUPERVISE`, the step that
            // was running, not to a guard release that never happens here.
            // tests::the_detach_row_belongs_to_the_supervise_step
            tctx.emit(StageEvent::text(
                ctx.run_id,
                Some(step::RUN_SUPERVISE),
                detached_line(&log),
            ));
            session::clear_live_session(ctx.run_id);
            // No phase published: `Detached` is derived from the
            // `session-state.json` this arm just wrote, and the slot is
            // already empty (`guarded` cleared it at the launch boundary).
            Ok(0)
        }

        Reason::Normal { rc, log } => {
            // The two closing prints come FIRST: run.sh's EXIT trap does not
            // fire until `exit $rc`, below both, so `audio: restored output ->
            // …` and `dashboard: closed` land *after* the status line, never
            // before it.
            // tests::a_normal_exits_guards_come_off_after_the_status_line_not_before_it
            tctx.emit(StageEvent::text(
                ctx.run_id,
                Some(step::RUN_TEARDOWN),
                String::new(),
            ));
            // This publication is the one that OUTLIVES `run` (see
            // [`RunPhaseScope::finalize_exited`]), so the Session screen can
            // show "Exited (code N)" until the next launch publishes over it.
            phase.finalize_exited(rc);
            tctx.emit(StageEvent::text(
                ctx.run_id,
                Some(step::RUN_TEARDOWN),
                wine_exit_line(rc, &log),
            ));
            // That EXIT trap: no `stop_wine`. The bottle's wineserver stays up
            // on a clean quit — `./demo.sh stop` is what kills it.
            //
            // #202: both calls are best effort. A `?` would skip
            // `clear_live_session`, leaking the handle so the next
            // `stop_session` burns its 30 s timeout on an already-fired token,
            // and would turn a clean quit into exit 1 — but wine has already
            // exited with `rc`, and that is the number to report.
            // tests::a_normal_exit_survives_a_failed_state_save
            if let Err(e) = held.release(&tctx, sess).await {
                tctx.step(step::RUN_TEARDOWN).warn(format!(
                    "could not save the session record while releasing the guards ({e}) — \
                     the guards came off, but a later Sabrage may still see this session on disk"
                ));
            }
            if let Err(e) = finish_record(&tctx, state_path, sess).await {
                tctx.step(step::RUN_TEARDOWN).warn(format!(
                    "could not update {} ({e}) — a later Sabrage may offer to reconcile a \
                     session that has already exited",
                    state_path.display()
                ));
            }
            session::clear_live_session(ctx.run_id);
            Ok(rc)
        }

        Reason::Cancelled { child } => {
            // run.sh's INT trap, verbatim and in order.
            tctx.emit(StageEvent::text(
                ctx.run_id,
                Some(step::RUN_TEARDOWN),
                String::new(),
            ));
            tctx.section(INT_TEARDOWN_LINE);
            stop_wine(&tctx, bottle).await;
            // Best effort, exactly as the `Normal` arm above is (#202): the
            // shell's INT trap runs every one of its commands and only then
            // re-signals itself, so a failed `session-state.json` write must
            // not skip the reap, the record or the live handle — leaking the
            // handle costs the next `stop_session` its whole 30 s timeout.
            // tests::a_cancelled_teardown_survives_a_failed_state_save
            if let Err(e) = held.release(&tctx, sess).await {
                tctx.step(step::RUN_TEARDOWN).warn(format!(
                    "could not save the session record while releasing the guards ({e}) — \
                     the guards came off, but a later Sabrage may still see this session on disk"
                ));
            }
            if let Some(mut c) = child {
                // The game dies with its wineserver; reap it rather than
                // leaving a zombie behind.
                let _ = tokio::time::timeout(WINE_EXIT_WAIT, c.child.wait()).await;
            }
            if let Err(e) = finish_record(&tctx, state_path, sess).await {
                tctx.step(step::RUN_TEARDOWN).warn(format!(
                    "could not update {} ({e}) — a later Sabrage may offer to reconcile a \
                     session that has already been stopped",
                    state_path.display()
                ));
            }
            session::clear_live_session(ctx.run_id);
            // The shell resignals itself so the process exits 130; the same
            // number, reached through `SabrageError::Cancelled::exit_code`.
            Err(SabrageError::Cancelled)
        }

        Reason::DryRun => {
            // Nothing ran, so nothing here can fail in a way worth an exit
            // code — but the live handle must be cleared either way.
            let released = held.release(&tctx, sess).await;
            session::clear_live_session(ctx.run_id);
            released.map(|()| 0)
        }

        Reason::Failed(e) => {
            // Best effort: a second failure here must not mask the first.
            let _ = held.release(&tctx, sess).await;
            let _ = finish_record(&tctx, state_path, sess).await;
            session::clear_live_session(ctx.run_id);
            Err(e)
        }
    }
}

/// A context whose executor is not bound to `ctx.cancel`.
///
/// A dry run reuses `ctx` unchanged, so the teardown's planned actions land in
/// the same plan the front-ends print. A real run gets a fresh
/// [`RealExecutor`] on a fresh token — see this module's header.
fn teardown_ctx(ctx: &StageCtx) -> StageCtx {
    if ctx.executor.is_dry_run() {
        return ctx.clone();
    }
    let cancel = CancellationToken::new();
    let executor: Arc<dyn Executor> = Arc::new(RealExecutor::new(
        ctx.run_id,
        ctx.sink.clone(),
        cancel.clone(),
    ));
    StageCtx {
        cancel,
        executor,
        ..ctx.clone()
    }
}

/// `lib.sh`'s `stop_wine`, on the 4 s advisory budget
/// ([`crate::stages::STOP_WINESERVER_WAIT`] — deliberately *not* the 5 s fatal
/// one this stage's own reset uses).
///
/// Reference: `scripts/demo/lib.sh`. Duplicates `stages::stop`'s function of
/// the same name: both are module-private, and the two differ in step id and
/// in return type (that one propagates, this one is best effort).
async fn stop_wine(ctx: &StageCtx, bottle: &Bottle) {
    let Some(wineserver) = ctx.paths.wineserver.clone() else {
        return;
    };
    let prefix = bottle.prefix.to_string_lossy().into_owned();

    let kill = ctx
        .child(wineserver.clone(), step::RUN_TEARDOWN)
        .arg("-k")
        .env("WINEPREFIX", prefix.clone());
    let _ = ctx.executor.run_child(&kill).await;

    let wait = ctx
        .child(wineserver, step::RUN_TEARDOWN)
        .arg("-w")
        .env("WINEPREFIX", prefix);
    let _ = tokio::time::timeout(STOP_WINESERVER_WAIT, ctx.executor.run_child(&wait)).await;
}

/// run.sh's `stop_helper`, a safety net only — the runtime spawns and owns the
/// helper, which dies with the game — so the reaped line is printed exactly
/// when something was found.
///
/// Matches on the **resolved executable path**
/// ([`crate::process::find_processes_by_exe`]), never `pkill -f`'s argv
/// substring (PARITY.md § Stop, "Each reap (leftover encoder helper"), and
/// kills through the executor as `/bin/kill -TERM <pid>`, once per matched
/// process.
async fn reap_helper(ctx: &StageCtx) {
    let procs = process::find_processes_by_exe(&ctx.paths.oxr_helper_staged);
    if procs.is_empty() {
        return;
    }
    for p in &procs {
        let spec = ctx
            .child("/bin/kill", step::RUN_TEARDOWN)
            .arg("-TERM")
            .arg(p.pid.to_string());
        let _ = ctx.executor.run_child(&spec).await;
    }
    ctx.emit(StageEvent::text(
        ctx.run_id,
        Some(step::RUN_TEARDOWN),
        HELPER_REAPED_LINE,
    ));
}

/// Remove `session-state.json`, but only when there is one.
///
/// The existence probe keeps a run that never wrote the file from adding a
/// phantom `would remove …` row to a dry run's plan.
async fn clear_state(ctx: &StageCtx, path: &Path, expected: RunId) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    // …and only when it is still OUR record (A9-3): a teardown landing after
    // the next launch wrote its own would delete a live session’s only
    // description. `state::clear` compares the *owner*, which on this machine
    // is this process, so run identity is the only discriminant. An unreadable
    // record is still removed; it describes nothing anyone can act on, and
    // leaving it blocks the next reconcile.
    // tests::a_late_teardown_never_clears_a_newer_runs_record
    if let Ok(Some(existing)) = state::load(path) {
        if existing.run_id != expected {
            ctx.step(step::RUN_TEARDOWN).info(format!(
                "{} now describes a newer session — left in place",
                path.display()
            ));
            return Ok(());
        }
    }
    state::clear(&*ctx.executor, path).await
}

/// Is a guard *this teardown was responsible for* still pending?
///
/// Deliberately **not** [`SessionState::has_pending_guards`]: that one also
/// counts `wired_forwards`, which `run` records as it creates them and never
/// removes, so it would keep a stale record after every `--wired` launch.
///
/// See tests::a_wired_run_whose_guards_came_off_still_clears_the_record.
fn teardown_pending(sess: &SessionState) -> bool {
    (sess.prev_audio_output.is_some() && !sess.guards.audio_restored)
        || (sess.dashboard.is_some() && !sess.guards.dashboard_closed)
}

/// The info row emitted instead of clearing a record whose guards are not all
/// released. Sabrage-only, and the mirror of `session::reconcile`'s own
/// `previous session record kept for a later restore`.
const RECORD_KEPT_LINE: &str = "session record kept for a later restore (a guard is still pending)";

/// Clear `session-state.json` — or keep it, when a guard is still pending.
///
/// The teardown counterpart of `session::reconcile::finish_record`, and the
/// same guarantee PARITY.md § Session (detach / reconcile), "A **Dead** or
/// **IdentityMismatch** recorded session" states for `AudioGuard::release`:
/// the record is only cleared once every recorded guard is released.
///
/// See tests::a_teardown_with_an_unrestorable_guard_keeps_the_record.
async fn finish_record(ctx: &StageCtx, path: &Path, sess: &SessionState) -> Result<()> {
    if !teardown_pending(sess) {
        return clear_state(ctx, path, sess.run_id).await;
    }
    state::save(&*ctx.executor, path, sess).await?;
    ctx.step(step::RUN_TEARDOWN).info(RECORD_KEPT_LINE);
    Ok(())
}

/// The line run.sh's `stop_helper` prints when it reaps a leftover helper.
///
/// `pub` (A1-3) so `sabrage-parity` can pin it against `run.sh` by calling the
/// real constant rather than copying a substring.
pub const HELPER_REAPED_LINE: &str = "encoder helper: reaped (left over from the runtime)";

/// run.sh's INT trap prints this after its blank line and before `stop_wine`.
///
/// `pub` (A1-3), same reason as [`HELPER_REAPED_LINE`].
pub const INT_TEARDOWN_LINE: &str = "interrupted: stopping wine";

/// run.sh's `print -r -- "wine exited with status $rc (log: $LOG)"`.
///
/// `pub` (A1-3), same reason as [`HELPER_REAPED_LINE`].
pub fn wine_exit_line(rc: i32, log: &Path) -> String {
    format!("wine exited with status {rc} (log: {})", log.display())
}

/// Sabrage-only: the detach announcement. Shaped like the shell's `-- …`
/// banners because it marks the same kind of transition, but emitted as
/// [`StageEvent::Text`] rather than a `Section` — it closes the run rather
/// than opening a block.
fn detached_line(log: &Path) -> String {
    format!(
        "-- detached: leaving the session running (log: {})",
        log.display()
    )
}

/// The refusal when [`session::reconcile`] finds a session that is still
/// running. Sabrage-only (PARITY.md § Session (detach / reconcile), "A
/// recorded **Live** session"): run.sh has no session record and would simply
/// reset wineserver under the running game.
fn already_running(ctx: &StageCtx, bottle: &Bottle, sess: &SessionState) -> SabrageError {
    let pid = sess.wine.as_ref().map_or(0, |w: &ProcInfo| w.pid);
    ctx.fatal(
        format!(
            "a session is already running (pid {pid}, started {}) — stop it first: \
             Stop in Sabrage, or ./demo.sh stop --bottle {}",
            iso_local(sess.started_at_unix_ms),
            bottle.name
        ),
        None,
    )
}

/// Local civil time, seconds precision, RFC 3339 — the same clock
/// `logs::wine_log_candidate` names files with, so the message and the log
/// name agree.
fn iso_local(unix_ms: u64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(unix_ms as i64)
        .single()
        .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, false))
        .unwrap_or_else(|| unix_ms.to_string())
}

#[cfg(test)]
mod tests;
