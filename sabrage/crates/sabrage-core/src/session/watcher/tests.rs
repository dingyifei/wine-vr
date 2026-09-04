use super::*;

#[test]
fn the_monitor_derives_its_source_paths_from_oxr_appsup() {
    let mut paths = Paths::new("/repo");
    paths.oxr_appsup = PathBuf::from("/home/Library/Application Support/OXRSys");
    let m = SessionMonitor::new(paths);
    assert_eq!(
        m.runtime_status_path(),
        PathBuf::from("/home/Library/Application Support/OXRSys/runtime_status.json")
    );
    assert_eq!(
        m.runtime_log_path(),
        PathBuf::from("/home/Library/Application Support/OXRSys/oxrsys-runtime.log")
    );
}

#[test]
fn parse_runtime_status_accepts_the_observed_and_minimal_documents_and_rejects_a_half_written_one()
{
    // The observed file, verbatim — `transport` is not modelled.
    let json = r#"{"state":"idle","transport":"","process_id":59004,
                       "application_name":"Beat Saber","updated_at_unix_ms":1786300214181}"#;
    let s = parse_runtime_status(json).expect("the observed document parses");
    assert_eq!(s.state, "idle");
    assert_eq!(s.process_id, Some(59004));
    assert_eq!(s.application_name.as_deref(), Some("Beat Saber"));
    assert_eq!(s.updated_at_unix_ms, 1786300214181);

    // Only `state` + `updated_at_unix_ms` are required.
    let bare = parse_runtime_status(r#"{"state":"idle","updated_at_unix_ms":1}"#)
        .expect("the minimal document parses");
    assert!(bare.process_id.is_none() && bare.application_name.is_none());

    assert!(parse_runtime_status(r#"{"state":"idle","updated_at_"#).is_none());
    assert!(parse_runtime_status("").is_none());
}

#[test]
fn the_watcher_duration_budgets() {
    let cases: &[(&str, Duration, Duration)] = &[
        (
            "runtime status maximum age",
            RUNTIME_STATUS_MAX_AGE,
            Duration::from_secs(3),
        ),
        (
            "startup grace",
            SESSION_STARTUP_GRACE,
            Duration::from_secs(30),
        ),
        (
            "post-fresh stall grace",
            STALL_GRACE_AFTER_FRESH,
            Duration::from_secs(10),
        ),
    ];
    for (label, actual, expected) in cases {
        assert_eq!(*actual, *expected, "{label}");
    }
}

#[test]
fn is_fresh_accepts_only_stamps_inside_both_budgets() {
    let now = 1_786_300_214_181u64;
    let cases: &[(&str, u64, u64, bool)] = &[
        ("exactly now", 1_000, 1_000, true),
        (
            "a slightly future stamp is skew, not staleness",
            1_000,
            900,
            true,
        ),
        (
            "exactly at the staleness budget",
            1_000,
            1_000 + 3_000,
            true,
        ),
        (
            "one ms past the staleness budget",
            1_000,
            1_000 + 3_001,
            false,
        ),
        ("ordinary skew is still believed", now + 1_000, now, true),
        (
            "exactly at the allowance",
            now + MAX_FUTURE_SKEW.as_millis() as u64,
            now,
            true,
        ),
        ("one ms past the future allowance", now + 2_001, now, false),
        (
            "r1:A9-7 regression: an hour ahead is a clock correction or a corrupt \
                 number, and believing it would suppress Stalled for that whole hour",
            now + 3_600_000,
            now,
            false,
        ),
    ];
    for (label, updated, now_ms, expected) in cases {
        assert_eq!(is_fresh(*updated, *now_ms), *expected, "{label}");
    }
}

/// A10-8. One predicate, two readers: the `External` phase the Session
/// screen shows and the door every mutating operation goes through. Two
/// spellings of "is the runtime live" let the UI say Idle while Settings
/// refused to save over the same file.
#[test]
fn runtime_status_live_is_freshness_and_a_live_pid_together() {
    let now = crate::session::now_unix_ms();
    let status = |pid: Option<u32>, at: u64| RuntimeStatus {
        state: "streaming".into(),
        process_id: pid,
        updated_at_unix_ms: at,
        application_name: None,
    };
    let me = std::process::id();
    assert!(runtime_status_live(&status(Some(me), now), now));
    assert!(
        !runtime_status_live(&status(Some(me), now - 60_000), now),
        "the file outlives the runtime"
    );
    assert!(
        !runtime_status_live(&status(Some(u32::MAX - 1), now), now),
        "fresh, but nothing is there"
    );
    assert!(
        !runtime_status_live(&status(None, now), now),
        "oxrsys always writes process_id; a file without one vouches for nothing"
    );
}

/// A9-6. The spdlog prefix oxrsys writes (`Config.cpp`'s
/// `[%Y-%m-%d %H:%M:%S.%e] [%l] %v`), read back as local wall-clock time —
/// the only thing that can say which session a preloaded log line belongs
/// to.
#[test]
fn parse_log_timestamp_reads_the_spdlog_prefix_as_local_time() {
    use chrono::TimeZone;
    let at = chrono::Local
        .with_ymd_and_hms(2026, 8, 10, 1, 30, 13)
        .single()
        .expect("an unambiguous local time");
    assert_eq!(
        parse_log_timestamp(
            "[2026-08-10 01:30:13.017] [info] OXRSys/ALVR: encoder ready 2064x2208 @72Hz \
                 100Mbps (HEVC, native helper)"
        ),
        Some(at.timestamp_millis() as u64 + 17)
    );
    // Anything that is not that prefix carries no time at all.
    assert!(parse_log_timestamp(
        "OXRSys/ALVR: encoder ready 2064x2208 @72Hz 100Mbps (HEVC, native helper)"
    )
    .is_none());
    assert!(parse_log_timestamp("[info] no date here]").is_none());
    assert!(parse_log_timestamp("").is_none());
}

fn fixture_log_lines() -> Vec<String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/phase3/oxrsys-runtime-sample.log.txt");
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
}

#[test]
fn parse_encoder_ready_reads_both_encoder_forms_from_the_fixture() {
    let cases = &[
        (
            "the HEVC native-helper form",
            "(HEVC, native helper)",
            "HEVC",
            "native helper",
            3008,
            1664,
            72,
            80,
        ),
        (
            "the H.264 in-process downgrade",
            "(H.264, in-process)",
            "H.264",
            "in-process",
            3008,
            1664,
            72,
            80,
        ),
    ];
    let lines = fixture_log_lines();
    for (label, needle, codec, path, width, height, refresh_hz, bitrate_mbps) in cases {
        let line = lines
            .iter()
            .find(|l| l.contains(*needle))
            .unwrap_or_else(|| panic!("fixture has no line for {label}"));
        let info = parse_encoder_ready(line).expect("parses");
        assert_eq!(
            (
                info.codec.as_str(),
                info.path.as_str(),
                info.width,
                info.height,
                info.refresh_hz,
                info.bitrate_mbps,
            ),
            (*codec, *path, *width, *height, *refresh_hz, *bitrate_mbps),
            "{label}"
        );
    }
}

#[test]
fn parse_encoder_ready_ignores_unrelated_fixture_lines() {
    for line in fixture_log_lines() {
        if !line.contains("encoder ready") {
            assert!(
                parse_encoder_ready(&line).is_none(),
                "false match on: {line}"
            );
        }
    }
    assert!(parse_encoder_ready("").is_none());
}

#[test]
fn parse_encoder_ready_works_on_the_bare_message_with_no_timestamp_prefix() {
    let info = parse_encoder_ready(
        "OXRSys/ALVR: encoder ready 2064x2208 @72Hz 100Mbps (HEVC, native helper)",
    )
    .unwrap();
    assert_eq!(
        (info.width, info.height, info.refresh_hz, info.bitrate_mbps),
        (2064, 2208, 72, 100)
    );
}

#[test]
fn the_encoder_ready_format_string_pin_is_unchanged() {
    // F11: if oxrsys ever changes this spdlog format string, this test
    // goes red before `parse_encoder_ready` silently starts missing every
    // encoder-ready line.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("repo root resolves");
    let cpp = root.join("ext/oxrsys/runtime/src/AlvrStreamingBackend.cpp");
    let text = std::fs::read_to_string(&cpp)
        .unwrap_or_else(|e| panic!("could not read {cpp:?} (submodule not checked out?): {e}"));
    assert!(
        text.contains("encoder ready {}x{} @{}Hz {}Mbps ({}, {})"),
        "the oxrsys spdlog format string changed — update parse_encoder_ready to match ({cpp:?})"
    );
}

mod snapshot_tests {
    use super::super::*;
    use crate::process::ProcInfo;
    use crate::session::{
        clear_live_session, lock_session_globals, publish_run_phase, set_live_session,
        LiveSessionHandle, RunPhaseInfo, LIVE_SESSION,
    };
    use std::path::{Path, PathBuf};
    use tokio_util::sync::CancellationToken;
    use uuid::Uuid;

    fn scratch(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sabrage-watcher-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn fixture_paths(root: &Path) -> Paths {
        let mut paths = Paths::new(root);
        paths.oxr_appsup = root.join("home/Library/Application Support/OXRSys");
        paths.sabrage_appsup = root.join("home/Library/Application Support/Sabrage");
        paths
    }

    /// Best-effort reset of the process-global live-session and run-phase
    /// slots before a test that must start from Idle. Cannot rule out a
    /// concurrent test in another module touching the same globals.
    fn force_idle() {
        if let Ok(mut g) = LIVE_SESSION.lock() {
            *g = None;
        }
        publish_run_phase(None);
    }

    /// A live handle for `run_id`, whose recorded identity is this test
    /// process — i.e. one `snapshot()` will read as still alive.
    fn live(run_id: uuid::Uuid) {
        set_live_session(LiveSessionHandle {
            run_id,
            bottle: "LiveBottle".into(),
            identity: ProcInfo::observe(std::process::id()).unwrap(),
            log_path: PathBuf::from("/repo/logs/live.log"),
            started_at_unix_ms: crate::session::now_unix_ms(),
            cancel: CancellationToken::new(),
            detach: CancellationToken::new(),
        });
    }

    /// Write a `session-state.json` under `paths` whose recorded wine pid
    /// is either this process (alive -> `Running`) or the pid that cannot
    /// exist (dead -> `Exited`).
    fn persisted(paths: &Paths, alive: bool) {
        std::fs::create_dir_all(&paths.sabrage_appsup).unwrap();
        let (pid, start) = if alive {
            let me = ProcInfo::observe(std::process::id()).unwrap();
            (me.pid, me.start_time)
        } else {
            (u32::MAX - 1, 1)
        };
        std::fs::write(
            paths.session_state_path(),
            format!(
                r#"{{"version":1,"runId":"00000000-0000-0000-0000-000000000000",
                        "bottle":"PersistedBottle","bsDir":"/games/bs","startedAtUnixMs":0,
                        "logPath":"/repo/logs/x.log",
                        "wine":{{"pid":{pid},"startTime":{start},"exe":""}}}}"#
            ),
        )
        .unwrap();
    }

    fn publish(phase: SessionPhase, run_id: uuid::Uuid, exit_code: Option<i32>) {
        publish_run_phase(Some(RunPhaseInfo {
            phase,
            run_id,
            bottle: "PublishedBottle".into(),
            exit_code,
        }));
    }

    /// A9-6. The runtime log is global and append-only across sessions and a
    /// new monitor preloads its last 200 lines, so an `encoder ready` line
    /// from an earlier session would show a healthy chip where "waiting for
    /// encoder…" belongs — and, since oxrsys emits that line once per
    /// session, hide an `(H.264, in-process)` downgrade for the whole run.
    #[tokio::test]
    async fn a_previous_sessions_encoder_line_is_never_published_for_a_new_run() {
        let _g = lock_session_globals();
        force_idle();

        let dir = scratch("encoder-previous-session");
        let paths = fixture_paths(&dir);
        std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
        let log_path = paths.oxr_appsup.join("oxrsys-runtime.log");
        // Yesterday's session, still in the file.
        std::fs::write(
            &log_path,
            "[2026-08-28 21:00:00.000] [info] OXRSys/ALVR: encoder ready 3008x1664 @72Hz \
                 80Mbps (HEVC, native helper)\n",
        )
        .unwrap();

        let mut m = SessionMonitor::new(paths);

        // A session that starts *after* this monitor: its own encoder line
        // has not been written yet.
        let run_id = Uuid::new_v4();
        set_live_session(LiveSessionHandle {
            run_id,
            bottle: "Steam".into(),
            identity: ProcInfo::observe(std::process::id()).unwrap(),
            log_path: PathBuf::from("/repo/logs/x.log"),
            started_at_unix_ms: crate::session::now_unix_ms() + 1,
            cancel: CancellationToken::new(),
            detach: CancellationToken::new(),
        });

        let s = m.snapshot().await;
        assert_eq!(s.phase, SessionPhase::Running);
        assert!(
            s.encoder.is_none(),
            "yesterday's chip must not be this session's: {:?}",
            s.encoder
        );

        // …and this session's own line, appended while it runs, IS the chip.
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&log_path)
            .unwrap();
        writeln!(
            f,
            "[2026-08-29 10:00:00.000] [info] OXRSys/ALVR: encoder ready 3008x1664 @72Hz \
                 80Mbps (H.264, in-process)"
        )
        .unwrap();
        let s = m.snapshot().await;
        let enc = s.encoder.expect("the running session's own line");
        assert_eq!(
            (enc.codec.as_str(), enc.path.as_str()),
            ("H.264", "in-process")
        );

        // A different run inherits nothing, even without an Idle edge in
        // between: the chip names the run it was parsed for.
        clear_live_session(run_id);
        let next = Uuid::new_v4();
        publish(SessionPhase::Launching, next, None);
        let s = m.snapshot().await;
        publish_run_phase(None);
        assert_eq!(s.phase, SessionPhase::Launching);
        assert!(s.encoder.is_none(), "a chip never crosses runs");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// One oxrsys log line, timestamped in the machine's local time the
    /// way spdlog writes it (`[%Y-%m-%d %H:%M:%S.%e] [%l] %v`).
    fn log_line(at_unix_ms: u64, message: &str) -> String {
        use chrono::TimeZone;
        let at = chrono::Local
            .timestamp_millis_opt(at_unix_ms as i64)
            .single()
            .expect("a representable local time");
        format!(
            "[{}] [info] {message}\n",
            at.format("%Y-%m-%d %H:%M:%S%.3f")
        )
    }

    /// A9-6, both halves. An adopted session — one that started before the
    /// monitor — believes a preloaded line stamped after it started and
    /// publishes the chip it names; a line stamped before it belongs to a
    /// previous session, and publishing that one shows a healthy `(HEVC,
    /// native helper)` chip where "waiting for encoder…" belongs.
    #[tokio::test]
    async fn an_adopted_session_only_inherits_lines_written_after_it_started() {
        let _g = lock_session_globals();

        for (row, line, want_codec) in [
                (
                    "r1:A9-6 regression: a line from before this session started is never published as this session's chip",
                    log_line(
                        crate::session::now_unix_ms() - 3_600_000,
                        "OXRSys/ALVR: encoder ready 3008x1664 @72Hz 80Mbps (HEVC, native helper)",
                    ),
                    None,
                ),
                (
                    "a line this session wrote before the monitor opened",
                    log_line(
                        crate::session::now_unix_ms() - 4_000,
                        "OXRSys/ALVR: encoder ready 3008x1664 @72Hz 80Mbps (H.264, in-process)",
                    ),
                    Some("H.264"),
                ),
                (
                    "the fix must not over-correct: a session that predates the monitor keeps the chip it negotiated",
                    log_line(
                        crate::session::now_unix_ms() - 4_000,
                        "OXRSys/ALVR: encoder ready 3008x1664 @72Hz 80Mbps (HEVC, native helper)",
                    ),
                    Some("HEVC"),
                ),
                (
                    "an undated line proves nothing",
                    "OXRSys/ALVR: encoder ready 3008x1664 @72Hz 80Mbps (HEVC, native helper)\n"
                        .to_string(),
                    None,
                ),
            ] {
                force_idle();
                let dir = scratch("encoder-adopted-window");
                let paths = fixture_paths(&dir);
                std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
                std::fs::write(paths.oxr_appsup.join("oxrsys-runtime.log"), &line).unwrap();

                let run_id = Uuid::new_v4();
                set_live_session(LiveSessionHandle {
                    run_id,
                    bottle: "Steam".into(),
                    identity: ProcInfo::observe(std::process::id()).unwrap(),
                    log_path: PathBuf::from("/repo/logs/x.log"),
                    started_at_unix_ms: crate::session::now_unix_ms() - 5_000,
                    cancel: CancellationToken::new(),
                    detach: CancellationToken::new(),
                });
                let mut m = SessionMonitor::new(paths);
                let s = m.snapshot().await;
                clear_live_session(run_id);

                assert_eq!(s.phase, SessionPhase::Running, "{row}");
                assert_eq!(s.encoder.map(|e| e.codec).as_deref(), want_codec, "{row}");
                std::fs::remove_dir_all(&dir).ok();
            }
    }

    /// A8-5. `demo.sh run` publishes no handle and writes no
    /// `session-state.json`, but the runtime it launched reports in every
    /// second. Reporting that as Idle ("No session running") is how a user
    /// launches a second game over a live one.
    #[tokio::test]
    async fn a_session_started_outside_sabrage_is_reported_not_called_idle() {
        let _g = lock_session_globals();
        force_idle();
        let now = crate::session::now_unix_ms();

        // Fresh status naming a process that is genuinely alive.
        {
            let dir = scratch("external-live");
            let paths = fixture_paths(&dir);
            std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
            std::fs::write(
                paths.oxr_appsup.join("runtime_status.json"),
                format!(
                    r#"{{"state":"streaming","process_id":{},"updated_at_unix_ms":{now}}}"#,
                    std::process::id()
                ),
            )
            .unwrap();

            let mut m = SessionMonitor::new(paths);
            let s = m.snapshot().await;
            assert_eq!(s.phase, SessionPhase::External);
            assert_eq!(s.pid, Some(std::process::id()));
            assert!(!s.owned_by_this_process, "it is not ours");
            assert!(s.run_id.is_none() && s.bottle.is_none());
            assert!(s.runtime_fresh);
            std::fs::remove_dir_all(&dir).ok();
        }

        // Never from freshness alone: the pid has to answer too.
        {
            let dir = scratch("external-dead-pid");
            let paths = fixture_paths(&dir);
            std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
            std::fs::write(
                paths.oxr_appsup.join("runtime_status.json"),
                format!(
                    r#"{{"state":"streaming","process_id":4294967294,"updated_at_unix_ms":{now}}}"#
                ),
            )
            .unwrap();
            let mut m = SessionMonitor::new(paths);
            assert_eq!(m.snapshot().await.phase, SessionPhase::Idle);
            std::fs::remove_dir_all(&dir).ok();
        }

        // …nor from a stale file, however alive the pid it names.
        {
            let dir = scratch("external-stale");
            let paths = fixture_paths(&dir);
            std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
            std::fs::write(
                paths.oxr_appsup.join("runtime_status.json"),
                format!(
                    r#"{{"state":"streaming","process_id":{},"updated_at_unix_ms":{}}}"#,
                    std::process::id(),
                    now - 60_000
                ),
            )
            .unwrap();
            let mut m = SessionMonitor::new(paths);
            assert_eq!(m.snapshot().await.phase, SessionPhase::Idle);
            std::fs::remove_dir_all(&dir).ok();
        }

        // Our own launch still outranks it: Preflight is the truth about
        // what this Sabrage is doing.
        {
            let dir = scratch("external-vs-preflight");
            let paths = fixture_paths(&dir);
            std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
            std::fs::write(
                paths.oxr_appsup.join("runtime_status.json"),
                format!(
                    r#"{{"state":"streaming","process_id":{},"updated_at_unix_ms":{now}}}"#,
                    std::process::id()
                ),
            )
            .unwrap();
            publish(SessionPhase::Preflight, Uuid::new_v4(), None);
            let mut m = SessionMonitor::new(paths);
            let s = m.snapshot().await;
            publish_run_phase(None);
            assert_eq!(s.phase, SessionPhase::Preflight);
            std::fs::remove_dir_all(&dir).ok();
        }
    }

    /// A13a-2, rendered. The door counts a running `Beat Saber.exe` as a live
    /// session for the window a `./demo.sh run` spends between its wine spawn
    /// and its first `runtime_status.json`. The phase carries it too:
    /// reporting `Idle` there leaves Launch and every Doctor Fix enabled, and
    /// each one then dies with the Fatal the door raises.
    #[tokio::test]
    async fn a_running_game_with_nothing_on_disk_is_external_not_idle() {
        let _g = lock_session_globals();
        force_idle();

        let dir = scratch("external-running-game");
        let paths = fixture_paths(&dir);
        std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
        let mut m = SessionMonitor::new(paths);
        assert_eq!(
            m.snapshot().await.phase,
            SessionPhase::Idle,
            "nothing on disk, no game: idle"
        );

        // Stand in for the wine child, argv exactly as wine spells it —
        // the shape `pgrep -f 'Beat Saber.exe'` and the door both match.
        let mut game = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg("sleep 20 # Z:\\games\\Beat Saber 1294\\Beat Saber.exe")
            .spawn()
            .expect("/bin/sh is on every macOS");

        // The process table is refreshed per call; give the spawn a moment
        // to appear rather than assuming it already has.
        let mut seen = SessionStatus::default();
        for _ in 0..50 {
            seen = m.snapshot().await;
            if seen.phase != SessionPhase::Idle {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let game_pid = game.id();
        let _ = game.kill();
        let _ = game.wait();

        assert_eq!(
            seen.phase,
            SessionPhase::External,
            "a running game the door refuses over must not render as Idle"
        );
        assert_eq!(seen.pid, Some(game_pid));
        assert!(!seen.owned_by_this_process, "it is not ours");
        assert!(seen.run_id.is_none() && seen.bottle.is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A9-7. The monitor is built once and outlives every session it
    /// watches, so the freshness history it accumulates has to name the
    /// run it belongs to. Session A's `ever_fresh` + last-fresh timestamp
    /// classified session B as `Stalled` the moment B passed the startup
    /// grace — the standby freeze reported for a launch that had never
    /// reported in at all.
    #[tokio::test]
    async fn freshness_history_never_crosses_from_one_session_to_the_next() {
        let _g = lock_session_globals();
        force_idle();

        let dir = scratch("stall-history");
        let paths = fixture_paths(&dir);
        std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
        let now = crate::session::now_unix_ms();
        // The `streaming` status session A left behind, long stale.
        std::fs::write(
            paths.oxr_appsup.join("runtime_status.json"),
            format!(
                r#"{{"state":"streaming","process_id":{},"updated_at_unix_ms":{}}}"#,
                std::process::id(),
                now - 60_000
            ),
        )
        .unwrap();

        // Session B: ours, running, well past the startup grace.
        let run_b = Uuid::new_v4();
        set_live_session(LiveSessionHandle {
            run_id: run_b,
            bottle: "Steam".into(),
            identity: ProcInfo::observe(std::process::id()).unwrap(),
            log_path: PathBuf::from("/repo/logs/x.log"),
            started_at_unix_ms: now - 60_000,
            cancel: CancellationToken::new(),
            detach: CancellationToken::new(),
        });

        // The history is session A's: not evidence about B.
        let mut m = SessionMonitor::new(paths.clone());
        m.ever_fresh = true;
        m.last_fresh_unix_ms = Some(now - 30_000);
        m.fresh_run_id = Some(Uuid::new_v4());
        let s = m.snapshot().await;
        assert_eq!(
            s.phase,
            SessionPhase::Running,
            "session B has never reported in; A's timestamps cannot stall it"
        );
        assert!(!s.runtime_fresh);

        // …and the same history, recorded for THIS run, still stalls it —
        // the reset must not disable stall detection.
        let mut m = SessionMonitor::new(paths);
        m.ever_fresh = true;
        m.last_fresh_unix_ms = Some(now - 30_000);
        m.fresh_run_id = Some(run_b);
        let s = m.snapshot().await;
        clear_live_session(run_b);
        assert_eq!(s.phase, SessionPhase::Stalled);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A9-5. The spawn fallback records `start_time: 0` when the child could
    /// not be observed (`Executor::spawn_detached`), so `is_same_process()`
    /// is false for it forever after. Reconciliation calls that alive pid
    /// `Unverifiable` and every door treats it as live; reporting `Exited`
    /// would put a Launch button over a session `run` refuses.
    #[tokio::test]
    async fn an_alive_pid_with_no_verifiable_start_time_is_never_reported_exited() {
        let _g = lock_session_globals();
        let unverifiable = ProcInfo {
            pid: std::process::id(),
            start_time: 0,
            exe: PathBuf::new(),
        };
        assert_eq!(
            crate::session::reconcile::classify_identity(Some(&unverifiable)),
            crate::session::reconcile::Classification::Unverifiable,
            "the premise: alive, and nothing about it can be checked"
        );

        // …as a live handle.
        {
            force_idle();
            let dir = scratch("unverifiable-handle");
            let paths = fixture_paths(&dir);
            std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
            let run_id = Uuid::new_v4();
            set_live_session(LiveSessionHandle {
                run_id,
                bottle: "Steam".into(),
                identity: unverifiable.clone(),
                log_path: PathBuf::from("/repo/logs/x.log"),
                started_at_unix_ms: crate::session::now_unix_ms(),
                cancel: CancellationToken::new(),
                detach: CancellationToken::new(),
            });
            let mut m = SessionMonitor::new(paths);
            let s = m.snapshot().await;
            clear_live_session(run_id);
            assert_eq!(s.phase, SessionPhase::Running);
            std::fs::remove_dir_all(&dir).ok();
        }

        // …and as a persisted record, which is the shape a Sabrage that
        // reopens onto that session reads.
        {
            force_idle();
            let dir = scratch("unverifiable-record");
            let paths = fixture_paths(&dir);
            std::fs::create_dir_all(&paths.sabrage_appsup).unwrap();
            std::fs::write(
                paths.session_state_path(),
                format!(
                    r#"{{"version":1,"runId":"00000000-0000-0000-0000-000000000000",
                            "bottle":"Steam","bsDir":"/games/bs","startedAtUnixMs":0,
                            "logPath":"/repo/logs/x.log",
                            "wine":{{"pid":{},"startTime":0,"exe":""}}}}"#,
                    std::process::id()
                ),
            )
            .unwrap();
            let mut m = SessionMonitor::new(paths.clone());
            let s = m.snapshot().await;
            assert_eq!(s.phase, SessionPhase::Running);
            assert!(
                crate::session::session_block_at(
                    &paths.session_state_path(),
                    &paths.oxr_appsup.join("runtime_status.json"),
                )
                .is_some(),
                "the door and the phase have to agree about this record"
            );
            std::fs::remove_dir_all(&dir).ok();
        }
    }

    /// A9-7. `runtime_status.json` is one global file that outlives every
    /// session: a stamp written *before* this session started describes the
    /// runtime that is gone, and believing it hides a stall in the one that
    /// is here.
    #[tokio::test]
    async fn a_status_written_before_this_session_started_is_not_fresh() {
        let _g = lock_session_globals();
        force_idle();
        let now = crate::session::now_unix_ms();

        let dir = scratch("status-predates-session");
        let paths = fixture_paths(&dir);
        std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
        // Written a second ago — recent by the clock, but before this
        // session began.
        std::fs::write(
            paths.oxr_appsup.join("runtime_status.json"),
            format!(
                r#"{{"state":"streaming","updated_at_unix_ms":{}}}"#,
                now - 1_000
            ),
        )
        .unwrap();

        let run_id = Uuid::new_v4();
        set_live_session(LiveSessionHandle {
            run_id,
            bottle: "Steam".into(),
            identity: ProcInfo::observe(std::process::id()).unwrap(),
            log_path: PathBuf::from("/repo/logs/x.log"),
            started_at_unix_ms: now,
            cancel: CancellationToken::new(),
            detach: CancellationToken::new(),
        });
        let mut m = SessionMonitor::new(paths);
        let s = m.snapshot().await;
        clear_live_session(run_id);

        assert_eq!(s.phase, SessionPhase::Running);
        assert!(
            !s.runtime_fresh,
            "the previous runtime's last word is not this session's heartbeat"
        );
        assert_eq!(
            s.runtime_state.as_deref(),
            Some("streaming"),
            "the state is still shown — it is the freshness that is withheld"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// #2/#100: the phase precedence [`SessionMonitor::snapshot`] implements
    /// — every row where two sources disagree, plus the live-only and
    /// persisted-only baselines those conflicts are measured against.
    #[tokio::test]
    async fn snapshot_phase_precedence_table() {
        let _g = lock_session_globals();
        force_idle();

        // Each row: (what is live, what is persisted, what is published)
        // -> the phase the snapshot must report.
        for (row, live_run, persist, published, want) in [
            (
                "published Stopping beats a live handle",
                true,
                None,
                Some(SessionPhase::Stopping),
                SessionPhase::Stopping,
            ),
            (
                "a live handle beats published Preflight",
                true,
                None,
                Some(SessionPhase::Preflight),
                SessionPhase::Running,
            ),
            (
                "a live handle beats published Launching",
                true,
                None,
                Some(SessionPhase::Launching),
                SessionPhase::Running,
            ),
            (
                "a live handle beats published Exited",
                true,
                None,
                Some(SessionPhase::Exited),
                SessionPhase::Running,
            ),
            (
                "a live handle alone is Running",
                true,
                None,
                None,
                SessionPhase::Running,
            ),
            (
                "published Launching beats persisted state",
                false,
                Some(true),
                Some(SessionPhase::Launching),
                SessionPhase::Launching,
            ),
            (
                "published Stopping beats persisted state",
                false,
                Some(true),
                Some(SessionPhase::Stopping),
                SessionPhase::Stopping,
            ),
            (
                "persisted state beats published Exited",
                false,
                Some(true),
                Some(SessionPhase::Exited),
                SessionPhase::Running,
            ),
            (
                "persisted state alone",
                false,
                Some(false),
                None,
                SessionPhase::Exited,
            ),
        ] {
            let dir = scratch(&format!("prec-{}", want as u8));
            let paths = fixture_paths(&dir);
            std::fs::create_dir_all(&paths.oxr_appsup).unwrap();

            publish_run_phase(None);
            let run_id = Uuid::new_v4();
            if live_run {
                live(run_id);
            }
            if let Some(alive) = persist {
                persisted(&paths, alive);
            }
            if let Some(phase) = published {
                publish(phase, run_id, Some(7));
            }

            let mut m = SessionMonitor::new(paths);
            let got = m.snapshot().await;
            // Run-id guarded, so this row cannot blank a handle some
            // other module's test legitimately owns right now.
            clear_live_session(run_id);
            publish_run_phase(None);

            assert_eq!(got.phase, want, "{row}");
            assert!(got.bottle.is_some(), "{row}");
            std::fs::remove_dir_all(&dir).ok();
        }
    }

    /// The identity a snapshot reports comes from the strongest source
    /// that has one — and a published `Exited`'s code rides along even
    /// when the phase itself was derived from `session-state.json`.
    #[tokio::test]
    async fn snapshot_identity_and_exit_code_sources() {
        let _g = lock_session_globals();
        force_idle();

        // Published only: identity comes from the publication (#100 —
        // without it the Session screen's Stop has no bottle).
        {
            let dir = scratch("ident-published");
            let paths = fixture_paths(&dir);
            std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
            publish_run_phase(None);
            let run_id = Uuid::new_v4();
            publish(SessionPhase::Launching, run_id, None);
            let mut m = SessionMonitor::new(paths);
            let s = m.snapshot().await;
            publish_run_phase(None);
            assert_eq!(s.phase, SessionPhase::Launching);
            assert_eq!(s.run_id, Some(run_id));
            assert_eq!(s.bottle.as_deref(), Some("PublishedBottle"));
            assert!(s.exit_code.is_none(), "only Exited carries a code");
            std::fs::remove_dir_all(&dir).ok();
        }

        // Published Exited with nothing else: the code is reported (#7).
        {
            let dir = scratch("ident-exited");
            let paths = fixture_paths(&dir);
            std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
            publish_run_phase(None);
            let run_id = Uuid::new_v4();
            publish(SessionPhase::Exited, run_id, Some(139));
            let mut m = SessionMonitor::new(paths);
            let s = m.snapshot().await;
            publish_run_phase(None);
            assert_eq!(s.phase, SessionPhase::Exited);
            assert_eq!(s.exit_code, Some(139));
            assert_eq!(s.bottle.as_deref(), Some("PublishedBottle"));
            std::fs::remove_dir_all(&dir).ok();
        }

        // Derived Exited (a dead pid on disk) + a published Exited: the
        // identity is the state file's, the code is the publication's.
        {
            let dir = scratch("ident-both-exited");
            let paths = fixture_paths(&dir);
            std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
            publish_run_phase(None);
            persisted(&paths, false);
            publish(SessionPhase::Exited, Uuid::new_v4(), Some(3));
            let mut m = SessionMonitor::new(paths);
            let s = m.snapshot().await;
            publish_run_phase(None);
            assert_eq!(s.phase, SessionPhase::Exited);
            assert_eq!(
                s.bottle.as_deref(),
                Some("PersistedBottle"),
                "the state file knows more than a published phase does"
            );
            assert_eq!(
                s.exit_code,
                Some(3),
                "the status line and the number it names must agree"
            );
            std::fs::remove_dir_all(&dir).ok();
        }

        // #200/#201: stale persisted state for run A (detached, still
        // "alive") while THIS process is in preflight for run B. The
        // phase is B's, so the identity must be B's too — otherwise the
        // Session screen routes Stop to `cancelStage(A)`, which the
        // RunRegistry has never heard of, and the launch of B carries on
        // behind a dead button. A's pid/log/started/detached go with it.
        {
            let dir = scratch("ident-stale-persisted-vs-preflight");
            let paths = fixture_paths(&dir);
            std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
            publish_run_phase(None);
            std::fs::create_dir_all(&paths.sabrage_appsup).unwrap();
            let me = ProcInfo::observe(std::process::id()).unwrap();
            std::fs::write(
                paths.session_state_path(),
                format!(
                    r#"{{"version":1,"runId":"11111111-1111-1111-1111-111111111111",
                            "bottle":"StaleBottleA","bsDir":"/games/bs","startedAtUnixMs":42,
                            "logPath":"/repo/logs/a.log",
                            "wine":{{"pid":{},"startTime":{},"exe":""}},"detached":true}}"#,
                    me.pid, me.start_time
                ),
            )
            .unwrap();

            let run_b = Uuid::new_v4();
            publish(SessionPhase::Preflight, run_b, None);
            let mut m = SessionMonitor::new(paths);
            let s = m.snapshot().await;
            publish_run_phase(None);

            assert_eq!(s.phase, SessionPhase::Preflight);
            assert_eq!(s.run_id, Some(run_b), "#200: the phase names its own run");
            assert_eq!(
                s.bottle.as_deref(),
                Some("PublishedBottle"),
                "#200: and its own bottle, not the stale one on disk"
            );
            assert!(s.pid.is_none(), "#200: A's pid must not ride under B");
            assert!(s.log_path.is_none(), "#200: nor A's log");
            assert!(s.started_at_unix_ms.is_none(), "#200: nor A's start time");
            assert!(!s.detached, "#200: B's preflight is not detached");
            assert!(
                s.owned_by_this_process,
                "#201: RUN_PHASE is in-process — a winning publication is ours"
            );
            std::fs::remove_dir_all(&dir).ok();
        }

        // #201: the same ownership claim with nothing on disk at all —
        // the ordinary launch window, where `LIVE_SESSION` is not
        // populated yet.
        {
            let dir = scratch("ident-owned-during-launch");
            let paths = fixture_paths(&dir);
            std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
            publish_run_phase(None);
            publish(SessionPhase::Launching, Uuid::new_v4(), None);
            let mut m = SessionMonitor::new(paths);
            let s = m.snapshot().await;
            publish_run_phase(None);
            assert_eq!(s.phase, SessionPhase::Launching);
            assert!(
                s.owned_by_this_process,
                "#201: no 'running outside this Sabrage instance' during our own launch"
            );
            std::fs::remove_dir_all(&dir).ok();
        }

        // A published Stopping over a live handle keeps the live
        // handle's identity — pid and log path included.
        {
            let dir = scratch("ident-stopping");
            let paths = fixture_paths(&dir);
            std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
            publish_run_phase(None);
            let run_id = Uuid::new_v4();
            live(run_id);
            publish(SessionPhase::Stopping, run_id, None);
            let mut m = SessionMonitor::new(paths);
            let s = m.snapshot().await;
            clear_live_session(run_id);
            publish_run_phase(None);
            assert_eq!(s.phase, SessionPhase::Stopping);
            assert_eq!(s.bottle.as_deref(), Some("LiveBottle"));
            assert!(s.pid.is_some() && s.owned_by_this_process);
            std::fs::remove_dir_all(&dir).ok();
        }
    }

    /// The phase transitions, in strict sequence inside one test rather than
    /// separate `#[tokio::test]`s: they read the process-global
    /// `LIVE_SESSION` slot and would race on separate threads. The residual
    /// cross-file risk is the caveat `session::mod`'s tests carry.
    #[tokio::test]
    async fn snapshot_phase_transitions() {
        let _g = lock_session_globals();
        force_idle();

        // Idle: nothing live, nothing persisted.
        {
            let dir = scratch("idle");
            let mut m = SessionMonitor::new(fixture_paths(&dir));
            let s = m.snapshot().await;
            assert_eq!(s.phase, SessionPhase::Idle);
            assert!(s.run_id.is_none() && s.pid.is_none());
        }

        // Detached: persisted state says so, and the recorded wine pid is
        // still alive (this test process, observed for real).
        {
            let dir = scratch("detached");
            let paths = fixture_paths(&dir);
            std::fs::create_dir_all(&paths.sabrage_appsup).unwrap();
            std::fs::create_dir_all(&paths.oxr_appsup).unwrap();

            let me = ProcInfo::observe(std::process::id()).unwrap();
            let json = format!(
                r#"{{"version":1,"runId":"00000000-0000-0000-0000-000000000000","bottle":"Steam","bsDir":"/games/bs","startedAtUnixMs":0,"logPath":"/repo/logs/x.log","wine":{{"pid":{},"startTime":{},"exe":"{}"}},"detached":true}}"#,
                me.pid,
                me.start_time,
                me.exe.display()
            );
            std::fs::write(paths.session_state_path(), json).unwrap();

            let mut m = SessionMonitor::new(paths);
            let s = m.snapshot().await;

            assert_eq!(s.phase, SessionPhase::Detached);
            assert!(!s.owned_by_this_process);
            assert!(
                s.detached,
                "F3: status.detached must mirror the persisted flag"
            );
        }

        // Exited: persisted state's recorded wine pid is dead.
        {
            let dir = scratch("exited");
            let paths = fixture_paths(&dir);
            std::fs::create_dir_all(&paths.sabrage_appsup).unwrap();
            std::fs::create_dir_all(&paths.oxr_appsup).unwrap();

            // u32::MAX - 1: the "cannot exist for this user" pid, same
            // idiom process.rs's own tests use.
            let json = r#"{"version":1,"runId":"00000000-0000-0000-0000-000000000000","bottle":"Steam","bsDir":"/games/bs","startedAtUnixMs":0,"logPath":"/repo/logs/x.log","wine":{"pid":4294967294,"startTime":1,"exe":""}}"#;
            std::fs::write(paths.session_state_path(), json).unwrap();

            let mut m = SessionMonitor::new(paths);
            let s = m.snapshot().await;

            assert_eq!(s.phase, SessionPhase::Exited);
            assert!(
                !s.detached,
                "no `detached` key in the fixture — must default false"
            );
        }

        // F2: the run stage's published phase wins over whatever would
        // otherwise be derived — here plain Idle — for the three phases only
        // it can know about. #100: it must also publish the identity, or the
        // Session screen offers a Stop button with no bottle to stop.
        for phase in [
            SessionPhase::Preflight,
            SessionPhase::Launching,
            SessionPhase::Stopping,
        ] {
            let dir = scratch(&format!("run-phase-{phase:?}"));
            let run_id = Uuid::new_v4();
            publish_run_phase(Some(RunPhaseInfo {
                phase,
                run_id,
                bottle: "Steam".into(),
                exit_code: None,
            }));
            let mut m = SessionMonitor::new(fixture_paths(&dir));
            let s = m.snapshot().await;
            publish_run_phase(None);
            assert_eq!(s.phase, phase, "run_phase() must override the derived Idle");
            assert_eq!(s.run_id, Some(run_id), "#100: the phase names its run");
            assert_eq!(
                s.bottle.as_deref(),
                Some("Steam"),
                "#100: the phase names its bottle — Stop needs it"
            );
        }

        // F16: the encoder chip must clear on the edge into Idle/Exited
        // rather than linger as a false-healthy chip for a session that no
        // longer exists. One monitor throughout, so `last_phase` really
        // transitions out of Running within one instance.
        {
            let dir = scratch("encoder-clears-on-exit");
            let paths = fixture_paths(&dir);
            std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
            std::fs::create_dir_all(&paths.sabrage_appsup).unwrap();
            let log_path = paths.oxr_appsup.join("oxrsys-runtime.log");
            let started = crate::session::now_unix_ms() - 1_000;
            // Timestamped, and after this session started: a preloaded
            // line has to prove whose it is (A9-6).
            std::fs::write(
                &log_path,
                log_line(
                    started,
                    "OXRSys/ALVR: encoder ready 3008x1664 @72Hz 80Mbps (HEVC, native helper)",
                ),
            )
            .unwrap();

            let run_id = Uuid::new_v4();
            set_live_session(LiveSessionHandle {
                run_id,
                bottle: "Steam".into(),
                identity: ProcInfo::observe(std::process::id()).unwrap(),
                log_path: PathBuf::from("/repo/logs/x.log"),
                started_at_unix_ms: started,
                cancel: CancellationToken::new(),
                detach: CancellationToken::new(),
            });

            let mut m = SessionMonitor::new(paths);
            let s_running = m.snapshot().await;
            assert_eq!(s_running.phase, SessionPhase::Running);
            assert!(
                s_running.encoder.is_some(),
                "the chip must populate while the session is live"
            );

            // The session ends: no live handle, no persisted state either
            // (this scenario never wrote `session-state.json`) — the
            // derived phase falls all the way back to Idle.
            clear_live_session(run_id);
            let s_idle = m.snapshot().await;
            assert_eq!(s_idle.phase, SessionPhase::Idle);
            assert!(
                s_idle.encoder.is_none(),
                "F16: the previous session's encoder chip must not survive into Idle"
            );
        }

        // Encoder chip: picked up from a fresh line appended after the monitor
        // already exists — Idle phase throughout, no live session.
        {
            let dir = scratch("encoder-chip");
            let paths = fixture_paths(&dir);
            std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
            let log_path = paths.oxr_appsup.join("oxrsys-runtime.log");
            std::fs::write(&log_path, b"").unwrap();

            let mut m = SessionMonitor::new(paths);
            let s0 = m.snapshot().await;
            assert!(s0.encoder.is_none());

            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&log_path)
                .unwrap();
            writeln!(
                    f,
                    "[2026-08-29 10:00:00.000] [info] OXRSys/ALVR: encoder ready 3008x1664 @72Hz 80Mbps (HEVC, native helper)"
                )
                .unwrap();

            let s1 = m.snapshot().await;
            let enc = s1.encoder.expect("encoder chip should now be set");
            assert_eq!(enc.codec, "HEVC");
            assert_eq!(enc.path, "native helper");
            assert_eq!(
                (enc.width, enc.height, enc.refresh_hz, enc.bitrate_mbps),
                (3008, 1664, 72, 80)
            );
        }

        let now = crate::session::now_unix_ms();

        // Running + fresh, via the live session.
        {
            let dir = scratch("live-running");
            let paths = fixture_paths(&dir);
            std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
            std::fs::write(
                paths.oxr_appsup.join("runtime_status.json"),
                format!(r#"{{"state":"streaming","updated_at_unix_ms":{now}}}"#),
            )
            .unwrap();
            let run_id = Uuid::new_v4();
            set_live_session(LiveSessionHandle {
                run_id,
                bottle: "Steam".into(),
                identity: ProcInfo::observe(std::process::id()).unwrap(),
                log_path: PathBuf::from("/repo/logs/x.log"),
                started_at_unix_ms: now,
                cancel: CancellationToken::new(),
                detach: CancellationToken::new(),
            });

            let mut m = SessionMonitor::new(paths);
            let s = m.snapshot().await;
            clear_live_session(run_id);

            assert_eq!(s.phase, SessionPhase::Running);
            assert!(s.owned_by_this_process);
            assert_eq!(s.run_id, Some(run_id));
            assert!(s.runtime_fresh);
            assert_eq!(s.runtime_state.as_deref(), Some("streaming"));
        }

        // Stalled: past the startup grace, the *streaming* heartbeat stale
        // for longer than the stall grace (simulated directly — no real
        // 30s+10s sleep). The file's last state must be `streaming`: that
        // is the only state oxrsys heartbeats.
        {
            let dir = scratch("live-stalled");
            let paths = fixture_paths(&dir);
            std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
            std::fs::write(
                paths.oxr_appsup.join("runtime_status.json"),
                format!(
                    r#"{{"state":"streaming","updated_at_unix_ms":{}}}"#,
                    now - 20_000
                ),
            )
            .unwrap();
            let run_id = Uuid::new_v4();
            set_live_session(LiveSessionHandle {
                run_id,
                bottle: "Steam".into(),
                identity: ProcInfo::observe(std::process::id()).unwrap(),
                log_path: PathBuf::from("/repo/logs/x.log"),
                started_at_unix_ms: now - 120_000,
                cancel: CancellationToken::new(),
                detach: CancellationToken::new(),
            });

            let mut m = SessionMonitor::new(paths);
            m.ever_fresh = true;
            m.last_fresh_unix_ms = Some(now - 20_000);
            // …recorded for THIS run: freshness history that names another
            // run is not evidence about this one (A9-7).
            m.fresh_run_id = Some(run_id);
            let s = m.snapshot().await;
            clear_live_session(run_id);

            assert_eq!(s.phase, SessionPhase::Stalled);
            assert!(!s.runtime_fresh);
        }

        // Idle runtime waiting for the headset: the file was written once
        // (`SetIdle`) and is now arbitrarily stale, past every grace — oxrsys
        // has no idle heartbeat, so this is Running, never Stalled.
        {
            let dir = scratch("live-idle-waiting");
            let paths = fixture_paths(&dir);
            std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
            std::fs::write(
                paths.oxr_appsup.join("runtime_status.json"),
                format!(
                    r#"{{"state":"idle","updated_at_unix_ms":{}}}"#,
                    now - 120_000
                ),
            )
            .unwrap();
            let run_id = Uuid::new_v4();
            set_live_session(LiveSessionHandle {
                run_id,
                bottle: "Steam".into(),
                identity: ProcInfo::observe(std::process::id()).unwrap(),
                log_path: PathBuf::from("/repo/logs/x.log"),
                started_at_unix_ms: now - 120_000,
                cancel: CancellationToken::new(),
                detach: CancellationToken::new(),
            });

            let mut m = SessionMonitor::new(paths);
            m.ever_fresh = true;
            m.last_fresh_unix_ms = Some(now - 100_000);
            let s = m.snapshot().await;
            clear_live_session(run_id);

            assert_eq!(s.phase, SessionPhase::Running);
            assert!(!s.runtime_fresh);
            assert_eq!(s.runtime_state.as_deref(), Some("idle"));
        }

        // Same staleness, but still inside the startup grace window: must
        // not flag Stalled.
        {
            let dir = scratch("live-grace");
            let paths = fixture_paths(&dir);
            std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
            let run_id = Uuid::new_v4();
            set_live_session(LiveSessionHandle {
                run_id,
                bottle: "Steam".into(),
                identity: ProcInfo::observe(std::process::id()).unwrap(),
                log_path: PathBuf::from("/repo/logs/x.log"),
                started_at_unix_ms: now - 5_000,
                cancel: CancellationToken::new(),
                detach: CancellationToken::new(),
            });

            let mut m = SessionMonitor::new(paths);
            m.ever_fresh = true;
            m.last_fresh_unix_ms = Some(now - 20_000);
            let s = m.snapshot().await;
            clear_live_session(run_id);

            assert_eq!(
                s.phase,
                SessionPhase::Running,
                "still inside the startup grace window"
            );
        }
    }
}
