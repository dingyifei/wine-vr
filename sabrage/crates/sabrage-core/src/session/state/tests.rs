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
