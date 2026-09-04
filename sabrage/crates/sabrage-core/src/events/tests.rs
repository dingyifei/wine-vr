use super::*;

fn rid() -> RunId {
    Uuid::nil()
}

#[test]
fn stage_round_trips_through_its_demo_sh_word() {
    for s in Stage::EVERY {
        assert_eq!(s.to_string().parse::<Stage>().unwrap(), s);
        assert_eq!(
            serde_json::to_string(&s).unwrap(),
            format!("\"{}\"", s.as_str())
        );
    }
    assert_eq!("setup".parse::<Stage>().unwrap(), Stage::Setup);
    let err = "doctor".parse::<Stage>().unwrap_err();
    assert_eq!(err.exit_code(), 2);
    assert_eq!(err.to_string(), "invalid input: unknown stage 'doctor'");
}

#[test]
fn only_bottle_stages_require_a_bottle() {
    assert!(!Stage::Setup.requires_bottle());
    assert!(!Stage::Build.requires_bottle());
    assert!(Stage::Install.requires_bottle());
    assert!(Stage::Run.requires_bottle());
    assert!(Stage::Stop.requires_bottle());
}

#[test]
fn step_ids_are_unique_and_prefixed_by_their_stage() {
    let all = step::all();
    let unique: std::collections::BTreeSet<_> = all.iter().collect();
    assert_eq!(unique.len(), all.len(), "duplicate step id");
    assert_eq!(all.len(), 28);
    for stage in Stage::EVERY {
        for s in stage.steps() {
            assert!(
                s.starts_with(&format!("{}.", stage.as_str())),
                "{s} is not prefixed by its stage"
            );
        }
    }
    assert_eq!(Stage::Run.steps(), step::RUN);
    assert_eq!(Stage::Run.steps().len(), 10);
    assert_eq!(Stage::Run.steps().first(), Some(&step::RUN_PREFLIGHT));
    assert_eq!(Stage::Run.steps().last(), Some(&step::RUN_TEARDOWN));
}

#[test]
fn events_serialize_internally_tagged_with_camel_case_fields() {
    let ev = StageEvent::StageFinished {
        run_id: rid(),
        stage: Stage::Install,
        ok: true,
        exit_code_equiv: 0,
    };
    let json = serde_json::to_value(&ev).unwrap();
    assert_eq!(json["kind"], "stageFinished");
    assert_eq!(json["stage"], "install");
    assert_eq!(json["exitCodeEquiv"], 0);

    let ev = StageEvent::ok(
        rid(),
        Some(step::INSTALL_BOTTLE),
        "ActiveRuntime registered",
    );
    let json = serde_json::to_value(&ev).unwrap();
    assert_eq!(json["kind"], "line");
    assert_eq!(json["severity"], "ok");
    assert_eq!(json["step"], "install.3.bottle");
    // Verbatim: no marker, no colour, no leading spaces.
    assert_eq!(json["text"], "ActiveRuntime registered");
}

#[test]
fn the_run_events_keep_their_wire_shape() {
    // Text is verbatim: leading spaces survive, and an empty line is legal
    // (run.sh's bare `print ""`).
    let ev = StageEvent::text(rid(), None, "   exe: C:\\Beat Saber.exe");
    let json = serde_json::to_value(&ev).unwrap();
    assert_eq!(json["kind"], "text");
    assert_eq!(json["text"], "   exe: C:\\Beat Saber.exe");
    assert_eq!(json["step"], serde_json::Value::Null);
    let empty = StageEvent::text(rid(), None, "");
    assert_eq!(serde_json::to_value(&empty).unwrap()["text"], "");

    let ev = StageEvent::Check {
        run_id: rid(),
        step: step::RUN_PREFLIGHT.into(),
        outcome: crate::checks::CheckOutcome::fail(
            "run.bridge-built",
            "bridge not built",
            "./demo.sh build",
        ),
        gate: crate::contract::Gate::Block,
    };
    let json = serde_json::to_value(&ev).unwrap();
    assert_eq!(json["kind"], "check");
    assert_eq!(json["gate"], "block");
    assert_eq!(json["outcome"]["slug"], "run.bridge-built");
    assert_eq!(json["outcome"]["status"], "fail");

    let ev = StageEvent::Launched {
        run_id: rid(),
        pid: 59004,
        start_time: 1786300214,
        log_path: "/repo/logs/beatsaber-20260829-101112.log".into(),
        started_at_unix_ms: 1786300214181,
    };
    let json = serde_json::to_value(&ev).unwrap();
    assert_eq!(json["kind"], "launched");
    assert_eq!(json["pid"], 59004);
    assert_eq!(json["startTime"], 1786300214u64);
    assert_eq!(json["startedAtUnixMs"], 1786300214181u64);
    assert_eq!(json["logPath"], "/repo/logs/beatsaber-20260829-101112.log");
}

#[test]
fn every_event_carries_its_run_id() {
    let evs = vec![
        StageEvent::StageStarted {
            run_id: rid(),
            stage: Stage::Setup,
        },
        StageEvent::Section {
            run_id: rid(),
            title: "global DXMT overlay".into(),
        },
        StageEvent::info(rid(), None, "x"),
        StageEvent::Output {
            run_id: rid(),
            step: step::BUILD_OXRSYS.into(),
            stream: Stream::Stderr,
            chunk: "[1/2] cc".into(),
            end: crate::process::ChunkEnd::Lf,
        },
        StageEvent::Progress {
            run_id: rid(),
            step: step::SETUP_PINNED.into(),
            label: "DXMT fork artifacts".into(),
            current: 1,
            total: Some(2),
        },
        StageEvent::AutoFixed {
            run_id: rid(),
            step: step::BUILD_HELPER.into(),
            fix: FixAction::RestageHelper,
            description: "restaged".into(),
        },
        StageEvent::NeedsAdmin {
            run_id: rid(),
            step: step::INSTALL_HOST_MANIFEST.into(),
            reason: "writes the host OpenXR registration".into(),
        },
        StageEvent::text(rid(), Some(step::RUN_LAUNCH), "   log: /repo/logs/x.log"),
        StageEvent::Check {
            run_id: rid(),
            step: step::RUN_PREFLIGHT.into(),
            outcome: crate::checks::CheckOutcome::pass("run.wine-exec", "wine present"),
            gate: crate::contract::Gate::Block,
        },
        StageEvent::Launched {
            run_id: rid(),
            pid: 4242,
            start_time: 1786300214,
            log_path: "/repo/logs/beatsaber-20260829-101112.log".into(),
            started_at_unix_ms: 1786300214181,
        },
        StageEvent::Fatal {
            run_id: rid(),
            message: "boom".into(),
            remedy: None,
            fix: None,
        },
        StageEvent::StageFinished {
            run_id: rid(),
            stage: Stage::Stop,
            ok: false,
            exit_code_equiv: 1,
        },
    ];
    for ev in &evs {
        assert_eq!(ev.run_id(), rid());
        let text = serde_json::to_string(ev).unwrap();
        assert_eq!(&serde_json::from_str::<StageEvent>(&text).unwrap(), ev);
    }
    assert_eq!(evs[3].step(), Some(step::BUILD_OXRSYS));
    assert_eq!(evs[2].step(), None);
    // Text carries its step; Launched never does.
    assert_eq!(evs[7].step(), Some(step::RUN_LAUNCH));
    assert_eq!(evs[8].step(), Some(step::RUN_PREFLIGHT));
    assert_eq!(evs[9].step(), None);
    assert_eq!(evs[10].step(), None);
}
