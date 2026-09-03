//! Reconciling a session Sabrage did not (or no longer) supervises.
//!
//! Run at startup and whenever the Session screen opens: read
//! [`state::SessionState`], work out what is actually true on the machine, and
//! either adopt the session, finish its cleanup, or refuse to touch it.
//!
//! # The three classifications, and why the third exists
//!
//! [`classify`] is pure — [`crate::process::ProcInfo::is_same_process`] and
//! [`crate::process::is_alive`], nothing else:
//!
//! * [`Classification::Live`] — the recorded pid is alive *and* reports the
//!   recorded start time. It really is our wine process; adopt it.
//! * [`Classification::Dead`] — the pid is gone. Whatever the guards say still
//!   needs undoing, and the wine child cannot be signalled because it does not
//!   exist. Safe to do everything ([`RestoreMode::Full`]).
//! * [`Classification::IdentityMismatch`] — a process with that pid exists, but
//!   its start time differs: the pid was **recycled** and now belongs to
//!   something else entirely. This is the case that must never be handled like
//!   `Dead`, because `Dead`'s cleanup includes reaping by identity — and here
//!   the identity is a stranger's, quite possibly the user's editor.
//!   [`RestoreMode::SafeOnly`] restores what is not attached to a pid (audio,
//!   adb forwards) and signals nothing.
//!
//! * [`Classification::Unverifiable`] — the recorded `start_time` is the 0 the
//!   spawn fallback writes when the pid could not be observed
//!   ([`crate::executor::Executor::spawn_detached`]), and that pid is *alive*.
//!   It can never match a real start time, so it is not `Live`; but calling it
//!   a recycled pid is a guess in the other direction, and acting on that guess
//!   means restoring the audio device and pulling the `--wired` forwards out
//!   from under what may well be the running session. Nothing is touched and
//!   the record is kept.
//!
//! # Records that are not ours to touch
//!
//! Three more shapes are reported and left exactly as they are
//! ([`Reconciled::Busy`]), because in each one the guards belong to somebody
//! who is still using them:
//!
//! * the record of a launch **this process is running right now** — before the
//!   wine spawn it has `wine: None`, which classifies as `Dead`, and its
//!   [`crate::session::LIVE_SESSION`] handle does not exist yet either. The
//!   run stage's published phase ([`crate::session::run_phase`]) is what names
//!   it, which is why reconciliation takes that as an ambient input;
//! * a record whose `owner_pid` is a live *foreign* process
//!   ([`state::has_live_foreign_owner`]) — the other front-end's session;
//! * a record written by a **newer** Sabrage
//!   ([`state::SessionState::is_supported_version`]) — it may describe a guard
//!   this build cannot undo, and rewriting it through this struct would erase
//!   that description.
//!
//! # Detach
//!
//! [`detach`] is the app-quit "leave it running" answer (critique.md's
//! app-quit issue): mark the on-disk state `detached`, fire the handle's
//! `detach` token so the supervisor stops without running teardown, and
//! **leave every guard in place**. A detached session's guards are pending on
//! purpose; a later reconcile that finds `detached: true` must not silently
//! undo the user's choice while the game is still running.
//!
//! # What the user sees
//!
//! Everything this module does is a Sabrage-only capability — `run.sh`'s guards
//! are shell traps, and `stop.sh` can only *warn* that the audio device is
//! still on BlackHole. So the rows have no shell counterpart to match; they are
//! listed here (and in `sabrage/PARITY.md`) as the contract instead:
//!
//! | | text |
//! |---|---|
//! | section | `reconciling the previous session` ([`RECONCILE_SECTION`]) |
//! | ok | `audio: restored output -> <dev> (previous session did not shut down cleanly)` |
//! | warn | `recorded output device '<dev>' is not connected — restored output -> <alt> instead (previous session did not shut down cleanly)` |
//! | warn | `could not restore the audio output (recorded device '<dev>' is not connected) — restore with: …` |
//! | ok | `ALVR dashboard closed (left over from the previous session)` |
//! | info | `cleared adb forward tcp:<port> on <serial>` |
//! | info | `previous session record kept for a later restore` |
//! | warn | `previous session record kept: Sabrage process <pid> is running this session` |
//! | warn | `previous session record kept: written by a newer Sabrage (schema v<n>, this build understands v<m>)` |
//! | warn | `previous session state kept: wine pid <pid> still alive` (stop only) |
//! | warn | `previous session state kept: wine pid <pid> is alive but could not be identified` (stop only) |
//! | warn | `previous session not fully restored: <error>` (stop only) |
//! | info | `the record is kept; stop again to retry` (stop only) |
//!
//! The section banner is emitted **lazily**, immediately before the first
//! *restoration* row, so a stale record with nothing left to undo reconciles in
//! silence. The two audio rows in the middle are the
//! recorded-device-is-gone path (the disconnected AirPods of the 2026-08-29
//! finding): the fallback switch counts as a restoration and takes the banner,
//! the "could not restore" row does not. The two stop-only rows above are emitted by
//! [`finish_stopped_session`] itself rather than by [`restore_with`], and — like
//! the "still alive" warn that predates them — carry no banner: they report why
//! there was no recovery, not a recovery that happened. Under `--dry-run` each
//! verb is swapped ("would restore", "would be closed", "would clear"), the
//! convention PARITY.md already records for the other stages' preview rows.
//!
//! # Failure policy: `stop` must not abort here
//!
//! [`finish_stopped_session`] reports its own failures ([`RECONCILE_FAILED`] +
//! [`RECONCILE_RETRY_HINT`]) and returns `Ok(())`. `stop.sh` has no step that
//! can end the script early — every one of its blocks is `|| true`-shaped or a
//! plain report — and this whole pass is *additive* to that stage, so a
//! reconcile that cannot finish (a recorded `SwitchAudioSource` or `adb` that
//! no longer exists, an unwritable store) must never cost the user the ports
//! and audio rows that follow it. The record is left on disk in that case,
//! un-cleared, with only the guards that really were released flagged: the next
//! `stop` picks up exactly where this one stopped.
//!
//! [`SabrageError::Cancelled`] is the one error that still reaches the caller —
//! a Cancel during `stop` must surface as exit 130, not as a warn row and a
//! green stage (`stages::stop`'s own Cancellation section).
//!
//! # Every mutation goes through the executor
//!
//! The device switch, the dashboard `SIGTERM` (as `/bin/kill -TERM <pid>`, the
//! same primitive [`crate::stages::stop`] uses — the [`crate::executor::Executor`]
//! trait has no "signal a pid" method), each `adb forward --remove`, every
//! [`state::save`] and the final [`state::clear`] are all
//! [`crate::executor::Executor`] calls, so `--dry-run` plans the whole recovery
//! instead of performing it. The one thing that is *not* — the audio probe:
//! the current default output device and the list of output devices to fall
//! back to — is read-only and goes through [`crate::process::capture`], which
//! is what that function exists for.

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
    /// identity simply cannot be checked, and every door in this codebase
    /// treats it as running (the module doc's fourth bullet,
    /// [`crate::session::session_block_at`]'s third signal). Rendering it as
    /// exited is how the Session screen offers Launch for a session the launch
    /// path then refuses.
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
    /// A9-1: `Busy` used to be indistinguishable from "nothing to carry
    /// forward" at the launch site, so a record protected *because* another
    /// front-end's session is live let the launch continue into preflight's
    /// auto-fixes, `adb forward --remove` and the bottle's `wineserver -k` —
    /// taking down the very session the classification had just refused to
    /// touch. Only this process's own in-flight record (`silent`) is safe to
    /// carry on over: it is this launch's own.
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
/// the launch path refuses on cannot disagree — an alive pid with the spawn
/// fallback's `start_time == 0` was `Unverifiable` (and therefore live) to
/// reconciliation while the monitor rendered it `Exited`, offering Launch for a
/// session `run` would then refuse.
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

// ── reconcile ─────────────────────────────────────────────────────────────────

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

    // ── records that are not ours to touch (module doc) ──────────────────────
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
    // A launch this process is running *right now*. Before the wine spawn its
    // record has `wine: None` — which classifies as `Dead` — and there is no
    // live handle yet, so the published phase is the only thing that knows.
    // Without this, remounting the Session screen mid-launch restores the audio
    // device, kills the dashboard this launch just spawned, pulls its forwards
    // and deletes its record, all under a launch that keeps going.
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

// ── stop's tail end ───────────────────────────────────────────────────────────

/// The reconcile pass [`crate::stages::stop`] runs after `wineserver -k` and
/// before its audio report.
///
/// `stop` must work on sessions Sabrage did not start — that is its whole
/// point — so it cannot rely on an in-process handle. It reads the same record
/// [`reconcile`] does, but *after* the kill, when the recorded wine pid should
/// already be gone:
///
/// * **dead** (the normal case) → [`RestoreMode::Full`] then [`state::clear`],
///   which is why the `stop.4.audio` row that follows can report the restored
///   device instead of stop.sh's "still BlackHole 2ch" warning;
/// * **identity mismatch** → [`RestoreMode::SafeOnly`] then clear;
/// * **still alive** (`wineserver -k` did not take) → one warn, and the record
///   is kept so the next `stop` can try again.
///
/// Three cases are skipped outright: no record at all, a record for a *different
/// bottle* (this `stop` never touched it — the stage is bottle-scoped), and a
/// record for a session **this process is supervising**, whose own teardown
/// will release the guards.
///
/// **Only ever fails with [`SabrageError::Cancelled`]** — see the module doc's
/// "Failure policy". Every other failure becomes two rows and `Ok(())`, so the
/// `stop` stage always reaches its ports and audio reports.
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
/// invoked it keeps going (module doc, "Failure policy").
///
/// [`SabrageError::Cancelled`] passes straight through: a Cancel must fail the
/// stage with exit 130 rather than be reported as a partial restore and then
/// forgotten. `stop`'s own between-step checkpoint would catch it a moment
/// later anyway, but a function that swallowed cancellation would be one more
/// place for that guarantee to rot.
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

// ── restoration ───────────────────────────────────────────────────────────────

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
/// read-only, hence [`crate::process::capture`] rather than the executor.
///
/// `None` when the binary is absent, `-c` is unrunnable, or `-c` exits
/// non-zero: the audio guard is then left **pending** rather than flagged,
/// because "we could not look" is not "there was nothing to do". The device
/// *list* is the softer of the two — a failed `-a` yields an empty pool, which
/// only costs the fallback, not the ordinary restore.
///
/// Both captures happen up front rather than lazily so the whole probe stays
/// one injection point (see [`reconcile_with`]); they run only when a record
/// actually has an unreleased audio guard. They are two independent read-only
/// probes, so they run **concurrently** ([`tokio::join!`]) rather than one
/// after the other — `-a`'s answer is read only by
/// [`crate::session::fallback_output_device`]'s not-connected branch, so most
/// restores pay for it without using it either way; concurrency at least
/// keeps that from costing two probes' latency in series.
///
/// `Ok(None)` is "we could not look" (no binary, a failed or wedged probe);
/// `Err(Cancelled)` is the one answer that is **not** that — see
/// [`probe_capture`].
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
/// [`PROBE_TIMEOUT`]. `None` for a spawn failure, for a probe that ran out of
/// time, and for a cancelled one — indistinguishable to every caller, because
/// all three mean "we could not look".
///
/// The token matters twice over: Cancel can interrupt a wedged CoreAudio probe
/// instead of waiting out the deadline with the operation lock held, and
/// `capture_with` kills the probe's whole **process group** on the way out, so
/// a `SwitchAudioSource` that forked cannot outlive its dropped leader. (A
/// `tokio::time::timeout` around `capture` did neither: it fired before
/// `capture`'s own deadline, so even the group kill never ran.)
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
/// longer connected (the AirPods of the 2026-08-29 finding, whose switch exits
/// non-zero with "Could not find an audio device named … Nothing was changed."
/// and would otherwise leave the Mac silent on BlackHole); or a warn naming the
/// device and the commands to fix it by hand, with the guard left **pending**
/// so the record survives for the next try.
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
/// serials. Never `--remove-all` (CLAUDE.md; PARITY.md).
///
/// Each removal that succeeds drops its port from the record; the guard is only
/// flagged once none are left. A removal that fails is *indeterminate* — the
/// device is usually simply gone, and with it the forward, but it may equally
/// be a transient adb failure over a still-installed `tcp:9943`, which silently
/// breaks the next WiFi discovery. Flagging the guard released on that would
/// clear the record and leave nothing that knows the port is still there; the
/// kept record is what the next launch or `stop` retries from.
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
        // Progress is written the moment it happens (the write-before-mutate
        // rule's other half, `session::state`'s header): a crash after the
        // `tcp:9943` removal but before the end of this loop must not leave a
        // record still claiming 9943 is installed. A retry of an
        // already-absent forward exits non-zero, and this function reads any
        // non-zero as "still installed" — so that phantom row would be kept
        // forever and `forwards_cleared` would never flip.
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
/// The live failure this exists for: a `stop` whose recorded output device had
/// disconnected switched nothing, cleared the record anyway, and left the user
/// on `BlackHole 2ch` with no machine-readable trace of what to restore. A
/// record whose guards are not all released is worth more on disk than off it —
/// the next launch or `stop` retries from it.
async fn finish_record(
    ctx: &StageCtx,
    path: &Path,
    state: &mut SessionState,
    mode: RestoreMode,
) -> Result<bool> {
    if restore_complete(state, mode) {
        // Run-id guarded: this pass may have taken seconds (an audio probe, a
        // `SIGTERM`, two `adb forward --remove`s), and a launch that started
        // meanwhile has already written its own record over this path. Only
        // the record this pass actually reconciled is ours to delete.
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

// ── row texts ─────────────────────────────────────────────────────────────────

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

// ── detach ────────────────────────────────────────────────────────────────────

/// Detach from a live session: mark the state file `detached`, fire the
/// handle's `detach` token, and leave every guard alone.
///
/// Takes [`Paths`] rather than a [`StageCtx`] because app-quit has no stage
/// context to hand — it is unwinding the app, not running a stage.
///
/// The supervisor is the authority here: firing `handle.detach` is what makes
/// it disarm both guards, mark the record `detached`, and drop out of
/// [`crate::session::LIVE_SESSION`]. This function triggers that, waits up to
/// [`DETACH_WAIT`] for the slot to be released (app-quit needs the bookkeeping
/// finished before the process goes away), and then — only once the supervisor
/// has **provably** let go through this detach — sets `detached` itself if the
/// flag did not make it to disk. That last step is a safety net, and it is
/// hedged three ways, because every one of these was a way to relabel a session
/// that in fact stopped:
///
/// * it runs only when the wait ended in the slot *clearing*, never on the
///   [`DETACH_WAIT`] timeout — a supervisor that is still holding the slot is
///   still writing to that record;
/// * it re-checks `handle.cancel` afterwards: a Stop that fired during the wait
///   is terminal and owns the teardown, and its record (kept because a guard
///   could not be released) must not come back as `detached: true` — the app
///   then tells the user the game "is still running, unsupervised" about a
///   session it stopped;
/// * it goes through [`state::mark_detached`], which creates nothing: a record
///   the supervisor already cleared stays cleared.
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
    // Stop is terminal, and detach is subordinate to it. Both tokens feed one
    // unbiased `select!` in the supervisor, so a detach fired *after* a Stop
    // can still win that race — disarming the guards, marking the record
    // `detached` and leaving wine running, while the Stop caller watches the
    // live slot empty and reports success. A Stop that has fired can never be
    // superseded here, and cancellation is monotonic, so this check cannot
    // race back the other way.
    //
    // This is also the return that silently absorbs the Tauri quit dialog's
    // stop-then-timeout arm (`commands::resolve_quit`, which fires `cancel`
    // and only then calls detach): nothing is detached there, and the message
    // that arm renders has to say so itself — this function cannot.
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
mod tests {
    use super::*;
    use crate::events::{Severity, StageEvent};
    use crate::executor::{DetachedChild, Executor, PlannedAction, PlannedKind};
    use crate::paths::{Bottle, Paths};
    use crate::process::ProcInfo;
    use crate::session::state::WiredForward;
    use crate::stages::{EventSink, StageCtx, StageOptions};
    use std::path::Path;
    use std::sync::{Arc, Mutex as StdMutex};
    use tokio_util::sync::CancellationToken;
    use uuid::Uuid;

    /// A pid no process can have on macOS (the default `kern.maxproc` ceiling
    /// is five digits), chosen instead of spawning a throwaway child so the
    /// "dead" case is deterministic and free of a pid-reuse race.
    ///
    /// Deliberately **not** `u32::MAX`: that is `-1` as an `i32`, and
    /// `kill(-1, …)` addresses *every* process the user can signal.
    const DEAD_PID: u32 = 2_147_483_646;

    // ── fixtures ─────────────────────────────────────────────────────────────

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sabrage-reconcile-{tag}-{}-{}",
            std::process::id(),
            Uuid::new_v4().simple()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A context whose Sabrage store and adb both point **inside the fixture**.
    ///
    /// `sabrage_appsup` is the load-bearing override: without it
    /// `Paths::new` derives it from the real `$HOME` and these tests would read
    /// — and with a real executor, write — the developer's own
    /// `session-state.json`.
    fn test_ctx(dir: &Path, dry_run: bool) -> (StageCtx, Arc<StdMutex<Vec<StageEvent>>>) {
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let s = seen.clone();
        let sink: EventSink = Arc::new(move |ev| s.lock().unwrap().push(ev));
        let mut paths = Paths::new(dir.join("repo"));
        paths.sabrage_appsup = dir.join("appsup");
        paths.adb = Some(dir.join("bin/adb"));
        let ctx = StageCtx::new(
            paths,
            StageOptions {
                dry_run,
                ..StageOptions::default()
            },
            sink,
            CancellationToken::new(),
        );
        (ctx, seen)
    }

    fn me() -> ProcInfo {
        ProcInfo::observe(std::process::id()).expect("this test process is observable")
    }

    fn recycled() -> ProcInfo {
        let mut p = me();
        p.start_time += 1;
        p
    }

    fn dead() -> ProcInfo {
        ProcInfo {
            pid: DEAD_PID,
            start_time: 1,
            exe: PathBuf::from("/Applications/CrossOver.app/…/wine"),
        }
    }

    /// A record with every guard still pending.
    fn pending(wine: Option<ProcInfo>, dashboard: Option<ProcInfo>) -> SessionState {
        SessionState {
            wine,
            dashboard,
            prev_audio_output: Some("MacBook Pro Speakers".into()),
            wired_forwards: vec![
                WiredForward {
                    serial: "1WMHH000X00000".into(),
                    port: 9943,
                },
                WiredForward {
                    serial: "1WMHH000X00000".into(),
                    port: 9944,
                },
            ],
            ..SessionState::new(
                Uuid::new_v4(),
                "Steam",
                "/games/Beat Saber 1294",
                "/repo/logs/beatsaber-20260829-101112.log",
                1_786_300_214_181,
            )
        }
    }

    fn write_state(ctx: &StageCtx, state: &SessionState) {
        let path = ctx.paths.session_state_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut bytes = serde_json::to_vec_pretty(state).unwrap();
        bytes.push(b'\n');
        std::fs::write(&path, bytes).unwrap();
    }

    /// Probe stub: the tool is installed, reports `device`, and lists nothing
    /// (the fallback pool only matters where a test says it does).
    fn probing(
        device: &str,
    ) -> impl FnOnce() -> std::future::Ready<Result<Option<AudioProbe>>> + '_ {
        probing_list(device, &[])
    }

    /// [`probing`] with a device list — `SwitchAudioSource -a -t output`.
    fn probing_list<'a>(
        device: &'a str,
        outputs: &'a [&'a str],
    ) -> impl FnOnce() -> std::future::Ready<Result<Option<AudioProbe>>> + 'a {
        move || {
            std::future::ready(Ok(Some(AudioProbe {
                bin: PathBuf::from("/fixture/SwitchAudioSource"),
                current: device.into(),
                outputs: outputs.iter().map(|o| o.to_string()).collect(),
            })))
        }
    }

    /// Probe stub: no `SwitchAudioSource` on this machine.
    fn no_probe() -> impl FnOnce() -> std::future::Ready<Result<Option<AudioProbe>>> {
        || std::future::ready(Ok(None))
    }

    /// Probe stub reporting a binary **inside the fixture that does not exist**,
    /// so a *real* executor's `run_child` fails at `spawn` with `ENOENT`. The
    /// cheapest way to make one reconcile mutation fail for real without
    /// touching the machine — nothing is executed, and the device is untouched.
    fn probing_a_vanished_binary(
        dir: &Path,
    ) -> impl FnOnce() -> std::future::Ready<Result<Option<AudioProbe>>> {
        let bin = dir.join("bin/SwitchAudioSource");
        move || {
            std::future::ready(Ok(Some(AudioProbe {
                bin,
                current: BLACKHOLE.to_string(),
                outputs: Vec::new(),
            })))
        }
    }

    /// The device of the 2026-08-29 finding: recorded at launch, disconnected
    /// by the time `stop` ran.
    const AIRPODS: &str = "Yifei\u{2019}s AirPods Pro";

    /// `SwitchAudioSource -a -t output` on that machine, verbatim and in order.
    const LIVE_OUTPUTS: [&str; 6] = [
        "BlackHole 2ch",
        "MacBook Pro Speakers",
        "Steam Streaming Microphone",
        "Steam Streaming Speakers",
        "Virtual Desktop Mic",
        "Virtual Desktop Speakers",
    ];

    /// A [`crate::executor::DryRunExecutor`] whose children come back
    /// **non-zero** whenever `device` is one of their arguments — the machine
    /// failure this fallback exists for, where
    /// `SwitchAudioSource -t output -s "…AirPods Pro"` printed `Could not find
    /// an audio device named … Nothing was changed.` and exited 1.
    ///
    /// Everything else delegates, and the inner executor still *sees* the
    /// child, so the plan records every attempt in order.
    #[derive(Debug)]
    struct FailSwitchTo {
        inner: Arc<dyn Executor>,
        device: std::ffi::OsString,
    }

    impl FailSwitchTo {
        fn around(inner: Arc<dyn Executor>, device: &str) -> Arc<FailSwitchTo> {
            Arc::new(FailSwitchTo {
                inner,
                device: device.into(),
            })
        }
    }

    /// `exit 1` as an [`std::process::ExitStatus`], the way `wait(2)` encodes it.
    fn exit_1() -> std::process::ExitStatus {
        use std::os::unix::process::ExitStatusExt;
        std::process::ExitStatus::from_raw(1 << 8)
    }

    impl Executor for FailSwitchTo {
        fn with_step(&self, step: crate::events::StepId) -> Arc<dyn Executor> {
            Arc::new(FailSwitchTo {
                inner: self.inner.with_step(step),
                device: self.device.clone(),
            })
        }
        fn is_dry_run(&self) -> bool {
            self.inner.is_dry_run()
        }
        fn planned(&self) -> Vec<PlannedAction> {
            self.inner.planned()
        }
        fn copy_if_changed<'a>(
            &'a self,
            src: &'a Path,
            dst: &'a Path,
        ) -> crate::executor::BoxFuture<'a, Result<crate::executor::Copied>> {
            self.inner.copy_if_changed(src, dst)
        }
        fn write_atomic<'a>(
            &'a self,
            path: &'a Path,
            bytes: &'a [u8],
        ) -> crate::executor::BoxFuture<'a, Result<()>> {
            self.inner.write_atomic(path, bytes)
        }
        fn remove_dir_all<'a>(&'a self, p: &'a Path) -> crate::executor::BoxFuture<'a, Result<()>> {
            self.inner.remove_dir_all(p)
        }
        fn remove_file<'a>(&'a self, p: &'a Path) -> crate::executor::BoxFuture<'a, Result<()>> {
            self.inner.remove_file(p)
        }
        fn create_dir_all<'a>(&'a self, p: &'a Path) -> crate::executor::BoxFuture<'a, Result<()>> {
            self.inner.create_dir_all(p)
        }
        fn dir_copy<'a>(
            &'a self,
            src: &'a Path,
            dst: &'a Path,
        ) -> crate::executor::BoxFuture<'a, Result<()>> {
            self.inner.dir_copy(src, dst)
        }
        fn download<'a>(
            &'a self,
            url: &'a str,
            dest: &'a Path,
            sha256: &'a str,
            label: &'a str,
        ) -> crate::executor::BoxFuture<'a, Result<crate::executor::Downloaded>> {
            self.inner.download(url, dest, sha256, label)
        }
        fn tar_xzf<'a>(
            &'a self,
            archive: &'a Path,
            into_dir: &'a Path,
        ) -> crate::executor::BoxFuture<'a, Result<()>> {
            self.inner.tar_xzf(archive, into_dir)
        }
        fn touch<'a>(&'a self, p: &'a Path) -> crate::executor::BoxFuture<'a, Result<()>> {
            self.inner.touch(p)
        }
        fn run_child<'a>(
            &'a self,
            spec: &'a crate::process::ChildSpec,
        ) -> crate::executor::BoxFuture<'a, Result<std::process::ExitStatus>> {
            let fails = spec.args.contains(&self.device);
            Box::pin(async move {
                let status = self.inner.run_child(spec).await?;
                Ok(if fails { exit_1() } else { status })
            })
        }
        fn spawn_detached<'a>(
            &'a self,
            spec: &'a crate::process::ChildSpec,
            stdio: crate::executor::DetachedStdio,
        ) -> crate::executor::BoxFuture<'a, Result<Option<DetachedChild>>> {
            self.inner.spawn_detached(spec, stdio)
        }
    }

    fn rows(seen: &Arc<StdMutex<Vec<StageEvent>>>) -> Vec<(Severity, String)> {
        seen.lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                StageEvent::Line {
                    severity,
                    text,
                    step,
                    ..
                } => {
                    assert_eq!(
                        step.as_deref(),
                        Some(STEP),
                        "every reconcile row is attributed"
                    );
                    Some((*severity, text.clone()))
                }
                _ => None,
            })
            .collect()
    }

    fn sections(seen: &Arc<StdMutex<Vec<StageEvent>>>) -> Vec<String> {
        seen.lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                StageEvent::Section { title, .. } => Some(title.clone()),
                _ => None,
            })
            .collect()
    }

    fn spawns(planned: &[PlannedAction]) -> Vec<String> {
        planned
            .iter()
            .filter(|p| p.kind == PlannedKind::Spawn)
            .map(|p| p.reason.clone())
            .collect()
    }

    // ── classify ─────────────────────────────────────────────────────────────

    #[test]
    fn classify_reads_pid_and_start_time_only() {
        assert_eq!(classify(&pending(None, None)), Classification::Dead);
        assert_eq!(classify(&pending(Some(dead()), None)), Classification::Dead);
        assert_eq!(classify(&pending(Some(me()), None)), Classification::Live);
        assert_eq!(
            classify(&pending(Some(recycled()), None)),
            Classification::IdentityMismatch,
        );
    }

    #[test]
    fn a_zero_pid_is_dead_never_a_process_group() {
        let zero = ProcInfo {
            pid: 0,
            start_time: 0,
            exe: PathBuf::from("/"),
        };
        assert_eq!(
            classify(&pending(Some(zero.clone()), None)),
            Classification::Dead
        );
        assert!(!signalable(&zero), "pid 0 must never be signalled");
        assert!(!signalable(&dead()), "a gone pid must never be signalled");
        assert!(
            !signalable(&recycled()),
            "a recycled pid must never be signalled"
        );
        assert!(signalable(&me()));
    }

    #[test]
    fn the_spawn_fallback_start_time_can_never_match() {
        // `spawn_detached` records start_time 0 when it cannot observe the pid;
        // a live pid with that record is the conservative branch, not Live —
        // and not `IdentityMismatch` either, which would claim to know the pid
        // was recycled and go on to undo the guards of what may be the running
        // session. Its own answer, which restores nothing (A9-5).
        let unobserved = ProcInfo {
            start_time: 0,
            ..me()
        };
        assert!(
            !unobserved.is_same_process(),
            "0 never matches a real start"
        );
        assert_eq!(
            classify(&pending(Some(unobserved.clone()), None)),
            Classification::Unverifiable
        );
        assert_eq!(restore_mode(Classification::Unverifiable), None);
        assert!(
            !signalable(&unobserved),
            "and nothing may be signalled on that identity"
        );
        // A start time that IS observed and differs really is a recycled pid.
        assert_eq!(
            classify(&pending(Some(recycled()), None)),
            Classification::IdentityMismatch
        );
    }

    // ── reconcile ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn no_state_file_is_no_session_and_says_nothing() {
        let dir = scratch("nosession");
        let (ctx, seen) = test_ctx(&dir, true);
        let out = reconcile_with(&ctx, None, None, no_probe()).await.unwrap();
        assert_eq!(out, Reconciled::NoSession);
        assert!(seen.lock().unwrap().is_empty());
        assert!(ctx.executor.planned().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Every live-shaped record is adopted untouched, whatever kind of live it is.
    #[tokio::test]
    async fn live_records_are_adopted_without_touching_anything() {
        let unobserved = ProcInfo {
            start_time: 0,
            ..me()
        };
        let cases: &[(&str, ProcInfo, Option<ProcInfo>, Classification)] = &[
            ("live", me(), Some(me()), Classification::Live),
            (
                "unverifiable [r1:A9-5]",
                unobserved,
                None,
                Classification::Unverifiable,
            ),
        ];
        for (i, (label, wine, dashboard, class)) in cases.iter().enumerate() {
            let dir = scratch(&format!("live-reconcile-{i}"));
            let (ctx, seen) = test_ctx(&dir, true);
            let state = pending(Some(wine.clone()), dashboard.clone());
            assert_eq!(classify(&state), *class, "{label}: classification");
            write_state(&ctx, &state);

            let out = reconcile_with(&ctx, None, None, probing(BLACKHOLE))
                .await
                .unwrap();
            assert_eq!(out, Reconciled::Live { state }, "{label}");
            assert!(
                seen.lock().unwrap().is_empty(),
                "{label}: adoption is silent"
            );
            assert!(
                ctx.executor.planned().is_empty(),
                "{label}: no SwitchAudioSource, no adb forward --remove, no clear"
            );
            assert!(
                ctx.paths.session_state_path().exists(),
                "{label}: a live session's record must survive"
            );
            std::fs::remove_dir_all(&dir).ok();
        }
    }

    #[tokio::test]
    async fn a_dead_session_restores_every_guard_then_clears_the_record() {
        let dir = scratch("dead");
        let (ctx, seen) = test_ctx(&dir, true);
        // Dashboard identity = this test process, so `is_same_process` holds and
        // the kill is *planned*. Under DryRunExecutor nothing is ever spawned.
        write_state(&ctx, &pending(Some(dead()), Some(me())));

        let out = reconcile_with(&ctx, None, None, probing(BLACKHOLE))
            .await
            .unwrap();
        let Reconciled::Dead {
            state,
            restored,
            pending,
        } = out
        else {
            panic!("expected Dead, got {out:?}");
        };
        assert!(!pending, "every guard was released, so the record went");
        assert!(state.guards.audio_restored);
        assert!(state.guards.dashboard_closed);
        assert!(state.guards.forwards_cleared);
        assert!(!state.has_pending_guards());

        assert_eq!(
            restored,
            vec![
                "audio: would restore output -> MacBook Pro Speakers (previous session did not \
                 shut down cleanly)",
                "ALVR dashboard would be closed (left over from the previous session)",
                "would clear adb forward tcp:9943 on 1WMHH000X00000",
                "would clear adb forward tcp:9944 on 1WMHH000X00000",
            ]
        );
        assert_eq!(sections(&seen), vec![RECONCILE_SECTION], "banner once");
        assert_eq!(
            rows(&seen).into_iter().map(|(s, _)| s).collect::<Vec<_>>(),
            vec![Severity::Ok, Severity::Ok, Severity::Info, Severity::Info],
        );

        // Mutations, in order: switch, save, kill, save, then each removal
        // followed by its own save (A9-4: progress has to be crash-durable —
        // a record that still claims a removed forward is installed can never
        // be completed, because re-removing an absent listener exits
        // non-zero), the final forwards save, and the clear. Every flag hits
        // disk before the next guard is touched.
        let planned = ctx.executor.planned();
        let kinds: Vec<PlannedKind> = planned.iter().map(|p| p.kind).collect();
        assert_eq!(
            kinds,
            vec![
                PlannedKind::Spawn,
                PlannedKind::CreateDir,
                PlannedKind::Write,
                PlannedKind::Spawn,
                PlannedKind::CreateDir,
                PlannedKind::Write,
                PlannedKind::Spawn,
                PlannedKind::CreateDir,
                PlannedKind::Write,
                PlannedKind::Spawn,
                PlannedKind::CreateDir,
                PlannedKind::Write,
                PlannedKind::CreateDir,
                PlannedKind::Write,
                PlannedKind::RemoveFile,
            ]
        );
        let argv = spawns(&planned);
        assert_eq!(
            argv[0],
            "/fixture/SwitchAudioSource -t output -s MacBook Pro Speakers"
        );
        assert_eq!(argv[1], format!("/bin/kill -TERM {}", std::process::id()));
        assert!(argv[2].ends_with("-s 1WMHH000X00000 forward --remove tcp:9943"));
        assert!(argv[3].ends_with("-s 1WMHH000X00000 forward --remove tcp:9944"));
        assert!(
            !argv.iter().any(|a| a.contains("--remove-all")),
            "never --remove-all: {argv:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn an_identity_mismatch_restores_the_pid_free_guards_and_signals_nothing() {
        let dir = scratch("mismatch");
        let (ctx, seen) = test_ctx(&dir, true);
        write_state(&ctx, &pending(Some(recycled()), Some(me())));

        let out = reconcile_with(&ctx, None, None, probing(BLACKHOLE))
            .await
            .unwrap();
        let Reconciled::IdentityMismatch {
            state,
            restored,
            pending,
        } = out
        else {
            panic!("expected IdentityMismatch, got {out:?}");
        };
        assert!(!pending);
        assert!(state.guards.audio_restored);
        assert!(
            !state.guards.dashboard_closed,
            "SafeOnly leaves the dashboard untouched and unflagged"
        );
        assert!(state.guards.forwards_cleared);
        assert_eq!(restored.len(), 3, "audio + two forwards, no dashboard");

        let argv = spawns(&ctx.executor.planned());
        assert!(
            !argv.iter().any(|a| a.contains("/bin/kill")),
            "a recycled pid must never be signalled: {argv:?}"
        );
        assert_eq!(sections(&seen), vec![RECONCILE_SECTION]);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A live process that is not this one, for the ownership guard. Killed on
    /// drop, so no fixture can leak a process onto the machine.
    struct ForeignProcess(std::process::Child);

    impl ForeignProcess {
        fn spawn() -> ForeignProcess {
            ForeignProcess(
                std::process::Command::new("/bin/sleep")
                    .arg("30")
                    .spawn()
                    .expect("/bin/sleep is on every macOS"),
            )
        }
        fn pid(&self) -> u32 {
            self.0.id()
        }
    }

    impl Drop for ForeignProcess {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    /// A9-1. The window between the first guard and the wine spawn: the record
    /// exists, `wine` is still `None` (so `classify` says `Dead`), and there is
    /// no live handle yet — only the run stage's published phase knows this
    /// launch is happening. Reconciling it (the Session screen remounts, and
    /// its `onMount` reconcile takes no lock) would restore the audio device
    /// mid-launch, `SIGTERM` the dashboard this launch just spawned, pull its
    /// `--wired` forwards and delete its record, under a launch that keeps
    /// going.
    #[tokio::test]
    async fn a_record_belonging_to_the_launch_in_progress_is_never_touched() {
        for phase in [
            crate::session::SessionPhase::Preflight,
            crate::session::SessionPhase::Launching,
            crate::session::SessionPhase::Stopping,
        ] {
            let dir = scratch("in-flight");
            let (ctx, seen) = test_ctx(&dir, true);
            // Exactly the pre-spawn record: guards taken, no wine child.
            let state = pending(None, Some(me()));
            write_state(&ctx, &state);

            let out = reconcile_with(
                &ctx,
                None,
                Some(crate::session::RunPhaseInfo {
                    phase,
                    run_id: state.run_id,
                    bottle: "Steam".into(),
                    exit_code: None,
                }),
                probing(BLACKHOLE),
            )
            .await
            .unwrap();

            assert_eq!(
                out,
                Reconciled::Busy {
                    state,
                    reason: RECORD_IN_FLIGHT.to_string(),
                    silent: true,
                },
                "{phase:?}"
            );
            assert!(
                ctx.executor.planned().is_empty(),
                "{phase:?}: no switch, no kill, no removal, no write"
            );
            assert!(
                seen.lock().unwrap().is_empty(),
                "{phase:?}: our own launch is not a warning"
            );
            assert!(
                ctx.paths.session_state_path().exists(),
                "{phase:?}: the launch still needs its record"
            );
            std::fs::remove_dir_all(&dir).ok();
        }
    }

    /// …and a phase published for a *different* run does not shield a stale
    /// record: that is the ordinary "recover before launching" case, and it is
    /// what `run` calls reconcile for.
    #[tokio::test]
    async fn a_launch_in_progress_does_not_shield_some_other_runs_record() {
        let dir = scratch("in-flight-other");
        let (ctx, _seen) = test_ctx(&dir, true);
        let mut state = pending(Some(dead()), None);
        state.wired_forwards.clear();
        write_state(&ctx, &state);

        let out = reconcile_with(
            &ctx,
            None,
            Some(crate::session::RunPhaseInfo {
                phase: crate::session::SessionPhase::Preflight,
                run_id: Uuid::new_v4(),
                bottle: "Steam".into(),
                exit_code: None,
            }),
            probing(BLACKHOLE),
        )
        .await
        .unwrap();

        assert!(matches!(out, Reconciled::Dead { .. }), "{out:?}");
        assert!(
            !ctx.executor.planned().is_empty(),
            "the previous session's guards are still this launch's to clean up"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A9-3. The other front-end's record: `sabrage run` in a terminal next to
    /// an open Sabrage. `owner_pid` says who is running it, and the field's own
    /// documentation has always said reconcile must not touch its guards.
    #[tokio::test]
    async fn a_record_a_live_foreign_process_owns_is_reported_and_left_alone() {
        let dir = scratch("owned-elsewhere");
        let (ctx, seen) = test_ctx(&dir, true);
        let foreign = ForeignProcess::spawn();
        let mut state = pending(None, Some(me()));
        state.set_owner(foreign.pid());
        write_state(&ctx, &state);

        let out = reconcile_with(&ctx, None, None, probing(BLACKHOLE))
            .await
            .unwrap();

        assert_eq!(
            out,
            Reconciled::Busy {
                state,
                reason: owned_elsewhere_row(foreign.pid()),
                silent: false,
            }
        );
        assert!(ctx.executor.planned().is_empty());
        assert_eq!(
            rows(&seen),
            vec![(Severity::Warn, owned_elsewhere_row(foreign.pid()))],
            "the user is told why nothing was restored"
        );
        assert!(ctx.paths.session_state_path().exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A9-8. A record from a newer Sabrage may describe a guard this build
    /// cannot undo: rewriting it through this struct would drop that
    /// description, and clearing it would throw it away entirely.
    #[tokio::test]
    async fn a_newer_schema_record_is_reported_and_never_rewritten() {
        let dir = scratch("newer-schema");
        let (ctx, seen) = test_ctx(&dir, true);
        let path = ctx.paths.session_state_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut state = pending(Some(dead()), None);
        state.version = state::SESSION_STATE_VERSION + 1;
        let mut json = serde_json::to_value(&state).unwrap();
        json["futureGuard"] = serde_json::json!({ "somethingWeCannotUndo": true });
        std::fs::write(&path, serde_json::to_vec_pretty(&json).unwrap()).unwrap();

        let out = reconcile_with(&ctx, None, None, probing(BLACKHOLE))
            .await
            .unwrap();

        assert!(
            matches!(&out, Reconciled::Busy { reason, .. } if *reason == newer_schema_row(2)),
            "{out:?}"
        );
        assert!(
            ctx.executor.planned().is_empty(),
            "a guard we do not understand is not a guard we may release"
        );
        assert_eq!(rows(&seen), vec![(Severity::Warn, newer_schema_row(2))]);
        assert!(path.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A9-5. `start_time == 0` is the spawn fallback's "could not observe",
    /// not evidence of a recycled pid. While that pid is alive, dismantling
    /// the session's guards — switching the audio device back, pulling the
    /// `--wired` forwards that carry the stream — may be disconnecting the
    /// running session.
    /// A9-1. Every `Busy` except this process's own in-flight record is a
    /// record somebody is still using — and the launch path has to be able to
    /// *tell*, or it carries on into preflight's auto-fixes, `adb forward
    /// --remove` and the bottle's `wineserver -k`, taking down the session the
    /// classification just refused to touch.
    #[tokio::test]
    async fn every_busy_but_our_own_in_flight_record_is_a_refusal() {
        let dir = scratch("busy-refusal");
        let (ctx, _seen) = test_ctx(&dir, true);

        // Our own launch, mid-flight: nothing is wrong, nothing to refuse.
        let mine = pending(Some(dead()), None);
        write_state(&ctx, &mine);
        let out = reconcile_with(
            &ctx,
            None,
            Some(crate::session::RunPhaseInfo {
                phase: crate::session::SessionPhase::Preflight,
                run_id: mine.run_id,
                bottle: "Steam".into(),
                exit_code: None,
            }),
            probing(BLACKHOLE),
        )
        .await
        .unwrap();
        assert!(matches!(out, Reconciled::Busy { silent: true, .. }));
        assert_eq!(out.busy_refusal(), None);

        // Another live front-end's record: a refusal, carrying the row the
        // user saw.
        let foreign = ForeignProcess::spawn();
        let mut theirs = pending(None, None);
        theirs.set_owner(foreign.pid());
        write_state(&ctx, &theirs);
        let out = reconcile_with(&ctx, None, None, probing(BLACKHOLE))
            .await
            .unwrap();
        assert_eq!(
            out.busy_refusal(),
            Some(owned_elsewhere_row(foreign.pid()).as_str())
        );

        // A newer schema: likewise.
        let mut newer = pending(Some(dead()), None);
        newer.version = state::SESSION_STATE_VERSION + 1;
        write_state(&ctx, &newer);
        let out = reconcile_with(&ctx, None, None, probing(BLACKHOLE))
            .await
            .unwrap();
        assert_eq!(
            out.busy_refusal(),
            Some(newer_schema_row(newer.version).as_str())
        );

        // And the outcomes a launch may carry on over say nothing.
        assert_eq!(Reconciled::NoSession.busy_refusal(), None);
        assert_eq!(
            (Reconciled::Dead {
                state: mine.clone(),
                restored: Vec::new(),
                pending: false,
            })
            .busy_refusal(),
            None
        );

        assert!(ctx.executor.planned().is_empty(), "nothing was touched");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A9-4. `adb forward --remove` that comes back non-zero is
    /// indeterminate — usually the device is gone and took the forward with
    /// it, but it may be a still-installed `tcp:9943` that will silently break
    /// the next WiFi discovery. Flagging the guard released on that clears the
    /// record and leaves nothing on the machine that knows the port is there.
    #[tokio::test]
    async fn a_forward_that_could_not_be_removed_keeps_the_record() {
        let dir = scratch("forward-stuck");
        let (mut ctx, seen) = test_ctx(&dir, true);
        // Only the 9943 removal fails; 9944's succeeds.
        ctx.executor = FailSwitchTo::around(ctx.executor.clone(), "tcp:9943");
        let mut state = pending(Some(dead()), None);
        state.prev_audio_output = None;
        write_state(&ctx, &state);

        let out = reconcile_with(&ctx, None, None, no_probe()).await.unwrap();
        let Reconciled::Dead {
            state,
            restored,
            pending,
        } = out
        else {
            panic!("expected Dead, got {out:?}");
        };

        assert!(pending, "the record is kept, so the caller is told");
        assert!(
            !state.guards.forwards_cleared,
            "one removal did not take, so the guard is not released"
        );
        assert_eq!(
            state
                .wired_forwards
                .iter()
                .map(|f| f.port)
                .collect::<Vec<_>>(),
            vec![9943],
            "the port that is still installed stays on the record; the removed one goes"
        );
        assert_eq!(
            restored,
            vec!["would clear adb forward tcp:9944 on 1WMHH000X00000".to_string()],
            "only what really happened is reported"
        );
        assert_eq!(
            rows(&seen).last().cloned(),
            Some((Severity::Info, RECORD_KEPT.to_string()))
        );

        let planned = ctx.executor.planned();
        assert!(
            !planned.iter().any(|p| p.kind == PlannedKind::RemoveFile),
            "the record is what the next stop reads: {planned:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A9-4. The removals are not atomic with the record that describes them:
    /// a crash (or a Cancel, or a power loss) between two `adb forward
    /// --remove`s must leave a record naming only what is *still* installed.
    /// Removing 9943 and then crashing used to leave both ports on disk, and
    /// the retry's `--remove` of an already-absent listener exits non-zero —
    /// which this module reads as "still installed", so the phantom row was
    /// kept forever and the guard never released.
    #[tokio::test]
    async fn a_removal_that_took_is_on_disk_before_the_next_one_is_tried() {
        let dir = scratch("forward-crash-resume");
        let (ctx, _seen) = test_ctx(&dir, false);
        let record = ctx.paths.session_state_path();
        let snapshot = dir.join("record-as-the-second-removal-saw-it.json");

        // A stub `adb` that fails the *second* removal — and, before failing,
        // copies the record exactly as it stands at that moment. That copy is
        // what a crash right there would have left behind.
        let adb = ctx
            .paths
            .adb
            .clone()
            .expect("test_ctx points adb at the fixture");
        std::fs::create_dir_all(adb.parent().unwrap()).unwrap();
        std::fs::write(
            &adb,
            format!(
                "#!/bin/sh\ncase \"$*\" in\n  *9944*) cp '{}' '{}'; exit 1;;\nesac\nexit 0\n",
                record.display(),
                snapshot.display()
            ),
        )
        .unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&adb, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let mut state = pending(Some(dead()), None);
        state.prev_audio_output = None; // no audio probe in this test
        write_state(&ctx, &state);

        let out = reconcile_with(&ctx, None, None, no_probe()).await.unwrap();
        let Reconciled::Dead { pending, .. } = out else {
            panic!("expected Dead, got {out:?}");
        };
        assert!(pending, "9944 is still installed, so the record is kept");

        let mid = state::load(&snapshot)
            .unwrap()
            .expect("the stub copied the record");
        assert_eq!(
            mid.wired_forwards
                .iter()
                .map(|f| f.port)
                .collect::<Vec<_>>(),
            vec![9944],
            "a crash after the 9943 removal must not leave 9943 on the record"
        );
        assert!(!mid.guards.forwards_cleared);

        let after = state::load(&record).unwrap().expect("the record is kept");
        assert_eq!(
            after
                .wired_forwards
                .iter()
                .map(|f| f.port)
                .collect::<Vec<_>>(),
            vec![9944]
        );
        assert!(!after.guards.forwards_cleared);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn a_record_this_process_still_supervises_is_reported_but_never_touched() {
        let dir = scratch("owned");
        let (ctx, seen) = test_ctx(&dir, true);
        let state = pending(Some(dead()), Some(me()));
        write_state(&ctx, &state);

        let out = reconcile_with(&ctx, Some(state.run_id), None, probing(BLACKHOLE))
            .await
            .unwrap();
        assert_eq!(
            out,
            Reconciled::Dead {
                state,
                restored: Vec::new(),
                pending: false,
            }
        );
        assert!(
            ctx.executor.planned().is_empty(),
            "the supervise loop owns it"
        );
        assert!(seen.lock().unwrap().is_empty());
        assert!(ctx.paths.session_state_path().exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn a_corrupt_record_warns_instead_of_blocking_the_next_launch() {
        let dir = scratch("corrupt");
        let (ctx, seen) = test_ctx(&dir, true);
        let path = ctx.paths.session_state_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{not json").unwrap();

        let out = reconcile_with(&ctx, None, None, no_probe()).await.unwrap();
        assert_eq!(out, Reconciled::NoSession);

        let seen_rows = rows(&seen);
        assert_eq!(seen_rows[0].0, Severity::Warn);
        assert!(seen_rows[0]
            .1
            .starts_with("previous session state unreadable: "));
        assert_eq!(seen_rows[1].0, Severity::Info);
        assert_eq!(
            seen_rows[1].1,
            format!("delete {} to clear this warning", path.display())
        );
        assert!(path.exists(), "a record we could not read is not deleted");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The 2026-08-29 finding: the recorded device (AirPods) had disconnected,
    /// so the switch exited non-zero, the Mac stayed on BlackHole — silent —
    /// and the record was cleared anyway. Now the built-in speakers take over.
    #[tokio::test]
    async fn a_recorded_device_that_is_gone_falls_back_to_the_built_in_output() {
        let dir = scratch("dead-audio-fallback");
        let (mut ctx, seen) = test_ctx(&dir, true);
        ctx.executor = FailSwitchTo::around(ctx.executor.clone(), AIRPODS);
        let mut state = pending(Some(dead()), None);
        state.prev_audio_output = Some(AIRPODS.into());
        state.wired_forwards.clear();
        write_state(&ctx, &state);

        let out = reconcile_with(&ctx, None, None, probing_list(BLACKHOLE, &LIVE_OUTPUTS))
            .await
            .unwrap();
        let Reconciled::Dead {
            state,
            restored,
            pending,
        } = out
        else {
            panic!("expected Dead, got {out:?}");
        };
        assert!(!pending, "the fallback IS a restore, so the record goes");
        let expected = format!(
            "recorded output device '{AIRPODS}' is not connected — would restore output -> \
             MacBook Pro Speakers instead (previous session did not shut down cleanly)"
        );
        assert_eq!(restored, vec![expected.clone()]);
        assert!(
            state.guards.audio_restored,
            "landing somewhere audible IS a restore"
        );
        // A warn, not an ok: this is not the device the user had.
        assert_eq!(rows(&seen), vec![(Severity::Warn, expected)]);
        assert_eq!(sections(&seen), vec![RECONCILE_SECTION], "it is a recovery");

        let planned = ctx.executor.planned();
        assert_eq!(
            spawns(&planned),
            vec![
                format!("/fixture/SwitchAudioSource -t output -s {AIRPODS}"),
                "/fixture/SwitchAudioSource -t output -s MacBook Pro Speakers".to_string(),
            ],
            "the recorded device is always tried first"
        );
        assert!(
            planned.iter().any(|p| p.kind == PlannedKind::RemoveFile),
            "every guard is done, so the record goes: {planned:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Nothing audible on the list: say what failed, name the recorded device
    /// and the two commands, and **keep the record** so the next launch or stop
    /// can try again.
    #[tokio::test]
    async fn with_nothing_to_fall_back_to_the_record_is_kept_with_the_remedy() {
        let dir = scratch("dead-audio-stuck");
        let (mut ctx, seen) = test_ctx(&dir, true);
        ctx.executor = FailSwitchTo::around(ctx.executor.clone(), AIRPODS);
        let mut state = pending(Some(dead()), None);
        state.prev_audio_output = Some(AIRPODS.into());
        state.wired_forwards.clear();
        write_state(&ctx, &state);

        let out = reconcile_with(
            &ctx,
            None,
            None,
            // Only the loopback and the streaming virtuals are connected.
            probing_list(
                BLACKHOLE,
                &[
                    "BlackHole 2ch",
                    "Virtual Desktop Mic",
                    "Virtual Desktop Speakers",
                ],
            ),
        )
        .await
        .unwrap();
        let Reconciled::Dead {
            state,
            restored,
            pending,
        } = out
        else {
            panic!("expected Dead, got {out:?}");
        };
        assert!(
            pending,
            "a kept record must be distinguishable from a finished one: the launch that \
             follows has to know `prev_audio_output` is still owed"
        );
        assert!(restored.is_empty(), "nothing was restored");
        assert!(!state.guards.audio_restored, "the guard stays pending");
        assert_eq!(
            rows(&seen),
            vec![
                (
                    Severity::Warn,
                    crate::session::audio_unrestorable_line(AIRPODS)
                ),
                (Severity::Info, RECORD_KEPT.to_string()),
            ]
        );
        assert!(
            sections(&seen).is_empty(),
            "no recovery happened, so no banner"
        );

        let planned = ctx.executor.planned();
        assert_eq!(
            spawns(&planned),
            vec![format!("/fixture/SwitchAudioSource -t output -s {AIRPODS}")],
            "no fallback was attempted: every device on offer is virtual"
        );
        assert!(
            !planned.iter().any(|p| p.kind == PlannedKind::RemoveFile),
            "the record is what a later restore reads: {planned:?}"
        );
        assert!(
            planned.iter().any(|p| p.kind == PlannedKind::Write),
            "…and it is re-saved, pending guard and all: {planned:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The same failure through `stop` — the entry point the live incident came
    /// in on (`sabrage stop --bottle Steam` after the owning Sabrage was
    /// `kill -9`ed).
    #[tokio::test]
    async fn stop_keeps_the_record_when_the_audio_could_not_be_restored() {
        let dir = scratch("stop-audio-stuck");
        let (mut ctx, seen) = stop_ctx(&dir, true);
        ctx.executor = FailSwitchTo::around(ctx.executor.clone(), AIRPODS);
        let mut state = pending(Some(dead()), None);
        state.prev_audio_output = Some(AIRPODS.into());
        state.wired_forwards.clear();
        write_state(&ctx, &state);

        finish_stopped_session_with(
            &ctx,
            None,
            None,
            probing_list(BLACKHOLE, &["BlackHole 2ch"]),
        )
        .await
        .expect("a stuck audio guard is a row, never a failed stage");

        assert_eq!(
            rows(&seen),
            vec![
                (
                    Severity::Warn,
                    crate::session::audio_unrestorable_line(AIRPODS)
                ),
                (Severity::Info, RECORD_KEPT.to_string()),
            ]
        );
        let planned = ctx.executor.planned();
        assert!(
            !planned.iter().any(|p| p.kind == PlannedKind::RemoveFile),
            "the next stop must still know what to restore: {planned:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The other half of the keep-the-record rule: a guard that was never taken
    /// counts as done, so a record with nothing in it is still cleared.
    #[tokio::test]
    async fn a_record_with_no_guards_at_all_is_still_cleared_in_silence() {
        let dir = scratch("dead-inert");
        let (ctx, seen) = test_ctx(&dir, true);
        let mut state = pending(Some(dead()), None);
        state.prev_audio_output = None;
        state.wired_forwards.clear();
        write_state(&ctx, &state);

        let out = reconcile_with(&ctx, None, None, no_probe()).await.unwrap();
        let Reconciled::Dead { restored, .. } = out else {
            panic!("expected Dead, got {out:?}");
        };
        assert!(restored.is_empty());
        assert!(seen.lock().unwrap().is_empty(), "nothing to announce");
        assert_eq!(
            ctx.executor
                .planned()
                .iter()
                .map(|p| p.kind)
                .collect::<Vec<_>>(),
            vec![PlannedKind::RemoveFile],
            "an inert guard is a released guard"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    // ── restore_with ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn a_missing_switchaudiosource_leaves_the_audio_guard_pending() {
        let dir = scratch("no-tool");
        let (ctx, _seen) = test_ctx(&dir, true);
        let mut state = pending(Some(dead()), None);
        state.wired_forwards.clear();

        let restored = restore_with(&ctx, &mut state, RestoreMode::Full, no_probe())
            .await
            .unwrap();
        assert!(restored.is_empty());
        assert!(
            !state.guards.audio_restored,
            "'could not look' is not 'nothing to do'"
        );
        assert!(ctx.executor.planned().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn an_already_released_record_restores_nothing_twice() {
        let dir = scratch("idempotent");
        let (ctx, seen) = test_ctx(&dir, true);
        let mut state = pending(Some(dead()), Some(me()));
        state.guards.audio_restored = true;
        state.guards.dashboard_closed = true;
        state.guards.forwards_cleared = true;

        let restored = restore_with(&ctx, &mut state, RestoreMode::Full, probing(BLACKHOLE))
            .await
            .unwrap();
        assert!(restored.is_empty());
        assert!(ctx.executor.planned().is_empty());
        assert!(sections(&seen).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn a_dead_dashboard_identity_is_flagged_without_a_signal() {
        let dir = scratch("dead-dashboard");
        let (ctx, _seen) = test_ctx(&dir, true);
        let mut state = pending(Some(dead()), Some(dead()));
        state.prev_audio_output = None;
        state.wired_forwards.clear();

        let restored = restore_with(&ctx, &mut state, RestoreMode::Full, no_probe())
            .await
            .unwrap();
        assert!(
            restored.is_empty(),
            "nothing was closed, so nothing is reported"
        );
        assert!(state.guards.dashboard_closed);
        assert!(spawns(&ctx.executor.planned()).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn without_adb_the_forwards_stay_pending() {
        let dir = scratch("no-adb");
        let (mut ctx, _seen) = test_ctx(&dir, true);
        ctx.paths.adb = None;
        let mut state = pending(Some(dead()), None);
        state.prev_audio_output = None;

        let restored = restore_with(&ctx, &mut state, RestoreMode::Full, no_probe())
            .await
            .unwrap();
        assert!(restored.is_empty());
        assert!(!state.guards.forwards_cleared);
        assert!(ctx.executor.planned().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The one `restore_with` test with a real executor. It is deliberately shaped so that no
    /// child can be spawned: the only pending guard is the audio device, and
    /// the probe reports it is already back — so the whole run is a flag flip
    /// plus one atomic write into the fixture directory.
    #[tokio::test]
    async fn each_flag_reaches_disk_as_its_guard_is_released() {
        let dir = scratch("persist");
        let (ctx, seen) = test_ctx(&dir, false);
        assert!(!ctx.executor.is_dry_run());

        let mut state = pending(Some(dead()), None);
        state.wired_forwards.clear();
        write_state(&ctx, &state);

        let restored = restore_with(
            &ctx,
            &mut state,
            RestoreMode::Full,
            probing("MacBook Pro Speakers"),
        )
        .await
        .unwrap();
        assert!(
            restored.is_empty(),
            "nothing was performed, so nothing is reported"
        );
        assert!(sections(&seen).is_empty(), "no banner for a silent pass");

        let path = ctx.paths.session_state_path();
        let on_disk = state::load(&path).unwrap().expect("record still present");
        assert!(
            on_disk.guards.audio_restored,
            "the flag was saved, not just set"
        );
        assert_eq!(on_disk.run_id, state.run_id);
        std::fs::remove_dir_all(&dir).ok();
    }

    // ── row texts ────────────────────────────────────────────────────────────

    #[test]
    fn the_row_texts_are_stable_in_both_verbs() {
        assert_eq!(
            audio_row(false, "MacBook Pro Speakers"),
            "audio: restored output -> MacBook Pro Speakers (previous session did not shut down \
             cleanly)"
        );
        assert_eq!(
            audio_row(true, "MacBook Pro Speakers"),
            "audio: would restore output -> MacBook Pro Speakers (previous session did not shut \
             down cleanly)"
        );
        assert_eq!(
            audio_fallback_row(false, AIRPODS, "MacBook Pro Speakers"),
            format!(
                "recorded output device '{AIRPODS}' is not connected — restored output -> \
                 MacBook Pro Speakers instead (previous session did not shut down cleanly)"
            )
        );
        assert_eq!(
            audio_fallback_row(true, AIRPODS, "Built-in Output"),
            format!(
                "recorded output device '{AIRPODS}' is not connected — would restore output -> \
                 Built-in Output instead (previous session did not shut down cleanly)"
            )
        );
        assert_eq!(
            dashboard_row(false),
            "ALVR dashboard closed (left over from the previous session)"
        );
        assert_eq!(
            dashboard_row(true),
            "ALVR dashboard would be closed (left over from the previous session)"
        );
        assert_eq!(
            forward_row(false, 9943, "1WMHH000X00000"),
            "cleared adb forward tcp:9943 on 1WMHH000X00000"
        );
        assert_eq!(
            forward_row(true, 9944, "1WMHH000X00000"),
            "would clear adb forward tcp:9944 on 1WMHH000X00000"
        );
        assert_eq!(RECONCILE_SECTION, "reconciling the previous session");
        assert_eq!(RECONCILE_FAILED, "previous session not fully restored");
        assert_eq!(
            RECONCILE_RETRY_HINT,
            "the record is kept; stop again to retry"
        );
        assert_eq!(
            RECORD_KEPT,
            "previous session record kept for a later restore"
        );
        assert_eq!(STEP, "session.reconcile");
    }

    // ── the wire shapes the TS mirror pins ───────────────────────────────────

    #[test]
    fn the_reconcile_types_serialize_camel_case() {
        assert_eq!(
            serde_json::to_value(Classification::IdentityMismatch).unwrap(),
            "identityMismatch"
        );
        assert_eq!(
            serde_json::to_value(RestoreMode::SafeOnly).unwrap(),
            "safeOnly"
        );
        assert_eq!(
            serde_json::to_value(Reconciled::NoSession).unwrap(),
            serde_json::json!({ "kind": "noSession" })
        );
        let ev = Reconciled::Dead {
            state: pending(Some(dead()), None),
            restored: vec!["audio: restored output -> X".into()],
            pending: true,
        };
        let j = serde_json::to_value(&ev).unwrap();
        assert_eq!(j["kind"], "dead");
        assert_eq!(j["pending"], true);
        assert_eq!(j["state"]["prevAudioOutput"], "MacBook Pro Speakers");
        assert_eq!(serde_json::from_value::<Reconciled>(j).unwrap(), ev);
    }

    // ── finish_stopped_session (stop's tail) ─────────────────────────────────

    fn stop_ctx(dir: &Path, dry_run: bool) -> (StageCtx, Arc<StdMutex<Vec<StageEvent>>>) {
        let (mut ctx, seen) = test_ctx(dir, dry_run);
        ctx.bottle = Some(Bottle::unvalidated("Steam"));
        (ctx, seen)
    }

    /// `stop` names a surviving wine pid in its own words — one sentence per
    /// live classification — and keeps the record either way.
    #[tokio::test]
    async fn stop_names_a_surviving_wine_pid_and_keeps_the_record() {
        let pid = std::process::id();
        let cases: &[(&str, ProcInfo, String)] = &[
            (
                "live",
                me(),
                format!("previous session state kept: wine pid {pid} still alive"),
            ),
            (
                "unverifiable",
                ProcInfo {
                    start_time: 0,
                    ..me()
                },
                format!(
                    "previous session state kept: wine pid {pid} is alive but could not be identified"
                ),
            ),
        ];
        for (i, (label, wine, warning)) in cases.iter().enumerate() {
            let dir = scratch(&format!("stop-alive-{i}"));
            let (ctx, seen) = stop_ctx(&dir, true);
            write_state(&ctx, &pending(Some(wine.clone()), None));

            finish_stopped_session_with(&ctx, None, None, probing(BLACKHOLE))
                .await
                .unwrap();

            assert_eq!(
                rows(&seen),
                vec![(Severity::Warn, warning.clone())],
                "{label}"
            );
            assert!(
                ctx.executor.planned().is_empty(),
                "{label}: nothing restored while it lives"
            );
            assert!(
                ctx.paths.session_state_path().exists(),
                "{label}: the record survives"
            );
            std::fs::remove_dir_all(&dir).ok();
        }
    }

    #[tokio::test]
    async fn stop_restores_and_clears_a_session_it_did_not_start() {
        let dir = scratch("stop-dead");
        let (ctx, seen) = stop_ctx(&dir, true);
        let mut state = pending(Some(dead()), None);
        state.wired_forwards.clear();
        write_state(&ctx, &state);

        finish_stopped_session_with(&ctx, None, None, probing(BLACKHOLE))
            .await
            .unwrap();

        assert_eq!(
            rows(&seen),
            vec![(
                Severity::Ok,
                "audio: would restore output -> MacBook Pro Speakers (previous session did not \
                 shut down cleanly)"
                    .to_string()
            )]
        );
        assert_eq!(sections(&seen), vec![RECONCILE_SECTION]);
        let kinds: Vec<PlannedKind> = ctx.executor.planned().iter().map(|p| p.kind).collect();
        assert_eq!(
            kinds,
            vec![
                PlannedKind::Spawn,
                PlannedKind::CreateDir,
                PlannedKind::Write,
                PlannedKind::RemoveFile,
            ],
            "switch, save the flag, then clear the record"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn stop_ignores_a_record_belonging_to_another_bottle() {
        let dir = scratch("stop-other-bottle");
        let (ctx, seen) = stop_ctx(&dir, true);
        let mut state = pending(Some(dead()), None);
        state.bottle = "SomeOtherBottle".into();
        write_state(&ctx, &state);

        finish_stopped_session_with(&ctx, None, None, probing(BLACKHOLE))
            .await
            .unwrap();

        assert!(seen.lock().unwrap().is_empty());
        assert!(ctx.executor.planned().is_empty());
        assert!(
            ctx.paths.session_state_path().exists(),
            "another bottle's record is not this stop's to clear"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn stop_leaves_a_session_this_process_supervises_to_its_own_teardown() {
        let dir = scratch("stop-owned");
        let (ctx, seen) = stop_ctx(&dir, true);
        let state = pending(Some(dead()), Some(me()));
        write_state(&ctx, &state);

        finish_stopped_session_with(&ctx, Some(state.run_id), None, probing(BLACKHOLE))
            .await
            .unwrap();

        assert!(seen.lock().unwrap().is_empty());
        assert!(ctx.executor.planned().is_empty());
        assert!(ctx.paths.session_state_path().exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Finding #6. A reconcile mutation that genuinely fails — here the
    /// recorded `SwitchAudioSource` is gone, so the switch cannot even be
    /// spawned — must be **reported**, not propagated: `stop` has rows left to
    /// print (ports, audio) and `stop.sh` has no step that can end the script.
    #[tokio::test]
    async fn a_failed_restore_is_reported_and_the_record_is_kept_for_the_next_stop() {
        let dir = scratch("stop-restore-failed");
        // A real executor, deliberately shaped so the only child it can reach is
        // a path that does not exist: the spawn fails, nothing runs.
        let (ctx, seen) = stop_ctx(&dir, false);
        assert!(!ctx.executor.is_dry_run());
        let mut state = pending(Some(dead()), None);
        state.wired_forwards.clear();
        write_state(&ctx, &state);

        finish_stopped_session_with(&ctx, None, None, probing_a_vanished_binary(&dir))
            .await
            .expect("a failed restore must not fail the caller");

        let seen_rows = rows(&seen);
        assert_eq!(seen_rows.len(), 2, "{seen_rows:?}");
        assert_eq!(seen_rows[0].0, Severity::Warn);
        assert!(
            seen_rows[0]
                .1
                .starts_with("previous session not fully restored: "),
            "{:?}",
            seen_rows[0].1
        );
        assert_eq!(
            seen_rows[1],
            (Severity::Info, RECONCILE_RETRY_HINT.to_string())
        );
        assert!(
            sections(&seen).is_empty(),
            "nothing was restored to announce"
        );

        let on_disk = state::load(&ctx.paths.session_state_path())
            .unwrap()
            .expect("the record survives so the next stop can retry");
        assert!(
            !on_disk.guards.audio_restored,
            "the guard that could not be released stays pending"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The one error the policy above must **not** absorb: a Cancel during
    /// `stop` has to reach the stage and become exit 130.
    #[tokio::test]
    async fn a_cancelled_reconcile_still_reaches_the_caller() {
        let dir = scratch("stop-cancelled");
        let (ctx, seen) = stop_ctx(&dir, true);

        let err = tolerate_reconcile_failure(&ctx, Err(SabrageError::Cancelled)).unwrap_err();
        assert!(matches!(err, SabrageError::Cancelled));
        assert!(
            seen.lock().unwrap().is_empty(),
            "cancellation is not a partial-restore report"
        );

        // …while any other error is absorbed into the two rows.
        tolerate_reconcile_failure(&ctx, Err(SabrageError::fatal_bare("boom")))
            .expect("only cancellation propagates");
        assert_eq!(
            rows(&seen),
            vec![
                (Severity::Warn, format!("{RECONCILE_FAILED}: boom")),
                (Severity::Info, RECONCILE_RETRY_HINT.to_string()),
            ]
        );
        tolerate_reconcile_failure(&ctx, Ok(())).expect("success stays silent");
        assert_eq!(rows(&seen).len(), 2, "Ok emits nothing");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn stop_without_a_record_is_a_silent_noop() {
        let dir = scratch("stop-none");
        let (ctx, seen) = stop_ctx(&dir, true);
        finish_stopped_session_with(&ctx, None, None, no_probe())
            .await
            .unwrap();
        assert!(seen.lock().unwrap().is_empty());
        assert!(ctx.executor.planned().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    // ── detach ───────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn detach_fires_only_the_detach_token_and_marks_the_record() {
        let dir = scratch("detach");
        let (ctx, _seen) = test_ctx(&dir, false);
        let state = pending(Some(me()), Some(me()));
        write_state(&ctx, &state);

        let handle = LiveSessionHandle {
            run_id: state.run_id,
            bottle: state.bottle.clone(),
            identity: me(),
            log_path: state.log_path.clone(),
            started_at_unix_ms: state.started_at_unix_ms,
            cancel: CancellationToken::new(),
            detach: CancellationToken::new(),
        };

        detach(&ctx.paths, &handle).await.unwrap();

        assert!(handle.detach.is_cancelled());
        assert!(
            !handle.cancel.is_cancelled(),
            "detaching must never trigger the teardown path"
        );
        let on_disk = state::load(&ctx.paths.session_state_path())
            .unwrap()
            .expect("the record survives a detach");
        assert!(on_disk.detached);
        assert_eq!(on_disk.guards, state.guards, "guards are left in place");
        assert_eq!(
            on_disk.prev_audio_output.as_deref(),
            Some("MacBook Pro Speakers"),
            "the device stays on BlackHole, and the record still says what it was"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A9-9. Stop is terminal. Both tokens feed one unbiased `select!` in the
    /// supervisor, so a detach fired after a Stop can win that race, disarm the
    /// guards and leave wine running — while the Stop caller watches the live
    /// slot empty and reports success to the user.
    #[tokio::test]
    async fn detach_does_nothing_once_stop_has_already_fired() {
        let dir = scratch("detach-after-stop");
        let (ctx, _seen) = test_ctx(&dir, false);
        let state = pending(Some(me()), Some(me()));
        write_state(&ctx, &state);

        let handle = LiveSessionHandle {
            run_id: state.run_id,
            bottle: state.bottle.clone(),
            identity: me(),
            log_path: state.log_path.clone(),
            started_at_unix_ms: state.started_at_unix_ms,
            cancel: CancellationToken::new(),
            detach: CancellationToken::new(),
        };
        handle.cancel.cancel(); // Stop got there first.

        detach(&ctx.paths, &handle).await.unwrap();

        assert!(
            !handle.detach.is_cancelled(),
            "a Stop that has fired cannot be superseded by a detach"
        );
        let on_disk = state::load(&ctx.paths.session_state_path())
            .unwrap()
            .expect("the record is the teardown\u{2019}s to clear, not ours to rewrite");
        assert!(
            !on_disk.detached,
            "the record must not claim a detach that did not happen"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A9-9. Stop can also win *during* detach's wait: it fires the terminal
    /// token and then releases the live slot, and its teardown legitimately
    /// **keeps** the record when a guard could not be released (a disconnected
    /// output device). Writing `detached: true` over that record is how the app
    /// came to tell the user a session it had just stopped "detached instead of
    /// stopping — it is still running, unsupervised".
    #[tokio::test]
    async fn detach_does_not_relabel_a_session_stopped_during_the_wait() {
        let dir = scratch("detach-stop-wins-the-race");
        let (ctx, _seen) = test_ctx(&dir, false);
        let state = pending(Some(me()), Some(me()));
        write_state(&ctx, &state);

        let handle = LiveSessionHandle {
            run_id: state.run_id,
            bottle: state.bottle.clone(),
            identity: me(),
            log_path: state.log_path.clone(),
            started_at_unix_ms: state.started_at_unix_ms,
            cancel: CancellationToken::new(),
            detach: CancellationToken::new(),
        };

        // The supervisor holds the slot for the first few polls; then a Stop
        // fires its terminal token and the slot empties — exactly that order,
        // which is what the supervisor's teardown does.
        let polls = std::sync::atomic::AtomicU32::new(0);
        let cancel = handle.cancel.clone();
        detach_with(&ctx.paths, &handle, Duration::from_secs(5), |_| {
            let n = polls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n < 3 {
                return true;
            }
            cancel.cancel();
            false
        })
        .await
        .unwrap();

        let on_disk = state::load(&ctx.paths.session_state_path())
            .unwrap()
            .expect("the teardown kept the record");
        assert!(
            !on_disk.detached,
            "a session the Stop path tore down must not read as detached"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The other half: the wait can simply run out. A supervisor that still
    /// holds the live slot is still writing to that record, so the safety net
    /// must not fire — its whole justification is that the supervisor has
    /// already let go.
    #[tokio::test]
    async fn detach_that_times_out_leaves_the_record_alone() {
        let dir = scratch("detach-timeout");
        let (ctx, _seen) = test_ctx(&dir, false);
        let state = pending(Some(me()), Some(me()));
        write_state(&ctx, &state);
        let path = ctx.paths.session_state_path();
        let before = std::fs::read(&path).unwrap();

        let handle = LiveSessionHandle {
            run_id: state.run_id,
            bottle: state.bottle.clone(),
            identity: me(),
            log_path: state.log_path.clone(),
            started_at_unix_ms: state.started_at_unix_ms,
            cancel: CancellationToken::new(),
            detach: CancellationToken::new(),
        };

        // Never released: the supervisor is wedged, or slower than the wait.
        detach_with(&ctx.paths, &handle, Duration::from_millis(120), |_| true)
            .await
            .unwrap();

        assert!(handle.detach.is_cancelled(), "the token still fires");
        assert_eq!(
            std::fs::read(&path).unwrap(),
            before,
            "a timed-out wait writes nothing at all"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn detach_creates_nothing_when_the_supervisor_already_cleared_the_record() {
        let dir = scratch("detach-cleared");
        let (ctx, _seen) = test_ctx(&dir, false);
        std::fs::create_dir_all(ctx.paths.sabrage_appsup.clone()).unwrap();

        let handle = LiveSessionHandle {
            run_id: Uuid::new_v4(),
            bottle: "Steam".into(),
            identity: me(),
            log_path: PathBuf::from("/repo/logs/x.log"),
            started_at_unix_ms: 1,
            cancel: CancellationToken::new(),
            detach: CancellationToken::new(),
        };

        detach(&ctx.paths, &handle).await.unwrap();
        assert!(handle.detach.is_cancelled());
        assert!(!ctx.paths.session_state_path().exists());
        std::fs::remove_dir_all(&dir).ok();
    }
}
