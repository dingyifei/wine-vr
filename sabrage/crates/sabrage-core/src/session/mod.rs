//! The live session: what is running, who owns it, and how to reach it.
//!
//! `run.sh` has no concept of a session — the shell script *is* the session,
//! and everything it knows lives in shell locals that vanish with the process.
//! Sabrage needs three things the shell does not:
//!
//! 1. **A status other screens can read.** The sidebar dot, the Session pill,
//!    and every Stop button read one [`SessionStatus`] (design-app §3's
//!    `session://status` global event).
//! 2. **A handle the *rest of the app* can act on.** [`LiveSessionHandle`] is
//!    how `stop_session()` reaches a running launch that is being supervised on
//!    another task, and how app-quit runs the INT path instead of letting a
//!    dropped future SIGKILL the game (critique.md's app-quit issue).
//! 3. **State that survives this process.** [`state::SessionState`] is written
//!    to disk *before* each guarded mutation, so a `SIGKILL` or a power loss
//!    still leaves enough behind to put the Mac's audio device back — the one
//!    thing `stop.sh` can only warn about (design-core §3.2, §4.2).
//!
//! # Two cancellation tokens, on purpose
//!
//! [`LiveSessionHandle`] carries `cancel` **and** `detach`, and they mean
//! opposite things:
//!
//! * `cancel` — the INT/TERM path. Stop wine (`wineserver -k` + bounded wait),
//!   then release every guard: restore the audio device, close the dashboard,
//!   reap a stray helper. What Ctrl-C does to `demo.sh run`.
//! * `detach` — stop *supervising* and **leak the guards on purpose**. The
//!   session keeps running, the dashboard stays open, the audio device stays
//!   on BlackHole, and `session-state.json` stays on disk with `detached:
//!   true` so a later Sabrage (or `stop`) can finish the job. This is what
//!   app-quit offers as the alternative to killing the user's game.
//!
//! Conflating them is the bug critique.md names: a `Drop` that tears the
//! session down is neither a clean teardown nor a clean detach.

pub mod reconcile;
pub mod state;
pub mod watcher;

use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::events::RunId;
use crate::process::ProcInfo;

// ── status ────────────────────────────────────────────────────────────────────

/// Where a session is in its life.
///
/// Derived from several signals at once, never from one
/// ([`watcher::SessionMonitor`]): wine-child liveness, `runtime_status.json`
/// **freshness** (never its mere existence — the file outlives the process),
/// and the encoder/battery log cadence behind the standby-freeze heuristic
/// (design-core §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionPhase {
    /// Nothing running, nothing to reconcile.
    #[default]
    Idle,
    /// Launch preflight + prepare: checks, wineserver reset, Goldberg.
    Preflight,
    /// Guards taken and the wine child spawned; no frames yet.
    Launching,
    /// The runtime is alive and reporting.
    Running,
    /// Alive but not streaming — the documented standby freeze. `state` alone
    /// is a false-healthy signal here, which is why this phase exists.
    Stalled,
    /// Teardown in progress (`wineserver -k`, guard release).
    Stopping,
    /// The wine child is gone; `exit_code` says with what.
    Exited,
    /// Still running, no longer supervised by this process. Guards were left
    /// in place deliberately — see this module's header.
    Detached,
    /// A session **nothing in Sabrage started**: no live handle, no
    /// `session-state.json`, but `runtime_status.json` is fresh and the
    /// process it names is alive. That is a `demo.sh run` in another terminal,
    /// which is a supported way to run this pipeline — reporting it as
    /// [`SessionPhase::Idle`] ("No session running") invites a second launch
    /// over a live game.
    ///
    /// Derived conservatively and never from file freshness alone
    /// ([`watcher::SessionMonitor::snapshot`]); it carries the runtime's pid
    /// and nothing else, because nothing else about that session is knowable
    /// from here.
    External,
}

/// The encoder configuration one session actually negotiated.
///
/// Parsed from the oxrsys log line
/// `OXRSys/ALVR: encoder ready {w}x{h} @{hz}Hz {mbps}Mbps ({codec}, {path})`,
/// e.g. `… (HEVC, native helper)` or `… (H.264, in-process)`. The `path` half
/// is the one that matters operationally: `in-process` means the arm64 helper
/// did not take, and the session silently downgraded to Rosetta H.264 — the
/// regression CLAUDE.md records as having reached a live session once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncoderInfo {
    /// `"HEVC"` / `"H.264"`, verbatim from the log line.
    pub codec: String,
    /// `"native helper"` / `"in-process"`, verbatim from the log line.
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub refresh_hz: u32,
    pub bitrate_mbps: u32,
}

/// One snapshot of the session, as broadcast on `session://status`.
///
/// `Default` is the idle snapshot: no session, nothing stale, nothing owned.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStatus {
    pub phase: SessionPhase,
    pub run_id: Option<RunId>,
    pub bottle: Option<String>,
    pub pid: Option<u32>,
    pub started_at_unix_ms: Option<u64>,
    /// Set once the wine child has exited — wine's own status, the value
    /// `./demo.sh run` exits with.
    pub exit_code: Option<i32>,
    pub log_path: Option<String>,
    /// The most recent `encoder ready` line, when one has been seen.
    pub encoder: Option<EncoderInfo>,
    /// `runtime_status.json`'s `state` field, treated as an **opaque string**:
    /// the enum is unverified upstream (design-core §10, unverified fact 1).
    pub runtime_state: Option<String>,
    /// Is `runtime_state` recent enough to believe?
    /// ([`watcher::RUNTIME_STATUS_MAX_AGE`]). The file persists after the
    /// runtime dies, so existence proves nothing.
    pub runtime_fresh: bool,
    /// Did *this* Sabrage process launch the session it is describing? False
    /// for a session recovered from `session-state.json` after a restart.
    pub owned_by_this_process: bool,
    /// Running, but nothing is supervising it and its guards were left in
    /// place deliberately.
    pub detached: bool,
}

// ── live handle ───────────────────────────────────────────────────────────────

/// The in-process handle to a running session.
///
/// Cloneable and cheap: `stop_session()`, the app-quit hook, and the status
/// watcher each hold one. Both tokens are documented in this module's header —
/// `cancel` tears down, `detach` walks away.
#[derive(Clone)]
pub struct LiveSessionHandle {
    pub run_id: RunId,
    pub bottle: String,
    /// pid + start time: the pair that distinguishes this process from a
    /// recycled pid ([`ProcInfo::is_same_process`]).
    pub identity: ProcInfo,
    pub log_path: PathBuf,
    pub started_at_unix_ms: u64,
    /// The INT path: stop wine, then restore every guard.
    pub cancel: CancellationToken,
    /// Stop supervising and **leak the guards on purpose** — the session keeps
    /// running without us.
    pub detach: CancellationToken,
}

impl std::fmt::Debug for LiveSessionHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveSessionHandle")
            .field("run_id", &self.run_id)
            .field("bottle", &self.bottle)
            .field("pid", &self.identity.pid)
            .field("log_path", &self.log_path)
            .finish()
    }
}

/// The at-most-one live session this process owns.
///
/// A `std::sync::Mutex` rather than tokio's: every access is a clone or a
/// replace, never held across an `.await`, and the Tauri command layer reads it
/// from synchronous contexts.
pub static LIVE_SESSION: LazyLock<Mutex<Option<LiveSessionHandle>>> =
    LazyLock::new(|| Mutex::new(None));

/// The current live session, if this process owns one.
pub fn live_session() -> Option<LiveSessionHandle> {
    LIVE_SESSION.lock().ok().and_then(|g| g.clone())
}

/// Publish the live session. Replaces any previous handle — a second launch
/// cannot start while the first holds the operation lock through spawn.
pub fn set_live_session(handle: LiveSessionHandle) {
    if let Ok(mut g) = LIVE_SESSION.lock() {
        *g = Some(handle);
    }
}

/// The live session's run id, without cloning the rest of the handle.
///
/// [`LiveSessionHandle`] carries two [`CancellationToken`]s, a [`PathBuf`] and
/// a [`String`]; a caller that only needs to compare identities — a detach
/// poll loop, a reconcile that only asks "is this the session I own?" —
/// should not pay for cloning those on every check.
pub fn live_session_run_id() -> Option<RunId> {
    LIVE_SESSION
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|h| h.run_id))
}

/// Is the live session (if any) exactly the one named by `run_id`?
pub fn live_session_is(run_id: RunId) -> bool {
    live_session_run_id() == Some(run_id)
}

/// Clear the live session, **only** when the stored handle belongs to `run_id`.
///
/// The run-id guard is what stops a late teardown from erasing a newer
/// session's handle: run A's supervise task can notice its child exited after
/// run B has already published itself.
pub fn clear_live_session(run_id: RunId) {
    if let Ok(mut g) = LIVE_SESSION.lock() {
        if g.as_ref().is_some_and(|h| h.run_id == run_id) {
            *g = None;
        }
    }
}

// ── the run stage's published phase ───────────────────────────────────────────

/// The run stage's own account of where a launch currently is, with the
/// identity that goes with it.
///
/// [`SessionPhase::Preflight`], [`SessionPhase::Launching`] and
/// [`SessionPhase::Stopping`] exist **only** in [`crate::stages::run`]'s head:
/// preflight has spawned nothing, so there is no [`LIVE_SESSION`] handle and no
/// `session-state.json` to derive them from, and teardown has already started
/// clearing both. Publishing them here is what lets
/// [`watcher::SessionMonitor::snapshot`] report a launch in progress instead of
/// "No session" for the whole of it.
///
/// The identity fields are not decoration. A phase with no `run_id`/`bottle`
/// is a Session screen that offers a Stop button it cannot wire up — it would
/// take the operation lock and then die on "bottle name required". Anything
/// that publishes a phase publishes who it belongs to, in one value, so the
/// two can never be out of step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunPhaseInfo {
    pub phase: SessionPhase,
    pub run_id: RunId,
    pub bottle: String,
    /// Wine's status, on a [`SessionPhase::Exited`] publication only — the
    /// number `./demo.sh run` exits with. `None` for every other phase.
    pub exit_code: Option<i32>,
}

/// The at-most-one phase the run stage is currently reporting.
///
/// `None` means "the run stage has nothing to say right now": `snapshot()`
/// then falls back to whatever [`LIVE_SESSION`] and `session-state.json`
/// describe. Deliberately **not** serialized anywhere — it is in-process state
/// about a launch this process is running, and a launch that outlives this
/// process is described by `session-state.json` instead.
///
/// A `std::sync::Mutex` for the same reason [`LIVE_SESSION`] is one: every
/// access is a short get/set, never held across an `.await`.
static RUN_PHASE: Mutex<Option<RunPhaseInfo>> = Mutex::new(None);

/// Publish (or, with `None`, clear) the run stage's phase.
///
/// Never fails: a poisoned lock is treated the same as "nothing to publish"
/// rather than propagated — a phase update must never be able to panic the
/// caller mid-launch.
pub fn publish_run_phase(info: Option<RunPhaseInfo>) {
    if let Ok(mut g) = RUN_PHASE.lock() {
        *g = info;
    }
}

/// What the run stage is currently reporting, if anything.
pub fn run_phase() -> Option<RunPhaseInfo> {
    RUN_PHASE.lock().ok().and_then(|g| g.clone())
}

/// Clear the published phase, **only** when it belongs to `run_id`.
///
/// The same run-id guard [`clear_live_session`] carries, for the same reason:
/// `run()` releases the operation lock at the launch boundary, so a detached
/// or cancelled run can still be unwinding while the next launch has already
/// published its own `Preflight`. Clearing unconditionally would blank the
/// newer run's phase.
pub(crate) fn clear_run_phase(run_id: RunId) {
    if let Ok(mut g) = RUN_PHASE.lock() {
        if g.as_ref().is_some_and(|i| i.run_id == run_id) {
            *g = None;
        }
    }
}

// ── one answer to "is a session live?" ────────────────────────────────────────

/// Why a mutating operation must not start right now, or `None` when nothing on
/// this machine looks like a live session.
///
/// Seven signals, cheapest first, and deliberately **not** just the in-process
/// [`live_session`] slot: the session a Settings save or a Doctor button would
/// break may have been launched by the other front-end, by an earlier run of
/// this process, or by `./demo.sh run`, none of which publish anything here.
///
/// 1. this process's own live-session handle;
/// 2. the run stage's published phase — a launch that has not spawned yet;
/// 3. `session-state.json`, when its recorded wine identity is still
///    [`reconcile::Classification::Live`] (or
///    [`reconcile::Classification::Unverifiable`], which is alive as far as
///    anyone can tell) — the other front-end, or a session that outlived the
///    process that started it;
/// 4. the same record's `owner_pid`, for the window *before* that front-end's
///    launch has spawned anything ([`state::has_live_foreign_owner`]);
/// 5. that record being present but unparseable — it may be describing a live
///    session, and nothing here can prove it is not;
/// 6. a **fresh** `runtime_status.json` naming a live pid
///    ([`watcher::runtime_status_live`], the same predicate
///    [`watcher::SessionMonitor`] derives [`SessionPhase::External`] from, so
///    the door and the phase cannot disagree);
/// 7. a live `Beat Saber.exe` on the process table ([`running_game_pid`]) —
///    the only signal that needs nothing to have been *written*, and therefore
///    the only one that covers a `./demo.sh run` between its wine spawn and
///    its first streaming status.
///
/// A live CrossOver `wineserver` is deliberately *not* a signal: it is alive for
/// any CrossOver app the user has open, and blocking `build` on that would be
/// wrong. The two fixes whose file a CrossOver process really can clobber keep
/// their own narrower wineserver probes.
pub fn live_session_reason(paths: &crate::paths::Paths) -> Option<String> {
    live_session_block(paths).map(|b| b.reason)
}

/// One blocking session, as [`live_session_block`] found it.
///
/// The prose *and* the bottle, because the two callers need different halves: a
/// stage refusal renders the reason, and the config refusal has to name the
/// bottle in its `./demo.sh stop --bottle <name>` remedy. `bottle` is `None` for
/// the one signal that cannot know it — a `runtime_status.json` written by a
/// runtime that was launched by `./demo.sh run`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionBlock {
    /// Why the machine looks busy, in the voice every refusal renders.
    pub reason: String,
    /// The bottle that session belongs to, when the signal carries one.
    pub bottle: Option<String>,
}

/// [`live_session_reason`] with the bottle kept.
///
/// **The** live-session predicate: every "not while the game is running" door —
/// [`crate::stages::live_session_block`] (stage refusals and every gated
/// Doctor fix), [`crate::config::blocking_session`] (the Settings writer) and
/// `store::goldberg`'s revert — goes through this one function, so a state that
/// blocks one of them blocks all of them. They used to carry three weaker
/// copies each missing a different signal, which is how a `./demo.sh run`
/// session could have its `steam_api64.dll` replaced underneath it.
pub fn live_session_block(paths: &crate::paths::Paths) -> Option<SessionBlock> {
    session_block_at(
        &paths.session_state_path(),
        &paths.oxr_appsup.join("runtime_status.json"),
    )
}

/// [`live_session_block`] against two explicit paths — what tests use, so no
/// test can consult the developer's own machine.
pub(crate) fn session_block_at(
    state_path: &std::path::Path,
    runtime_status_path: &std::path::Path,
) -> Option<SessionBlock> {
    fn block(reason: String, bottle: Option<&str>) -> Option<SessionBlock> {
        Some(SessionBlock {
            reason,
            bottle: bottle.map(str::to_string),
        })
    }

    if let Some(h) = live_session() {
        return block(
            format!(
                "this Sabrage process is supervising a session for bottle '{}' (wine pid {})",
                h.bottle, h.identity.pid
            ),
            Some(&h.bottle),
        );
    }

    if let Some(info) = run_phase() {
        if matches!(
            info.phase,
            SessionPhase::Preflight
                | SessionPhase::Launching
                | SessionPhase::Running
                | SessionPhase::Stalled
                | SessionPhase::Stopping
        ) {
            return block(
                format!(
                    "a launch for bottle '{}' is in progress ({:?})",
                    info.bottle, info.phase
                ),
                Some(&info.bottle),
            );
        }
    }

    match state::load(state_path) {
        Ok(Some(s)) => {
            if reconcile::classify(&s).is_live() {
                let pid = s.wine.as_ref().map(|w| w.pid).unwrap_or(0);
                return block(
                    format!(
                        "a session for bottle '{}' is still running (wine pid {pid})",
                        s.bottle
                    ),
                    Some(&s.bottle),
                );
            }
            if state::has_live_foreign_owner(&s) {
                return block(
                    format!(
                        "Sabrage process {} is running a session for bottle '{}'",
                        s.owner_pid, s.bottle
                    ),
                    Some(&s.bottle),
                );
            }
        }
        // A record that exists but will not parse is the one case where the
        // conservative answer is the only safe one: it may still be describing
        // a live session, and the question every caller is asking is "may I
        // overwrite something a running game has open". `store::goldberg`'s
        // predicate always read it this way; folding the predicates together
        // gives every other door the same conservatism, with a remedy the
        // `./demo.sh stop` line cannot offer.
        Err(_) => {
            return block(
                format!(
                    "{} cannot be read, so Sabrage cannot tell whether a session is live \
                     (delete it if no game is running)",
                    state_path.display()
                ),
                None,
            )
        }
        Ok(None) => {}
    }

    if let Ok(text) = std::fs::read_to_string(runtime_status_path) {
        if let Some(rs) = watcher::parse_runtime_status(&text) {
            // [`watcher::runtime_status_live`], not a second local reading of
            // the same file: the phase the Session screen renders and the door
            // this function is have to agree, or Sabrage says "No session
            // running" while Settings refuses to save (A10-8).
            if watcher::runtime_status_live(&rs, now_unix_ms()) {
                return block(
                    format!(
                        "the oxrsys runtime is reporting a live session (state '{}')",
                        rs.state
                    ),
                    None,
                );
            }
        }
    }

    // Last, because it is the only signal that costs a full process-table walk
    // — and the only one that needs nothing to have been *written*. See
    // [`running_game_pid`].
    if let Some(pid) = running_game_pid() {
        return block(
            format!(
                "{GAME_EXE} is running (pid {pid}) — a session Sabrage did not start and \
                 cannot see any other way"
            ),
            None,
        );
    }

    None
}

/// The game process every launch of this pipeline ends in, however it was
/// started — `./demo.sh run`, `sabrage run`, or CrossOver's own UI.
///
/// Matched on the command line, not the exe path: wine puts the game's path on
/// argv as a single `Z:\…\Beat Saber.exe` argument, which is the same shape
/// `pgrep -f 'Beat Saber.exe'` (and `stages::stop`'s survivor probe) matches.
const GAME_EXE: &str = "Beat Saber.exe";

/// The pid of a running Beat Saber, if there is one.
///
/// The signal none of the other six can replace: a `./demo.sh run` writes no
/// session record, publishes no handle, and — crucially — its runtime does not
/// write `runtime_status.json` until *streaming* begins, which is minutes after
/// Goldberg has been installed and the game spawned. In that window every
/// file-based signal reads idle, so Settings, a Doctor fix, or the Goldberg
/// revert would happily rewrite files the running game has open (A13a-2). This
/// closes it from the wine spawn onward.
///
/// What it cannot close is the window *before* the spawn — Goldberg is
/// installed several steps earlier — which needs a marker both front-ends take;
/// that is a shared-pipeline change (contract + `scripts/demo/run.sh` + here),
/// not something this function can do alone.
pub(crate) fn running_game_pid() -> Option<u32> {
    crate::process::find_processes_by_cmdline(GAME_EXE)
        .first()
        .map(|p| p.pid)
}

/// Refuse `action` while any session is live — the single policy every "not
/// while the game is running" caller shares, over [`live_session_reason`]'s
/// seven signals.
///
/// The error carries `stop.sh`'s own remedy, so a GUI caller renders the same
/// row the config and doctor refusals already render.
///
/// This form reads the machine's real support directories: both paths it needs
/// are derived from `$HOME`, so the empty repo root below is never consulted.
/// Anything that already has a [`crate::paths::Paths`] — and every test — must
/// call [`ensure_idle_in`] instead, or it will consult the developer's own
/// session.
pub fn ensure_idle(action: &str) -> std::result::Result<(), crate::error::SabrageError> {
    ensure_idle_in(&crate::paths::Paths::new(PathBuf::new()), action)
}

/// [`ensure_idle`] scoped to one [`crate::paths::Paths`].
pub fn ensure_idle_in(
    paths: &crate::paths::Paths,
    action: &str,
) -> std::result::Result<(), crate::error::SabrageError> {
    ensure_idle_at(
        &paths.session_state_path(),
        &paths.oxr_appsup.join("runtime_status.json"),
        action,
    )
}

/// [`ensure_idle`] against explicit paths.
fn ensure_idle_at(
    state_path: &std::path::Path,
    runtime_status_path: &std::path::Path,
    action: &str,
) -> std::result::Result<(), crate::error::SabrageError> {
    match session_block_at(state_path, runtime_status_path).map(|b| b.reason) {
        None => Ok(()),
        Some(reason) => Err(crate::error::SabrageError::fatal(
            format!(
                "refusing to {action} while a session is live — {reason}; stop the session first"
            ),
            "./demo.sh stop --bottle <name>",
        )),
    }
}

/// What stopping *this* session means — decided once, from the status every
/// caller already has in view.
///
/// The three answers are not interchangeable: firing the operation-locked
/// `stop` stage during `Preflight`/`Launching` blocks on the very launch it is
/// trying to stop (`run` holds [`crate::stages::OPERATION_LOCK`] until the
/// wine child is up), and running the stage over a session this process
/// supervises tears it down from outside instead of through its own INT path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopPlan {
    /// A launch of ours that has not published its handle yet: fire that run's
    /// cancellation token (no lock needed).
    CancelRun(RunId),
    /// A session this process supervises: fire [`LiveSessionHandle::cancel`]
    /// and wait for the slot to empty.
    FireLiveToken,
    /// Anything else — a session on disk, an external one, or none at all:
    /// run the bottle-scoped `stop` stage, exactly `stop.sh`'s situation.
    RunStopStage,
}

/// Which of [`StopPlan`]'s three answers `status` calls for. Pure.
pub fn stop_plan(status: &SessionStatus) -> StopPlan {
    match status.phase {
        SessionPhase::Preflight | SessionPhase::Launching => match status.run_id {
            // Without a run id there is nothing to cancel; the stage is the
            // only remaining lever, lock and all.
            Some(id) => StopPlan::CancelRun(id),
            None => StopPlan::RunStopStage,
        },
        SessionPhase::Running | SessionPhase::Stalled | SessionPhase::Stopping
            if status.owned_by_this_process && !status.detached =>
        {
            StopPlan::FireLiveToken
        }
        _ => StopPlan::RunStopStage,
    }
}

/// Wall-clock milliseconds since the Unix epoch.
///
/// The clock for `started_at_unix_ms` and for comparing against
/// `runtime_status.json`'s `updated_at_unix_ms`, which is written on the same
/// scale. A clock before the epoch yields 0 rather than panicking — a status
/// snapshot must never be able to abort a run.
pub fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ── the audio device to fall back to ──────────────────────────────────────────

/// Substrings that mark an output device as *not a speaker*.
///
/// `BlackHole` is the loopback `run` routed the Mac to — restoring onto it is
/// not a restore at all, it is the state we are escaping. `Virtual Desktop` and
/// `Steam Streaming` are the other virtual outputs a Mac used for PCVR
/// streaming usually carries; picking one of those leaves the user just as
/// silent as picking BlackHole, only less obviously so.
const VIRTUAL_OUTPUT_MARKERS: [&str; 3] = ["BlackHole", "Virtual Desktop", "Steam Streaming"];

/// What a Mac with no separate speakers calls its built-in output.
const BUILT_IN_OUTPUT: &str = "Built-in Output";

/// Which device to restore the Mac's output to when the **recorded** one is
/// gone — the AirPods that disconnected while the session was running, which
/// makes `SwitchAudioSource -t output -s "…AirPods Pro"` exit non-zero with
/// `Could not find an audio device named … Nothing was changed.` and leaves the
/// Mac on `BlackHole 2ch`, silent.
///
/// `devices` is `SwitchAudioSource -a -t output`, in its own order. Two tiers:
///
/// 1. the built-in output — the first `Mac… Speakers` (MacBook Pro/Air, Mac
///    Studio, Mac mini) or exactly `Built-in Output`. It is the one device that
///    is always physically there, so it is always a safe landing;
/// 2. failing that, the first device that is not one of
///    [`VIRTUAL_OUTPUT_MARKERS`] — some real output beats no output.
///
/// `None` means every device on offer was virtual (or there were none): the
/// caller must then say so and print the remedy rather than switch to
/// something that stays silent.
pub fn fallback_output_device(devices: &[String]) -> Option<String> {
    devices
        .iter()
        .find(|d| is_built_in_output(d))
        .or_else(|| devices.iter().find(|d| !is_virtual_output(d)))
        .cloned()
}

/// `^Mac.* Speakers$` or exactly `Built-in Output`, without a regex crate.
fn is_built_in_output(name: &str) -> bool {
    name == BUILT_IN_OUTPUT || (name.starts_with("Mac") && name.ends_with(" Speakers"))
}

/// Does the name carry one of [`VIRTUAL_OUTPUT_MARKERS`]?
fn is_virtual_output(name: &str) -> bool {
    VIRTUAL_OUTPUT_MARKERS.iter().any(|m| name.contains(m))
}

/// `recorded output device '<prev>' is not connected — restored output -> <alt> instead`
///
/// The row both restore paths print when the fallback above took over.
/// [`reconcile`]'s recovery pass appends its own "previous session did not
/// shut down cleanly" parenthetical; the guard release prints it as-is.
pub fn audio_fallback_line(dry_run: bool, previous: &str, fallback: &str) -> String {
    let verb = if dry_run { "would restore" } else { "restored" };
    format!(
        "recorded output device '{previous}' is not connected — {verb} output -> {fallback} instead"
    )
}

/// The row printed when there is nothing to fall back to either: what failed,
/// and the two commands that fix it by hand.
///
/// Naming the recorded device is the whole point — the failure this replaces
/// left the user on `BlackHole 2ch` with no record at all of what to restore.
pub fn audio_unrestorable_line(previous: &str) -> String {
    format!(
        "could not restore the audio output (recorded device '{previous}' is not connected) — \
         restore with: SwitchAudioSource -t output -s '{previous}'   \
         (list: SwitchAudioSource -a -t output)"
    )
}

/// Serializes — and resets — [`RUN_PHASE`] for the tests that touch it.
///
/// Unit tests share one process and the standard harness runs them on several
/// threads at once, so a test asserting "nothing is published" genuinely races
/// a test that legitimately publishes — observed as a real flake, not a
/// theoretical one. **Every** test that reads or writes the published run phase
/// must hold this guard, not just the ones that expect emptiness: a writer that
/// skips it is exactly what the readers are being protected from.
///
/// Acquiring also empties the slot, so a test starts from Idle whatever its
/// predecessor left behind — including the deliberately surviving `Exited`
/// publication [`crate::stages::run`]'s normal teardown ends on. Poisoning is
/// ignored: a panicking test has already failed, and its neighbours must still
/// be able to run.
///
/// [`LIVE_SESSION`] is deliberately **not** reset here. Tests in other modules
/// set that slot without holding this guard (`logs`'s live-session resolution
/// test, for one), and blanking it on every acquisition would pull it out from
/// under them. Serializing that slot too is worth doing — it needs those tests
/// to take this guard first.
#[cfg(test)]
pub(crate) fn lock_session_globals() -> SessionGlobalsGuard {
    static LOCK: Mutex<()> = Mutex::new(());
    let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if let Ok(mut g) = RUN_PHASE.lock() {
        *g = None;
    }
    SessionGlobalsGuard(guard)
}

/// What [`lock_session_globals`] hands back.
///
/// A newtype around the `MutexGuard` rather than the guard itself, because
/// async tests hold it across `.await` points and `clippy::await_holding_lock`
/// flags a raw guard there. The concern behind that lint — a std lock blocking
/// an async runtime's worker — cannot arise: `#[tokio::test]` drives one future
/// to completion on the test's own thread, so no other task on that runtime can
/// be waiting for this lock. The contention is between the test harness's OS
/// threads, which is exactly what a std `Mutex` is for.
#[cfg(test)]
pub(crate) struct SessionGlobalsGuard(#[allow(dead_code)] std::sync::MutexGuard<'static, ()>);

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use uuid::Uuid;

    fn handle(run_id: RunId) -> LiveSessionHandle {
        LiveSessionHandle {
            run_id,
            bottle: "Steam".into(),
            identity: ProcInfo {
                pid: 4242,
                start_time: 1786300214,
                exe: PathBuf::from("/Applications/CrossOver.app/…/wine"),
            },
            log_path: PathBuf::from("/repo/logs/beatsaber-20260829-101112.log"),
            started_at_unix_ms: 1786300214181,
            cancel: CancellationToken::new(),
            detach: CancellationToken::new(),
        }
    }

    #[test]
    fn the_idle_status_is_the_default() {
        let s = SessionStatus::default();
        assert_eq!(s.phase, SessionPhase::Idle);
        assert!(s.run_id.is_none() && s.pid.is_none() && s.encoder.is_none());
        assert!(!s.runtime_fresh && !s.owned_by_this_process && !s.detached);
    }

    #[test]
    fn status_serializes_camel_case_for_the_ipc_mirror() {
        let s = SessionStatus {
            phase: SessionPhase::Stalled,
            run_id: Some(Uuid::nil()),
            bottle: Some("Steam".into()),
            pid: Some(59004),
            started_at_unix_ms: Some(1786300214181),
            exit_code: None,
            log_path: Some("/repo/logs/x.log".into()),
            encoder: Some(EncoderInfo {
                codec: "HEVC".into(),
                path: "native helper".into(),
                width: 2064,
                height: 2208,
                refresh_hz: 72,
                bitrate_mbps: 100,
            }),
            runtime_state: Some("streaming".into()),
            runtime_fresh: true,
            owned_by_this_process: true,
            detached: false,
        };
        let j = serde_json::to_value(&s).unwrap();
        assert_eq!(j["phase"], "stalled");
        assert_eq!(j["startedAtUnixMs"], 1786300214181u64);
        assert_eq!(j["runtimeFresh"], true);
        assert_eq!(j["ownedByThisProcess"], true);
        assert_eq!(j["encoder"]["refreshHz"], 72);
        assert_eq!(j["encoder"]["bitrateMbps"], 100);
        assert_eq!(j["encoder"]["path"], "native helper");
        assert_eq!(serde_json::from_value::<SessionStatus>(j).unwrap(), s);
    }

    #[test]
    fn every_phase_has_a_camel_case_wire_word() {
        for (phase, word) in [
            (SessionPhase::Idle, "idle"),
            (SessionPhase::Preflight, "preflight"),
            (SessionPhase::Launching, "launching"),
            (SessionPhase::Running, "running"),
            (SessionPhase::Stalled, "stalled"),
            (SessionPhase::Stopping, "stopping"),
            (SessionPhase::Exited, "exited"),
            (SessionPhase::Detached, "detached"),
            (SessionPhase::External, "external"),
        ] {
            assert_eq!(serde_json::to_value(phase).unwrap(), word);
        }
    }

    // ── the one liveness policy ──────────────────────────────────────────────

    fn state_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sabrage-session-policy-{tag}-{}-{}",
            std::process::id(),
            Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn ensure_idle_refuses_for_every_source_that_can_know_about_a_session() {
        let _g = lock_session_globals();
        let dir = state_dir("ensure-idle");
        let path = dir.join("session-state.json");
        // Inside the fixture: no test may read the developer's own machine.
        let status = dir.join("runtime_status.json");

        // Nothing anywhere.
        assert!(ensure_idle_at(&path, &status, "edit the runtime config").is_ok());

        // 1: a session this process supervises.
        let run = Uuid::new_v4();
        set_live_session(handle(run));
        let err = ensure_idle_at(&path, &status, "edit the runtime config").unwrap_err();
        assert_eq!(
            err.to_string(),
            "refusing to edit the runtime config while a session is live — this Sabrage process \
             is supervising a session for bottle 'Steam' (wine pid 4242); stop the session first"
        );
        assert_eq!(
            err.remedy(),
            Some("./demo.sh stop --bottle <name>"),
            "the GUI renders the same remedy the config fixer already renders"
        );
        clear_live_session(run);
        assert!(ensure_idle_at(&path, &status, "x").is_ok());

        // 2: our own launch, before it publishes a handle.
        for phase in [
            SessionPhase::Preflight,
            SessionPhase::Launching,
            SessionPhase::Stopping,
        ] {
            publish_run_phase(Some(RunPhaseInfo {
                phase,
                run_id: run,
                bottle: "Steam".into(),
                exit_code: None,
            }));
            assert!(
                ensure_idle_at(&path, &status, "x").is_err(),
                "{phase:?} is a live session"
            );
        }
        publish_run_phase(Some(RunPhaseInfo {
            phase: SessionPhase::Exited,
            run_id: run,
            bottle: "Steam".into(),
            exit_code: Some(0),
        }));
        assert!(
            ensure_idle_at(&path, &status, "x").is_ok(),
            "a finished launch is not a live session"
        );
        publish_run_phase(None);

        // 3: a record on disk whose wine child is this very process.
        let me = ProcInfo::observe(std::process::id()).unwrap();
        let mut s = state::SessionState::new(Uuid::new_v4(), "Bottled", "/g", "/l", 1);
        s.wine = Some(me);
        std::fs::write(&path, serde_json::to_vec_pretty(&s).unwrap()).unwrap();
        let err = ensure_idle_at(&path, &status, "rebuild").unwrap_err();
        assert!(err.to_string().contains("bottle 'Bottled'"), "{err}");

        // 4: the *other* front-end's launch, before it has spawned anything —
        // no wine identity to classify, only `owner_pid` knows.
        let foreign = std::process::Command::new("/bin/sleep")
            .arg("30")
            .spawn()
            .expect("/bin/sleep is on every macOS");
        let mut theirs = state::SessionState::new(Uuid::new_v4(), "Theirs", "/g", "/l", 1);
        theirs.set_owner(foreign.id());
        std::fs::write(&path, serde_json::to_vec_pretty(&theirs).unwrap()).unwrap();
        let err = ensure_idle_at(&path, &status, "rebuild").unwrap_err();
        assert!(
            err.to_string()
                .contains(&format!("Sabrage process {} is running", foreign.id())),
            "{err}"
        );
        let mut foreign = foreign;
        let _ = foreign.kill();
        let _ = foreign.wait();
        std::fs::remove_file(&path).unwrap();

        // 5: a `demo.sh run` session — nothing of ours anywhere, but the
        // runtime is reporting in right now, naming a process that is alive.
        // Both halves, because this is `watcher::runtime_status_live`, the
        // same predicate the Session screen derives `External` from (A10-8): a
        // door that refused on freshness alone said "a session is live" over a
        // file the UI was calling Idle.
        let write_status = |pid: Option<u32>, at: u64| {
            let pid = pid
                .map(|p| format!(r#""process_id":{p},"#))
                .unwrap_or_default();
            std::fs::write(
                &status,
                format!(r#"{{"state":"streaming",{pid}"updated_at_unix_ms":{at}}}"#),
            )
            .unwrap();
        };
        write_status(Some(std::process::id()), now_unix_ms());
        let err = ensure_idle_at(&path, &status, "rebuild").unwrap_err();
        assert!(
            err.to_string().contains("the oxrsys runtime is reporting"),
            "{err}"
        );
        // …and a stale one is not a session, however alive the pid it names.
        write_status(Some(std::process::id()), now_unix_ms() - 600_000);
        assert!(ensure_idle_at(&path, &status, "rebuild").is_ok());
        // …nor a fresh one whose process is gone.
        write_status(Some(u32::MAX - 1), now_unix_ms());
        assert!(ensure_idle_at(&path, &status, "rebuild").is_ok());
        // …nor one that names no process at all: oxrsys always writes
        // `process_id`, so a file without one is not evidence of anything.
        write_status(None, now_unix_ms());
        assert!(ensure_idle_at(&path, &status, "rebuild").is_ok());
        std::fs::remove_file(&status).unwrap();

        // Back to the on-disk record for the last case.
        std::fs::write(&path, serde_json::to_vec_pretty(&s).unwrap()).unwrap();

        // …and a record whose wine child is long gone is not a session.
        s.wine = Some(ProcInfo {
            pid: u32::MAX - 1,
            start_time: 1,
            exe: PathBuf::new(),
        });
        std::fs::write(&path, serde_json::to_vec_pretty(&s).unwrap()).unwrap();
        assert!(ensure_idle_at(&path, &status, "rebuild").is_ok());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A13a-2. `./demo.sh run` installs Goldberg, resets wineserver and spawns
    /// wine long before its runtime writes a `runtime_status.json` — the file
    /// only appears once streaming begins. Every file-based signal reads idle
    /// through that whole window, so the game itself has to be a signal, or
    /// Settings/Doctor/Revert rewrite files the running game has open.
    #[test]
    fn a_running_game_is_a_live_session_even_with_nothing_on_disk() {
        let _g = lock_session_globals();
        let dir = state_dir("running-game");
        let path = dir.join("session-state.json");
        let status = dir.join("runtime_status.json");
        assert!(
            ensure_idle_at(&path, &status, "rebuild").is_ok(),
            "nothing on disk, no game: idle"
        );

        // Stand in for the wine child: a process whose argv carries the game's
        // Windows path, exactly as wine spells it (`Z:\…\Beat Saber.exe`) and
        // exactly what `pgrep -f 'Beat Saber.exe'` matches.
        let mut game = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg("sleep 20 # Z:\\games\\Beat Saber 1294\\Beat Saber.exe")
            .spawn()
            .expect("/bin/sh is on every macOS");

        // The process table is refreshed per call; give the spawn a moment to
        // appear rather than assuming it already has.
        let mut err = None;
        for _ in 0..50 {
            match ensure_idle_at(&path, &status, "rebuild") {
                Err(e) => {
                    err = Some(e);
                    break;
                }
                Ok(()) => std::thread::sleep(std::time::Duration::from_millis(20)),
            }
        }
        let _ = game.kill();
        let _ = game.wait();

        let err = err.expect("a running Beat Saber blocks every mutating door");
        assert!(
            err.to_string().contains("Beat Saber.exe is running"),
            "{err}"
        );
        assert_eq!(err.remedy(), Some("./demo.sh stop --bottle <name>"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn stop_plan_decides_from_the_status_alone() {
        let run = Uuid::new_v4();
        let with = |phase, owned, detached, run_id| SessionStatus {
            phase,
            run_id,
            owned_by_this_process: owned,
            detached,
            ..SessionStatus::default()
        };

        // A launch of ours that has not published a handle: cancel the run —
        // the stop stage would block on the lock that launch is holding.
        for phase in [SessionPhase::Preflight, SessionPhase::Launching] {
            assert_eq!(
                stop_plan(&with(phase, true, false, Some(run))),
                StopPlan::CancelRun(run)
            );
            assert_eq!(
                stop_plan(&with(phase, true, false, None)),
                StopPlan::RunStopStage,
                "nothing to cancel without a run id"
            );
        }

        // A session we supervise: its own INT path.
        for phase in [
            SessionPhase::Running,
            SessionPhase::Stalled,
            SessionPhase::Stopping,
        ] {
            assert_eq!(
                stop_plan(&with(phase, true, false, Some(run))),
                StopPlan::FireLiveToken
            );
            assert_eq!(
                stop_plan(&with(phase, false, false, Some(run))),
                StopPlan::RunStopStage,
                "somebody else's session is stopped by the stage, as stop.sh does"
            );
        }

        // Detached, external, exited, idle: the bottle-scoped stage.
        for phase in [
            SessionPhase::Detached,
            SessionPhase::External,
            SessionPhase::Exited,
            SessionPhase::Idle,
        ] {
            assert_eq!(
                stop_plan(&with(
                    phase,
                    true,
                    phase == SessionPhase::Detached,
                    Some(run)
                )),
                StopPlan::RunStopStage
            );
        }
    }

    /// `live_session_run_id`/`live_session_is` must agree with the
    /// full-clone `live_session().map(|h| h.run_id)` shape they replace at
    /// the hot call sites (a detach poll loop, `reconcile`'s ownership
    /// check) — same answer, without cloning the handle's tokens/paths.
    #[test]
    fn live_session_run_id_agrees_with_the_full_handle_clone() {
        let _g = lock_session_globals();
        assert_eq!(live_session_run_id(), None);
        assert!(!live_session_is(Uuid::new_v4()));

        let a = Uuid::new_v4();
        set_live_session(handle(a));
        assert_eq!(live_session_run_id(), live_session().map(|h| h.run_id));
        assert_eq!(live_session_run_id(), Some(a));
        assert!(live_session_is(a));
        assert!(!live_session_is(Uuid::new_v4()));

        clear_live_session(a);
        assert_eq!(live_session_run_id(), None);
        assert!(!live_session_is(a));
    }

    #[test]
    fn the_live_slot_is_set_and_cleared_by_run_id() {
        let _g = lock_session_globals();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        set_live_session(handle(a));
        assert!(live_session_is(a));

        // A stale teardown for a different run must not clear the current one.
        clear_live_session(b);
        assert!(live_session_is(a));

        clear_live_session(a);
        assert!(live_session().is_none());
        // Idempotent.
        clear_live_session(a);
        assert!(live_session().is_none());
    }

    #[test]
    fn the_two_tokens_are_independent() {
        let h = handle(Uuid::new_v4());
        h.detach.cancel();
        assert!(h.detach.is_cancelled());
        assert!(
            !h.cancel.is_cancelled(),
            "detaching must never trigger the teardown path"
        );
        assert!(format!("{h:?}").contains("4242"));
        assert!(h.log_path.starts_with(Path::new("/repo/logs")));
    }

    #[test]
    fn the_run_phase_slot_carries_identity_and_clears_only_for_its_own_run() {
        let _g = lock_session_globals();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();

        publish_run_phase(Some(RunPhaseInfo {
            phase: SessionPhase::Preflight,
            run_id: a,
            bottle: "Steam".into(),
            exit_code: None,
        }));
        let info = run_phase().expect("published");
        assert_eq!(info.phase, SessionPhase::Preflight);
        assert_eq!(info.run_id, a);
        assert_eq!(info.bottle, "Steam", "a phase always names its bottle");
        assert!(info.exit_code.is_none());

        // A late clear from a *different* run must not blank this one.
        clear_run_phase(b);
        assert_eq!(run_phase().map(|i| i.run_id), Some(a));

        // Exited carries wine's status.
        publish_run_phase(Some(RunPhaseInfo {
            phase: SessionPhase::Exited,
            run_id: a,
            bottle: "Steam".into(),
            exit_code: Some(3),
        }));
        assert_eq!(run_phase().and_then(|i| i.exit_code), Some(3));

        clear_run_phase(a);
        assert!(run_phase().is_none());
        // Idempotent, and a clear against an empty slot is a no-op.
        clear_run_phase(a);
        assert!(run_phase().is_none());
        publish_run_phase(None);
        assert!(run_phase().is_none());
    }

    #[test]
    fn now_unix_ms_is_a_plausible_wall_clock() {
        // Well past 2020, and milliseconds rather than seconds.
        assert!(now_unix_ms() > 1_600_000_000_000);
    }

    // ── the audio fallback ───────────────────────────────────────────────────

    fn devices(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn the_fallback_prefers_the_built_in_speakers() {
        // The live list from the 2026-08-29 finding, in `SwitchAudioSource -a
        // -t output` order: the recorded AirPods are simply not on it any more.
        assert_eq!(
            fallback_output_device(&devices(&[
                "BlackHole 2ch",
                "MacBook Pro Speakers",
                "Steam Streaming Microphone",
                "Steam Streaming Speakers",
                "Virtual Desktop Mic",
                "Virtual Desktop Speakers",
            ])),
            Some("MacBook Pro Speakers".to_string())
        );
        // Every Mac naming the pattern has to cover, built-in first even when
        // it is listed last.
        assert_eq!(
            fallback_output_device(&devices(&["BlackHole 2ch", "MacBook Air Speakers"])),
            Some("MacBook Air Speakers".to_string())
        );
        assert_eq!(
            fallback_output_device(&devices(&["Mac Studio Speakers"])),
            Some("Mac Studio Speakers".to_string())
        );
        assert_eq!(
            fallback_output_device(&devices(&["Mac mini Speakers"])),
            Some("Mac mini Speakers".to_string())
        );
        assert_eq!(
            fallback_output_device(&devices(&["Steam Streaming Speakers", "Built-in Output"])),
            Some("Built-in Output".to_string()),
            "the built-in output outranks anything earlier in the list"
        );
    }

    #[test]
    fn a_real_device_beats_no_device_but_a_virtual_one_never_wins() {
        // No built-in on the list: the first device that is not virtual.
        assert_eq!(
            fallback_output_device(&devices(&[
                "BlackHole 2ch",
                "Virtual Desktop Speakers",
                "Studio Display Speakers",
            ])),
            Some("Studio Display Speakers".to_string())
        );
        // Only the loopback and the streaming virtuals: switching to any of
        // them is still silence, so the caller must say so instead.
        assert_eq!(
            fallback_output_device(&devices(&[
                "BlackHole 2ch",
                "Virtual Desktop Mic",
                "Virtual Desktop Speakers",
            ])),
            None
        );
        assert_eq!(fallback_output_device(&devices(&["BlackHole 16ch"])), None);
        assert_eq!(fallback_output_device(&[]), None);
    }

    #[test]
    fn the_fallback_row_texts_are_stable() {
        assert_eq!(
            audio_fallback_line(false, "Yifei’s AirPods Pro", "MacBook Pro Speakers"),
            "recorded output device 'Yifei’s AirPods Pro' is not connected — restored output -> \
             MacBook Pro Speakers instead"
        );
        assert_eq!(
            audio_fallback_line(true, "Yifei’s AirPods Pro", "MacBook Pro Speakers"),
            "recorded output device 'Yifei’s AirPods Pro' is not connected — would restore output \
             -> MacBook Pro Speakers instead"
        );
        assert_eq!(
            audio_unrestorable_line("Yifei’s AirPods Pro"),
            "could not restore the audio output (recorded device 'Yifei’s AirPods Pro' is not \
             connected) — restore with: SwitchAudioSource -t output -s 'Yifei’s AirPods Pro'   \
             (list: SwitchAudioSource -a -t output)"
        );
    }
}
