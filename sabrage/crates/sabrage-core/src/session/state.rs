//! `session-state.json` — the crash-recovery record.
//!
//! `~/Library/Application Support/Sabrage/session-state.json`
//! ([`crate::paths::Paths::session_state_path`]), written by the launch path
//! and read by [`super::reconcile`]. `run.sh`'s guards are shell traps, so a
//! `SIGKILL`, a panic or a power loss skips them and leaves the Mac's output
//! device on `BlackHole 2ch`, an unattributable ALVR dashboard and (after
//! `--wired`) two adb forwards that break WiFi discovery on the next run,
//! with nothing on the machine describing them. This file lets Sabrage undo
//! them (design-core §4.2; PARITY.md
//! § Session (detach / reconcile), "A **Dead** or **IdentityMismatch**
//! recorded session").
//!
//! The record is saved *before* each guarded mutation and again after each
//! guard is released, so recovery is idempotent: a flag that is already `true`
//! means that guard is done, and a crash at any instant leaves a file
//! describing only work that still needs doing. Every optional field and flag
//! carries `#[serde(default)]`, so an older file still loads;
//! [`SESSION_STATE_VERSION`] covers what defaults cannot.
//!
//! One record path is shared by both front-ends (the app and the `sabrage`
//! CLI), and an atomic rename stops a torn read but not a lost update. [`save`],
//! [`clear`], [`clear_run`] and [`mark_detached`] therefore compare what is on
//! disk under `lock_record`: a record a live foreign process owns and a
//! record from a newer schema are refused, and [`clear_run`] and
//! [`mark_detached`] additionally leave another run's record exactly as it is
//! (`Ok`), the newer owner being responsible for it. Serializing a whole
//! [`super::reconcile`] pass across several such calls is the operation lock's
//! job. See tests::{a_save_for_a_different_run_does_not_clobber_a_live_owners_record,
//! a_newer_schema_record_is_never_overwritten_or_deleted,
//! clear_run_only_removes_the_record_it_names}.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Result, SabrageError};
use crate::events::RunId;
use crate::executor::Executor;
use crate::process::ProcInfo;

/// Schema version of [`SessionState`]. Bump only for a change `#[serde(default)]`
/// cannot absorb.
pub const SESSION_STATE_VERSION: u32 = 1;

/// One `adb forward tcp:<port> tcp:<port>` created by a `--wired` launch.
///
/// Per-serial, because the removal must be too: `adb forward --remove` names
/// exactly these ports on exactly this device, never `--remove-all`
/// (PARITY.md § Invariants that must NOT change (byte/behavior parity), "adb `forward --remove`
/// per-serial for exactly tcp:9943+9944"; CLAUDE.md's `--wired` note).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WiredForward {
    pub serial: String,
    pub port: u16,
}

/// Which guards have already been released.
///
/// Set one at a time, each by its own [`save`], so recovery never re-runs a
/// guard that was already undone. All default to `false`: an older file, or a
/// crash before the first flip, means "nothing has been released yet", which
/// is the safe reading.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GuardFlags {
    /// The Mac's default output device has been put back.
    pub audio_restored: bool,
    /// The `alvr_dashboard` this run spawned has been closed.
    pub dashboard_closed: bool,
    /// The `--wired` forwards this run created have been removed.
    pub forwards_cleared: bool,
}

/// Everything a later process needs to finish this session's cleanup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionState {
    /// [`SESSION_STATE_VERSION`] at write time.
    pub version: u32,
    pub run_id: RunId,
    pub bottle: String,
    pub bs_dir: PathBuf,
    pub started_at_unix_ms: u64,
    pub log_path: PathBuf,
    /// The Sabrage (or `sabrage` CLI) process that owns this session. A live
    /// `owner_pid` that is not us means another front-end is supervising —
    /// reconcile must not touch its guards.
    #[serde(default)]
    pub owner_pid: u32,
    /// The owner's start time (seconds since the epoch), when it could be
    /// observed at write time.
    ///
    /// Pids are recycled, so `owner_pid` alone keeps a record whose owner died
    /// in the pre-spawn window foreign-owned — neither overwritable nor
    /// clearable, wedging the next launch — as soon as anything else reuses
    /// that pid; paired with the pid this is the same guard
    /// [`ProcInfo::is_same_process`] gives `wine`. `None` in records written
    /// before this field existed, where the pid alone is all there is.
    /// See tests::a_live_foreign_owner_is_recognised_only_while_its_session_can_still_be_running.
    #[serde(default)]
    pub owner_started_at: Option<u64>,
    /// The wine child's identity. `None` between guard acquisition and the
    /// spawn — a window the file deliberately covers.
    #[serde(default)]
    pub wine: Option<ProcInfo>,
    /// The `alvr_dashboard` this run spawned, if it did.
    #[serde(default)]
    pub dashboard: Option<ProcInfo>,
    /// The device name `SwitchAudioSource -c -t output` reported **before** the
    /// switch. `None` when audio was never rerouted (`--no-audio`, no
    /// BlackHole, `protocol != alvr`, or the switch itself failed).
    #[serde(default)]
    pub prev_audio_output: Option<String>,
    /// Forwards created by this launch, and only those.
    #[serde(default)]
    pub wired_forwards: Vec<WiredForward>,
    #[serde(default)]
    pub guards: GuardFlags,
    /// The user chose to leave this session running unsupervised. Its guards
    /// are still in place **on purpose**; nothing may restore them behind the
    /// user's back.
    #[serde(default)]
    pub detached: bool,
}

impl SessionState {
    /// A fresh record for a launch that has not mutated anything yet.
    pub fn new(
        run_id: RunId,
        bottle: impl Into<String>,
        bs_dir: impl Into<PathBuf>,
        log_path: impl Into<PathBuf>,
        started_at_unix_ms: u64,
    ) -> SessionState {
        SessionState {
            version: SESSION_STATE_VERSION,
            run_id,
            bottle: bottle.into(),
            bs_dir: bs_dir.into(),
            started_at_unix_ms,
            log_path: log_path.into(),
            // Overwritten with the identity pair below — `set_owner` is the
            // one place that writes the two together.
            owner_pid: 0,
            owner_started_at: None,
            wine: None,
            dashboard: None,
            prev_audio_output: None,
            wired_forwards: Vec::new(),
            guards: GuardFlags::default(),
            detached: false,
        }
        .owned_by_this_process()
    }

    /// Stamp this process as the owner, identity and all.
    fn owned_by_this_process(mut self) -> SessionState {
        self.set_owner(std::process::id());
        self
    }

    /// Point this record's owner at `pid` — the **pair** `owner_pid` +
    /// [`SessionState::owner_started_at`], which is what
    /// [`has_live_foreign_owner`] reads. Setting the number alone describes a
    /// record no writer produces (a pid from one process with another's
    /// identity), so every fabricator of a foreign-owned record goes through
    /// this.
    pub(crate) fn set_owner(&mut self, pid: u32) {
        self.owner_pid = pid;
        self.owner_started_at = ProcInfo::observe(pid).map(|p| p.start_time);
    }

    /// Was this record written by a Sabrage this one understands?
    ///
    /// `false` means a **newer** schema: the file may describe a guard this
    /// binary has never heard of, so the mutating paths report such a record
    /// and leave it exactly as it is
    /// (tests::a_newer_schema_record_is_never_overwritten_or_deleted).
    pub fn is_supported_version(&self) -> bool {
        self.version <= SESSION_STATE_VERSION
    }

    /// Is there any guard left for a recovery to undo?
    ///
    /// The audio device only counts while it has not been restored, and the
    /// forwards only while they have not been cleared — the idempotence rule
    /// this file's header describes, in one place.
    pub fn has_pending_guards(&self) -> bool {
        (self.prev_audio_output.is_some() && !self.guards.audio_restored)
            || (self.dashboard.is_some() && !self.guards.dashboard_closed)
            || (!self.wired_forwards.is_empty() && !self.guards.forwards_cleared)
    }
}

/// Read the state file.
///
/// * absent → `Ok(None)` — the normal case, meaning "no session to reconcile";
/// * present but unreadable or malformed → `Err` ([`SabrageError::Io`]).
///
/// The second case is deliberately not folded into `None`: a corrupt file may
/// still describe a live session with a rerouted audio device, and reporting
/// "nothing to recover" for it leaves the user with no sound and no
/// explanation (tests::a_corrupt_file_is_an_error_never_a_silent_none).
pub fn load(path: &Path) -> Result<Option<SessionState>> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(SabrageError::io(path, e)),
    };
    serde_json::from_str(&text).map(Some).map_err(|e| {
        SabrageError::io(
            path,
            std::io::Error::new(std::io::ErrorKind::InvalidData, e),
        )
    })
}

/// Does `state` describe a session **another live process** is responsible
/// for?
///
/// True only when all three hold: `owner_pid` is neither 0 (an older record
/// that never wrote one) nor this process; that pid is alive **and** still the
/// process that recorded it (a record written before
/// [`SessionState::owner_started_at`] existed can only trust the bare pid);
/// and the session has not visibly ended — the recorded wine child is still
/// that same process, or there is no wine child yet (the pre-spawn window,
/// where the record is the only description of guards already taken).
///
/// A record whose wine pid is gone is therefore never protected, whoever
/// wrote it: it is a leftover, and undoing its guards is the job. The one
/// residual false positive — a recycled `owner_pid` in a record written
/// before `owner_started_at` existed — costs a kept record and a reported
/// row, never a mutation. See
/// tests::a_live_foreign_owner_is_recognised_only_while_its_session_can_still_be_running.
pub fn has_live_foreign_owner(state: &SessionState) -> bool {
    if state.owner_pid == 0 || state.owner_pid == std::process::id() {
        return false;
    }
    if !crate::process::is_alive(state.owner_pid) {
        return false;
    }
    if let Some(started_at) = state.owner_started_at {
        // Alive, but is it the same process? A pid the OS handed to something
        // else is not an owner, and treating it as one wedges every later
        // launch behind a record nothing can clear.
        match ProcInfo::observe(state.owner_pid) {
            Some(now) if now.start_time == started_at => {}
            _ => return false,
        }
    }
    state
        .wine
        .as_ref()
        .map(|w| w.is_same_process())
        .unwrap_or(true)
}

/// How long [`lock_record`] waits for the other front-end's read-modify-write
/// before going ahead without the lock.
///
/// The window it protects is two file operations long, so a wait this size is
/// already generous; degrading rather than blocking forever is the same choice
/// [`crate::stages::acquire_operation_lock`]'s file lock makes — a machine
/// whose support directory cannot be locked must still be able to stop a
/// session.
const RECORD_LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// Poll interval inside [`RECORD_LOCK_WAIT`].
const RECORD_LOCK_POLL: std::time::Duration = std::time::Duration::from_millis(50);

/// Take the advisory lock that serializes this record's read-modify-write
/// across processes — `<record>.lock`, i.e.
/// [`crate::paths::Paths::session_state_lock_path`] for the real record.
///
/// A separate file rather than the record itself, so the lock survives the
/// atomic rename that replaces it. Held by the returned `File`: dropping it
/// releases the `flock`. `None` — no lock, carry on — for every failure,
/// including a holder that will not let go: the compare-and-swap in [`save`]
/// and [`clear`] is what makes the lost update *safe*, and this only makes it
/// rare.
async fn lock_record(record_path: &Path) -> Option<std::fs::File> {
    let path = record_path.with_extension("lock");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok()?;
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .ok()?;
    let deadline = tokio::time::Instant::now() + RECORD_LOCK_WAIT;
    loop {
        match file.try_lock() {
            Ok(()) => return Some(file),
            Err(std::fs::TryLockError::WouldBlock) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(RECORD_LOCK_POLL).await;
            }
            _ => return None,
        }
    }
}

/// The refusal both [`save`] and [`clear`] raise for a record written by a
/// **newer** Sabrage.
///
/// Enforced here and not only in [`super::reconcile::untouchable`], because a
/// path that never went through reconcile (a launch that carried on past a
/// `Reconciled::Busy`, a teardown, a guard flag flip) would otherwise rewrite
/// a v2 record through this v1 struct
/// (tests::a_newer_schema_record_is_never_overwritten_or_deleted).
fn newer_schema(verb: &str, path: &Path, existing: &SessionState) -> SabrageError {
    SabrageError::fatal(
        format!(
            "refusing to {verb} {} — it was written by a newer Sabrage (schema v{}, this build \
             understands v{SESSION_STATE_VERSION}) and may describe a guard this build cannot undo",
            path.display(),
            existing.version,
        ),
        "./demo.sh stop --bottle <name>",
    )
}

/// The refusal both [`save`] and [`clear`] raise for a record that belongs to
/// another live front-end.
fn owned_elsewhere(verb: &str, path: &Path, existing: &SessionState) -> SabrageError {
    SabrageError::fatal(
        format!(
            "refusing to {verb} {} — it describes the session Sabrage process {} is running \
             (bottle '{}')",
            path.display(),
            existing.owner_pid,
            existing.bottle
        ),
        "./demo.sh stop --bottle <name>",
    )
}

/// Write the state file atomically (pretty JSON plus a trailing newline).
///
/// Goes through the [`Executor`] like every other mutation, so `--dry-run`
/// plans the write instead of performing it. Pretty-printed because a human
/// reading this file is exactly the situation it exists for.
///
/// # Errors
///
/// Refuses a record on disk that describes a **different** run another live
/// process owns ([`has_live_foreign_owner`]) — the ordinary guard-by-guard
/// flag flip over one's own run is unaffected — and any record written by a
/// newer Sabrage, one's own run included. The check and the write happen
/// under `lock_record`, which a dry run skips, having nothing to
/// serialize. See
/// tests::a_save_for_a_different_run_does_not_clobber_a_live_owners_record.
pub async fn save(executor: &dyn Executor, path: &Path, state: &SessionState) -> Result<()> {
    let _lock = if executor.is_dry_run() {
        None
    } else {
        lock_record(path).await
    };
    if let Ok(Some(existing)) = load(path) {
        if existing.run_id != state.run_id && has_live_foreign_owner(&existing) {
            return Err(owned_elsewhere("overwrite", path, &existing));
        }
        if !existing.is_supported_version() {
            return Err(newer_schema("overwrite", path, &existing));
        }
    }
    write_record(executor, path, state).await
}

/// [`save`]'s write half, with no refusals and no locking of its own — for the
/// callers that already hold [`lock_record`] and have already decided.
async fn write_record(executor: &dyn Executor, path: &Path, state: &SessionState) -> Result<()> {
    if let Some(parent) = path.parent() {
        executor.create_dir_all(parent).await?;
    }
    let mut bytes = serde_json::to_vec_pretty(state)
        .map_err(|e| SabrageError::io(path, std::io::Error::other(e)))?;
    bytes.push(b'\n');
    executor.write_atomic(path, &bytes).await
}

/// Set `detached: true` on the record for `run_id` — and **only** if that is
/// still the record on disk. `true` when the flag was written.
///
/// [`super::reconcile::detach`]'s safety net. The load, the compare and the
/// write happen under one `lock_record`, and an absent record is left
/// absent: re-creating it would resurrect a session the supervisor cleared
/// while this call was in flight
/// (tests::mark_detached_never_recreates_a_cleared_record).
pub async fn mark_detached(executor: &dyn Executor, path: &Path, run_id: RunId) -> Result<bool> {
    let _lock = if executor.is_dry_run() {
        None
    } else {
        lock_record(path).await
    };
    let Ok(Some(mut existing)) = load(path) else {
        return Ok(false);
    };
    if existing.run_id != run_id || existing.detached {
        return Ok(false);
    }
    if has_live_foreign_owner(&existing) {
        return Err(owned_elsewhere("overwrite", path, &existing));
    }
    if !existing.is_supported_version() {
        return Err(newer_schema("overwrite", path, &existing));
    }
    existing.detached = true;
    write_record(executor, path, &existing).await?;
    Ok(true)
}

/// Remove the state file, whichever session it describes. A missing file is
/// success — clearing twice (clean teardown, then a reconcile that already ran)
/// must not fail.
///
/// Refuses, like [`save`], when the record on disk belongs to another live
/// front-end or was written by a newer Sabrage.
///
/// Prefer [`clear_run`] wherever the caller knows *which* run it is finishing:
/// this form deletes whatever it finds, so a slow teardown can delete the
/// record a newer launch has already written.
pub async fn clear(executor: &dyn Executor, path: &Path) -> Result<()> {
    clear_inner(executor, path, None).await
}

/// [`clear`], but only if the record on disk is still `expected`'s.
///
/// The compare-and-swap the single shared record path needs: neither the
/// atomic rename nor `lock_record` stops a *late* teardown (or a reconcile
/// that started before a launch did) from deleting a **newer** run's record —
/// its audio device, dashboard and forwards description and all. A different
/// run on disk is not an error: the file is left exactly as it is and `Ok(())`
/// is returned, because the newer owner is responsible for it now. See
/// tests::clear_run_only_removes_the_record_it_names.
pub async fn clear_run(executor: &dyn Executor, path: &Path, expected: RunId) -> Result<()> {
    clear_inner(executor, path, Some(expected)).await
}

/// [`clear`]/[`clear_run`]'s shared body: the refusals, then the optional
/// expected-run compare, all under [`lock_record`] so the record cannot be
/// replaced between the read and the removal.
async fn clear_inner(executor: &dyn Executor, path: &Path, expected: Option<RunId>) -> Result<()> {
    let _lock = if executor.is_dry_run() {
        None
    } else {
        lock_record(path).await
    };
    if let Ok(Some(existing)) = load(path) {
        if has_live_foreign_owner(&existing) {
            return Err(owned_elsewhere("delete", path, &existing));
        }
        if !existing.is_supported_version() {
            return Err(newer_schema("delete", path, &existing));
        }
        if expected.is_some_and(|run_id| run_id != existing.run_id) {
            return Ok(());
        }
    }
    executor.remove_file(path).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::{DryRunExecutor, PlannedKind, RealExecutor};
    use crate::stages::null_sink;
    use tokio_util::sync::CancellationToken;
    use uuid::Uuid;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sabrage-session-state-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn real() -> RealExecutor {
        RealExecutor::new(Uuid::nil(), null_sink(), CancellationToken::new())
    }

    fn sample() -> SessionState {
        SessionState {
            wine: Some(ProcInfo {
                pid: 4242,
                start_time: 1786300214,
                exe: PathBuf::from("/Applications/CrossOver.app/…/wine"),
            }),
            dashboard: Some(ProcInfo {
                pid: 4243,
                start_time: 1786300215,
                exe: PathBuf::from("/repo/ext/ALVR/target/release/alvr_dashboard"),
            }),
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
                Uuid::nil(),
                "Steam",
                "/games/Beat Saber 1294",
                "/repo/logs/beatsaber-20260829-101112.log",
                1786300214181,
            )
        }
    }

    #[tokio::test]
    async fn round_trips_through_the_file() {
        let dir = scratch("roundtrip");
        let path = dir.join("nested/session-state.json");
        let state = sample();

        assert_eq!(load(&path).unwrap(), None, "absent file is None, not Err");
        save(&real(), &path, &state).await.unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.ends_with("}\n"), "pretty JSON plus one newline");
        assert!(
            text.contains("\"prevAudioOutput\""),
            "camelCase on the wire"
        );
        assert!(text.contains("\"startTime\""), "ProcInfo is camelCase too");
        assert_eq!(load(&path).unwrap(), Some(state));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_minimal_file_loads_on_defaults() {
        // Everything `#[serde(default)]` omitted — the shape an older Sabrage
        // (or a hand-edited file) can produce.
        let json = r#"{
            "version": 1,
            "runId": "00000000-0000-0000-0000-000000000000",
            "bottle": "Steam",
            "bsDir": "/games/bs",
            "startedAtUnixMs": 1786300214181,
            "logPath": "/repo/logs/x.log"
        }"#;
        let s: SessionState = serde_json::from_str(json).unwrap();
        assert_eq!(s.owner_pid, 0);
        assert!(s.wine.is_none() && s.dashboard.is_none());
        assert!(s.prev_audio_output.is_none());
        assert!(s.wired_forwards.is_empty());
        assert_eq!(s.guards, GuardFlags::default());
        assert!(!s.detached);
        assert!(!s.has_pending_guards());
    }

    #[test]
    fn a_corrupt_file_is_an_error_never_a_silent_none() {
        let dir = scratch("corrupt");
        let path = dir.join("session-state.json");
        std::fs::write(&path, b"{not json").unwrap();
        let err = load(&path).unwrap_err();
        assert_eq!(err.kind(), "io");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pending_guards_track_the_flags_one_at_a_time() {
        let mut s = sample();
        assert!(s.has_pending_guards());
        s.guards.audio_restored = true;
        assert!(s.has_pending_guards(), "dashboard + forwards still pending");
        s.guards.dashboard_closed = true;
        assert!(s.has_pending_guards(), "forwards still pending");
        s.guards.forwards_cleared = true;
        assert!(!s.has_pending_guards());

        // A session that never rerouted anything has nothing to undo.
        let bare = SessionState::new(Uuid::nil(), "Steam", "/g", "/l", 0);
        assert!(!bare.has_pending_guards());
        assert_eq!(bare.version, SESSION_STATE_VERSION);
        assert_eq!(bare.owner_pid, std::process::id());
    }

    #[tokio::test]
    async fn a_dry_run_plans_the_write_instead_of_performing_it() {
        let dir = scratch("dry");
        let path = dir.join("session-state.json");
        let ex = DryRunExecutor::new(Uuid::nil(), null_sink(), CancellationToken::new());
        save(&ex, &path, &sample()).await.unwrap();
        assert!(!path.exists(), "dry run wrote the state file");
        let kinds: Vec<PlannedKind> = ex.planned().iter().map(|p| p.kind).collect();
        assert_eq!(kinds, vec![PlannedKind::CreateDir, PlannedKind::Write]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A live process whose pid is not ours, for the ownership guard. Killed
    /// on drop, so the fixture cannot leak a process into the test machine.
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

    #[test]
    fn a_live_foreign_owner_is_recognised_only_while_its_session_can_still_be_running() {
        let foreign = ForeignProcess::spawn();
        let mut s = sample();

        // Our own record, whoever it describes.
        assert!(!has_live_foreign_owner(&s), "owner_pid is this process");

        // Another live process, and the recorded wine pid is not observably
        // gone — hands off.
        s.set_owner(foreign.pid());
        s.wine = None;
        assert!(
            has_live_foreign_owner(&s),
            "the pre-spawn window is covered"
        );

        // Same pid, a different process: a recycled `owner_pid` is not an
        // owner. Without the recorded start time such a record can be neither
        // overwritten nor cleared, and it wedges the next launch.
        let real_start = s.owner_started_at;
        s.owner_started_at = Some(real_start.unwrap_or(0).wrapping_add(1_000));
        assert!(
            !has_live_foreign_owner(&s),
            "a recycled owner pid must not wedge the record"
        );
        s.owner_started_at = real_start;

        // Same owner, but the session's wine child is provably gone: a
        // leftover, and undoing its guards is exactly the job.
        s.wine = Some(ProcInfo {
            pid: u32::MAX - 1,
            start_time: 1,
            exe: PathBuf::new(),
        });
        assert!(!has_live_foreign_owner(&s));

        // An owner that has itself exited is never protected either.
        s.wine = None;
        s.set_owner(u32::MAX - 1);
        assert!(!has_live_foreign_owner(&s));
        s.set_owner(0);
        assert!(
            !has_live_foreign_owner(&s),
            "an older record wrote no owner"
        );
    }

    #[tokio::test]
    async fn a_save_for_a_different_run_does_not_clobber_a_live_owners_record() {
        let dir = scratch("cas");
        let path = dir.join("session-state.json");
        let foreign = ForeignProcess::spawn();

        let mut theirs = sample();
        theirs.set_owner(foreign.pid());
        theirs.wine = None; // mid-launch, guards taken, nothing spawned yet
        save(&real(), &path, &theirs).await.unwrap();
        let bytes = std::fs::read(&path).unwrap();

        let mine = SessionState::new(Uuid::new_v4(), "Steam", "/g", "/l", 1);
        let err = save(&real(), &path, &mine).await.unwrap_err();
        assert!(err.to_string().contains("refusing to overwrite"), "{err:?}");
        let err = clear(&real(), &path).await.unwrap_err();
        assert!(err.to_string().contains("refusing to delete"), "{err:?}");
        assert_eq!(
            std::fs::read(&path).unwrap(),
            bytes,
            "the other front-end's record is byte-identical afterwards"
        );

        // The owner's own writes still go through.
        theirs.guards.audio_restored = true;
        save(&real(), &path, &theirs).await.unwrap();

        // …and once that process is gone, the record is a leftover like any
        // other: the next launch may replace it.
        drop(foreign);
        save(&real(), &path, &mine).await.unwrap();
        assert_eq!(load(&path).unwrap().unwrap().run_id, mine.run_id);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A9-8. The version rule is the module header's, not reconcile's: a path
    /// that never went through reconciliation (a launch that carried on, a
    /// teardown, a guard flag flip) must not rewrite a v2 record through this
    /// v1 struct, nor delete it.
    #[tokio::test]
    async fn a_newer_schema_record_is_never_overwritten_or_deleted() {
        let dir = scratch("newer-schema-guard");
        let path = dir.join("session-state.json");
        let v2 = r#"{
  "version": 2,
  "runId": "00000000-0000-0000-0000-000000000000",
  "bottle": "Steam",
  "bsDir": "/games/bs",
  "startedAtUnixMs": 1786300214181,
  "logPath": "/repo/logs/x.log",
  "futureGuard": {"somethingWeCannotUndo": true}
}
"#;
        std::fs::write(&path, v2).unwrap();

        let err = save(&real(), &path, &sample()).await.unwrap_err();
        assert!(err.to_string().contains("refusing to overwrite"), "{err:?}");
        assert!(err.to_string().contains("schema v2"), "{err:?}");
        let err = clear(&real(), &path).await.unwrap_err();
        assert!(err.to_string().contains("refusing to delete"), "{err:?}");
        let err = clear_run(&real(), &path, Uuid::nil()).await.unwrap_err();
        assert!(err.to_string().contains("refusing to delete"), "{err:?}");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            v2,
            "the newer record is byte-identical afterwards"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A9-3. `clear` alone is not a compare-and-swap: it deletes whatever it
    /// finds. A teardown for a run that has already been superseded must leave
    /// the newer run's record — its audio device and forwards description —
    /// exactly where it is.
    #[tokio::test]
    async fn clear_run_only_removes_the_record_it_names() {
        let dir = scratch("clear-run-cas");
        let path = dir.join("session-state.json");

        let run_a = Uuid::new_v4();
        let mut run_b = sample();
        run_b.run_id = Uuid::new_v4();
        save(&real(), &path, &run_b).await.unwrap();

        clear_run(&real(), &path, run_a).await.unwrap();
        assert!(path.exists(), "run A's teardown deleted run B's record");
        assert_eq!(load(&path).unwrap().unwrap().run_id, run_b.run_id);

        clear_run(&real(), &path, run_b.run_id).await.unwrap();
        assert!(!path.exists(), "its own run's record is removed");

        // A missing file is success for both forms.
        clear_run(&real(), &path, run_b.run_id).await.unwrap();

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// An absent record stays absent: re-creating it would resurrect a session
    /// the supervisor already cleared. A record that belongs to a different run
    /// stays byte-identical: its run, not this one, owns that file.
    #[tokio::test]
    async fn mark_detached_never_recreates_a_cleared_record() {
        let dir = scratch("detach-no-create");
        let path = dir.join("session-state.json");

        // Case 1 — absent record: no file created, returns false.
        let mine = Uuid::new_v4();
        let result = mark_detached(&real(), &path, mine).await.unwrap();
        assert!(
            !result,
            "mark_detached should return false for absent record"
        );
        assert!(!path.exists(), "mark_detached created a file from nothing");

        // Case 2 — foreign record (different run_id): file left byte-identical,
        // returns false.
        let other = Uuid::new_v4();
        let mut state = sample();
        state.run_id = other;
        save(&real(), &path, &state).await.unwrap();
        let bytes_before = std::fs::read(&path).unwrap();

        let result = mark_detached(&real(), &path, mine).await.unwrap();
        assert!(!result, "mark_detached should return false for foreign run");
        assert_eq!(
            std::fs::read(&path).unwrap(),
            bytes_before,
            "the foreign record must be byte-identical afterwards"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn clearing_is_idempotent() {
        let dir = scratch("clear");
        let path = dir.join("session-state.json");
        save(&real(), &path, &sample()).await.unwrap();
        clear(&real(), &path).await.unwrap();
        assert!(!path.exists());
        clear(&real(), &path).await.unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
