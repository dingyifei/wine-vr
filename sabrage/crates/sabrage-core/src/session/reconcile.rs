//! Reconcile a session Sabrage does not supervise.
//!
//! Run at app start, when the Session screen opens, at the head of
//! [`crate::stages::run`] (a `Live` classification refuses the Launch there),
//! and — as `finish_stopped_session` — from [`crate::stages::stop`].
//!
//! `Live` is adopted untouched; `Dead` gets the full restore (audio device,
//! ALVR dashboard, `--wired` forwards); a recycled pid (`IdentityMismatch`)
//! gets the pid-free restore and signals nothing. The record is cleared only
//! once every guard that mode may release is released, and kept otherwise —
//! `SafeOnly` never asks about the dashboard (`restore_complete`).
//! `Unverifiable`, newer-schema, live-foreign and in-flight records are
//! left as they are — the newer-schema and live-foreign ones with a warn row,
//! this process's own in-flight record silently. [`detach`] marks the record and leaves every
//! guard in place. The row texts live on this file's consts and `*_row` fns,
//! plus [`crate::session::audio_unrestorable_line`]; the shell has no
//! counterpart — PARITY.md § Session (detach / reconcile), "A recorded
//! **Live** session".
//!
//! Every mutation goes through [`crate::executor::Executor`], so `--dry-run`
//! plans the recovery instead of performing it; the audio probe is read-only
//! and goes through [`crate::process::capture`].
//!
//! # Failure policy
//!
//! Landmine: `finish_stopped_session` propagates only
//! [`SabrageError::Cancelled`]; every other failure becomes rows plus `Ok(())`
//! with the record kept, so `stop` still reaches its ports and audio reports.
//! See tests::{a_cancelled_reconcile_still_reaches_the_caller,
//! a_failed_restore_is_reported_and_the_record_is_kept_for_the_next_stop}.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::state::{self, SessionState};
use super::LiveSessionHandle;
use crate::error::{Result, SabrageError};
use crate::events::{RunId, StepId};
use crate::paths::{which, Paths};
use crate::process::{self, ProcInfo};
use crate::stages::StageCtx;

/// The step every reconcile row and child is attributed to.
///
/// Not one of [`crate::events::step`]'s stage-scoped ids on purpose:
/// reconciliation is not a step of `run` or of `stop`, it is a cross-cutting
/// recovery pass that both stages (and app start) invoke. Same shape as
/// `fixes::adb`'s `fix.remove-adb-forwards`.
pub const STEP: StepId = "session.reconcile";

/// The section banner that precedes the first restoration row.
pub const RECONCILE_SECTION: &str = "reconciling the previous session";

/// The device `run.sh` routes the Mac's output to, and therefore the only
/// current device that proves the previous session's switch is still in force.
const BLACKHOLE: &str = "BlackHole 2ch";

/// How long [`detach`] waits for the supervisor to let go of
/// [`crate::session::LIVE_SESSION`] before returning anyway.
const DETACH_WAIT: Duration = Duration::from_secs(5);

/// Poll interval inside [`DETACH_WAIT`].
const DETACH_POLL: Duration = Duration::from_millis(50);

/// What [`classify`] concluded about the recorded wine process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Classification {
    /// Same pid, same start time: our process, still running.
    Live,
    /// The pid is gone.
    Dead,
    /// The pid is alive but is **not** our process: a recycled pid.
    IdentityMismatch,
    /// The pid is alive and the record's identity cannot be checked at all —
    /// `start_time` is the spawn fallback's 0. Treated like [`Live`]: nothing
    /// is touched.
    ///
    /// [`Live`]: Classification::Live
    Unverifiable,
}

impl Classification {
    /// Is the recorded process alive as far as anything here can tell?
    ///
    /// [`Classification::Unverifiable`] counts: it is the alive-pid case whose
    /// identity cannot be checked, and every door treats it as running
    /// ([`crate::session::session_block_at`]'s third signal,
    /// [`crate::session::watcher`]'s phase). Rendering it as exited is how the
    /// Session screen offers Launch for a session the launch path then refuses.
    pub fn is_live(self) -> bool {
        matches!(self, Classification::Live | Classification::Unverifiable)
    }
}

/// The outcome of a [`reconcile`] pass, as reported to the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Reconciled {
    /// No `session-state.json`: nothing to do.
    NoSession,
    /// The session is still running. Nothing was restored — its guards belong
    /// to it.
    Live { state: SessionState },
    /// The session's process is gone; its leftover guards were undone.
    /// `restored` is one human line per action, for the reconciliation banner.
    Dead {
        state: SessionState,
        restored: Vec<String>,
        /// A guard could **not** be released, so the record was kept rather
        /// than cleared (see [`finish_record`]). The next launch must not
        /// overwrite it blind: `state.prev_audio_output` is still the device
        /// the Mac needs to go back to, and by now the machine is on BlackHole,
        /// so this launch's own `SwitchAudioSource -c` would record the
        /// loopback as the thing to restore to.
        #[serde(default)]
        pending: bool,
    },
    /// The recorded pid now belongs to something else. Only the pid-free
    /// guards were restored ([`RestoreMode::SafeOnly`]).
    IdentityMismatch {
        state: SessionState,
        restored: Vec<String>,
        /// As [`Reconciled::Dead`]'s.
        #[serde(default)]
        pending: bool,
    },
    /// The record is somebody's — a launch in flight here, another live
    /// front-end's, or a newer Sabrage's. It was **reported and nothing else**:
    /// nothing restored, nothing signalled, the file left exactly as it was.
    /// `reason` is the row the user saw.
    Busy {
        state: SessionState,
        reason: String,
        /// `true` only for *this* process's own in-flight launch, the one
        /// shape that is not worth a row (and not worth refusing over —
        /// nothing is wrong). Every other `Busy` describes a record somebody
        /// else is still using: see [`Reconciled::busy_refusal`], which is
        /// what a launch must refuse on.
        #[serde(default)]
        silent: bool,
    },
}

impl Reconciled {
    /// The reason a caller about to mutate the machine must **stop**, or
    /// `None` when this outcome licenses carrying on.
    ///
    /// A9-1: every `Busy` but this process's own in-flight record (`silent`)
    /// is a refusal. Carrying on would take preflight's auto-fixes, `adb
    /// forward --remove` and the bottle's `wineserver -k` into the very
    /// session the classification had just refused to touch. See
    /// tests::every_busy_but_our_own_in_flight_record_is_a_refusal.
    pub fn busy_refusal(&self) -> Option<&str> {
        match self {
            Reconciled::Busy {
                reason,
                silent: false,
                ..
            } => Some(reason),
            _ => None,
        }
    }
}

/// How much of a stale session's cleanup the restore pass may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RestoreMode {
    /// [`Classification::Dead`]: restore the audio device, reap the recorded
    /// dashboard **by identity**, and remove the recorded `--wired` forwards.
    Full,
    /// [`Classification::IdentityMismatch`]: audio and forwards only. **Never
    /// signal a pid** — the recorded ones are somebody else's now.
    SafeOnly,
}

/// Classify the recorded wine process. Pure; no I/O beyond the process table.
///
/// A record with no `wine` identity at all (a crash between guard acquisition
/// and the spawn) is [`Classification::Dead`]: the guards are real, the child
/// never was.
///
/// A recorded pid of 0 is also `Dead`. It cannot come from a real spawn, and
/// `kill(0, …)` addresses the *caller's whole process group* — the one number
/// that must never reach a liveness probe, let alone a signal.
pub fn classify(state: &SessionState) -> Classification {
    classify_identity(state.wine.as_ref())
}

/// [`classify`] over a bare identity, for the caller that has one without a
/// record around it: [`crate::session::watcher::SessionMonitor`]'s live-handle
/// branch, whose `ProcInfo` is the same spawn-time identity `wine` holds.
///
/// One predicate, so the phase the Session screen shows and the classification
/// the launch path refuses on cannot disagree over an alive pid carrying the
/// spawn fallback's `start_time == 0`. See
/// tests::the_spawn_fallback_start_time_can_never_match.
pub fn classify_identity(wine: Option<&ProcInfo>) -> Classification {
    let Some(wine) = wine else {
        return Classification::Dead;
    };
    if wine.pid == 0 {
        return Classification::Dead;
    }
    if wine.is_same_process() {
        Classification::Live
    } else if !process::is_alive(wine.pid) {
        Classification::Dead
    } else if wine.start_time == 0 {
        // Alive, and nothing about it can be checked: the spawn fallback never
        // observed a start time. "Recycled pid" would be a guess, and acting on
        // it dismantles a session that may be the real one.
        Classification::Unverifiable
    } else {
        Classification::IdentityMismatch
    }
}

/// Which [`RestoreMode`] a classification licenses, or `None` when nothing may
/// be touched at all.
fn restore_mode(class: Classification) -> Option<RestoreMode> {
    match class {
        Classification::Live | Classification::Unverifiable => None,
        Classification::Dead => Some(RestoreMode::Full),
        Classification::IdentityMismatch => Some(RestoreMode::SafeOnly),
    }
}

/// Read the state file and act on it, emitting nothing on the happy path.
///
/// Uses `ctx.paths.session_state_path()`, and every mutation goes through
/// `ctx.executor`, so a dry run plans the restore instead of performing it.
///
/// A session **this process is still supervising** (its `run_id` matches
/// [`crate::session::live_session`]) is reported but never touched: its own
/// teardown owns those guards, and racing it would restore the audio device
/// twice or clear the record out from under it.
pub async fn reconcile(ctx: &StageCtx) -> Result<Reconciled> {
    let live = crate::session::live_session_run_id();
    reconcile_with(ctx, live, crate::session::run_phase(), || {
        current_output_device(ctx)
    })
    .await
}

/// [`reconcile`] with all three ambient inputs injected: the live session's run
/// id, the run stage's published phase, and the current-output-device probe.
/// The public entry point supplies the real ones; tests supply deterministic
/// ones, so neither the global [`crate::session::LIVE_SESSION`] and
/// [`crate::session::run_phase`] slots nor `SwitchAudioSource` is involved.
async fn reconcile_with<F, Fut>(
    ctx: &StageCtx,
    live_run_id: Option<RunId>,
    run_phase: Option<crate::session::RunPhaseInfo>,
    probe: F,
) -> Result<Reconciled>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<Option<AudioProbe>>>,
{
    let path = ctx.paths.session_state_path();
    let Some(mut state) = load_record(ctx, &path)? else {
        return Ok(Reconciled::NoSession);
    };

    if let Some(reason) = untouchable(&state, run_phase.as_ref()) {
        // Silent for our own in-flight launch — there is nothing wrong and
        // nothing for the user to do — and a row for the two that a person may
        // need to explain.
        if !reason.silent {
            ctx.step(STEP).warn(reason.text.clone());
        }
        return Ok(Reconciled::Busy {
            state,
            reason: reason.text,
            silent: reason.silent,
        });
    }

    let class = classify(&state);

    // Ours, and still supervised here: report the classification honestly but
    // restore nothing and keep the file.
    if live_run_id.is_some_and(|id| id == state.run_id) {
        return Ok(match class {
            Classification::Live | Classification::Unverifiable => Reconciled::Live { state },
            Classification::Dead => Reconciled::Dead {
                state,
                restored: Vec::new(),
                pending: false,
            },
            Classification::IdentityMismatch => Reconciled::IdentityMismatch {
                state,
                restored: Vec::new(),
                pending: false,
            },
        });
    }

    // Still running — or running as far as anyone here can tell: the session
    // owns its guards. If it is also `detached` and this process is not its
    // owner, the GUI offers Re-attach — but that is a UI decision, and nothing
    // here may touch the machine.
    let Some(mode) = restore_mode(class) else {
        return Ok(Reconciled::Live { state });
    };
    let (restored, pending) = restore_and_finish(ctx, &path, &mut state, mode, probe).await?;
    Ok(match class {
        Classification::IdentityMismatch => Reconciled::IdentityMismatch {
            state,
            restored,
            pending,
        },
        _ => Reconciled::Dead {
            state,
            restored,
            pending,
        },
    })
}

/// Read the record, turning both "there is none" and "it cannot be read" into
/// `None` — the second with the two rows that explain it.
///
/// An unreadable record is never silent (a rerouted audio device with no
/// explanation is the failure `session-state.json` exists to prevent) and never
/// fatal (a corrupt record must not block every future launch).
fn load_record(ctx: &StageCtx, path: &Path) -> Result<Option<SessionState>> {
    match state::load(path) {
        Ok(Some(s)) => Ok(Some(s)),
        Ok(None) => Ok(None),
        Err(e) => {
            ctx.step(STEP)
                .warn(format!("previous session state unreadable: {e}"));
            ctx.step(STEP)
                .info(format!("delete {} to clear this warning", path.display()));
            Ok(None)
        }
    }
}

/// Why a record may not be touched, and whether saying so is worth a row.
struct Untouchable {
    text: String,
    silent: bool,
}

/// The three "not ours" shapes, in one place so `reconcile` and `stop`'s tail
/// cannot drift apart. `None` means the record is fair game.
fn untouchable(
    state: &SessionState,
    run_phase: Option<&crate::session::RunPhaseInfo>,
) -> Option<Untouchable> {
    // Before the wine spawn the record has `wine: None` (which classifies as
    // `Dead`) and no live handle exists, so only the published phase can keep
    // a mid-launch reconcile from tearing down the launch's own guards.
    // See tests::a_record_belonging_to_the_launch_in_progress_is_never_touched.
    if let Some(info) = run_phase {
        let in_flight = matches!(
            info.phase,
            crate::session::SessionPhase::Preflight
                | crate::session::SessionPhase::Launching
                | crate::session::SessionPhase::Stopping
        );
        if in_flight && info.run_id == state.run_id {
            return Some(Untouchable {
                text: RECORD_IN_FLIGHT.to_string(),
                silent: true,
            });
        }
    }
    if state::has_live_foreign_owner(state) {
        return Some(Untouchable {
            text: owned_elsewhere_row(state.owner_pid),
            silent: false,
        });
    }
    if !state.is_supported_version() {
        return Some(Untouchable {
            text: newer_schema_row(state.version),
            silent: false,
        });
    }
    None
}

/// The reconcile pass [`crate::stages::stop`] runs after `wineserver -k` and
/// before its audio report, on records this process does not supervise.
///
/// A dead wine pid gets [`RestoreMode::Full`], a recycled pid
/// [`RestoreMode::SafeOnly`]; the record is then cleared, or kept when a guard
/// could not be released. A still-alive one gets one warn and the record is
/// kept for the next `stop`. No record, another bottle's record, and a session
/// this process supervises are skipped.
///
/// # Errors
///
/// Only [`SabrageError::Cancelled`]. Every other failure becomes two rows and
/// `Ok(())` so the stage still reaches its ports and audio reports. See
/// tests::{stop_restores_and_clears_a_session_it_did_not_start,
/// stop_ignores_a_record_belonging_to_another_bottle,
/// a_failed_restore_is_reported_and_the_record_is_kept_for_the_next_stop}.
pub(crate) async fn finish_stopped_session(ctx: &StageCtx) -> Result<()> {
    let live = crate::session::live_session_run_id();
    finish_stopped_session_with(ctx, live, crate::session::run_phase(), || {
        current_output_device(ctx)
    })
    .await
}

/// [`finish_stopped_session`] with the live-session id and the device probe
/// injected — see [`reconcile_with`]. Applies the failure policy;
/// [`finish_stopped_session_inner`] is the part that can fail.
async fn finish_stopped_session_with<F, Fut>(
    ctx: &StageCtx,
    live_run_id: Option<RunId>,
    run_phase: Option<crate::session::RunPhaseInfo>,
    probe: F,
) -> Result<()>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<Option<AudioProbe>>>,
{
    let result = finish_stopped_session_inner(ctx, live_run_id, run_phase, probe).await;
    tolerate_reconcile_failure(ctx, result)
}

/// The warn a swallowed reconcile failure prints, with the error appended.
const RECONCILE_FAILED: &str = "previous session not fully restored";

/// The info that follows it: nothing was lost, and `stop` is idempotent.
const RECONCILE_RETRY_HINT: &str = "the record is kept; stop again to retry";

/// Turn a reconcile failure into two rows and `Ok(())`, so the stage that
/// invoked it keeps going.
///
/// [`SabrageError::Cancelled`] passes straight through: a Cancel must fail the
/// stage with exit 130 rather than be reported as a partial restore. See
/// tests::a_cancelled_reconcile_still_reaches_the_caller.
fn tolerate_reconcile_failure(ctx: &StageCtx, result: Result<()>) -> Result<()> {
    match result {
        Ok(()) => Ok(()),
        Err(SabrageError::Cancelled) => Err(SabrageError::Cancelled),
        Err(e) => {
            ctx.step(STEP).warn(format!("{RECONCILE_FAILED}: {e}"));
            ctx.step(STEP).info(RECONCILE_RETRY_HINT);
            Ok(())
        }
    }
}

/// [`finish_stopped_session_with`] without the failure policy — everything that
/// can legitimately return `Err`.
async fn finish_stopped_session_inner<F, Fut>(
    ctx: &StageCtx,
    live_run_id: Option<RunId>,
    run_phase: Option<crate::session::RunPhaseInfo>,
    probe: F,
) -> Result<()>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<Option<AudioProbe>>>,
{
    let path = ctx.paths.session_state_path();
    let Some(mut state) = load_record(ctx, &path)? else {
        return Ok(());
    };

    // Bottle-scoped: this stage's `wineserver -k` only touched one bottle.
    match ctx.bottle.as_ref() {
        Some(b) if b.name == state.bottle => {}
        _ => return Ok(()),
    }

    // Our own live session: the supervise loop runs its own teardown.
    if live_run_id.is_some_and(|id| id == state.run_id) {
        return Ok(());
    }

    // Somebody else's record — the same three shapes `reconcile` refuses,
    // reported here as one warn because `stop` is the stage a person runs when
    // they expected the machine to be put back.
    if let Some(reason) = untouchable(&state, run_phase.as_ref()) {
        if !reason.silent {
            ctx.step(STEP).warn(reason.text);
        }
        return Ok(());
    }

    let class = classify(&state);
    let Some(mode) = restore_mode(class) else {
        let pid = state.wine.as_ref().map(|w| w.pid).unwrap_or_default();
        ctx.step(STEP).warn(match class {
            Classification::Unverifiable => format!(
                "previous session state kept: wine pid {pid} is alive but could not be identified"
            ),
            _ => format!("previous session state kept: wine pid {pid} still alive"),
        });
        return Ok(());
    };
    restore_and_finish(ctx, &path, &mut state, mode, probe).await?;
    Ok(())
}

/// What the audio probe came back with: the `SwitchAudioSource` binary that
/// answered, the device it named, and every output device present right now.
///
/// The binary path travels with the answer so the restore uses the *same*
/// executable the probe did, and so a test can stub all three parts without
/// `SwitchAudioSource` being installed.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AudioProbe {
    bin: PathBuf,
    /// `-c -t output`: where the Mac's output is pointed right now.
    current: String,
    /// `-a -t output`: the pool [`crate::session::fallback_output_device`]
    /// picks from when the *recorded* device is no longer connected.
    outputs: Vec<String>,
}

/// `SwitchAudioSource -c -t output` and `-a -t output`, `$(…)`-trimmed — both
/// read-only, hence [`crate::process::capture`] rather than the executor. The
/// two are independent, so they run concurrently ([`tokio::join!`]), and only
/// for a record that still has an unreleased audio guard.
///
/// `Ok(None)` is "we could not look" (no binary, a failed or wedged `-c`) and
/// leaves the audio guard **pending** rather than flagged; a failed `-a` only
/// costs the fallback pool.
///
/// # Errors
///
/// [`SabrageError::Cancelled`] only — see `probe_capture` and
/// tests::a_missing_switchaudiosource_leaves_the_audio_guard_pending.
async fn current_output_device(ctx: &StageCtx) -> Result<Option<AudioProbe>> {
    let Some(bin) = which("SwitchAudioSource") else {
        return Ok(None);
    };
    let current_spec = ctx.child(bin.clone(), STEP).args(["-c", "-t", "output"]);
    let listing_spec = ctx.child(bin.clone(), STEP).args(["-a", "-t", "output"]);
    let (current, listing) = tokio::join!(
        probe_capture(&current_spec, &ctx.cancel),
        probe_capture(&listing_spec, &ctx.cancel)
    );
    let (current, listing) = (current?, listing?);
    let Some(current) = current.filter(|c| c.status.success()) else {
        return Ok(None);
    };
    let outputs = match listing {
        Some(l) if l.status.success() => output_device_names(&l.stdout),
        _ => Vec::new(),
    };
    Ok(Some(AudioProbe {
        bin,
        current: current.stdout_trimmed().to_string(),
        outputs,
    }))
}

/// How long one audio probe may take before it is treated as no answer.
///
/// `SwitchAudioSource -c -t output` returns in milliseconds; a probe that has
/// not answered in this long is wedged on a CoreAudio call, and waiting for it
/// blocks the whole of `run`/`stop` — with the operation lock held — behind a
/// read-only question whose failure mode is already handled (`None` leaves the
/// audio guard pending and the record on disk). Shorter than
/// [`crate::process::DEFAULT_PROBE_TIMEOUT`] on purpose, which is why the
/// deadline is passed explicitly rather than taken from
/// [`crate::process::capture`].
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// [`crate::process::capture_with`] under the *operation's* token and
/// `PROBE_TIMEOUT`. `None` for a spawn failure, for a probe that ran out of
/// time, and for a cancelled one — indistinguishable to every caller, because
/// all three mean "we could not look".
///
/// The token lets Cancel interrupt a wedged CoreAudio probe instead of waiting
/// out the deadline with the operation lock held, and `capture_with` kills the
/// probe's whole **process group**, so a `SwitchAudioSource` that forked cannot
/// outlive its dropped leader. Not a `tokio::time::timeout` around `capture`:
/// it fires before `capture`'s own deadline, so the group kill never runs — a
/// timing property, and the test for it was decided against.
async fn probe_capture(
    spec: &crate::process::ChildSpec,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<Option<process::Captured>> {
    match process::capture_with(spec, cancel, PROBE_TIMEOUT).await {
        Ok(captured) => Ok(Some(captured)),
        // Cancellation is not "we could not look": swallowing it here would
        // let the restore carry on — emitting its could-not-restore warn and
        // keeping the record — for a `stop` the user just cancelled, which
        // must fail with exit 130 (module doc, "Failure policy").
        Err(SabrageError::Cancelled) => Err(SabrageError::Cancelled),
        Err(_) => Ok(None),
    }
}

/// `SwitchAudioSource -a -t output`'s stdout as one device name per line,
/// blank lines dropped and nothing else touched (device names are matched
/// whole-line, exactly as `grep -qx` matches them elsewhere).
fn output_device_names(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// Undo the guards `state` still records as un-released, honouring `mode`,
/// with the device probe injected (see [`reconcile_with`]).
///
/// Order is audio → dashboard → forwards, each followed by its own
/// [`state::save`] the moment its flag flips: a crash between two guards must
/// leave a record describing only the work that is still outstanding. Returns
/// one human line per action actually performed (empty when there was nothing
/// left to undo).
async fn restore_with<F, Fut>(
    ctx: &StageCtx,
    state: &mut SessionState,
    mode: RestoreMode,
    probe: F,
) -> Result<Vec<String>>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<Option<AudioProbe>>>,
{
    let path = ctx.paths.session_state_path();
    let mut banner = Banner::new(ctx);
    let mut restored: Vec<String> = Vec::new();

    restore_audio(ctx, &path, state, probe, &mut banner, &mut restored).await?;
    // SafeOnly skips the dashboard entirely: its whole definition is "signal no
    // pid".
    if mode == RestoreMode::Full {
        restore_dashboard(ctx, &path, state, &mut banner, &mut restored).await?;
    }
    restore_forwards(ctx, &path, state, &mut banner, &mut restored).await?;

    Ok(restored)
}

/// `SwitchAudioSource -t output -s <device>`; `true` when it took.
async fn switch_output(ctx: &StageCtx, bin: &Path, device: &str) -> Result<bool> {
    let spec = ctx
        .child(bin.to_path_buf(), STEP)
        .args(["-t", "output", "-s", device]);
    Ok(ctx.executor.run_child(&spec).await?.success())
}

/// Put the Mac's output device back — but only while it is *still* BlackHole:
/// a user who already switched it back by hand must not have their choice
/// overwritten by a recovery pass.
///
/// Three outcomes, in order of preference: the recorded device; the fallback
/// [`crate::session::fallback_output_device`] picks when the recorded one is no
/// longer connected (its switch exits non-zero with "Could not find an audio
/// device named … Nothing was changed." and would otherwise leave the Mac
/// silent on BlackHole); or a warn naming the device and the commands to fix it
/// by hand, with the guard left **pending** so the record survives for the next
/// try. See tests::a_recorded_device_that_is_gone_falls_back_to_the_built_in_output.
async fn restore_audio<F, Fut>(
    ctx: &StageCtx,
    path: &Path,
    state: &mut SessionState,
    probe: F,
    banner: &mut Banner<'_>,
    restored: &mut Vec<String>,
) -> Result<()>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<Option<AudioProbe>>>,
{
    let Some(prev) = state.prev_audio_output.clone() else {
        return Ok(());
    };
    if state.guards.audio_restored {
        return Ok(());
    }
    // `None` is "we could not look", which is not "there was nothing to do":
    // the guard stays pending.
    let Some(p) = probe().await? else {
        return Ok(());
    };
    let dry = ctx.executor.is_dry_run();

    if p.current != BLACKHOLE {
        // Already back where it belongs — nothing to undo, and nothing to say
        // about it.
        state.guards.audio_restored = true;
        return state::save(&*ctx.executor, path, state).await;
    }

    if switch_output(ctx, &p.bin, &prev).await? {
        let line = audio_row(dry, &prev);
        banner.show();
        ctx.step(STEP).ok(line.clone());
        restored.push(line);
        state.guards.audio_restored = true;
        return state::save(&*ctx.executor, path, state).await;
    }

    if let Some(alt) = super::fallback_output_device(&p.outputs) {
        if switch_output(ctx, &p.bin, &alt).await? {
            let line = audio_fallback_row(dry, &prev, &alt);
            banner.show();
            ctx.step(STEP).warn(line.clone());
            restored.push(line);
            state.guards.audio_restored = true;
            return state::save(&*ctx.executor, path, state).await;
        }
    }

    // No banner: this row reports why there was no recovery, not a recovery
    // that happened — the same rule the stop-only rows follow.
    ctx.step(STEP).warn(super::audio_unrestorable_line(&prev));
    Ok(())
}

/// Reap the `alvr_dashboard` this session spawned, by identity.
///
/// Flagged either way: whether we killed it, it had already exited, or the pid
/// now belongs to a stranger, there is nothing further this record can ask
/// anyone to do.
async fn restore_dashboard(
    ctx: &StageCtx,
    path: &Path,
    state: &mut SessionState,
    banner: &mut Banner<'_>,
    restored: &mut Vec<String>,
) -> Result<()> {
    let Some(dashboard) = state.dashboard.clone() else {
        return Ok(());
    };
    if state.guards.dashboard_closed {
        return Ok(());
    }

    let mut closed = false;
    if signalable(&dashboard) {
        let spec = ctx
            .child("/bin/kill", STEP)
            .arg("-TERM")
            .arg(dashboard.pid.to_string());
        closed = ctx.executor.run_child(&spec).await?.success();
    }
    if closed {
        let line = dashboard_row(ctx.executor.is_dry_run());
        banner.show();
        ctx.step(STEP).ok(line.clone());
        restored.push(line);
    }
    state.guards.dashboard_closed = true;
    state::save(&*ctx.executor, path, state).await
}

/// Remove exactly the recorded `--wired` forwards, on exactly the recorded
/// serials; never `--remove-all`
/// (PARITY.md § Invariants that must NOT change (byte/behavior parity),
/// "adb `forward --remove` per-serial for exactly tcp:9943+9944").
///
/// Each removal that succeeds drops its port from the record; the guard is only
/// flagged once none are left. A failed removal is *indeterminate* — usually a
/// vanished device, but equally a transient adb failure over a still-installed
/// `tcp:9943`, which silently breaks the next WiFi discovery — so the record is
/// kept for the next launch or `stop`. See
/// tests::a_forward_that_could_not_be_removed_keeps_the_record.
async fn restore_forwards(
    ctx: &StageCtx,
    path: &Path,
    state: &mut SessionState,
    banner: &mut Banner<'_>,
    restored: &mut Vec<String>,
) -> Result<()> {
    if state.wired_forwards.is_empty() || state.guards.forwards_cleared {
        return Ok(());
    }
    let Some(adb) = ctx.paths.adb.clone() else {
        return Ok(());
    };
    let dry = ctx.executor.is_dry_run();

    let mut still_installed = Vec::new();
    for fwd in state.wired_forwards.clone() {
        let local = format!("tcp:{}", fwd.port);
        let spec = ctx.child(adb.clone(), STEP).args([
            "-s",
            fwd.serial.as_str(),
            "forward",
            "--remove",
            local.as_str(),
        ]);
        // run.sh's `&&`: a failed removal prints nothing and is not an error.
        if !ctx.executor.run_child(&spec).await?.success() {
            still_installed.push(fwd);
            continue;
        }
        // Progress is written the moment it happens: a retry of an
        // already-absent forward exits non-zero, which this loop reads as
        // "still installed", so a crash mid-loop would strand a phantom port.
        // See tests::a_removal_that_took_is_on_disk_before_the_next_one_is_tried.
        state.wired_forwards.retain(|f| f != &fwd);
        state::save(&*ctx.executor, path, state).await?;
        let line = forward_row(dry, fwd.port, &fwd.serial);
        banner.show();
        ctx.step(STEP).info(line.clone());
        restored.push(line);
    }

    state.wired_forwards = still_installed;
    state.guards.forwards_cleared = state.wired_forwards.is_empty();
    state::save(&*ctx.executor, path, state).await
}

/// The info emitted instead of clearing a record whose guards are not all
/// released yet.
const RECORD_KEPT: &str = "previous session record kept for a later restore";

/// Did `mode` finish everything it was allowed to do?
///
/// "Finished" is per guard `flag set OR nothing was ever recorded` — an inert
/// guard (no `prev_audio_output`, no dashboard, no `--wired` forwards) is done
/// by definition, which is exactly [`SessionState::has_pending_guards`]
/// negated.
///
/// [`RestoreMode::SafeOnly`] is the exception: it never signals a pid, so it
/// can never flag the dashboard, and asking about it would keep every
/// recycled-pid record on disk forever. The question there is only whether the
/// pid-free guards are done.
fn restore_complete(state: &SessionState, mode: RestoreMode) -> bool {
    let audio_done = state.prev_audio_output.is_none() || state.guards.audio_restored;
    let forwards_done = state.wired_forwards.is_empty() || state.guards.forwards_cleared;
    match mode {
        RestoreMode::Full => !state.has_pending_guards(),
        RestoreMode::SafeOnly => audio_done && forwards_done,
    }
}

/// Restore what `mode` allows and then either clear the record or keep it —
/// the two halves every classification that may touch the machine runs, in the
/// one place both entry points call.
///
/// Returns the restoration rows and **whether the record was kept**: a kept
/// record is not a finished recovery, and the caller has to be able to tell the
/// difference. A `Dead` that cleared the file and a `Dead` that could not
/// restore the audio device are otherwise the same value, and the launch that
/// follows overwrites the only copy of the device name (`stages::run`'s
/// `carried_audio_device` is the reader).
async fn restore_and_finish<F, Fut>(
    ctx: &StageCtx,
    path: &Path,
    state: &mut SessionState,
    mode: RestoreMode,
    probe: F,
) -> Result<(Vec<String>, bool)>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<Option<AudioProbe>>>,
{
    let restored = restore_with(ctx, state, mode, probe).await?;
    let kept = finish_record(ctx, path, state, mode).await?;
    Ok((restored, kept))
}

/// Clear the record — or keep it, when a guard is still pending. `true` when
/// it was kept.
///
/// A record whose guards are not all released is worth more on disk than off
/// it: clearing it strands the user on `BlackHole 2ch` with no machine-readable
/// trace of what to restore, and the next launch or `stop` retries from the
/// kept record. See
/// tests::with_nothing_to_fall_back_to_the_record_is_kept_with_the_remedy.
async fn finish_record(
    ctx: &StageCtx,
    path: &Path,
    state: &mut SessionState,
    mode: RestoreMode,
) -> Result<bool> {
    if restore_complete(state, mode) {
        // Run-id guarded: reconciliation can take seconds, and a launch that
        // started meanwhile has written its own record at this path. Only the
        // record carrying the run-id we read is ours to delete.
        state::clear_run(&*ctx.executor, path, state.run_id).await?;
        return Ok(false);
    }
    state::save(&*ctx.executor, path, state).await?;
    ctx.step(STEP).info(RECORD_KEPT);
    Ok(true)
}

/// May this identity be signalled?
///
/// Both halves matter: pid 0 addresses the caller's own process group, and
/// [`ProcInfo::is_same_process`] is the recycled-pid guard. A `false` here is
/// always the conservative answer — nothing gets signalled.
fn signalable(id: &ProcInfo) -> bool {
    id.pid != 0 && id.is_same_process()
}

/// Emits [`RECONCILE_SECTION`] once, immediately before the first row.
///
/// Lazy on purpose: a stale record whose guards were all released already (a
/// crash between the last `save` and the final `clear`) reconciles silently
/// rather than announcing a recovery that never happened.
struct Banner<'a> {
    ctx: &'a StageCtx,
    shown: bool,
}

impl<'a> Banner<'a> {
    fn new(ctx: &'a StageCtx) -> Banner<'a> {
        Banner { ctx, shown: false }
    }

    fn show(&mut self) {
        if !self.shown {
            self.shown = true;
            self.ctx.section(RECONCILE_SECTION);
        }
    }
}

/// `audio: restored output -> <dev> (previous session did not shut down cleanly)`
///
/// The first half is run.sh's own `restore_audio` line verbatim; the
/// parenthetical is what distinguishes a recovery from a normal teardown.
fn audio_row(dry_run: bool, previous: &str) -> String {
    let verb = if dry_run { "would restore" } else { "restored" };
    format!("audio: {verb} output -> {previous} (previous session did not shut down cleanly)")
}

/// `recorded output device '<prev>' is not connected — restored output -> <alt>
/// instead (previous session did not shut down cleanly)`
///
/// The fallback's counterpart to [`audio_row`], parenthetical and all: the
/// shared half lives in [`crate::session::audio_fallback_line`] so the guard
/// release prints exactly the same sentence.
fn audio_fallback_row(dry_run: bool, previous: &str, fallback: &str) -> String {
    format!(
        "{} (previous session did not shut down cleanly)",
        super::audio_fallback_line(dry_run, previous, fallback)
    )
}

/// `ALVR dashboard closed (left over from the previous session)`
fn dashboard_row(dry_run: bool) -> String {
    let verb = if dry_run { "would be closed" } else { "closed" };
    format!("ALVR dashboard {verb} (left over from the previous session)")
}

/// `cleared adb forward tcp:<port> on <serial>`
fn forward_row(dry_run: bool, port: u16, serial: &str) -> String {
    let verb = if dry_run { "would clear" } else { "cleared" };
    format!("{verb} adb forward tcp:{port} on {serial}")
}

/// The reason a record belonging to *this* process's launch is left alone.
/// Never printed — there is nothing wrong and nothing for anyone to do — but
/// it travels in [`Reconciled::Busy`] so a caller can say why nothing happened.
const RECORD_IN_FLIGHT: &str = "session record belongs to the launch in progress";

/// `previous session record kept: Sabrage process <pid> is running this session`
fn owned_elsewhere_row(owner_pid: u32) -> String {
    format!("previous session record kept: Sabrage process {owner_pid} is running this session")
}

/// `previous session record kept: written by a newer Sabrage (schema v<n>, this
/// build understands v<m>)`
fn newer_schema_row(version: u32) -> String {
    format!(
        "previous session record kept: written by a newer Sabrage (schema v{version}, this build \
         understands v{})",
        state::SESSION_STATE_VERSION
    )
}

/// Detach from a live session: mark the state file `detached`, fire the
/// handle's `detach` token, and leave every guard in place. Takes [`Paths`]
/// rather than a [`StageCtx`] because app-quit has no stage context.
///
/// The supervisor is the authority: firing `handle.detach` is what disarms both
/// guards, marks the record, and drops out of
/// [`crate::session::LIVE_SESSION`]. This waits up to `DETACH_WAIT` for that
/// slot to clear and only then sets `detached` itself as a safety net — never
/// on the timeout, never once `handle.cancel` has fired, and through
/// [`state::mark_detached`], which creates nothing. See tests::{
/// detach_does_not_relabel_a_session_stopped_during_the_wait,
/// detach_that_times_out_leaves_the_record_alone,
/// detach_creates_nothing_when_the_supervisor_already_cleared_the_record}.
pub async fn detach(ctx_paths: &Paths, handle: &LiveSessionHandle) -> Result<()> {
    detach_with(ctx_paths, handle, DETACH_WAIT, |run_id| {
        crate::session::live_session_is(run_id)
    })
    .await
}

/// [`detach`] with its two ambient inputs injected — the wait budget and the
/// "is the supervisor still holding the slot" question.
///
/// Same reason [`reconcile_with`] exists: the real ones read the process-global
/// [`crate::session::LIVE_SESSION`], and a test that occupied that slot for the
/// length of a [`DETACH_WAIT`] would be publishing a live session to every
/// other test in the binary.
async fn detach_with<F>(
    ctx_paths: &Paths,
    handle: &LiveSessionHandle,
    wait: Duration,
    still_supervised: F,
) -> Result<()>
where
    F: Fn(RunId) -> bool,
{
    // Stop is terminal and detach is subordinate to it: both tokens feed one
    // unbiased `select!`, so a detach fired after a Stop could otherwise win
    // that race and leave wine running under a caller reporting success.
    // See tests::detach_does_nothing_once_stop_has_already_fired.
    //
    // This is also the return that absorbs `commands::resolve_quit`'s
    // stop-then-timeout arm: nothing is detached there, so that arm's own
    // message has to say so.
    if handle.cancel.is_cancelled() {
        return Ok(());
    }
    handle.detach.cancel();

    let deadline = tokio::time::Instant::now() + wait;
    let cleared = loop {
        if !still_supervised(handle.run_id) {
            break true;
        }
        if tokio::time::Instant::now() >= deadline {
            break false;
        }
        tokio::time::sleep(DETACH_POLL).await;
    };

    // Not our record to write: either the supervisor never let go (and is
    // therefore still writing to it), or a Stop won the race while we waited
    // and owns the teardown — including the decision to keep the record.
    if !cleared || handle.cancel.is_cancelled() {
        return Ok(());
    }

    let executor = crate::executor::RealExecutor::new(
        handle.run_id,
        crate::stages::null_sink(),
        tokio_util::sync::CancellationToken::new(),
    );
    // Best-effort: a session that keeps running with `detached: false` on disk
    // still reconciles correctly (Live restores nothing); the flag only decides
    // whether the GUI offers Re-attach.
    let _ = state::mark_detached(&executor, &ctx_paths.session_state_path(), handle.run_id).await;

    Ok(())
}

#[cfg(test)]
mod tests;
