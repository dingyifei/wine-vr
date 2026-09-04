use super::*;
use crate::events::Severity;
use crate::executor::PlannedKind;
use crate::paths::Paths;
use crate::stages::{StageCtx, StageOptions};
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use tokio_util::sync::CancellationToken;

/// The production probe: real `lsof`, real deadline.
async fn probe_ports(ctx: &StageCtx) -> Option<String> {
    stale_listeners(ctx, Path::new("lsof"), process::DEFAULT_PROBE_TIMEOUT)
        .await
        .expect("not cancelled")
}

#[tokio::test]
async fn stale_listeners_is_well_formed_cmd_pid_pairs_with_a_trailing_space() {
    // Ground truth is machine state (paths.rs's own testing pattern): assert
    // the shape invariant rather than a fixed value.
    let (ctx, _seen) = test_ctx(StageOptions::default());
    let Some(stale) = probe_ports(&ctx).await else {
        return; // lsof did not answer on this machine; covered below
    };
    if stale.is_empty() {
        return;
    }
    assert!(stale.ends_with(' '), "{stale:?} missing trailing space");
    for token in stale.trim_end().split(' ') {
        assert!(
            token.ends_with(')') && token.contains('('),
            "{token:?} is not CMD(PID)"
        );
    }
}

#[tokio::test]
async fn report_ports_matches_a_direct_probe() {
    let (ctx, seen) = test_ctx(StageOptions::default());
    report_ports(&ctx).await;
    let Some(stale) = probe_ports(&ctx).await else {
        return; // lsof did not answer on this machine; covered below
    };
    let evs = seen.lock().unwrap().clone();
    let line = evs.last().expect("one row emitted");
    match (line, stale.is_empty()) {
        (
            crate::events::StageEvent::Line {
                severity: Severity::Ok,
                text,
                step,
                ..
            },
            true,
        ) => {
            assert_eq!(text, "streaming ports free");
            assert_eq!(step.as_deref(), Some(step::STOP_PORTS));
        }
        (
            crate::events::StageEvent::Line {
                severity: Severity::Warn,
                text,
                ..
            },
            false,
        ) => {
            assert_eq!(text, &format!("streaming ports still held by: {stale}"));
        }
        (other, empty) => panic!("unexpected row {other:?} (stale empty: {empty})"),
    }
}

/// An executable that never answers within any test's budget, at a unique
/// scratch path. Returns `(dir, bin)`; the caller removes `dir`.
fn never_answers(tag: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "sabrage-stop-probe-{tag}-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let bin = dir.join("wedged-probe.sh");
    std::fs::write(&bin, "#!/bin/sh\nsleep 300\n").unwrap();
    std::fs::set_permissions(&bin, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    (dir, bin)
}

/// A budget far below [`process::DEFAULT_PROBE_TIMEOUT`] but far above the
/// cost of spawning and killing one `/bin/sh`.
const PROBE_TEST_BUDGET: Duration = Duration::from_secs(5);
const PROBE_TEST_DEADLINE: Duration = Duration::from_millis(300);

/// A cancelled token stops the `lsof` probe instead of holding `stop` — and
/// the process-wide operation lock — until the wedged probe answers.
#[tokio::test]
async fn stale_listeners_honors_an_already_cancelled_token() {
    let (dir, bin) = never_answers("ports-cancel");
    let (ctx, _seen) = test_ctx(StageOptions::default());
    ctx.cancel.cancel();

    let started = tokio::time::Instant::now();
    let err = stale_listeners(&ctx, &bin, process::DEFAULT_PROBE_TIMEOUT)
        .await
        .expect_err("a cancelled probe must not answer with a port list");
    assert!(matches!(err, SabrageError::Cancelled), "{err:?}");
    assert!(started.elapsed() < PROBE_TEST_BUDGET, "the probe ran on");
    std::fs::remove_dir_all(&dir).ok();
}

/// A cancelled ports probe emits no row at all, green or otherwise.
#[tokio::test]
async fn a_cancelled_ports_probe_emits_no_row_at_all() {
    let (dir, bin) = never_answers("ports-cancel-row");
    let (ctx, seen) = test_ctx(StageOptions::default());
    ctx.cancel.cancel();

    report_ports_with(&ctx, &bin, process::DEFAULT_PROBE_TIMEOUT).await;
    assert!(
        rows(&seen).is_empty(),
        "a cancelled probe spoke anyway: {:?}",
        rows(&seen)
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// r2:A5-5 regression: a probe that blows its deadline warns instead of
/// claiming free ports.
#[tokio::test]
async fn a_wedged_lsof_warns_instead_of_reporting_free_ports() {
    let (dir, bin) = never_answers("ports-deadline");
    let (ctx, seen) = test_ctx(StageOptions::default());

    let started = tokio::time::Instant::now();
    report_ports_with(&ctx, &bin, PROBE_TEST_DEADLINE).await;
    assert!(started.elapsed() < PROBE_TEST_BUDGET, "the probe ran on");

    assert_eq!(
        rows(&seen),
        vec![(Severity::Warn, ports_unreadable_warn(PROBE_TEST_DEADLINE))],
        "claimed free ports on the strength of a probe that never answered"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// The audio twin: `SwitchAudioSource` blocked on a degraded CoreAudio
/// server used to fall through `unwrap_or_default()` into the green
/// `"audio output: "` row, naming no device at all.
///
/// r2:A5-5 regression: a probe past its deadline warns instead of naming an
/// empty current device.
#[tokio::test]
async fn a_wedged_switchaudiosource_warns_instead_of_naming_an_empty_device() {
    let (dir, bin) = never_answers("audio-deadline");
    let (ctx, seen) = test_ctx(StageOptions::default());

    let started = tokio::time::Instant::now();
    report_audio_with(&ctx, &bin, PROBE_TEST_DEADLINE).await;
    assert!(started.elapsed() < PROBE_TEST_BUDGET, "the probe ran on");

    assert_eq!(
        rows(&seen),
        vec![(Severity::Warn, audio_unreadable_warn(PROBE_TEST_DEADLINE))],
        "a probe that never answered named the current device"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn current_output_device_honors_an_already_cancelled_token() {
    let (dir, bin) = never_answers("audio-cancel");
    let (ctx, seen) = test_ctx(StageOptions::default());
    ctx.cancel.cancel();

    let started = tokio::time::Instant::now();
    let err = current_output_device(&ctx, &bin, process::DEFAULT_PROBE_TIMEOUT)
        .await
        .expect_err("a cancelled probe must not answer with a device name");
    assert!(matches!(err, SabrageError::Cancelled), "{err:?}");
    assert!(started.elapsed() < PROBE_TEST_BUDGET, "the probe ran on");

    report_audio_with(&ctx, &bin, process::DEFAULT_PROBE_TIMEOUT).await;
    assert!(rows(&seen).is_empty(), "{:?}", rows(&seen));
    std::fs::remove_dir_all(&dir).ok();
}

/// A missing binary is *not* a wedged one: stop.sh's `2>/dev/null` folds a
/// command-not-found into an empty `$STALE`, so the shell's green row still
/// prints. Only the deadline (which the shell cannot reach) gets the warn.
#[tokio::test]
async fn a_missing_lsof_still_reports_the_shells_free_ports_row() {
    let (ctx, seen) = test_ctx(StageOptions::default());
    report_ports_with(
        &ctx,
        Path::new("/nonexistent/sabrage/lsof"),
        process::DEFAULT_PROBE_TIMEOUT,
    )
    .await;
    assert_eq!(
        rows(&seen),
        vec![(Severity::Ok, "streaming ports free".to_string())]
    );
}

/// (label, (pid, exe path) per survivor, expected line).
type SurvivorCase<'a> = (&'a str, &'a [(u32, &'a str)], &'a str);

#[test]
fn format_survivors_matches_the_pgrep_lf_shape() {
    let cases: &[SurvivorCase<'_>] = &[
        ("no survivors", &[], ""),
        (
            "two survivors, pgrep -lf shape",
            &[
                (111, "/repo/ext/oxrsys/build-x64/Beat Saber.exe"),
                (222, "/other/place/Beat Saber.exe"),
            ],
            "111 Beat Saber.exe 222 Beat Saber.exe ",
        ),
        (
            "path with no file name falls back to the suffix",
            &[(7, "/")],
            "7 Beat Saber.exe ",
        ),
    ];
    for (label, input, expected) in cases {
        let procs: Vec<ProcInfo> = input
            .iter()
            .map(|(pid, exe)| ProcInfo {
                pid: *pid,
                start_time: 0,
                exe: PathBuf::from(*exe),
            })
            .collect();
        assert_eq!(format_survivors(&procs), *expected, "{label}");
    }
}

/// Finding #8: the survivor row agrees with a direct argv probe — same matches,
/// same text, for both the empty and the non-empty case.
#[test]
fn report_survivors_matches_a_direct_probe() {
    let (ctx, seen) = test_ctx(StageOptions::default());
    let survivors = process::find_processes_by_cmdline(BEAT_SABER_EXE_SUFFIX);
    report_survivors(&ctx, survivors.clone());
    let evs = seen.lock().unwrap().clone();
    let line = evs.last().expect("one row emitted");
    let crate::events::StageEvent::Line {
        severity,
        text,
        step,
        ..
    } = line
    else {
        panic!("expected a Line event, got {line:?}");
    };
    assert_eq!(step.as_deref(), Some(step::STOP_WINESERVER));
    if survivors.is_empty() {
        assert_eq!(*severity, Severity::Ok);
        assert_eq!(text, "game and wineserver down");
    } else {
        assert_eq!(*severity, Severity::Warn);
        assert!(text.starts_with("Beat Saber processes survived: "));
    }
}

#[test]
fn audio_report_branches_on_the_exact_blackhole_name() {
    assert_eq!(audio_report("BlackHole 2ch"), AudioReport::StillBlackhole);
    // Whole-string comparison, not substring: a name that merely contains
    // "BlackHole 2ch" does not count (mirrors `[ "$CUR" = "BlackHole 2ch" ]`,
    // not a grep).
    assert_eq!(
        audio_report("BlackHole 2ch (aggregate)"),
        AudioReport::Restored("BlackHole 2ch (aggregate)".to_string())
    );
    assert_eq!(
        audio_report("MacBook Pro Speakers"),
        AudioReport::Restored("MacBook Pro Speakers".to_string())
    );
    assert_eq!(audio_report(""), AudioReport::Restored(String::new()));
}

#[test]
fn audio_branch_text_is_verbatim() {
    assert_eq!(
        AUDIO_STILL_BLACKHOLE_WARN,
        "Mac audio output is still BlackHole 2ch (a run that died uncleanly could not \
             restore it)"
    );
    assert_eq!(
        AUDIO_RESTORE_HINT,
        "restore with: SwitchAudioSource -t output -s '<device>'   (list: SwitchAudioSource \
             -a -t output)"
    );
}

fn test_ctx(opts: StageOptions) -> (StageCtx, Arc<StdMutex<Vec<crate::events::StageEvent>>>) {
    let seen = Arc::new(StdMutex::new(Vec::new()));
    let s = seen.clone();
    let sink: crate::stages::EventSink = Arc::new(move |ev| s.lock().unwrap().push(ev));
    let mut paths = Paths::new("/nonexistent/sabrage-stop-test");
    // Load-bearing: `Paths::new` derives `session-state.json` from the real
    // `$HOME`, so without this override a stop test would read — and with a
    // real executor delete — the developer's own live session record.
    paths.sabrage_appsup = scratch_dir();
    let ctx = StageCtx::new(paths, opts, sink, CancellationToken::new());
    (ctx, seen)
}

/// A unique path under the system temp dir. **Not** created: tests that only
/// need "no session record here" want it absent, and the one test that
/// writes a record creates it itself.
fn scratch_dir() -> PathBuf {
    std::env::temp_dir().join(format!(
        "sabrage-stop-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4().simple()
    ))
}

/// (label, `ctx.paths.wineserver`, the WHOLE recorded plan as (kind, reason)).
type StopWineCase<'a> = (&'a str, Option<&'a str>, &'a [(PlannedKind, &'a str)]);

/// Both machines are simulated by overriding `ctx.paths.wineserver`:
/// `Paths::new` probes for a real CrossOver.app unconditionally, so neither
/// row may depend on whether this Mac has one.
#[tokio::test]
async fn dry_run_stop_wine_plans_the_wineserver_pair_only_when_crossover_is_present() {
    let cases: &[StopWineCase<'_>] = &[
        (
            "wineserver present",
            Some("/nonexistent/sabrage/wineserver"),
            &[
                (PlannedKind::Spawn, "/nonexistent/sabrage/wineserver -k"),
                (PlannedKind::Spawn, "/nonexistent/sabrage/wineserver -w"),
            ],
        ),
        ("no wineserver on this machine", None, &[]),
    ];
    for (label, wineserver, expected) in cases {
        let (mut ctx, _seen) = test_ctx(StageOptions {
            dry_run: true,
            ..Default::default()
        });
        ctx.paths.wineserver = wineserver.map(PathBuf::from);
        let bottle = Bottle::unvalidated("SabrageStopTest");

        stop_wine(&ctx, &bottle).await.expect("not cancelled");

        let planned: Vec<(PlannedKind, String)> = ctx
            .executor
            .planned()
            .into_iter()
            .map(|p| (p.kind, p.reason))
            .collect();
        let want: Vec<(PlannedKind, String)> = expected
            .iter()
            .map(|(kind, reason)| (*kind, (*reason).to_string()))
            .collect();
        assert_eq!(planned, want, "{label}");
    }
}

/// A [`ReapMsg`] whose three texts are distinguishable at a glance.
const TEST_REAP_MSG: ReapMsg = ReapMsg {
    killed: "found",
    survived: "survived",
    would: "would find",
};

#[tokio::test]
async fn dry_run_reap_plans_a_kill_per_match_and_reports_once() {
    // find_processes_by_exe on a path nothing runs from: not-found branch.
    let (ctx, seen) = test_ctx(StageOptions {
        dry_run: true,
        ..Default::default()
    });
    let matched = reap(
        &ctx,
        process::find_processes_by_exe(Path::new("/nonexistent/sabrage/helper")),
        step::STOP_REAP,
        Some(TEST_REAP_MSG),
        Some("not found"),
    )
    .await
    .expect("not cancelled");
    assert!(!matched);
    let evs = seen.lock().unwrap().clone();
    assert_eq!(evs.len(), 1);
    assert!(matches!(
        &evs[0],
        crate::events::StageEvent::Line { text, .. } if text == "not found"
    ));
    assert!(ctx.executor.planned().is_empty(), "no kill for no match");
}

#[tokio::test]
async fn dry_run_reap_matches_this_test_binary_by_exact_path() {
    let exe = std::env::current_exe().expect("test binary path");
    let (ctx, seen) = test_ctx(StageOptions {
        dry_run: true,
        ..Default::default()
    });
    let matched = reap(
        &ctx,
        process::find_processes_by_exe(&exe),
        step::STOP_REAP,
        Some(TEST_REAP_MSG),
        Some("not found"),
    )
    .await
    .expect("not cancelled");
    assert!(matched);
    let evs = seen.lock().unwrap().clone();
    assert_eq!(evs.len(), 1);
    // A dry run signalled nothing, so it may not claim a kill: the row is
    // the future-tense variant, at Info.
    assert!(
        matches!(
            &evs[0],
            crate::events::StageEvent::Line { text, severity, .. }
                if text == TEST_REAP_MSG.would && *severity == Severity::Info
        ),
        "{evs:?}"
    );
    let planned = ctx.executor.planned();
    assert!(
        !planned.is_empty(),
        "this test process itself should match its own exe path"
    );
    assert!(planned
        .iter()
        .all(|p| p.kind == PlannedKind::Spawn && p.reason.contains("/bin/kill -TERM")));
}

/// Spawned as a child by the reap tests below, never as part of the suite
/// (`#[ignore]` plus the env gate). The child is a **copy** of this test
/// binary placed at a unique temp path named [`HELPER_BASENAME`], so
/// `find_processes_by_exe` matches exactly that copy and nothing else on
/// the machine — this harness process included.
#[test]
#[ignore = "spawned as a child by the reap tests; not a test of its own"]
fn sleeper_child() {
    let Ok(secs) = std::env::var("SABRAGE_TEST_SLEEP_SECS") else {
        return;
    };
    if std::env::var("SABRAGE_TEST_IGNORE_TERM").is_ok() {
        use nix::sys::signal::{signal, SigHandler, Signal};
        // SAFETY: installing SIG_IGN in a freshly spawned single-purpose
        // child, before it does anything else.
        unsafe { signal(Signal::SIGTERM, SigHandler::SigIgn) }.expect("ignore SIGTERM");
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let _ = std::fs::write(dir.join("ready"), b"1");
        }
    }
    std::thread::sleep(std::time::Duration::from_secs(
        secs.parse().unwrap_or(10).min(60),
    ));
}

/// A live child at a unique `…/oxrsys-encoder-helper` path. Killed and
/// removed on drop, whatever the test did.
struct Sleeper {
    child: std::process::Child,
    dir: PathBuf,
    exe: PathBuf,
}

impl Drop for Sleeper {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

fn spawn_sleeper(ignore_term: bool) -> Sleeper {
    let dir = scratch_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join(HELPER_BASENAME);
    std::fs::copy(std::env::current_exe().unwrap(), &exe).unwrap();

    let mut cmd = std::process::Command::new(&exe);
    cmd.args([
        "--exact",
        "stages::stop::tests::sleeper_child",
        "--ignored",
        "--nocapture",
    ])
    .env("SABRAGE_TEST_SLEEP_SECS", "20")
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null());
    if ignore_term {
        cmd.env("SABRAGE_TEST_IGNORE_TERM", "1");
    }
    let child = cmd.spawn().expect("spawn the copied test binary");
    let sleeper = Sleeper { child, dir, exe };

    // Wait for the child to have installed its disposition and reached the
    // sleep — it writes `ready` next to itself just before.
    let ready = sleeper.dir.join("ready");
    for _ in 0..200 {
        if ready.is_file() && !process::find_processes_by_exe(&sleeper.exe).is_empty() {
            return sleeper;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    panic!("the sleeper child never became ready at {:?}", sleeper.exe);
}

fn rows(seen: &Arc<StdMutex<Vec<crate::events::StageEvent>>>) -> Vec<(Severity, String)> {
    seen.lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            crate::events::StageEvent::Line { severity, text, .. } => {
                Some((*severity, text.clone()))
            }
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_real_reap_reports_the_kill_only_once_the_process_is_really_gone() {
    let sleeper = spawn_sleeper(false);
    let (ctx, seen) = test_ctx(StageOptions::default());

    let matched = reap(
        &ctx,
        process::find_processes_by_exe(&sleeper.exe),
        step::STOP_REAP,
        Some(TEST_REAP_MSG),
        None,
    )
    .await
    .expect("not cancelled");

    assert!(matched);
    assert_eq!(rows(&seen), vec![(Severity::Ok, "found".to_string())]);
    // The row is only allowed to exist because the process is gone.
    assert!(
        process::find_processes_by_exe(&sleeper.exe).is_empty(),
        "the killed row printed while the process was still alive"
    );
}

#[tokio::test]
async fn a_term_ignoring_process_gets_a_warn_row_not_a_green_killed_row() {
    let sleeper = spawn_sleeper(true);
    let pid = sleeper.child.id();
    let (ctx, seen) = test_ctx(StageOptions::default());

    reap(
        &ctx,
        process::find_processes_by_exe(&sleeper.exe),
        step::STOP_REAP,
        Some(TEST_REAP_MSG),
        None,
    )
    .await
    .expect("not cancelled");

    let rows = rows(&seen);
    assert_eq!(rows.len(), 1, "{rows:?}");
    let (severity, text) = &rows[0];
    assert_eq!(*severity, Severity::Warn, "{rows:?}");
    assert!(
        text.starts_with("survived: ") && text.contains(&pid.to_string()),
        "{text:?} should name the surviving pid {pid}"
    );
    // Still alive: the whole point of the warn.
    assert!(!process::find_processes_by_exe(&sleeper.exe).is_empty());
}

/// A stale identity — same pid, a start time that process never had — must
/// not be signalled at all: `reap` skips it.
///
/// r1:A5-5 regression: a pid whose start time no longer matches fails
/// `is_same_process`, so `wait_for_exit` never counts it alive.
#[tokio::test]
async fn reap_never_signals_a_pid_whose_identity_no_longer_matches() {
    let mut mismatched = ProcInfo::observe(std::process::id()).expect("observe self");
    mismatched.start_time += 1;
    assert!(!mismatched.is_same_process());
    // Sanity that the guard, not some other branch, is what protects us:
    // this pid *is* alive.
    assert!(ProcInfo::observe(std::process::id()).is_some());
    // Exercised through `wait_for_exit`'s own predicate rather than by
    // asking `reap` to kill this very test process.
    assert!(wait_for_exit(&[mismatched]).await.is_empty());
}

/// r1:A5-7 regression: a helper left over in another checkout is reported
/// whether or not this checkout's own reap matched.
#[test]
fn a_foreign_helper_is_reported_whatever_the_local_reap_did() {
    let sleeper = spawn_sleeper(false);
    let pid = sleeper.child.id();
    // (label, local_matched — whether this checkout's own exact-path
    // reap already killed a helper just before the scan)
    let cases: &[(&str, bool)] = &[
        (
            "r1:A5-7 regression: a foreign helper is reported instead of the not-found row",
            false,
        ),
        (
            "r2:A5-2 regression: a local helper match must not suppress the cross-checkout warn",
            true,
        ),
    ];
    for (label, local_matched) in cases {
        let (mut ctx, seen) = test_ctx(StageOptions::default());
        // "Another checkout": this repo root contains neither the sleeper
        // nor anything else.
        ctx.paths.root = PathBuf::from("/nonexistent/sabrage-stop-test");
        report_foreign_helpers(
            &ctx,
            &ctx.paths.root.clone(),
            process::find_processes_by_cmdline(HELPER_BASENAME),
            *local_matched,
        );

        let rows = rows(&seen);
        assert!(
            rows.iter().any(|(sev, t)| *sev == Severity::Warn
                && t.starts_with("leftover encoder helper from another checkout: ")
                && t.contains(&pid.to_string())),
            "{label}: {rows:?}"
        );
        assert!(
            !rows.iter().any(|(_, t)| t == NO_LEFTOVER_HELPER),
            "{label}: {rows:?}"
        );
    }
}

/// (label, cmdline matches by pid, `local_matched`, expected rows).
type ForeignHelperCase<'a> = (&'a str, &'a [u32], bool, &'a [(Severity, &'a str)]);

/// The gate on the shell's not-found row: it prints only when neither
/// scan found anything, so the killed row and `NO_LEFTOVER_HELPER` can
/// never both appear — and a cmdline match whose exe is not the helper
/// binary was never a foreign helper to begin with.
#[test]
fn the_not_found_row_prints_only_when_nothing_foreign_and_no_local_match() {
    // A live pid whose executable is NOT named oxrsys-encoder-helper: the
    // basename filter must drop it.
    let not_the_helper = [std::process::id()];
    let cases: &[ForeignHelperCase<'_>] = &[
        (
            "nothing foreign, no local match: the shell's not-found row",
            &[],
            false,
            &[(Severity::Ok, NO_LEFTOVER_HELPER)],
        ),
        (
            "nothing foreign, local reap already reported a kill: no row at all",
            &[],
            true,
            &[],
        ),
        (
            "a cmdline match that is not the helper binary is filtered out",
            &not_the_helper,
            false,
            &[(Severity::Ok, NO_LEFTOVER_HELPER)],
        ),
    ];
    for (label, pids, local_matched, expected) in cases {
        let (mut ctx, seen) = test_ctx(StageOptions::default());
        ctx.paths.root = PathBuf::from("/nonexistent/sabrage-stop-test");
        let matches: Vec<ProcInfo> = pids
            .iter()
            .map(|pid| ProcInfo::observe(*pid).expect("observe a live pid"))
            .collect();
        report_foreign_helpers(&ctx, &ctx.paths.root.clone(), matches, *local_matched);
        let want: Vec<(Severity, String)> = expected
            .iter()
            .map(|(sev, text)| (*sev, (*text).to_string()))
            .collect();
        assert_eq!(rows(&seen), want, "{label}");
    }
}

/// Finding #2: cancellation propagates out of the stop helpers instead of
/// being swallowed.
#[tokio::test]
async fn stop_wine_propagates_a_pre_cancelled_token_instead_of_swallowing_it() {
    let (mut ctx, _seen) = test_ctx(StageOptions {
        dry_run: true,
        ..Default::default()
    });
    ctx.paths.wineserver = Some(PathBuf::from("/nonexistent/sabrage/wineserver"));
    ctx.cancel.cancel();
    let bottle = Bottle::unvalidated("SabrageStopTest");

    let err = stop_wine(&ctx, &bottle).await.unwrap_err();
    assert!(matches!(err, SabrageError::Cancelled));
    // Under DryRunExecutor `run_child` never itself errors — the check must
    // come from `ctx.cancel` directly, not from `run_child`'s result.
    assert!(
        !ctx.executor.planned().is_empty(),
        "the -k spawn was still planned before the check"
    );
}

#[tokio::test]
async fn reap_propagates_a_pre_cancelled_token_instead_of_swallowing_it() {
    let exe = std::env::current_exe().expect("test binary path");
    let (ctx, seen) = test_ctx(StageOptions {
        dry_run: true,
        ..Default::default()
    });
    ctx.cancel.cancel();

    let err = reap(
        &ctx,
        process::find_processes_by_exe(&exe),
        step::STOP_REAP,
        Some(TEST_REAP_MSG),
        Some("not found"),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, SabrageError::Cancelled));
    // The closing "found" message must NOT have been emitted — cancellation
    // short-circuits before it.
    assert!(seen.lock().unwrap().is_empty());
}

/// Both wineserver shapes, because they exercise different code: with a
/// `wineserver` path `stop_wine` spawns and hits its own post-`run_child`
/// check; with `None` it returns `Ok(())` immediately and only [`run`]'s
/// between-step `checkpoint` can catch the cancellation.
#[tokio::test]
async fn a_pre_cancelled_run_yields_cancelled_and_never_reports_stage_finished_ok() {
    for wineserver in [Some(PathBuf::from("/nonexistent/sabrage/wineserver")), None] {
        let (mut ctx, seen) = test_ctx(StageOptions {
            dry_run: true,
            bottle_name: Some("SabrageStopTest".to_string()),
            ..Default::default()
        });
        // Bypass the real `~/Library/.../CrossOver/Bottles` filesystem check —
        // `require_bottle` only needs `ctx.bottle` to be `Some`.
        ctx.bottle = Some(Bottle::unvalidated("SabrageStopTest"));
        ctx.paths.wineserver = wineserver.clone();
        ctx.cancel.cancel();

        let err = crate::stages::run_stage_holding_lock(crate::stages::Stage::Stop, &ctx)
            .await
            .unwrap_err();
        assert!(matches!(err, SabrageError::Cancelled), "{wineserver:?}");

        let evs = seen.lock().unwrap().clone();
        assert!(
            !evs.iter()
                .any(|e| matches!(e, crate::events::StageEvent::StageFinished { ok: true, .. })),
            "a cancelled stop must never report StageFinished{{ok:true}} \
                 (wineserver={wineserver:?}): {evs:?}"
        );
        assert!(
            evs.iter().any(|e| matches!(
                e,
                crate::events::StageEvent::StageFinished {
                    ok: false,
                    exit_code_equiv: 130,
                    ..
                }
            )),
            "expected a failed StageFinished carrying exit code 130 \
                 (wineserver={wineserver:?}): {evs:?}"
        );
    }
}

/// A pid no process can have on macOS (`kern.maxproc` is five digits), so
/// the recorded wine process classifies as `Dead` without a pid-reuse race.
/// Deliberately not `u32::MAX`, which is `-1` as an `i32`.
const DEAD_PID: u32 = 2_147_483_646;

/// Finding #6, at the stage level: the reconcile pass between steps 3 and 4
/// is *additive*, so a failure inside it is reported rather than aborting the
/// stage before the audio row — `stop.sh` has no step that can end the script.
///
/// Deterministic and machine-independent: the record carries a `--wired`
/// forward and `adb` points at a nonexistent path, so `forward --remove`
/// fails at `spawn` with `ENOENT`. Nothing is spawned, signalled, or written
/// on the machine.
#[tokio::test]
async fn a_failed_reconcile_is_reported_and_the_stage_still_reaches_its_audio_row() {
    use crate::session::state::{SessionState, WiredForward};

    let dir = scratch_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let (mut ctx, seen) = test_ctx(StageOptions {
        dry_run: false,
        bottle_name: Some("SabrageStopTest".to_string()),
        ..Default::default()
    });
    assert!(!ctx.executor.is_dry_run());
    ctx.bottle = Some(Bottle::unvalidated("SabrageStopTest"));
    ctx.paths.sabrage_appsup = dir.clone();
    ctx.paths.wineserver = None;
    ctx.paths.oxr_helper_staged = PathBuf::from("/nonexistent/sabrage/helper");
    ctx.paths.alvr_dashboard = PathBuf::from("/nonexistent/sabrage/dashboard");
    ctx.paths.adb = Some(dir.join("bin/adb"));

    let mut state = SessionState::new(
        uuid::Uuid::new_v4(),
        "SabrageStopTest",
        "/games/Beat Saber 1294",
        "/repo/logs/beatsaber-20260829-101112.log",
        1_786_300_214_181,
    );
    state.wine = Some(ProcInfo {
        pid: DEAD_PID,
        start_time: 1,
        exe: PathBuf::from("/nonexistent/sabrage/wine"),
    });
    state.wired_forwards = vec![WiredForward {
        serial: "1WMHH000X00000".to_string(),
        port: 9943,
    }];
    let path = ctx.paths.session_state_path();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut bytes = serde_json::to_vec_pretty(&state).unwrap();
    bytes.push(b'\n');
    std::fs::write(&path, bytes).unwrap();

    crate::stages::run_stage_holding_lock(crate::stages::Stage::Stop, &ctx)
        .await
        .expect("a failed reconcile must not fail the stop stage");

    let evs = seen.lock().unwrap().clone();
    assert!(
        evs.iter()
            .any(|e| matches!(e, crate::events::StageEvent::StageFinished { ok: true, .. })),
        "{evs:?}"
    );
    let lines: Vec<(Severity, Option<String>, String)> = evs
        .iter()
        .filter_map(|e| match e {
            crate::events::StageEvent::Line {
                severity,
                step,
                text,
                ..
            } => Some((*severity, step.clone(), text.clone())),
            _ => None,
        })
        .collect();
    assert!(
        lines.iter().any(|(sev, step, text)| *sev == Severity::Warn
            && step.as_deref() == Some(crate::session::reconcile::STEP)
            && text.starts_with("previous session not fully restored: ")),
        "the failure is reported: {lines:?}"
    );
    // …and the audio row that comes after it still ran.
    let audio_rows = lines
        .iter()
        .filter(|(_, step, _)| step.as_deref() == Some(step::STOP_AUDIO))
        .count();
    if which("SwitchAudioSource").is_some() {
        // One row normally; two when this Mac's output happens to be sitting
        // on BlackHole 2ch (warn + restore hint). Either proves step 4 ran.
        assert!(audio_rows >= 1, "the audio row is the point: {lines:?}");
    } else {
        assert_eq!(audio_rows, 0, "silent without the tool: {lines:?}");
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// The narrower window finding #3 named: cancellation after the wineserver
/// kill, in the *reporting* half where no executor child is spawned and
/// nothing else observes the token. Deterministic: cancel **from the event
/// sink** the instant the first row (`report_survivors`'s) is emitted —
/// [`run`]'s next call is its own `checkpoint`.
#[tokio::test]
async fn cancellation_during_the_reporting_steps_still_fails_the_stage() {
    let seen = Arc::new(StdMutex::new(Vec::new()));
    let cancel = CancellationToken::new();
    let s = seen.clone();
    let c = cancel.clone();
    let sink: crate::stages::EventSink = Arc::new(move |ev| {
        if matches!(ev, crate::events::StageEvent::Line { .. }) {
            c.cancel();
        }
        s.lock().unwrap().push(ev);
    });
    let mut ctx = StageCtx::new(
        Paths::new("/nonexistent/sabrage-stop-test"),
        StageOptions {
            dry_run: true,
            bottle_name: Some("SabrageStopTest".to_string()),
            ..Default::default()
        },
        sink,
        cancel,
    );
    ctx.bottle = Some(Bottle::unvalidated("SabrageStopTest"));
    // No wineserver and nothing to reap, so every `run_child`-adjacent
    // check is unreachable: only `run`'s between-step checkpoints are left.
    ctx.paths.wineserver = None;
    ctx.paths.oxr_helper_staged = PathBuf::from("/nonexistent/sabrage/helper");
    ctx.paths.alvr_dashboard = PathBuf::from("/nonexistent/sabrage/dashboard");

    let err = crate::stages::run_stage_holding_lock(crate::stages::Stage::Stop, &ctx)
        .await
        .unwrap_err();
    assert!(matches!(err, SabrageError::Cancelled), "{err:?}");

    let evs = seen.lock().unwrap().clone();
    assert!(
        !evs.iter()
            .any(|e| matches!(e, crate::events::StageEvent::StageFinished { ok: true, .. })),
        "{evs:?}"
    );
    // The steps after the cancellation never ran.
    assert!(
        !evs.iter().any(|e| matches!(
            e,
            crate::events::StageEvent::Line { text, .. }
                if text.starts_with("streaming ports")
        )),
        "report_ports ran past a cancelled token: {evs:?}"
    );
}
