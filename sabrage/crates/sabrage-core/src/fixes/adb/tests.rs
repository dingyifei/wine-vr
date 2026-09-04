use super::*;
use crate::paths::Paths;
use crate::stages::{StageCtx, StageOptions};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;

#[test]
fn parse_forward_list_takes_serial_and_local_and_skips_unusable_lines() {
    type Expected = &'static [(&'static str, &'static str)];
    let cases: &[(&str, &str, Expected)] = &[
        (
            "serial and local kept, the remote field dropped",
            "192.168.1.5:5555 tcp:9943 tcp:9943\n192.168.1.5:5555 tcp:9944 tcp:9944\n",
            &[
                ("192.168.1.5:5555", "tcp:9943"),
                ("192.168.1.5:5555", "tcp:9944"),
            ],
        ),
        ("empty input", "", &[]),
        ("blank lines only", "\n\n", &[]),
        (
            "short line is skipped, the next row still parses",
            "onlyone\nser tcp:1 tcp:2\n",
            &[("ser", "tcp:1")],
        ),
    ];
    for (label, input, expected) in cases {
        let expected: Vec<(String, String)> = expected
            .iter()
            .map(|(serial, local)| ((*serial).to_string(), (*local).to_string()))
            .collect();
        assert_eq!(parse_forward_list(input), expected, "{label}");
    }
}

fn scratch(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("sabrage-adb-fix-{tag}-{}", std::process::id()))
}

/// A `/bin/sh` fixture standing in for `adb`: a plain shell script, not a
/// copied Mach-O binary — this crate's sandboxed test runner has been
/// observed to `SIGKILL` the latter before it can even run (see
/// `fixes::backend`'s test module header), but a script executes fine.
fn write_fake_adb(script_path: &Path, list_stdout: &str, log_path: &Path) {
    std::fs::create_dir_all(script_path.parent().unwrap()).unwrap();
    let script = format!(
        "#!/bin/sh\n\
             if [ \"$1\" = forward ] && [ \"$2\" = --list ]; then\n\
             \x20\x20cat <<'SABRAGE_EOF'\n{list_stdout}SABRAGE_EOF\n\
             \x20\x20exit 0\n\
             fi\n\
             if [ \"$1\" = -s ]; then\n\
             \x20\x20echo \"$2 $3 $4 $5\" >> {log}\n\
             \x20\x20exit 0\n\
             fi\n\
             exit 1\n",
        log = log_path.display(),
    );
    std::fs::write(script_path, script).unwrap();
    let mut perms = std::fs::metadata(script_path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(script_path, perms).unwrap();
}

/// A fake `adb` whose exit codes are the fixture: `list_exit` for
/// `forward --list`, `remove_exit` for every `-s <serial> forward --remove`.
fn write_fake_adb_failing(
    script_path: &Path,
    list_stdout: &str,
    list_exit: i32,
    remove_exit: i32,
    log_path: &Path,
) {
    std::fs::create_dir_all(script_path.parent().unwrap()).unwrap();
    let script = format!(
        "#!/bin/sh\n\
             if [ \"$1\" = forward ] && [ \"$2\" = --list ]; then\n\
             \x20\x20cat <<'SABRAGE_EOF'\n{list_stdout}SABRAGE_EOF\n\
             \x20\x20exit {list_exit}\n\
             fi\n\
             if [ \"$1\" = -s ]; then\n\
             \x20\x20echo \"$2 $3 $4 $5\" >> {log}\n\
             \x20\x20exit {remove_exit}\n\
             fi\n\
             exit 1\n",
        log = log_path.display(),
    );
    std::fs::write(script_path, script).unwrap();
    let mut perms = std::fs::metadata(script_path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(script_path, perms).unwrap();
}

fn ctx_with_adb(root: &Path, adb: PathBuf, dry_run: bool) -> StageCtx {
    let mut paths = Paths::new(root);
    paths.adb = Some(adb);
    // The standalone fix consults the live-session policy; point both stores
    // at the scratch root so it reads fixtures, never the real machine.
    paths.sabrage_appsup = root.join("Sabrage");
    paths.oxr_appsup = root.join("OXRSys");
    let opts = StageOptions {
        dry_run,
        ..StageOptions::default()
    };
    let sink: EventSink = std::sync::Arc::new(|_| {});
    StageCtx::new(paths, opts, sink, CancellationToken::new())
}

#[tokio::test]
async fn no_adb_is_a_silent_noop() {
    let root = scratch("no-adb");
    let mut paths = Paths::new(&root);
    paths.adb = None;
    paths.sabrage_appsup = root.join("Sabrage");
    paths.oxr_appsup = root.join("OXRSys");
    let sink: EventSink = std::sync::Arc::new(|_| {});
    let ctx = StageCtx::new(
        paths,
        StageOptions::default(),
        sink.clone(),
        CancellationToken::new(),
    );
    let report = remove_adb_forwards(&ctx, &sink).await.unwrap();
    assert!(!report.changed);
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn clean_forward_list_is_a_noop() {
    let root = scratch("clean");
    let adb = root.join("adb.sh");
    let log = root.join("removed.log");
    write_fake_adb(&adb, "SER tcp:5555 tcp:5555\n", &log);

    let ctx = ctx_with_adb(&root, adb, false);
    let sink: EventSink = ctx.sink.clone();
    let report = remove_adb_forwards(&ctx, &sink).await.unwrap();
    assert!(!report.changed);
    assert!(!log.exists(), "nothing stale -> nothing removed");
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn removes_exactly_the_two_stale_ports_per_serial_never_remove_all() {
    let root = scratch("remove");
    let adb = root.join("adb.sh");
    let log = root.join("removed.log");
    write_fake_adb(
        &adb,
        "SERIALX tcp:9943 tcp:9943\nSERIALX tcp:9944 tcp:9944\nSERIALX tcp:5555 tcp:5555\n",
        &log,
    );

    let ctx = ctx_with_adb(&root, adb, false);
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let s = seen.clone();
    let sink: EventSink = std::sync::Arc::new(move |ev| s.lock().unwrap().push(ev));

    let report = remove_adb_forwards(&ctx, &sink).await.unwrap();
    assert!(report.changed);
    assert!(report.description.contains("tcp:9943"));
    assert!(report.description.contains("tcp:9944"));
    assert!(!report.description.contains("5555"));

    let log_text = std::fs::read_to_string(&log).unwrap();
    let mut lines: Vec<&str> = log_text.lines().collect();
    lines.sort_unstable();
    assert_eq!(
        lines,
        vec![
            "SERIALX forward --remove tcp:9943",
            "SERIALX forward --remove tcp:9944",
        ],
        "must remove exactly the two stale ports, per-serial, never --remove-all"
    );
    assert!(!log_text.contains("--remove-all"));

    let texts: Vec<String> = seen
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            StageEvent::Line { text, severity, .. } => {
                assert_eq!(
                    *severity,
                    crate::events::Severity::Info,
                    "run.sh uses `info`, not `ok`, here"
                );
                Some(text.clone())
            }
            _ => None,
        })
        .collect();
    assert!(texts.iter().any(|t| t
            == "cleared stale adb forward tcp:9943 on SERIALX (left over from a --wired launch — would otherwise break WiFi discovery)"));
    assert!(texts.iter().any(|t| t
            == "cleared stale adb forward tcp:9944 on SERIALX (left over from a --wired launch — would otherwise break WiFi discovery)"));

    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn dry_run_reports_would_clear_and_still_invokes_the_planned_spawn() {
    let root = scratch("dry");
    let adb = root.join("adb.sh");
    let log = root.join("removed.log");
    write_fake_adb(&adb, "SERIALX tcp:9943 tcp:9943\n", &log);

    let ctx = ctx_with_adb(&root, adb, true);
    let sink: EventSink = ctx.sink.clone();
    let report = remove_adb_forwards(&ctx, &sink).await.unwrap();
    assert!(report.changed);
    assert!(report
        .description
        .starts_with("would clear stale adb forward tcp:9943"));
    assert!(!log.exists(), "dry run must not actually spawn the removal");

    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn the_step_id_is_the_fixs_own_by_default_and_the_callers_with_at() {
    // #16c: one implementation serves both step ids.
    let root = scratch("step-id");
    let adb = root.join("adb.sh");
    let log = root.join("removed.log");
    write_fake_adb(&adb, "SERIALX tcp:9943 tcp:9943\n", &log);

    let ctx = ctx_with_adb(&root, adb, true);
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let s = seen.clone();
    let sink: EventSink = std::sync::Arc::new(move |ev| s.lock().unwrap().push(ev));

    let steps = |seen: &std::sync::Mutex<Vec<StageEvent>>| -> Vec<Option<String>> {
        seen.lock()
            .unwrap()
            .iter()
            .filter(|e| matches!(e, StageEvent::Line { .. }))
            .map(|e| e.step().map(str::to_string))
            .collect()
    };

    remove_adb_forwards(&ctx, &sink).await.unwrap();
    assert_eq!(steps(&seen), vec![Some(STEP.to_string())]);
    assert_eq!(STEP, "fix.remove-adb-forwards");

    seen.lock().unwrap().clear();
    remove_adb_forwards_at(&ctx, &sink, crate::events::step::RUN_ADB_FORWARDS)
        .await
        .unwrap();
    assert_eq!(steps(&seen), vec![Some("run.2.adb-forwards".to_string())]);

    std::fs::remove_dir_all(&root).ok();
}

/// A removal that failed must never be reported like a clean forwarding
/// table while the WiFi-breaking forward is still installed.
#[tokio::test]
async fn a_failed_removal_is_never_reported_as_a_clean_table() {
    let root = scratch("remove-fails");
    let adb = root.join("adb.sh");
    let log = root.join("removed.log");
    write_fake_adb_failing(
        &adb,
        "SERIALX tcp:9943 tcp:9943\nSERIALX tcp:9944 tcp:9944\n",
        0,
        1,
        &log,
    );

    let ctx = ctx_with_adb(&root, adb, false);
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let s = seen.clone();
    let sink: EventSink = std::sync::Arc::new(move |ev| s.lock().unwrap().push(ev));

    let report = remove_adb_forwards(&ctx, &sink).await.unwrap();
    assert!(!report.changed, "nothing was actually cleared");
    assert_ne!(report.description, "no stale adb port forwards to clear");
    assert!(
        report.description.contains("SERIALX tcp:9943"),
        "{report:?}"
    );
    assert!(
        report.description.contains("SERIALX tcp:9944"),
        "{report:?}"
    );

    let warns: Vec<String> = seen
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            StageEvent::Line { text, severity, .. }
                if *severity == crate::events::Severity::Warn =>
            {
                Some(text.clone())
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        warns.len(),
        2,
        "one warn per unremovable forward: {warns:?}"
    );
    assert!(warns[0].starts_with("could not clear adb forward tcp:9943 on SERIALX"));

    std::fs::remove_dir_all(&root).ok();
}

/// `adb forward --list` that cannot be answered is not an empty table.
#[tokio::test]
async fn a_query_failure_is_reported_as_a_query_failure() {
    let root = scratch("list-fails");
    let adb = root.join("adb.sh");
    let log = root.join("removed.log");
    write_fake_adb_failing(&adb, "", 1, 0, &log);

    let ctx = ctx_with_adb(&root, adb, false);
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let s = seen.clone();
    let sink: EventSink = std::sync::Arc::new(move |ev| s.lock().unwrap().push(ev));

    let report = remove_adb_forwards(&ctx, &sink).await.unwrap();
    assert!(!report.changed);
    assert!(
        report
            .description
            .starts_with("could not query adb forwards ("),
        "{report:?}"
    );
    assert!(
        report.description.contains("tcp:9943/tcp:9944"),
        "{report:?}"
    );
    assert!(!log.exists(), "a failed query must remove nothing");
    assert!(seen.lock().unwrap().iter().any(|e| matches!(
        e,
        StageEvent::Line { severity, .. } if *severity == crate::events::Severity::Warn
    )));

    std::fs::remove_dir_all(&root).ok();
}

/// An `adb` that cannot be spawned at all is the other half of "adb could
/// not tell us": exactly one `warn` naming the query failure and the two
/// ports that may still be installed, plus an `unchanged` report carrying
/// the same text. Both are what Doctor renders as this row's fix notice
/// (A4-5; the UI half is `ui/src/screens/Doctor.svelte`'s `runFix`).
#[tokio::test]
async fn an_unspawnable_adb_warns_once_and_never_reports_a_clean_table() {
    let root = scratch("list-unspawnable");
    std::fs::create_dir_all(&root).unwrap();
    let adb = root.join("no-such-adb");
    assert!(!adb.exists());

    let ctx = ctx_with_adb(&root, adb, false);
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let s = seen.clone();
    let sink: EventSink = std::sync::Arc::new(move |ev| s.lock().unwrap().push(ev));

    let report = remove_adb_forwards(&ctx, &sink).await.unwrap();
    assert!(!report.changed);
    assert_ne!(report.description, "no stale adb port forwards to clear");
    assert!(
        report
            .description
            .starts_with("could not query adb forwards ("),
        "{report:?}"
    );
    assert!(
        report
            .description
            .ends_with("— stale tcp:9943/tcp:9944 forwards may still be installed"),
        "{report:?}"
    );

    let warns: Vec<String> = seen
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            StageEvent::Line { text, severity, .. }
                if *severity == crate::events::Severity::Warn =>
            {
                Some(text.clone())
            }
            _ => None,
        })
        .collect();
    assert_eq!(warns, vec![report.description.clone()], "{warns:?}");

    std::fs::remove_dir_all(&root).ok();
}

/// The `info` row is rendered in one place, so `sabrage-parity` can compare
/// the live string against run.sh instead of a copy (A1-3).
#[test]
fn the_cleared_forward_line_is_the_one_renderer() {
    assert_eq!(
            cleared_forward_line("cleared", "tcp:9943", "SERIALX"),
            "cleared stale adb forward tcp:9943 on SERIALX (left over from a --wired launch — would otherwise break WiFi discovery)"
        );
    assert!(cleared_forward_line("would clear", "tcp:9944", "SerB")
        .starts_with("would clear stale adb forward tcp:9944 on SerB "));
}

/// The `--wired` session's own forwards: the standalone fix (a Doctor
/// button) must refuse, while the launch path — which clears leftovers
/// *before* a session exists — keeps working.
#[tokio::test]
async fn the_standalone_fix_refuses_during_a_live_session_but_the_launch_path_does_not() {
    let _g = crate::session::lock_session_globals();
    let root = scratch("live");
    let adb = root.join("adb.sh");
    let log = root.join("removed.log");
    write_fake_adb(&adb, "SERIALX tcp:9943 tcp:9943\n", &log);

    let ctx = ctx_with_adb(&root, adb, false);
    let state_path = ctx.paths.session_state_path();
    std::fs::create_dir_all(state_path.parent().unwrap()).unwrap();
    let mut state = crate::session::state::SessionState::new(
        uuid::Uuid::new_v4(),
        "FixtureBottle",
        "/bs",
        "/log",
        0,
    );
    state.wine = crate::process::ProcInfo::observe(std::process::id());
    std::fs::write(&state_path, serde_json::to_string(&state).unwrap()).unwrap();

    let sink: EventSink = ctx.sink.clone();
    let err = remove_adb_forwards(&ctx, &sink).await.unwrap_err();
    assert!(
        err.to_string()
            .starts_with("refusing to remove adb port forwards while a session is live"),
        "{err}"
    );
    assert!(!log.exists(), "adb must not be spawned while refusing");

    // The launch path is the same removal without the gate.
    let report = remove_adb_forwards_at(&ctx, &sink, crate::events::step::RUN_ADB_FORWARDS)
        .await
        .unwrap();
    assert!(report.changed);
    assert!(log.exists());

    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn multiple_serials_are_each_targeted_independently() {
    let root = scratch("multi-serial");
    let adb = root.join("adb.sh");
    let log = root.join("removed.log");
    write_fake_adb(
        &adb,
        "SerA tcp:9943 tcp:9943\nSerB tcp:9944 tcp:9944\n",
        &log,
    );

    let ctx = ctx_with_adb(&root, adb, false);
    let sink: EventSink = ctx.sink.clone();
    let report = remove_adb_forwards(&ctx, &sink).await.unwrap();
    assert!(report.changed);

    let log_text = std::fs::read_to_string(&log).unwrap();
    let mut lines: Vec<&str> = log_text.lines().collect();
    lines.sort_unstable();
    assert_eq!(
        lines,
        vec![
            "SerA forward --remove tcp:9943",
            "SerB forward --remove tcp:9944"
        ]
    );

    std::fs::remove_dir_all(&root).ok();
}
