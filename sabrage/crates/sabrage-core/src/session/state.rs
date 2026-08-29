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
//! defaults cannot cover.

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
            owner_pid: std::process::id(),
            wine: None,
            dashboard: None,
            prev_audio_output: None,
            wired_forwards: Vec::new(),
            guards: GuardFlags::default(),
            detached: false,
        }
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

/// Write the state file atomically (pretty JSON plus a trailing newline).
///
/// Goes through the [`Executor`] like every other mutation, so `--dry-run`
/// plans the write instead of performing it. Pretty-printed because a human
/// reading this file is exactly the situation it exists for.
pub async fn save(executor: &dyn Executor, path: &Path, state: &SessionState) -> Result<()> {
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
pub async fn clear(executor: &dyn Executor, path: &Path) -> Result<()> {
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
