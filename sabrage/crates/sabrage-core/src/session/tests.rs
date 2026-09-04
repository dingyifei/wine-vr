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

    // 4b: a record that exists but will not parse — the conservative
    // answer, because it may still be describing a live session.
    std::fs::write(&path, b"{ not json").unwrap();
    let err = ensure_idle_at(&path, &status, "rebuild").unwrap_err();
    assert!(err.to_string().contains("cannot be read"), "{err}");
    std::fs::remove_file(&path).unwrap();

    // 5: a `demo.sh run` session — nothing of ours anywhere, but the
    // runtime is reporting in right now, naming a live process. Both
    // halves are asserted because a door that refused on freshness alone
    // said "a session is live" over a file the UI was calling Idle (A10-8).
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

/// The slot is owned by its run id: a teardown for another run must not
/// erase it, and its own clear is idempotent. The cheap
/// `live_session_run_id` projection must give the same answer as the
/// full-clone `live_session().map(|h| h.run_id)` it replaces at the hot
/// call sites (a detach poll loop, `reconcile`'s ownership check) —
/// two independent bodies, so the agreement is asserted, not assumed.
#[test]
fn the_live_slot_is_set_and_cleared_by_run_id() {
    let _g = lock_session_globals();
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    set_live_session(handle(a));
    assert!(live_session_is(a));
    assert_eq!(live_session_run_id(), Some(a));
    assert_eq!(live_session_run_id(), live_session().map(|h| h.run_id));
    assert!(!live_session_is(b));

    // A stale teardown for a different run must not clear the current one.
    clear_live_session(b);
    assert!(live_session_is(a));

    clear_live_session(a);
    assert!(live_session().is_none());
    assert_eq!(live_session_run_id(), None);
    assert!(!live_session_is(a));
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

fn devices(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| n.to_string()).collect()
}

/// Both tiers of the fallback policy in one table: a built-in output
/// wherever it sits in the list, else the first non-virtual device, and
/// `None` when every candidate is virtual or there are none — `None` is
/// what makes the caller print the remedy instead of switching the Mac to
/// something that stays silent.
#[test]
fn the_fallback_picks_the_built_in_output_then_any_real_one() {
    let cases: &[(&str, &[&str], Option<&str>)] = &[
        (
            // A live `SwitchAudioSource -a -t output` list, in its own
            // order: the recorded AirPods are simply not on it any more.
            "observed 2026-08-29 list: built-in among the virtuals",
            &[
                "BlackHole 2ch",
                "MacBook Pro Speakers",
                "Steam Streaming Microphone",
                "Steam Streaming Speakers",
                "Virtual Desktop Mic",
                "Virtual Desktop Speakers",
            ],
            Some("MacBook Pro Speakers"),
        ),
        (
            "MacBook Air Speakers, listed last, still wins",
            &["BlackHole 2ch", "MacBook Air Speakers"],
            Some("MacBook Air Speakers"),
        ),
        (
            "Mac Studio Speakers",
            &["Mac Studio Speakers"],
            Some("Mac Studio Speakers"),
        ),
        (
            "Mac mini Speakers",
            &["Mac mini Speakers"],
            Some("Mac mini Speakers"),
        ),
        (
            "the built-in output outranks anything earlier in the list",
            &["Steam Streaming Speakers", "Built-in Output"],
            Some("Built-in Output"),
        ),
        (
            "no built-in on the list: the first device that is not virtual",
            &[
                "BlackHole 2ch",
                "Virtual Desktop Speakers",
                "Studio Display Speakers",
            ],
            Some("Studio Display Speakers"),
        ),
        (
            "every candidate is virtual: switching to any of them is still silence",
            &[
                "BlackHole 2ch",
                "Virtual Desktop Mic",
                "Virtual Desktop Speakers",
            ],
            None,
        ),
        (
            "the marker matches as a substring, so BlackHole 16ch is virtual too",
            &["BlackHole 16ch"],
            None,
        ),
        ("empty list", &[], None),
    ];
    for (label, list, expected) in cases {
        assert_eq!(
            fallback_output_device(&devices(list)),
            expected.map(str::to_string),
            "{label}"
        );
    }
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
