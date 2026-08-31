//! `session-state.json` — the crash-recovery record.
//!
//! `~/Library/Application Support/Sabrage/session-state.json`
//! ([`crate::paths::Paths::session_state_path`]). Written by the launch path,
//! read by [`super::reconcile`]. It exists for exactly one reason: `run.sh`'s
//! guards are shell traps, and a `SIGKILL`, a panic, or a power loss skips
//! traps entirely — leaving the Mac's default output device on `BlackHole 2ch`
//! with nothing on the machine able to say what it was before, an ALVR
//! dashboard nobody can attribute, and (after `--wired`) two adb forwards that
//! silently break WiFi discovery on the next run. `stop.sh` can only *warn*
//! about all three. This file is what lets Sabrage actually undo them
//! (design-core §4.2; PARITY.md "Persisted audio-device restore").
//!
//! # The invariant (write-before-mutate)
//!
//! **The file is saved BEFORE each guarded mutation, and each guard flag is
//! flipped by its own `save()` after that guard is released.** Concretely:
//!
//! | order | what happens |
//! |---|---|
//! | 1 | `prev_audio_output` recorded and [`save`]d — *then* `SwitchAudioSource -t output -s "BlackHole 2ch"` runs |
//! | 2 | `alvr_dashboard` spawned — its [`crate::process::ProcInfo`] recorded and [`save`]d immediately |
//! | 3 | `--wired` forwards created — recorded and [`save`]d as they are made |
//! | 4 | wine spawned — its identity recorded and [`save`]d |
//! | 5 | on teardown, each guard released → its [`GuardFlags`] bit set → [`save`] |
//! | 6 | all guards released and the child reaped → [`clear`] |
//!
//! Saving *after* the mutation would leave the exact window this file exists to
//! close: crash between "audio switched" and "audio recorded" and the device is
//! unrecoverable. The cost is a redundant save when nothing crashes, which is
//! one small atomic write per guard.
//!
//! Recovery is therefore **idempotent by construction**: a flag that is already
//! `true` means that guard was released, so reconcile skips it; a crash at any
//! instant leaves a file describing only work that still needs doing.
//!
//! # Forward compatibility
//!
//! Every optional field and every flag carries `#[serde(default)]`, so a file
//! written by an older Sabrage still loads (the phase that adds a field must
//! not strand a user mid-session). [`SESSION_STATE_VERSION`] is for the case
//! defaults cannot cover, and it is enforced in one direction only:
//! [`SessionState::is_supported_version`] is false for a record written by a
//! *newer* Sabrage, and every mutating path
//! ([`super::reconcile`]) must then report it and leave it alone. Rewriting a
//! v2 record through a v1 struct would silently drop the guard that version
//! added, and clearing it afterwards would throw away the only description of
//! a mutation this binary cannot undo.
//!
//! # One file, more than one front-end
//!
//! There is exactly one record path per machine
//! ([`crate::paths::Paths::session_state_path`]) and two front-ends that write
//! it — the Sabrage app and the `sabrage` CLI. An atomic rename stops a torn
//! read; it does **not** stop a lost update. [`save`] and [`clear`] therefore
//! refuse to touch a record that names a *live foreign* `owner_pid`
//! ([`has_live_foreign_owner`]), which is the contract `owner_pid`'s own
//! documentation already states. The compare against what is on disk happens
//! under the advisory lock at
//! [`crate::paths::Paths::session_state_lock_path`], so the other front-end
//! cannot write between the read and the rename. The lock is best-effort (a
//! machine that cannot lock must still be able to stop a session) — the
//! compare is what makes the remaining race *safe*, and the lock is what makes
//! it rare. What neither covers is a whole `reconcile` pass, which reads,
//! restores and rewrites across several of these calls; serializing that is
//! the operation lock's job.

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
/// Per-serial, because the removal must be too: `adb forward --remove` is
/// applied to exactly these ports on exactly this device, never
/// `--remove-all` (PARITY.md; CLAUDE.md's `--wired` note).
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
    /// `owner_pid` alone is a number, and pids are recycled: without this, a
    /// record written in the pre-spawn window whose owner then died stayed
    /// "foreign-owned" forever as soon as anything else reused that pid — and
    /// a foreign-owned record can be neither overwritten nor cleared, so the
    /// next launch died mid-run with a remedy (`./demo.sh stop`) that could not
    /// clear it either. Paired with `owner_pid` this is the same recycled-pid
    /// guard [`ProcInfo::is_same_process`] gives `wine`. `None` in records
    /// written before this field existed, where the pid alone is all there is.
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
    /// `false` means the file comes from a **newer** schema: it may describe a
    /// guard this binary has never heard of, so rewriting it through this
    /// struct would drop that guard's description and clearing it would throw
    /// away the only record of a mutation nothing here can undo. The mutating
    /// paths report such a record and leave it exactly as it is.
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
/// The second case is deliberately **not** folded into `None`: a corrupt file
/// may still be describing a live session with a rerouted audio device, and
/// silently reporting "nothing to recover" is how a user ends up with no sound
/// and no explanation. The caller surfaces it and can then remove the file
/// itself.
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
/// The three conditions together, because each one alone is wrong:
///
/// * `owner_pid` is neither 0 (an older record that never wrote one) nor this
///   process — our own records are ours to rewrite;
/// * that pid is still alive **and is still the process that wrote the
///   record** — a crashed owner's record is exactly what recovery exists for,
///   and refusing to touch it would strand the audio device forever. The
///   recorded [`SessionState::owner_started_at`] is what makes the second half
///   decidable; a record from before that field existed still has to trust the
///   bare pid;
/// * the session itself has not visibly ended: either the recorded wine child
///   is still that same process, or there is no wine child *yet* — the
///   pre-spawn window where the guards are already taken and the record is the
///   only description of them.
///
/// A record whose wine pid is gone is therefore never protected by this, no
/// matter who wrote it: it is a leftover, and undoing its guards is the whole
/// job. The residual false positive is a recycled `owner_pid` in a record
/// written before `owner_started_at` existed, which costs one kept record and
/// one row, never a mutation.
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
/// Refuses when what is already on disk describes a **different** run that
/// another live process owns ([`has_live_foreign_owner`]): the single record
/// path is shared by both front-ends, and an atomic rename prevents a torn
/// read, not a lost update. Saving over one's own run — the ordinary
/// guard-by-guard flag flip — is unaffected, and so is saving over a record
/// whose owner is gone. The check and the write happen under
/// [`lock_record`], so the other front-end cannot slip its own write between
/// them; a dry run takes no lock, having nothing to serialize.
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
    }
    if let Some(parent) = path.parent() {
        executor.create_dir_all(parent).await?;
    }
    let mut bytes = serde_json::to_vec_pretty(state)
        .map_err(|e| SabrageError::io(path, std::io::Error::other(e)))?;
    bytes.push(b'\n');
    executor.write_atomic(path, &bytes).await
}

/// Remove the state file. A missing file is success — clearing twice (clean
/// teardown, then a reconcile that already ran) must not fail.
///
/// Refuses, like [`save`], when the record on disk belongs to another live
/// front-end: a late teardown here must not delete the description of guards
/// that process still holds.
pub async fn clear(executor: &dyn Executor, path: &Path) -> Result<()> {
    let _lock = if executor.is_dry_run() {
        None
    } else {
        lock_record(path).await
    };
    if let Ok(Some(existing)) = load(path) {
        if has_live_foreign_owner(&existing) {
            return Err(owned_elsewhere("delete", path, &existing));
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
        // owner. Without the recorded start time this record could never be
        // overwritten *or* cleared, and the next launch died on it.
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

        // The owner's own writes still go through, and so does anyone's once
        // that process is gone.
        theirs.guards.audio_restored = true;
        save(&real(), &path, &theirs).await.unwrap();

        // …and once that process is gone, the record is a leftover like any
        // other: the next launch may replace it.
        drop(foreign);
        save(&real(), &path, &mine).await.unwrap();
        assert_eq!(load(&path).unwrap().unwrap().run_id, mine.run_id);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_newer_schema_is_recognised_and_never_downgraded() {
        let json = r#"{
            "version": 2,
            "runId": "00000000-0000-0000-0000-000000000000",
            "bottle": "Steam",
            "bsDir": "/games/bs",
            "startedAtUnixMs": 1786300214181,
            "logPath": "/repo/logs/x.log",
            "futureGuard": {"somethingWeCannotUndo": true}
        }"#;
        let s: SessionState = serde_json::from_str(json).unwrap();
        assert_eq!(s.version, 2);
        assert!(
            !s.is_supported_version(),
            "a v2 record must not be rewritten through the v1 struct"
        );
        assert!(sample().is_supported_version());
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
