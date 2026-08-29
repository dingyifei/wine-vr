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
        ] {
            assert_eq!(serde_json::to_value(phase).unwrap(), word);
        }
    }

    #[test]
    fn the_live_slot_is_set_and_cleared_by_run_id() {
        let _g = lock_session_globals();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        set_live_session(handle(a));
        assert_eq!(live_session().map(|h| h.run_id), Some(a));

        // A stale teardown for a different run must not clear the current one.
        clear_live_session(b);
        assert_eq!(live_session().map(|h| h.run_id), Some(a));

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
}
