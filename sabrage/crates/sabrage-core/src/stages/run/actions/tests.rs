use super::*;
use crate::events::{Severity, Stage};
use crate::executor::{DryRunExecutor, Executor, PlannedKind};
use crate::paths::Paths;
use crate::stages::{EventSink, StageOptions};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "sabrage-run-actions-{tag}-{}-{}",
        std::process::id(),
        Uuid::new_v4().as_simple()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A `StageCtx` whose every path lives under `root` — no real `$HOME`, no
/// CrossOver, no adb, and a [`DryRunExecutor`] so nothing is written.
fn dry_ctx(root: &Path, opts: StageOptions) -> (StageCtx, Arc<Mutex<Vec<StageEvent>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s = seen.clone();
    let sink: EventSink = Arc::new(move |ev| s.lock().unwrap().push(ev));
    let run_id = Uuid::new_v4();
    let cancel = CancellationToken::new();
    let executor: Arc<dyn Executor> =
        Arc::new(DryRunExecutor::new(run_id, sink.clone(), cancel.clone()));

    let mut paths = Paths::new(root);
    paths.oxr_appsup = root.join("appsup-oxrsys");
    paths.sabrage_appsup = root.join("appsup-sabrage");
    paths.adb = None;
    paths.wine = None;
    paths.wineserver = None;

    let ctx = StageCtx::with_executor(paths, opts, sink, cancel, executor, run_id);
    (ctx, seen)
}

/// A fresh [`SessionState`] for `adb_forward_hygiene`'s write-before-mutate
/// tests — the run id and paths do not matter to those tests, only that a
/// record exists for it to persist forwards into.
fn fresh_state() -> SessionState {
    SessionState::new(Uuid::nil(), "Steam", "/games/bs", "/repo/logs/x.log", 1)
}

fn bottle(root: &Path) -> Bottle {
    Bottle {
        name: "Steam".to_string(),
        prefix: root.join("bottle"),
        sys32: root.join("bottle/drive_c/windows/system32"),
    }
}

fn texts(evs: &[StageEvent]) -> Vec<String> {
    evs.iter()
        .filter_map(|e| match e {
            StageEvent::Text { text, .. } => Some(text.clone()),
            StageEvent::Section { title, .. } => Some(format!("-- {title}")),
            StageEvent::Line { severity, text, .. } => Some(format!("[{severity}] {text}")),
            _ => None,
        })
        .collect()
}

#[test]
fn the_guarded_actions_are_listed_and_launch_is_last() {
    // The two guarded ones are in the list but implemented in `guards`.
    assert!(LAUNCH_ACTION_IDS.contains(&"audio-route"));
    assert!(LAUNCH_ACTION_IDS.contains(&"dashboard"));
    // Launch is always last.
    assert_eq!(LAUNCH_ACTION_IDS[6], "launch-wine");
}

#[test]
fn one_step_id_per_action_plus_the_three_run_only_phases() {
    // Every launch action maps onto a `run.*` step; the state machine adds
    // preflight, supervise and teardown.
    assert_eq!(
        Stage::Run.steps().len(),
        LAUNCH_ACTION_IDS.len() + 3,
        "run's step list and the contract's action list must stay aligned"
    );
}

#[test]
fn first_device_serial_matches_the_awk_program() {
    // NR>1 skips the header; only an exact `device` state counts.
    assert_eq!(
        first_device_serial("List of devices attached\n1WMHH0X\tdevice\n"),
        Some("1WMHH0X".to_string())
    );
    assert_eq!(
        first_device_serial("List of devices attached\nabc\tunauthorized\ndef\tdevice\n"),
        Some("def".to_string())
    );
    // The header itself is never a candidate, even though its second field
    // is a word.
    assert_eq!(first_device_serial("List of devices attached\n"), None);
    assert_eq!(first_device_serial(""), None);
    assert_eq!(first_device_serial("List of devices attached\n\n\n"), None);
    // `offline` / `no permissions` rows are skipped.
    assert_eq!(
        first_device_serial("List of devices attached\nabc\toffline\n"),
        None
    );
}

#[tokio::test]
async fn a_non_wired_run_without_adb_does_nothing_at_all() {
    let root = scratch("no-adb");
    let (ctx, seen) = dry_ctx(&root, StageOptions::default());
    let mut sess = fresh_state();
    let state_path = ctx.paths.session_state_path();
    adb_forward_hygiene(&ctx, &mut sess, &state_path)
        .await
        .unwrap();
    assert!(sess.wired_forwards.is_empty());
    assert!(seen.lock().unwrap().is_empty());
    assert!(ctx.executor.planned().is_empty());
    std::fs::remove_dir_all(&root).unwrap();
}

#[tokio::test]
async fn the_non_wired_forward_cleanup_is_stamped_with_the_run_stages_step() {
    // #16c: the removal is `fix.remove-adb-forwards`'s code, but here it
    // is the run stage's step 2 — its rows must sort and group with the
    // rest of the launch, not with a fix that is not running.
    let root = scratch("forward-step-id");
    let (mut ctx, seen) = dry_ctx(&root, StageOptions::default());
    ctx.paths.adb = Some(fake_forward_list_adb(
        &root,
        "SERIALX tcp:9943 tcp:9943\nSERIALX tcp:5555 tcp:5555\n",
    ));

    let mut sess = fresh_state();
    let state_path = ctx.paths.session_state_path();
    adb_forward_hygiene(&ctx, &mut sess, &state_path)
        .await
        .unwrap();
    assert!(
        sess.wired_forwards.is_empty(),
        "a non-wired run creates nothing"
    );

    let evs = seen.lock().unwrap().clone();
    let rows: Vec<(Option<&str>, String)> = evs
        .iter()
        .filter_map(|e| match e {
            StageEvent::Line { text, .. } => Some((e.step(), text.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].0, Some(step::RUN_ADB_FORWARDS));
    assert!(
        rows[0]
            .1
            .starts_with("would clear stale adb forward tcp:9943 on SERIALX"),
        "{}",
        rows[0].1
    );
    assert!(
        !rows
            .iter()
            .any(|(s, _)| *s == Some("fix.remove-adb-forwards")),
        "the launch path must not borrow the standalone fix's step id"
    );
    std::fs::remove_dir_all(&root).ok();
}

/// A `/bin/sh` script standing in for `adb`, answering `forward --list`
/// with `list_stdout` and succeeding at everything else.
fn fake_forward_list_adb(dir: &Path, list_stdout: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("adb-forwards");
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\n\
                 if [ \"$1\" = forward ] && [ \"$2\" = --list ]; then\n\
                 \x20 printf '%s' '{list_stdout}'\n\
                 fi\n\
                 exit 0\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[tokio::test]
async fn wired_without_adb_dies_with_run_shs_text() {
    let root = scratch("wired-no-adb");
    let (ctx, _) = dry_ctx(
        &root,
        StageOptions {
            wired: true,
            ..Default::default()
        },
    );
    let mut sess = fresh_state();
    let state_path = ctx.paths.session_state_path();
    let err = adb_forward_hygiene(&ctx, &mut sess, &state_path)
        .await
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "--wired needs adb (Android platform-tools) on PATH or under ~/Library/Android/sdk"
    );
    std::fs::remove_dir_all(&root).unwrap();
}

/// A `/bin/sh` script standing in for `adb`, so the adb branches run
/// without an Android SDK, a device, or any real forward.
///
/// `forward_exit` controls the exit status of `adb forward` calls;
/// `--remove` always succeeds. For a rollback whose removal also fails,
/// use [`every_call_fails_adb`].
fn fake_adb(dir: &Path, devices_stdout: &str, forward_exit: i32) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("adb");
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\n\
                 for a in \"$@\"; do\n\
                 \x20 case \"$a\" in devices) printf '%s' '{devices_stdout}'; exit 0;; esac\n\
                 done\n\
                 echo \"$@\" >> \"$(dirname \"$0\")/adb.calls\"\n\
                 case \"$*\" in *--remove*) exit 0;; esac\n\
                 exit {forward_exit}\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// [`fake_adb`]'s harsher sibling: every non-`devices` call exits nonzero,
/// so the rollback's own `forward --remove` fails as well.
fn every_call_fails_adb(dir: &Path, devices_stdout: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("adb");
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\n\
                 for a in \"$@\"; do\n\
                 \x20 case \"$a\" in devices) printf '%s' '{devices_stdout}'; exit 0;; esac\n\
                 done\n\
                 echo \"$@\" >> \"$(dirname \"$0\")/adb.calls\"\n\
                 exit 1\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[tokio::test]
async fn wired_with_no_device_dies_with_run_shs_text() {
    let root = scratch("wired-no-device");
    let (mut ctx, _) = dry_ctx(
        &root,
        StageOptions {
            wired: true,
            ..Default::default()
        },
    );
    ctx.paths.adb = Some(fake_adb(&root, "List of devices attached\n", 0));
    let mut sess = fresh_state();
    let state_path = ctx.paths.session_state_path();
    let err = adb_forward_hygiene(&ctx, &mut sess, &state_path)
        .await
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "--wired: no Quest over adb — connect USB and check 'adb devices'"
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[tokio::test]
async fn wired_plans_both_forwards_and_reports_them() {
    let root = scratch("wired-ok");
    let (mut ctx, seen) = dry_ctx(
        &root,
        StageOptions {
            wired: true,
            ..Default::default()
        },
    );
    ctx.paths.adb = Some(fake_adb(
        &root,
        "List of devices attached\n1WMHH0X\tdevice\n",
        0,
    ));

    let mut sess = fresh_state();
    let state_path = ctx.paths.session_state_path();
    adb_forward_hygiene(&ctx, &mut sess, &state_path)
        .await
        .unwrap();
    assert_eq!(
        sess.wired_forwards,
        vec![
            WiredForward {
                serial: "1WMHH0X".into(),
                port: 9943
            },
            WiredForward {
                serial: "1WMHH0X".into(),
                port: 9944
            },
        ]
    );
    // Two planned spawns, never a `--remove-all` — plus the two
    // write-before-mutate `session-state.json` saves (one per forward).
    let plan = ctx.executor.planned();
    assert_eq!(
        plan.iter().filter(|p| p.kind == PlannedKind::Spawn).count(),
        2
    );
    let spawns: Vec<_> = plan
        .iter()
        .filter(|p| p.kind == PlannedKind::Spawn)
        .collect();
    for (p, port) in spawns.iter().zip(["tcp:9943", "tcp:9944"]) {
        assert!(
            p.reason
                .ends_with(&format!("-s 1WMHH0X forward {port} {port}")),
            "{}",
            p.reason
        );
    }
    assert!(!plan.iter().any(|p| p.reason.contains("--remove-all")));
    assert_eq!(
        texts(&seen.lock().unwrap()),
        vec![
            "[info] wired mode: adb forward tcp:9943/tcp:9944 up on 1WMHH0X \
                 (a later non-wired run clears these two)"
                .to_string()
        ]
    );
    std::fs::remove_dir_all(&root).unwrap();
}

/// `adb.calls` — one line per non-`devices` invocation of [`fake_adb`].
fn adb_calls(root: &Path) -> Vec<String> {
    std::fs::read_to_string(root.join("adb.calls"))
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

/// A real (non-dry) ctx over the scratch root, so the fake adb actually
/// runs and its exit status decides the branch.
fn real_ctx(root: &Path, opts: StageOptions) -> (StageCtx, Arc<Mutex<Vec<StageEvent>>>) {
    let (mut ctx, seen) = dry_ctx(root, opts);
    let run_id = ctx.run_id;
    ctx.executor = Arc::new(crate::executor::RealExecutor::new(
        run_id,
        ctx.sink.clone(),
        ctx.cancel.clone(),
    ));
    (ctx, seen)
}

/// A nonzero `adb forward` removes both ports before dying, matching
/// scripts/demo/run.sh # launch-action: adb-forward-hygiene.
#[tokio::test]
async fn a_failed_forward_removes_both_ports_and_dies_with_run_shs_text() {
    let root = scratch("wired-fail");
    let (mut ctx, _) = real_ctx(
        &root,
        StageOptions {
            wired: true,
            ..Default::default()
        },
    );
    ctx.paths.adb = Some(fake_adb(
        &root,
        "List of devices attached\n1WMHH0X\tdevice\n",
        1,
    ));

    let mut sess = fresh_state();
    let state_path = ctx.paths.session_state_path();
    let err = adb_forward_hygiene(&ctx, &mut sess, &state_path)
        .await
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "adb forward tcp:9943 tcp:9943 failed on 1WMHH0X — check the USB connection \
             (adb devices)"
    );
    let calls = adb_calls(&root);
    assert!(
        calls
            .iter()
            .any(|c| c.contains("forward --remove tcp:9943")),
        "{calls:?}"
    );
    assert!(
        calls
            .iter()
            .any(|c| c.contains("forward --remove tcp:9944")),
        "{calls:?}"
    );
    assert!(
        sess.wired_forwards.is_empty(),
        "the in-memory record must reflect the rollback"
    );
    std::fs::remove_dir_all(&root).ok();
}

/// A7-2: the rollback's own `--remove` can fail too, and then the forward
/// is *indeterminate* — it may still be installed on the device. Clearing
/// the record on that outcome deleted the only thing that would ever retry
/// it, and the stale `tcp:9943` silently breaks the next WiFi run.
#[tokio::test]
async fn a_rollback_whose_removal_fails_keeps_the_forward_on_record() {
    let root = scratch("wired-rollback-fail");
    let (mut ctx, _) = real_ctx(
        &root,
        StageOptions {
            wired: true,
            ..Default::default()
        },
    );
    ctx.paths.adb = Some(every_call_fails_adb(
        &root,
        "List of devices attached\n1WMHH0X\tdevice\n",
    ));

    let mut sess = fresh_state();
    let state_path = ctx.paths.session_state_path();
    let err = adb_forward_hygiene(&ctx, &mut sess, &state_path)
        .await
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "adb forward tcp:9943 tcp:9943 failed on 1WMHH0X — check the USB connection \
             (adb devices)"
    );
    // Both removals still attempted (scripts/demo/run.sh # launch-action: adb-forward-hygiene).
    let calls = adb_calls(&root);
    for port in ["tcp:9943", "tcp:9944"] {
        assert!(
            calls
                .iter()
                .any(|c| c.contains(&format!("forward --remove {port}"))),
            "{calls:?}"
        );
    }
    // …and neither succeeded, so the record survives — in memory and on
    // disk, which is where the next reconcile/stop retries it from.
    assert_eq!(
        sess.wired_forwards,
        vec![WiredForward {
            serial: "1WMHH0X".into(),
            port: 9943
        }],
        "an indeterminate removal must stay on record"
    );
    let on_disk = state::load(&state_path).unwrap().unwrap();
    assert_eq!(on_disk.wired_forwards, sess.wired_forwards);
    assert!(
        !on_disk.guards.forwards_cleared,
        "nothing may claim the forwards are cleared"
    );
    std::fs::remove_dir_all(&root).ok();
}

/// A7-4: a cancellation between the two ports used to return through `?`
/// and skip the rollback entirely — leaving `tcp:9943` on the device with
/// nothing on disk naming it, which is exactly the stale forward that
/// silently breaks the next WiFi run.
#[tokio::test]
async fn a_cancellation_mid_loop_still_rolls_the_first_forward_back() {
    let root = scratch("wired-cancel");
    let (mut ctx, _) = real_ctx(
        &root,
        StageOptions {
            wired: true,
            ..Default::default()
        },
    );
    ctx.paths.adb = Some(slow_second_port_adb(
        &root,
        "List of devices attached\n1WMHH0X\tdevice\n",
    ));

    // Cancel once the fake adb is *inside* the second port's `forward` (it
    // drops a marker before sleeping): a fixed timer either fires before the
    // first forward exists or wastes real seconds.
    let cancel = ctx.cancel.clone();
    let marker = root.join("adb.second");
    tokio::spawn(async move {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !marker.exists() && std::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        cancel.cancel();
    });

    let mut sess = fresh_state();
    let state_path = ctx.paths.session_state_path();
    let err = adb_forward_hygiene(&ctx, &mut sess, &state_path)
        .await
        .unwrap_err();
    assert!(matches!(err, SabrageError::Cancelled), "{err:?}");
    let calls = adb_calls(&root);
    assert!(
        calls
            .iter()
            .any(|c| c.contains("forward --remove tcp:9943")),
        "the rollback must run on a fresh executor, not the cancelled one: {calls:?}"
    );
    assert!(
        sess.wired_forwards.is_empty(),
        "the in-memory record must reflect the rollback"
    );
    let on_disk = state::load(&state_path).unwrap().unwrap();
    assert!(
        on_disk.wired_forwards.is_empty(),
        "the persisted record is fixed up on the fresh executor too: {:?}",
        on_disk.wired_forwards
    );
    std::fs::remove_dir_all(&root).ok();
}

/// A failed write-before-mutate save for the SECOND port leaves through the
/// same door as a failed `adb forward`: the first forward comes back down
/// and the in-memory record says so.
#[tokio::test]
async fn a_failed_save_between_the_ports_still_rolls_the_first_forward_back() {
    let root = scratch("wired-save-fail");
    let (mut ctx, _) = real_ctx(
        &root,
        StageOptions {
            wired: true,
            ..Default::default()
        },
    );
    let state_path = ctx.paths.session_state_path();
    let state_dir = state_path.parent().unwrap().to_path_buf();
    std::fs::create_dir_all(&state_dir).unwrap();
    ctx.paths.adb = Some(readonly_state_dir_after_first_port_adb(
        &root,
        "List of devices attached\n1WMHH0X\tdevice\n",
        &state_dir,
    ));

    let mut sess = fresh_state();
    let err = adb_forward_hygiene(&ctx, &mut sess, &state_path)
        .await
        .unwrap_err();
    // Restore before asserting so a failure still cleans up its scratch.
    std::fs::set_permissions(
        &state_dir,
        <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o755),
    )
    .unwrap();
    assert!(
        !matches!(err, SabrageError::Cancelled),
        "a disk error is not a cancellation: {err:?}"
    );
    let calls = adb_calls(&root);
    assert!(
        calls
            .iter()
            .any(|c| c.contains("forward tcp:9943 tcp:9943")),
        "the first forward must have gone up before the save failed: {calls:?}"
    );
    assert!(
        !calls
            .iter()
            .any(|c| c.contains("forward tcp:9944 tcp:9944")),
        "the second forward must never be attempted after its save failed: {calls:?}"
    );
    assert!(
        calls
            .iter()
            .any(|c| c.contains("forward --remove tcp:9943")),
        "the failed save must roll the first forward back: {calls:?}"
    );
    assert!(
        sess.wired_forwards.is_empty(),
        "the in-memory record must reflect the rollback"
    );
    std::fs::remove_dir_all(&root).ok();
}

/// A cancel landing on the `adb devices` probe reads as a cancellation,
/// not as "no Quest over adb" — and the probe itself is interruptible.
#[tokio::test]
async fn a_cancel_during_the_device_probe_is_a_cancellation() {
    let root = scratch("wired-probe-cancel");
    let (mut ctx, _) = real_ctx(
        &root,
        StageOptions {
            wired: true,
            ..Default::default()
        },
    );
    ctx.paths.adb = Some(slow_devices_adb(&root));

    let cancel = ctx.cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        cancel.cancel();
    });

    let mut sess = fresh_state();
    let state_path = ctx.paths.session_state_path();
    let err = adb_forward_hygiene(&ctx, &mut sess, &state_path)
        .await
        .unwrap_err();
    assert!(matches!(err, SabrageError::Cancelled), "{err:?}");
    std::fs::remove_dir_all(&root).ok();
}

/// An `adb` whose `devices` never answers.
fn slow_devices_adb(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("adb");
    std::fs::write(&path, "#!/bin/sh\nsleep 30\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// [`fake_adb`], but `tcp:9944` hangs — so a cancellation can land between
/// the two forwards.
fn slow_second_port_adb(dir: &Path, devices_stdout: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("adb");
    std::fs::write(
            &path,
            format!(
                "#!/bin/sh\n\
                 for a in \"$@\"; do\n\
                 \x20 case \"$a\" in devices) printf '%s' '{devices_stdout}'; exit 0;; esac\n\
                 done\n\
                 echo \"$@\" >> \"$(dirname \"$0\")/adb.calls\"\n\
                 case \"$*\" in *--remove*) ;; *9944*) : > \"$(dirname \"$0\")/adb.second\"; sleep 30;; esac\n\
                 exit 0\n"
            ),
        )
        .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// Like [`slow_second_port_adb`], but instead of sleeping it turns the
/// session-state directory read-only as a side effect of the FIRST port's
/// `forward`, so the record write for the second port fails while
/// `tcp:9943` is live on the "device".
fn readonly_state_dir_after_first_port_adb(
    dir: &Path,
    devices_stdout: &str,
    state_dir: &Path,
) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("adb");
    let state_dir = state_dir.display();
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\n\
                 for a in \"$@\"; do\n\
                 \x20 case \"$a\" in devices) printf '%s' '{devices_stdout}'; exit 0;; esac\n\
                 done\n\
                 echo \"$@\" >> \"$(dirname \"$0\")/adb.calls\"\n\
                 case \"$*\" in *--remove*) ;; *9943*) chmod 0555 '{state_dir}';; esac\n\
                 exit 0\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// `bs_dir` with a `steam_api64.dll` under the Plugins path, plus the
/// Goldberg dll at `third_party/gbe/`.
fn goldberg_fixture(root: &Path, api_bytes: &[u8], gbe_bytes: &[u8]) -> PathBuf {
    let bs_dir = root.join("Beat Saber 1294");
    let plugins = bs_dir.join("Beat Saber_Data/Plugins/x86_64");
    std::fs::create_dir_all(&plugins).unwrap();
    std::fs::write(plugins.join("steam_api64.dll"), api_bytes).unwrap();
    let gbe = root.join("third_party/gbe");
    std::fs::create_dir_all(&gbe).unwrap();
    std::fs::write(gbe.join("steam_api64.dll"), gbe_bytes).unwrap();
    bs_dir
}

fn goldberg_ctx(root: &Path, bs_dir: PathBuf) -> (StageCtx, Arc<Mutex<Vec<StageEvent>>>) {
    let (mut ctx, seen) = dry_ctx(
        root,
        StageOptions {
            bs_dir_override: Some(bs_dir),
            ..Default::default()
        },
    );
    // `Paths::new(root)` already points gbe_dll at <root>/third_party/gbe.
    ctx.bs_dir = ctx.opts.bs_dir_override.clone().unwrap();
    (ctx, seen)
}

#[tokio::test]
async fn goldberg_dies_when_no_steam_api_dll_exists() {
    let root = scratch("gbe-missing");
    let bs_dir = root.join("empty");
    std::fs::create_dir_all(&bs_dir).unwrap();
    let (ctx, _) = goldberg_ctx(&root, bs_dir.clone());
    let err = goldberg_stage(&ctx).await.unwrap_err();
    assert_eq!(
        err.to_string(),
        format!(
            "steam_api64.dll not found under {} — is this a complete Beat Saber install?",
            bs_dir.display()
        )
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[tokio::test]
async fn goldberg_backs_up_once_installs_and_writes_the_exact_artifacts() {
    let root = scratch("gbe-install");
    let bs_dir = goldberg_fixture(&root, b"REAL-STEAM", b"GOLDBERG");
    let (ctx, seen) = goldberg_ctx(&root, bs_dir.clone());

    goldberg_stage(&ctx).await.unwrap();

    let api_dir = bs_dir.join("Beat Saber_Data/Plugins/x86_64");
    let plan = ctx.executor.planned();
    let describe: Vec<String> = plan.iter().map(|p| p.describe()).collect();

    // 1. backup, 2. install, 3. appid, 4. mkdir, 5-7. flag files.
    assert_eq!(
        plan.iter().map(|p| p.kind).collect::<Vec<_>>(),
        vec![
            PlannedKind::Copy,
            PlannedKind::Copy,
            PlannedKind::Write,
            PlannedKind::CreateDir,
            PlannedKind::Write,
            PlannedKind::Write,
            PlannedKind::Write,
        ],
        "{describe:#?}"
    );
    assert_eq!(
        plan[0].dst.as_deref(),
        Some(api_dir.join("steam_api64.dll.orig-steam").as_path())
    );
    assert_eq!(
        plan[1].src.as_deref(),
        Some(root.join("third_party/gbe/steam_api64.dll").as_path())
    );
    assert_eq!(
        plan[2].dst.as_deref(),
        Some(api_dir.join("steam_appid.txt").as_path())
    );
    assert_eq!(
        plan[3].dst.as_deref(),
        Some(api_dir.join("steam_settings").as_path())
    );
    for (p, name) in plan[4..].iter().zip(GOLDBERG_FLAG_FILES) {
        assert_eq!(
            p.dst.as_deref(),
            Some(api_dir.join("steam_settings").join(name).as_path())
        );
    }

    assert_eq!(
        texts(&seen.lock().unwrap()),
        vec![
            "-- Goldberg".to_string(),
            format!(
                "[ok] installed goldberg -> {}",
                api_dir.join("steam_api64.dll").display()
            ),
        ]
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[tokio::test]
async fn goldberg_skips_the_backup_when_one_exists_and_reports_already_installed() {
    let root = scratch("gbe-idempotent");
    let bs_dir = goldberg_fixture(&root, b"GOLDBERG", b"GOLDBERG");
    let api_dir = bs_dir.join("Beat Saber_Data/Plugins/x86_64");
    std::fs::write(api_dir.join("steam_api64.dll.orig-steam"), b"REAL-STEAM").unwrap();
    let (ctx, seen) = goldberg_ctx(&root, bs_dir);

    goldberg_stage(&ctx).await.unwrap();

    // No backup copy, no install copy — only the appid + settings writes.
    let plan = ctx.executor.planned();
    assert_eq!(
        plan.iter().map(|p| p.kind).collect::<Vec<_>>(),
        vec![
            PlannedKind::Write,
            PlannedKind::CreateDir,
            PlannedKind::Write,
            PlannedKind::Write,
            PlannedKind::Write,
        ]
    );
    assert_eq!(
        texts(&seen.lock().unwrap()),
        vec![
            "-- Goldberg".to_string(),
            "[info] goldberg already installed".to_string()
        ]
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[tokio::test]
async fn goldberg_installs_the_dll_and_flag_files_and_never_refreshes_the_backup() {
    // Which bytes ended up where is the point, so this one runs for real —
    // against a fixture tree under the temp dir, never the user's game.
    let root = scratch("gbe-bytes");
    let bs_dir = goldberg_fixture(&root, b"REAL-STEAM", b"GOLDBERG");
    let api_dir = bs_dir.join("Beat Saber_Data/Plugins/x86_64");
    let (mut ctx, _) = goldberg_ctx(&root, bs_dir.clone());
    let run_id = ctx.run_id;
    ctx.executor = Arc::new(crate::executor::RealExecutor::new(
        run_id,
        crate::stages::null_sink(),
        CancellationToken::new(),
    ));

    goldberg_stage(&ctx).await.unwrap();

    assert_eq!(
        std::fs::read(api_dir.join("steam_api64.dll.orig-steam")).unwrap(),
        b"REAL-STEAM",
        "the backup holds the ORIGINAL dll"
    );
    assert_eq!(
        std::fs::read(api_dir.join("steam_api64.dll")).unwrap(),
        b"GOLDBERG"
    );
    for name in GOLDBERG_FLAG_FILES {
        let p = api_dir.join("steam_settings").join(name);
        assert!(p.is_file(), "{name} missing");
        assert_eq!(std::fs::read(&p).unwrap(), b"", "{name} must be empty");
    }

    // Second pass: the backup is not refreshed with the Goldberg dll.
    goldberg_stage(&ctx).await.unwrap();
    assert_eq!(
        std::fs::read(api_dir.join("steam_api64.dll.orig-steam")).unwrap(),
        b"REAL-STEAM"
    );
    std::fs::remove_dir_all(&root).unwrap();
}

/// A7-5: the live dll is already Goldberg and there is no `.orig-steam`.
/// run.sh's bytes are unchanged (the backup is still minted — artifact
/// parity), but the row says what that backup actually holds, so nothing
/// downstream can call copying it back a restore.
#[tokio::test]
async fn goldberg_says_so_when_the_backup_it_mints_is_itself_goldberg() {
    let root = scratch("gbe-already");
    let bs_dir = goldberg_fixture(&root, b"GOLDBERG", b"GOLDBERG");
    let (ctx, seen) = goldberg_ctx(&root, bs_dir.clone());

    goldberg_stage(&ctx).await.unwrap();

    let api_dir = bs_dir.join("Beat Saber_Data/Plugins/x86_64");
    let backup = api_dir.join("steam_api64.dll.orig-steam");
    assert_eq!(
        texts(&seen.lock().unwrap()),
        vec![
            "-- Goldberg".to_string(),
            format!(
                "[warn] steam_api64.dll was already the Goldberg build, so {} is a copy of \
                     Goldberg — the real Steam dll was never seen here and cannot be restored",
                backup.display()
            ),
            "[info] goldberg already installed".to_string(),
        ]
    );
    // The backup is still planned — run.sh mints it here too.
    let plan = ctx.executor.planned();
    assert_eq!(plan[0].kind, PlannedKind::Copy);
    assert_eq!(plan[0].dst.as_deref(), Some(backup.as_path()));
    std::fs::remove_dir_all(&root).ok();
}

/// A7-5: a `.orig-steam` that is not a regular file (here: a directory)
/// used to skip the backup and then install Goldberg anyway — the only
/// local copy of the real Steam dll gone, with the stage reporting
/// success. Fail closed instead, before the live dll is touched.
#[tokio::test]
async fn goldberg_refuses_when_the_backup_name_is_not_a_regular_file() {
    let root = scratch("gbe-backup-conflict");
    let bs_dir = goldberg_fixture(&root, b"REAL-STEAM", b"GOLDBERG");
    let api_dir = bs_dir.join("Beat Saber_Data/Plugins/x86_64");
    let backup = api_dir.join("steam_api64.dll.orig-steam");
    std::fs::create_dir(&backup).unwrap();

    // A real executor: the point is that nothing on disk changes.
    let (mut ctx, _) = goldberg_ctx(&root, bs_dir.clone());
    let run_id = ctx.run_id;
    ctx.executor = Arc::new(crate::executor::RealExecutor::new(
        run_id,
        crate::stages::null_sink(),
        CancellationToken::new(),
    ));

    let err = goldberg_stage(&ctx).await.unwrap_err();
    assert!(
        err.to_string().contains(&backup.display().to_string()),
        "the die must name the offending path: {err}"
    );
    assert_eq!(
        std::fs::read(api_dir.join("steam_api64.dll")).unwrap(),
        b"REAL-STEAM",
        "the live dll must not be overwritten without a usable backup"
    );
    assert!(backup.is_dir(), "nothing was written over the conflict");
    std::fs::remove_dir_all(&root).ok();
}

/// A7-3 / A13a-1: when the minted backup is itself a Goldberg build, that
/// fact is *recorded* — a transient warn row cannot be consulted by
/// `store::goldberg`'s revert, which otherwise only recognises the build
/// its own pin names.
#[tokio::test]
async fn goldberg_records_the_provenance_of_a_backup_that_is_itself_goldberg() {
    let root = scratch("gbe-provenance");
    // An UNPINNED Goldberg build: `already_goldberg` compares against the
    // configured gbe_dll, so this is recognised where a pin would not be.
    let bs_dir = goldberg_fixture(&root, b"SOME-OTHER-GOLDBERG", b"SOME-OTHER-GOLDBERG");
    let api_dir = bs_dir.join("Beat Saber_Data/Plugins/x86_64");
    let backup = api_dir.join("steam_api64.dll.orig-steam");
    let (mut ctx, _) = goldberg_ctx(&root, bs_dir.clone());
    let run_id = ctx.run_id;
    ctx.executor = Arc::new(crate::executor::RealExecutor::new(
        run_id,
        crate::stages::null_sink(),
        CancellationToken::new(),
    ));

    assert!(
        !goldberg_backup_is_goldberg(&ctx.paths, &backup),
        "no record before the stage runs"
    );
    goldberg_stage(&ctx).await.unwrap();

    assert!(
        goldberg_backup_is_goldberg(&ctx.paths, &backup),
        "the poisoned backup must leave a durable record"
    );
    let marker = goldberg_backup_marker(&ctx.paths, &backup);
    assert!(
        marker.starts_with(&ctx.paths.sabrage_appsup),
        "the record lives in Sabrage's own store, never in the game dir: {}",
        marker.display()
    );
    assert!(
        !api_dir
            .join("steam_api64.dll.orig-steam.provenance")
            .exists(),
        "no Sabrage-only artifact next to the game's dlls"
    );
    std::fs::remove_dir_all(&root).ok();
}

/// The other half of the pin: an ordinary Steam backup leaves NO record,
/// so the marker cannot start refusing legitimate reverts.
#[tokio::test]
async fn goldberg_records_nothing_for_an_ordinary_steam_backup() {
    let root = scratch("gbe-provenance-clean");
    let bs_dir = goldberg_fixture(&root, b"REAL-STEAM", b"GOLDBERG");
    let backup = bs_dir
        .join("Beat Saber_Data/Plugins/x86_64")
        .join("steam_api64.dll.orig-steam");
    let (mut ctx, _) = goldberg_ctx(&root, bs_dir.clone());
    let run_id = ctx.run_id;
    ctx.executor = Arc::new(crate::executor::RealExecutor::new(
        run_id,
        crate::stages::null_sink(),
        CancellationToken::new(),
    ));

    goldberg_stage(&ctx).await.unwrap();
    assert!(backup.is_file(), "the stage minted the backup");
    assert!(!goldberg_backup_is_goldberg(&ctx.paths, &backup));
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn goldberg_falls_back_to_the_game_root_dll() {
    let root = scratch("gbe-root-dll");
    let bs_dir = root.join("Beat Saber 1294");
    std::fs::create_dir_all(&bs_dir).unwrap();
    std::fs::write(bs_dir.join("steam_api64.dll"), b"REAL").unwrap();
    std::fs::create_dir_all(root.join("third_party/gbe")).unwrap();
    std::fs::write(root.join("third_party/gbe/steam_api64.dll"), b"GBE").unwrap();
    let (ctx, _) = goldberg_ctx(&root, bs_dir.clone());

    goldberg_stage(&ctx).await.unwrap();
    let plan = ctx.executor.planned();
    assert_eq!(
        plan[2].dst.as_deref(),
        Some(bs_dir.join("steam_appid.txt").as_path()),
        "steam_appid.txt lands next to the dll that was found"
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn wine_spec_is_run_shs_argv() {
    let root = scratch("wine-spec");
    let b = bottle(&root);
    let bs_dir = b
        .prefix
        .join("drive_c/Program Files (x86)/Steam/steamapps/common/BS");
    let (mut ctx, _) = dry_ctx(
        &root,
        StageOptions {
            bottle_name: Some("Steam".into()),
            bs_dir_override: Some(bs_dir.clone()),
            ..Default::default()
        },
    );
    ctx.bs_dir = bs_dir;
    ctx.paths.wine = Some(PathBuf::from("/Applications/CrossOver.app/x/bin/wine"));

    let spec = wine_spec(&ctx, &b);
    assert_eq!(
        spec.display(),
        "/Applications/CrossOver.app/x/bin/wine --bottle Steam --no-update --cx-app \
             C:\\Program Files (x86)\\Steam\\steamapps\\common\\BS\\Beat Saber.exe"
    );
    assert_eq!(spec.step, step::RUN_LAUNCH);
    let keys: Vec<&str> = spec.env.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(
        keys,
        vec![
            "XR_RUNTIME_JSON",
            "CX_GRAPHICS_BACKEND",
            "WINEDEBUG",
            "SteamAppId",
            "SteamGameId"
        ]
    );
    assert!(spec.env_path.is_some(), "a Finder-launched .app needs PATH");
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn the_banner_is_one_section_with_every_text_row_on_the_launch_step() {
    let evs = banner_events(
        Uuid::nil(),
        "Steam",
        "Z:\\games\\Beat Saber.exe",
        Path::new("/repo/logs/beatsaber-20260829-101112.log"),
    );
    // Only the banner headline is a Section; everything else is verbatim Text.
    assert_eq!(
        evs.iter()
            .filter(|e| matches!(e, StageEvent::Section { .. }))
            .count(),
        1
    );
    for ev in &evs {
        if matches!(ev, StageEvent::Text { .. }) {
            assert_eq!(ev.step(), Some(step::RUN_LAUNCH));
        }
    }
}

#[test]
fn eexist_is_the_only_retryable_spawn_error() {
    assert!(is_already_exists(&SabrageError::io(
        "/x",
        std::io::Error::new(std::io::ErrorKind::AlreadyExists, "exists")
    )));
    assert!(!is_already_exists(&SabrageError::io(
        "/x",
        std::io::Error::new(std::io::ErrorKind::PermissionDenied, "nope")
    )));
    assert!(!is_already_exists(&SabrageError::Cancelled));
}

#[test]
fn survivors_render_as_pid_basename_pairs_with_a_trailing_space() {
    let procs = vec![
        ProcInfo {
            pid: 12,
            start_time: 1,
            exe: PathBuf::from("/x/bin/wineserver"),
        },
        ProcInfo {
            pid: 34,
            start_time: 1,
            exe: PathBuf::new(),
        },
    ];
    assert_eq!(
        format_survivors(&procs, "wineserver"),
        "12 wineserver 34 wineserver "
    );
    assert_eq!(format_survivors(&[], "wineserver"), "");
}

#[tokio::test]
async fn wineserver_reset_plans_k_then_w_and_reports_down() {
    let root = scratch("ws-reset");
    let b = bottle(&root);
    let (mut ctx, seen) = dry_ctx(&root, StageOptions::default());
    ctx.paths.wineserver = Some(PathBuf::from("/cx/bin/wineserver"));

    wineserver_reset(&ctx, &b).await.unwrap();

    let plan = ctx.executor.planned();
    assert_eq!(
        plan.iter().map(|p| p.reason.clone()).collect::<Vec<_>>(),
        vec![
            "/cx/bin/wineserver -k".to_string(),
            "/cx/bin/wineserver -w".to_string()
        ]
    );
    assert_eq!(
        texts(&seen.lock().unwrap()),
        vec![
            "-- resetting wineserver for bottle 'Steam'".to_string(),
            "[ok] wineserver down".to_string()
        ]
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[tokio::test]
async fn wineserver_reset_without_crossover_still_reports_down() {
    let root = scratch("ws-none");
    let b = bottle(&root);
    let (ctx, seen) = dry_ctx(&root, StageOptions::default());
    wineserver_reset(&ctx, &b).await.unwrap();
    assert!(ctx.executor.planned().is_empty());
    assert_eq!(
        texts(&seen.lock().unwrap()).last().map(String::as_str),
        Some("[ok] wineserver down")
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[tokio::test]
async fn adb_reverse_cleanup_is_silent_without_adb_or_on_the_legacy_protocol() {
    let root = scratch("reverse");
    let (ctx, seen) = dry_ctx(&root, StageOptions::default());
    let alvr = PreflightFacts {
        protocol: "alvr".into(),
        encoder_process: "auto".into(),
    };
    adb_reverse_cleanup(&ctx, &alvr).await.unwrap();
    assert!(seen.lock().unwrap().is_empty());

    let legacy = PreflightFacts {
        protocol: "oxrsys".into(),
        encoder_process: "auto".into(),
    };
    adb_reverse_cleanup(&ctx, &legacy).await.unwrap();
    assert!(seen.lock().unwrap().is_empty());
    assert!(ctx.executor.planned().is_empty());
    std::fs::remove_dir_all(&root).unwrap();
}

#[tokio::test]
async fn adb_reverse_cleanup_removes_all_reverse_tunnels_on_the_alvr_path() {
    let root = scratch("reverse-alvr");
    let (mut ctx, seen) = dry_ctx(&root, StageOptions::default());
    ctx.paths.adb = Some(fake_adb(
        &root,
        "List of devices attached\n1WMHH0X\tdevice\n",
        0,
    ));
    adb_reverse_cleanup(
        &ctx,
        &PreflightFacts {
            protocol: "alvr".into(),
            encoder_process: "auto".into(),
        },
    )
    .await
    .unwrap();

    let plan = ctx.executor.planned();
    assert_eq!(plan.len(), 1);
    assert!(
        plan[0].reason.ends_with("-s 1WMHH0X reverse --remove-all"),
        "{}",
        plan[0].reason
    );
    assert_eq!(
        texts(&seen.lock().unwrap()),
        vec![
            "[info] Quest 1WMHH0X: cleared adb reverse tunnels (ALVR manages its own)".to_string()
        ]
    );
    assert_eq!(
        seen.lock().unwrap()[0].step(),
        Some(step::RUN_ADB_REVERSE),
        "rows are attributed to their launch action's step"
    );
    // Severity is `info`, matching scripts/demo/run.sh # launch-action: adb-reverse-cleanup.
    assert!(matches!(
        &seen.lock().unwrap()[0],
        StageEvent::Line {
            severity: Severity::Info,
            ..
        }
    ));
    std::fs::remove_dir_all(&root).unwrap();
}
