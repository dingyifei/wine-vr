use super::*;
use crate::events::{RunId, Severity};
use crate::executor::{DryRunExecutor, PlannedKind};
use crate::paths::Paths;
use crate::stages::{EventSink, StageOptions};
use std::sync::Mutex;
use uuid::Uuid;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "sabrage-run-mod-{tag}-{}-{}",
        std::process::id(),
        Uuid::new_v4().as_simple()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Fixture context: every path under `root`, a dry-run executor, no
/// CrossOver, no adb, no dashboard binary. Nothing here can reach the real
/// machine.
fn dry_ctx(root: &Path, opts: StageOptions) -> (StageCtx, Arc<Mutex<Vec<StageEvent>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s = seen.clone();
    let sink: EventSink = Arc::new(move |ev| s.lock().unwrap().push(ev));
    let run_id: RunId = Uuid::new_v4();
    let cancel = CancellationToken::new();
    let mut paths = Paths::new(root);
    paths.oxr_appsup = root.join("appsup-oxrsys");
    paths.sabrage_appsup = root.join("appsup-sabrage");
    paths.adb = None;
    paths.wine = None;
    paths.wineserver = None;
    let executor: Arc<dyn Executor> =
        Arc::new(DryRunExecutor::new(run_id, sink.clone(), cancel.clone()));
    let ctx = StageCtx::with_executor(paths, opts, sink, cancel, executor, run_id);
    (ctx, seen)
}

fn bottle(root: &Path) -> Bottle {
    Bottle {
        name: "Steam".to_string(),
        prefix: root.join("bottle"),
        sys32: root.join("bottle/drive_c/windows/system32"),
    }
}

fn facts() -> PreflightFacts {
    PreflightFacts {
        protocol: "alvr".to_string(),
        encoder_process: "auto".to_string(),
    }
}

fn fresh(root: &Path) -> SessionState {
    SessionState::new(Uuid::nil(), "Steam", root, PathBuf::new(), 1786300214181)
}

fn rows(evs: &[StageEvent]) -> Vec<String> {
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
fn the_closing_lines_are_run_shs_verbatim() {
    assert_eq!(
        wine_exit_line(0, Path::new("/repo/logs/beatsaber-20260829-101112.log")),
        "wine exited with status 0 (log: /repo/logs/beatsaber-20260829-101112.log)"
    );
    // wine's own status is propagated unchanged.
    assert_eq!(
        wine_exit_line(139, Path::new("/l.log")),
        "wine exited with status 139 (log: /l.log)"
    );
    assert_eq!(
        detached_line(Path::new("/l.log")),
        "-- detached: leaving the session running (log: /l.log)"
    );
}

#[test]
fn the_already_running_refusal_names_the_pid_the_time_and_both_stop_routes() {
    let root = scratch("already-running");
    let (ctx, seen) = dry_ctx(&root, StageOptions::default());
    let mut sess = fresh(&root);
    sess.wine = Some(ProcInfo {
        pid: 59004,
        start_time: 1786300214,
        exe: PathBuf::from("/cx/bin/wine"),
    });
    let err = already_running(&ctx, &bottle(&root), &sess);
    let msg = err.to_string();
    assert!(
        msg.starts_with("a session is already running (pid 59004, started "),
        "{msg}"
    );
    assert!(
        msg.ends_with(") — stop it first: Stop in Sabrage, or ./demo.sh stop --bottle Steam"),
        "{msg}"
    );
    // A `die` always announces itself.
    assert!(matches!(
        seen.lock().unwrap().last(),
        Some(StageEvent::Fatal { .. })
    ));
    assert_eq!(err.exit_code(), 1);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn iso_local_renders_a_civil_timestamp_and_never_panics() {
    let s = iso_local(1786300214181);
    assert!(s.starts_with("20"), "{s}");
    assert_eq!(s.matches(':').count(), 3, "date T time + offset: {s}");
    // Out-of-range input degrades to the raw number rather than panicking.
    let far = i64::MAX as u64;
    assert_eq!(iso_local(far), far.to_string());
}

#[tokio::test]
async fn teardown_gets_an_executor_a_fired_token_cannot_veto() {
    let root = scratch("teardown-ctx");
    let (ctx, _) = dry_ctx(&root, StageOptions::default());
    // A dry run keeps its own executor so the plan stays in one place.
    ctx.cancel.cancel();
    let t = teardown_ctx(&ctx);
    assert!(t.executor.is_dry_run());
    assert!(t.cancel.is_cancelled(), "dry run reuses ctx unchanged");

    // A real run gets a fresh token, so `wineserver -k` still runs.
    let mut real = ctx.clone();
    real.executor = Arc::new(RealExecutor::new(
        ctx.run_id,
        ctx.sink.clone(),
        ctx.cancel.clone(),
    ));
    let t = teardown_ctx(&real);
    assert!(!t.executor.is_dry_run());
    assert!(!t.cancel.is_cancelled());
    assert_eq!(t.run_id, ctx.run_id);
    assert_eq!(t.paths, ctx.paths);
    std::fs::remove_dir_all(&root).unwrap();
}

#[tokio::test]
async fn a_normal_exit_prints_the_blank_line_then_the_status() {
    let root = scratch("teardown-normal");
    let (ctx, seen) = dry_ctx(&root, StageOptions::default());
    let mut held = Guards::default();
    let mut sess = fresh(&root);
    let log = root.join("logs/beatsaber-20260829-101112.log");

    let _g = session::lock_session_globals();
    let mut phase = RunPhaseScope::new(ctx.run_id, "Steam");
    let rc = teardown(
        &ctx,
        &bottle(&root),
        &mut held,
        &mut sess,
        &ctx.paths.session_state_path(),
        Ok(Reason::Normal {
            rc: 0,
            log: log.clone(),
        }),
        &mut phase,
    )
    .await
    .unwrap();

    assert_eq!(rc, 0);
    assert_eq!(
        rows(&seen.lock().unwrap()),
        vec![String::new(), wine_exit_line(0, &log)]
    );
    // No `stop_wine` on the normal path — the bottle's wineserver stays up
    // (demo.sh parity), so nothing was planned at all.
    assert!(ctx.executor.planned().is_empty());
    std::fs::remove_dir_all(&root).unwrap();
}

#[tokio::test]
async fn a_normal_exits_guards_come_off_after_the_status_line_not_before_it() {
    // run.sh's EXIT trap fires only after the status line, so the restore
    // row is the LAST line of a clean quit, never the first.
    // Reference: scripts/demo/run.sh; trap order is on `Guards::release`.
    let root = scratch("teardown-normal-order");
    let (mut ctx, seen) = dry_ctx(&root, StageOptions::default());
    ctx.paths.wineserver = Some(PathBuf::from("/cx/bin/wineserver"));
    let mut sess = fresh(&root);
    let mut held = Guards {
        audio: Some(AudioGuard::armed_for_test(
            &ctx,
            "MacBook Pro Speakers",
            "/opt/homebrew/bin/SwitchAudioSource",
        )),
        dashboard: None,
    };
    let log = root.join("logs/beatsaber-20260829-101112.log");

    let _g = session::lock_session_globals();
    let mut phase = RunPhaseScope::new(ctx.run_id, "Steam");
    let rc = teardown(
        &ctx,
        &bottle(&root),
        &mut held,
        &mut sess,
        &ctx.paths.session_state_path(),
        Ok(Reason::Normal {
            rc: 0,
            log: log.clone(),
        }),
        &mut phase,
    )
    .await
    .unwrap();

    assert_eq!(rc, 0);
    assert_eq!(
        rows(&seen.lock().unwrap()),
        vec![
            String::new(),
            wine_exit_line(0, &log),
            "audio: restored output -> MacBook Pro Speakers".to_string(),
        ]
    );
    assert!(held.audio.is_none(), "the guard was released, not leaked");
    assert!(sess.guards.audio_restored);
    // Still no `stop_wine`, and this fixture can prove it: `dry_ctx` leaves
    // `paths.wineserver` None, so the fixture sets one up front — a stray
    // `wineserver -k` would now be planned, since the dry run records a
    // spawn's argv as the plan's reason.
    assert!(!ctx
        .executor
        .planned()
        .iter()
        .any(|p| p.reason.contains("wineserver")));
    std::fs::remove_dir_all(&root).unwrap();
}

/// A [`DryRunExecutor`] that refuses to `write_atomic` one particular
/// path. Everything else delegates, so teardown behaves exactly as it does
/// under a plain dry run and still touches nothing.
#[derive(Debug)]
struct DenyWriteTo {
    inner: Arc<dyn Executor>,
    deny: PathBuf,
}

impl DenyWriteTo {
    fn around(inner: Arc<dyn Executor>, deny: impl Into<PathBuf>) -> Arc<DenyWriteTo> {
        Arc::new(DenyWriteTo {
            inner,
            deny: deny.into(),
        })
    }
}

impl Executor for DenyWriteTo {
    fn with_step(&self, step: crate::events::StepId) -> Arc<dyn Executor> {
        Arc::new(DenyWriteTo {
            inner: self.inner.with_step(step),
            deny: self.deny.clone(),
        })
    }
    fn is_dry_run(&self) -> bool {
        self.inner.is_dry_run()
    }
    fn planned(&self) -> Vec<crate::executor::PlannedAction> {
        self.inner.planned()
    }
    fn copy_if_changed<'a>(
        &'a self,
        src: &'a Path,
        dst: &'a Path,
    ) -> crate::executor::BoxFuture<'a, Result<crate::executor::Copied>> {
        self.inner.copy_if_changed(src, dst)
    }
    fn write_atomic<'a>(
        &'a self,
        path: &'a Path,
        bytes: &'a [u8],
    ) -> crate::executor::BoxFuture<'a, Result<()>> {
        if path == self.deny {
            return Box::pin(async move {
                Err(SabrageError::io(
                    path,
                    std::io::Error::from(std::io::ErrorKind::PermissionDenied),
                ))
            });
        }
        self.inner.write_atomic(path, bytes)
    }
    fn remove_dir_all<'a>(&'a self, p: &'a Path) -> crate::executor::BoxFuture<'a, Result<()>> {
        self.inner.remove_dir_all(p)
    }
    fn remove_file<'a>(&'a self, p: &'a Path) -> crate::executor::BoxFuture<'a, Result<()>> {
        self.inner.remove_file(p)
    }
    fn create_dir_all<'a>(&'a self, p: &'a Path) -> crate::executor::BoxFuture<'a, Result<()>> {
        self.inner.create_dir_all(p)
    }
    fn dir_copy<'a>(
        &'a self,
        src: &'a Path,
        dst: &'a Path,
    ) -> crate::executor::BoxFuture<'a, Result<()>> {
        self.inner.dir_copy(src, dst)
    }
    fn download<'a>(
        &'a self,
        url: &'a str,
        dest: &'a Path,
        sha256: &'a str,
        label: &'a str,
    ) -> crate::executor::BoxFuture<'a, Result<crate::executor::Downloaded>> {
        self.inner.download(url, dest, sha256, label)
    }
    fn tar_xzf<'a>(
        &'a self,
        archive: &'a Path,
        into_dir: &'a Path,
    ) -> crate::executor::BoxFuture<'a, Result<()>> {
        self.inner.tar_xzf(archive, into_dir)
    }
    fn touch<'a>(&'a self, p: &'a Path) -> crate::executor::BoxFuture<'a, Result<()>> {
        self.inner.touch(p)
    }
    fn run_child<'a>(
        &'a self,
        spec: &'a crate::process::ChildSpec,
    ) -> crate::executor::BoxFuture<'a, Result<std::process::ExitStatus>> {
        self.inner.run_child(spec)
    }
    fn spawn_detached<'a>(
        &'a self,
        spec: &'a crate::process::ChildSpec,
        stdio: crate::executor::DetachedStdio,
    ) -> crate::executor::BoxFuture<'a, Result<Option<DetachedChild>>> {
        self.inner.spawn_detached(spec, stdio)
    }
}

/// #202: a clean quit whose `session-state.json` save fails stays a clean
/// quit. If `held.release(…)?` propagated, `clear_live_session` would never
/// run: the handle leaks for the app's lifetime, the next `stop_session`
/// spends its whole 30 s timeout on an already-fired token, and `run`
/// returns exit 1 for a wine process that exited 0.
#[tokio::test]
async fn a_normal_exit_survives_a_failed_state_save() {
    let root = scratch("teardown-normal-save-fails");
    let (mut ctx, seen) = dry_ctx(&root, StageOptions::default());
    let state_path = ctx.paths.session_state_path();
    ctx.executor = DenyWriteTo::around(ctx.executor.clone(), &state_path);

    // An armed audio guard, so the release genuinely reaches `state::save`
    // — which is the write the executor above refuses.
    let mut held = Guards {
        audio: Some(AudioGuard::armed_for_test(
            &ctx,
            "MacBook Pro Speakers",
            "/opt/homebrew/bin/SwitchAudioSource",
        )),
        dashboard: None,
    };
    let mut sess = fresh(&root);
    let log = root.join("logs/beatsaber-20260829-101112.log");

    let _g = session::lock_session_globals();
    session::set_live_session(LiveSessionHandle {
        run_id: ctx.run_id,
        bottle: "Steam".into(),
        identity: ProcInfo::observe(std::process::id()).unwrap(),
        log_path: log.clone(),
        started_at_unix_ms: 1786300214181,
        cancel: CancellationToken::new(),
        detach: CancellationToken::new(),
    });
    let mut phase = RunPhaseScope::new(ctx.run_id, "Steam");

    let rc = teardown(
        &ctx,
        &bottle(&root),
        &mut held,
        &mut sess,
        &state_path,
        Ok(Reason::Normal {
            rc: 3,
            log: log.clone(),
        }),
        &mut phase,
    )
    .await
    .expect("#202: a failed state save must not turn a clean quit into Err");

    assert_eq!(rc, 3, "wine's own status, not the save's failure");
    let printed = rows(&seen.lock().unwrap());
    assert!(
        printed.contains(&wine_exit_line(3, &log)),
        "the closing status line still lands: {printed:?}"
    );
    let warned = printed
        .iter()
        .find(|r| r.starts_with("[warn] could not save the session record"))
        .unwrap_or_else(|| panic!("#202: the save failure must be announced: {printed:?}"));
    assert!(
        warned.contains("session-state.json"),
        "the warn names the file that failed: {warned}"
    );
    assert!(
        session::live_session().is_none(),
        "#202: the live handle must be cleared even when the save failed"
    );
    drop(phase);
    std::fs::remove_dir_all(&root).unwrap();
}

/// A8-1: the record is only cleared once every guard it recorded is
/// released (PARITY.md § Session (detach / reconcile), "A **Dead** or
/// **IdentityMismatch** recorded session"). A device that could not be
/// switched back leaves `audio_restored` false — deleting the record then
/// leaves the Mac on `BlackHole 2ch` with nothing on disk to restore from.
#[tokio::test]
async fn a_teardown_with_an_unrestorable_guard_keeps_the_record() {
    let root = scratch("teardown-keeps-record");
    let (ctx, seen) = dry_ctx(&root, StageOptions::default());
    let mut held = Guards::default();
    let mut sess = fresh(&root);
    // The shape `AudioGuard::release` leaves behind when every switch
    // failed: a recorded device, and the flag still false.
    sess.prev_audio_output = Some("Yifei\u{2019}s AirPods Pro".into());
    assert!(teardown_pending(&sess));
    let log = root.join("l.log");

    let _g = session::lock_session_globals();
    let mut phase = RunPhaseScope::new(ctx.run_id, "Steam");
    let rc = teardown(
        &ctx,
        &bottle(&root),
        &mut held,
        &mut sess,
        &ctx.paths.session_state_path(),
        Ok(Reason::Normal { rc: 0, log }),
        &mut phase,
    )
    .await
    .unwrap();

    assert_eq!(rc, 0, "a kept record cannot change wine's status");
    let kinds: Vec<PlannedKind> = ctx.executor.planned().iter().map(|p| p.kind).collect();
    assert!(
        !kinds.contains(&PlannedKind::RemoveFile),
        "the record must survive the pending guard: {kinds:?}"
    );
    assert!(kinds.contains(&PlannedKind::Write), "…and be re-saved");
    assert!(
        rows(&seen.lock().unwrap()).contains(&format!("[info] {RECORD_KEPT_LINE}")),
        "the user is told why the record is still there"
    );
    std::fs::remove_dir_all(&root).unwrap();
}

/// The counter-test that keeps the gate honest: `run` records the `--wired`
/// forwards it creates and never removes them (permanent-vs-guarded), so
/// `has_pending_guards()` would keep the record after EVERY wired run.
/// Only the guards teardown actually releases may hold the record back.
#[tokio::test]
async fn a_wired_run_whose_guards_came_off_still_clears_the_record() {
    let root = scratch("teardown-wired-clears");
    let (ctx, _) = dry_ctx(&root, StageOptions::default());
    let mut held = Guards::default();
    let mut sess = fresh(&root);
    sess.wired_forwards = vec![crate::session::state::WiredForward {
        serial: "1WMHH000".into(),
        port: 9943,
    }];
    sess.prev_audio_output = Some("MacBook Pro Speakers".into());
    sess.guards.audio_restored = true;
    assert!(
        sess.has_pending_guards(),
        "the forwards are still on the phone — and that is not teardown's business"
    );
    assert!(!teardown_pending(&sess));
    // The record has to exist for a removal to be planned at all.
    std::fs::create_dir_all(&ctx.paths.sabrage_appsup).unwrap();
    std::fs::write(ctx.paths.session_state_path(), b"{}\n").unwrap();

    let _g = session::lock_session_globals();
    let mut phase = RunPhaseScope::new(ctx.run_id, "Steam");
    teardown(
        &ctx,
        &bottle(&root),
        &mut held,
        &mut sess,
        &ctx.paths.session_state_path(),
        Ok(Reason::Normal {
            rc: 0,
            log: root.join("l.log"),
        }),
        &mut phase,
    )
    .await
    .unwrap();

    let kinds: Vec<PlannedKind> = ctx.executor.planned().iter().map(|p| p.kind).collect();
    assert!(
        kinds.contains(&PlannedKind::RemoveFile),
        "nothing teardown owns is pending: the record goes: {kinds:?}"
    );
    std::fs::remove_dir_all(&root).unwrap();
}

/// A8-2 / A11-1: the INT path is best-effort exactly like the normal one.
/// A failed `session-state.json` write must not skip the reap, the record
/// or — above all — `clear_live_session`: a leaked handle turns every later
/// `stop_session` into a guaranteed 30 s timeout on an already-fired token.
#[tokio::test]
async fn a_cancelled_teardown_survives_a_failed_state_save() {
    let root = scratch("teardown-cancel-save-fails");
    let (mut ctx, seen) = dry_ctx(&root, StageOptions::default());
    let state_path = ctx.paths.session_state_path();
    ctx.executor = DenyWriteTo::around(ctx.executor.clone(), &state_path);
    ctx.paths.wineserver = Some(PathBuf::from("/cx/bin/wineserver"));

    let mut held = Guards {
        audio: Some(AudioGuard::armed_for_test(
            &ctx,
            "MacBook Pro Speakers",
            "/opt/homebrew/bin/SwitchAudioSource",
        )),
        dashboard: None,
    };
    let mut sess = fresh(&root);

    let _g = session::lock_session_globals();
    session::set_live_session(LiveSessionHandle {
        run_id: ctx.run_id,
        bottle: "Steam".into(),
        identity: ProcInfo::observe(std::process::id()).unwrap(),
        log_path: root.join("l.log"),
        started_at_unix_ms: 1786300214181,
        cancel: CancellationToken::new(),
        detach: CancellationToken::new(),
    });
    let mut phase = RunPhaseScope::new(ctx.run_id, "Steam");

    let err = teardown(
        &ctx,
        &bottle(&root),
        &mut held,
        &mut sess,
        &state_path,
        Err(SabrageError::Cancelled),
        &mut phase,
    )
    .await
    .unwrap_err();

    assert_eq!(err.exit_code(), 130, "still the shell's 130");
    let printed = rows(&seen.lock().unwrap());
    assert!(
        printed.contains(&"audio: restored output -> MacBook Pro Speakers".to_string()),
        "the device came back even though the record could not be written: {printed:?}"
    );
    assert!(
        printed
            .iter()
            .any(|r| r.starts_with("[warn] could not save the session record")),
        "the save failure is announced: {printed:?}"
    );
    assert!(
        session::live_session().is_none(),
        "the live handle must be cleared even when the save failed"
    );
    assert!(held.audio.is_none(), "the guard was released, not leaked");
    drop(phase);
    std::fs::remove_dir_all(&root).unwrap();
}

/// A8-2: detach persists FIRST and disarms second. A write that fails
/// leaves the guards armed — `run` drops them and each `Drop` fallback
/// undoes what it can — rather than a machine on BlackHole with nothing on
/// disk naming the device.
#[tokio::test]
async fn a_detach_that_cannot_write_its_record_keeps_the_guards_armed() {
    let root = scratch("teardown-detach-save-fails");
    let (mut ctx, seen) = dry_ctx(&root, StageOptions::default());
    let state_path = ctx.paths.session_state_path();
    ctx.executor = DenyWriteTo::around(ctx.executor.clone(), &state_path);

    let mut held = Guards {
        audio: Some(AudioGuard::armed_for_test(
            &ctx,
            "MacBook Pro Speakers",
            "/opt/homebrew/bin/SwitchAudioSource",
        )),
        dashboard: None,
    };
    let mut sess = fresh(&root);

    let _g = session::lock_session_globals();
    session::set_live_session(LiveSessionHandle {
        run_id: ctx.run_id,
        bottle: "Steam".into(),
        identity: ProcInfo::observe(std::process::id()).unwrap(),
        log_path: root.join("l.log"),
        started_at_unix_ms: 1786300214181,
        cancel: CancellationToken::new(),
        detach: CancellationToken::new(),
    });
    let mut phase = RunPhaseScope::new(ctx.run_id, "Steam");

    let rc = teardown(
        &ctx,
        &bottle(&root),
        &mut held,
        &mut sess,
        &state_path,
        Ok(Reason::Detached {
            log: root.join("l.log"),
        }),
        &mut phase,
    )
    .await
    .expect("detach still succeeds — the session keeps running");

    assert_eq!(rc, 0);
    assert!(
        sess.detached,
        "the flag is set before the write is attempted"
    );
    assert!(
        held.audio.is_some(),
        "a record that could not be written must not leave the guards unrecoverable"
    );
    let printed = rows(&seen.lock().unwrap());
    assert!(
        printed
            .iter()
            .any(|r| r.starts_with("[warn] could not record the detached session in ")),
        "{printed:?}"
    );
    assert!(
        session::live_session().is_none(),
        "the live handle is cleared on every teardown arm"
    );
    drop(phase);
    std::fs::remove_dir_all(&root).unwrap();
}

/// A7-1: a launch refused for a live session must change nothing
/// (PARITY.md § Session (detach / reconcile), "A recorded **Live**
/// session"). Reconciliation therefore runs BEFORE `preflight::run`,
/// whose two auto-fixes (the `cxbottle.conf` backend line, the helper
/// restage) are permanent and never unwound.
#[tokio::test]
async fn a_live_session_refuses_before_the_preflight_runs_a_single_check() {
    let _g = session::lock_session_globals();
    session::publish_run_phase(None);

    let root = scratch("run-live-refusal");
    let (mut ctx, seen) = dry_ctx(
        &root,
        StageOptions {
            bottle_name: Some("Steam".to_string()),
            bs_dir_override: Some(root.join("BeatSaber")),
            dry_run: true,
            ..Default::default()
        },
    );
    ctx.bottle = Some(bottle(&root));

    // A record whose wine identity is *this* process: alive, same start
    // time — `classify` says Live.
    let mut recorded = fresh(&root);
    recorded.wine = Some(ProcInfo::observe(std::process::id()).unwrap());
    std::fs::create_dir_all(&ctx.paths.sabrage_appsup).unwrap();
    std::fs::write(
        ctx.paths.session_state_path(),
        serde_json::to_vec_pretty(&recorded).unwrap(),
    )
    .unwrap();

    let err = run(&ctx, None).await.unwrap_err();
    assert!(
        err.to_string().starts_with("a session is already running"),
        "{err}"
    );
    let evs = seen.lock().unwrap().clone();
    assert!(
        !evs.iter().any(|e| matches!(e, StageEvent::Check { .. })),
        "not one preflight check ran: {:?}",
        rows(&evs)
    );
    assert!(
        ctx.executor.planned().is_empty(),
        "and nothing permanent was even planned: {:?}",
        ctx.executor.planned()
    );
    assert!(session::run_phase().is_none(), "the slot is emptied");
    std::fs::remove_dir_all(&root).ok();
}

/// A8-1: `reconcile` reads `session-state.json`, and a `./demo.sh run`
/// session writes no such file — the only trace it leaves on this machine
/// is a fresh `runtime_status.json`. A launch that ignored that trace would
/// walk into `wineserver_reset` and take the running game down.
#[tokio::test]
async fn a_shell_started_session_refuses_the_launch_before_anything_permanent() {
    let _g = session::lock_session_globals();
    session::publish_run_phase(None);

    let root = scratch("run-external-refusal");
    let (mut ctx, seen) = dry_ctx(
        &root,
        StageOptions {
            bottle_name: Some("Steam".to_string()),
            bs_dir_override: Some(root.join("BeatSaber")),
            dry_run: true,
            ..Default::default()
        },
    );
    ctx.bottle = Some(bottle(&root));

    // What oxrsys writes while it is streaming — and nothing else: no
    // session-state.json at all.
    std::fs::create_dir_all(&ctx.paths.oxr_appsup).unwrap();
    std::fs::write(
        ctx.paths.oxr_appsup.join("runtime_status.json"),
        format!(
            r#"{{"state":"streaming","process_id":{},"updated_at_unix_ms":{}}}"#,
            std::process::id(),
            session::now_unix_ms()
        ),
    )
    .unwrap();
    assert!(
        !ctx.paths.session_state_path().exists(),
        "the shell front-end leaves no record — that is the whole point"
    );

    let err = run(&ctx, None).await.unwrap_err();
    assert!(
        err.to_string().starts_with("a session is already running")
            && err.to_string().contains("the oxrsys runtime is reporting"),
        "{err}"
    );
    let evs = seen.lock().unwrap().clone();
    assert!(
        !evs.iter().any(|e| matches!(e, StageEvent::Check { .. })),
        "not one preflight check ran: {:?}",
        rows(&evs)
    );
    assert!(
        ctx.executor.planned().is_empty(),
        "no adb hygiene, no wineserver reset, no Goldberg swap: {:?}",
        ctx.executor.planned()
    );
    assert!(session::run_phase().is_none(), "the slot is emptied");
    std::fs::remove_dir_all(&root).ok();
}

/// A9-1 / A9-8: `reconcile` classifies a record it may not touch as
/// `Busy` and leaves the file alone. A launch that read that as "nothing
/// to carry" would keep going — through the bottle-scoped `wineserver -k`
/// that kills the very session the classification protects.
#[tokio::test]
async fn a_record_another_live_front_end_owns_refuses_the_launch() {
    let _g = session::lock_session_globals();
    session::publish_run_phase(None);

    let root = scratch("run-busy-refusal");
    let (mut ctx, seen) = dry_ctx(
        &root,
        StageOptions {
            bottle_name: Some("Steam".to_string()),
            bs_dir_override: Some(root.join("BeatSaber")),
            dry_run: true,
            ..Default::default()
        },
    );
    ctx.bottle = Some(bottle(&root));

    // A real, live process that is not this one — "another Sabrage is
    // running this session". No `wine` identity yet, i.e. the pre-spawn
    // window `owner_pid` exists for, so `classify` alone would call this
    // record Dead and reconcile would restore its guards.
    let mut owner = std::process::Command::new("/bin/sleep")
        .arg("30")
        .spawn()
        .unwrap();
    let mut foreign = fresh(&root);
    foreign.set_owner(owner.id());
    foreign.prev_audio_output = Some("MacBook Pro Speakers".into());
    std::fs::create_dir_all(&ctx.paths.sabrage_appsup).unwrap();
    std::fs::write(
        ctx.paths.session_state_path(),
        serde_json::to_vec_pretty(&foreign).unwrap(),
    )
    .unwrap();

    let out = run(&ctx, None).await;
    let _ = owner.kill();
    let _ = owner.wait();
    let err = out.unwrap_err();
    assert!(
        err.to_string()
            .starts_with("refusing to launch over the previous session record"),
        "{err}"
    );
    assert!(
        err.to_string().contains("./demo.sh stop --bottle Steam"),
        "the refusal names the way out: {err}"
    );
    let evs = seen.lock().unwrap().clone();
    assert!(
        !evs.iter().any(|e| matches!(e, StageEvent::Check { .. })),
        "not one preflight check ran: {:?}",
        rows(&evs)
    );
    assert!(
        ctx.executor.planned().is_empty(),
        "nothing was restored, removed or reset: {:?}",
        ctx.executor.planned()
    );
    assert!(
        ctx.paths.session_state_path().exists(),
        "and the other front-end's record is still exactly where it was"
    );
    std::fs::remove_dir_all(&root).ok();
}

/// A9-2: the record kept because an `adb forward --remove` failed carries
/// the exact `(serial, port)` the retry needs. Carrying only the audio
/// device left the next launch to overwrite the rest.
#[test]
fn a_kept_records_forwards_travel_into_the_next_launch() {
    let root = scratch("carried-forwards");
    let mut kept = fresh(&root);
    kept.prev_audio_output = Some("MacBook Pro Speakers".into());
    kept.wired_forwards = vec![
        WiredForward {
            serial: "1WMHH000".into(),
            port: 9943,
        },
        WiredForward {
            serial: "1WMHH000".into(),
            port: 9944,
        },
    ];

    assert_eq!(
        carry_forward(&kept),
        Carried {
            audio: Some("MacBook Pro Speakers".to_string()),
            forwards: kept.wired_forwards.clone(),
        }
    );

    // A removal that succeeded set the flag — and carries nothing, exactly
    // as a completed audio restore does.
    let mut done = kept.clone();
    done.guards.forwards_cleared = true;
    done.guards.audio_restored = true;
    assert_eq!(carry_forward(&done), Carried::default());

    assert_eq!(
        carry_forward(&fresh(&root)),
        Carried::default(),
        "r1:A9-2 regression: a session that never touched audio carries nothing"
    );

    // This launch's own hygiene records the same pair again; one removal
    // is enough.
    let mut both = kept.wired_forwards.clone();
    both.extend(kept.wired_forwards.clone());
    both.push(WiredForward {
        serial: "OTHER".into(),
        port: 9943,
    });
    dedup_forwards(&mut both);
    assert_eq!(both.len(), 3);
    assert_eq!(both[2].serial, "OTHER");
    std::fs::remove_dir_all(&root).unwrap();
}

/// A9-3: a teardown that lands after the next launch has written its own
/// record must not delete it. `state::clear` compares the owner, which on
/// this machine is this very process — run identity is the only thing that
/// tells the two apart.
#[tokio::test]
async fn a_late_teardown_never_clears_a_newer_runs_record() {
    let root = scratch("clear-state-cas");
    let (ctx, seen) = dry_ctx(&root, StageOptions::default());
    let path = ctx.paths.session_state_path();
    std::fs::create_dir_all(&ctx.paths.sabrage_appsup).unwrap();

    // The record on disk belongs to a LATER launch.
    let mut newer = fresh(&root);
    newer.run_id = Uuid::new_v4();
    std::fs::write(&path, serde_json::to_vec_pretty(&newer).unwrap()).unwrap();

    clear_state(&ctx, &path, Uuid::nil()).await.unwrap();
    assert!(
        !ctx.executor
            .planned()
            .iter()
            .any(|p| p.kind == PlannedKind::RemoveFile),
        "no removal was even planned: {:?}",
        ctx.executor.planned()
    );
    assert!(rows(&seen.lock().unwrap())
        .iter()
        .any(|r| r.contains("now describes a newer session")));

    // …and our own record still goes.
    std::fs::write(&path, serde_json::to_vec_pretty(&fresh(&root)).unwrap()).unwrap();
    clear_state(&ctx, &path, Uuid::nil()).await.unwrap();
    assert!(ctx
        .executor
        .planned()
        .iter()
        .any(|p| p.kind == PlannedKind::RemoveFile));
    std::fs::remove_dir_all(&root).unwrap();
}

#[tokio::test]
async fn the_cancelled_path_announces_itself_stops_wine_and_exits_130() {
    let root = scratch("teardown-cancel");
    let (mut ctx, seen) = dry_ctx(&root, StageOptions::default());
    ctx.paths.wineserver = Some(PathBuf::from("/cx/bin/wineserver"));
    ctx.cancel.cancel();

    let mut held = Guards::default();
    let mut sess = fresh(&root);
    let _g = session::lock_session_globals();
    let mut phase = RunPhaseScope::new(ctx.run_id, "Steam");
    let err = teardown(
        &ctx,
        &bottle(&root),
        &mut held,
        &mut sess,
        &ctx.paths.session_state_path(),
        Err(SabrageError::Cancelled),
        &mut phase,
    )
    .await
    .unwrap_err();

    assert_eq!(err.exit_code(), 130);
    assert_eq!(
        rows(&seen.lock().unwrap()),
        vec![String::new(), "-- interrupted: stopping wine".to_string()]
    );
    // `stop_wine`'s two children, on the 4 s advisory budget.
    assert_eq!(
        ctx.executor
            .planned()
            .iter()
            .map(|p| p.reason.clone())
            .collect::<Vec<_>>(),
        vec![
            "/cx/bin/wineserver -k".to_string(),
            "/cx/bin/wineserver -w".to_string()
        ]
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[tokio::test]
async fn detaching_marks_the_state_leaves_the_guards_and_keeps_the_file() {
    let root = scratch("teardown-detach");
    let (ctx, seen) = dry_ctx(&root, StageOptions::default());
    let mut held = Guards::default();
    let mut sess = fresh(&root);
    sess.prev_audio_output = Some("MacBook Pro Speakers".into());
    let log = root.join("l.log");

    let _g = session::lock_session_globals();
    let mut phase = RunPhaseScope::new(ctx.run_id, "Steam");
    let rc = teardown(
        &ctx,
        &bottle(&root),
        &mut held,
        &mut sess,
        &ctx.paths.session_state_path(),
        Ok(Reason::Detached { log: log.clone() }),
        &mut phase,
    )
    .await
    .unwrap();

    assert_eq!(rc, 0);
    assert!(sess.detached);
    assert!(
        sess.has_pending_guards(),
        "a detached session's guards stay pending ON PURPOSE"
    );
    assert_eq!(rows(&seen.lock().unwrap()), vec![detached_line(&log)]);
    // The state file is written, never cleared: the next reconcile needs it.
    let kinds: Vec<PlannedKind> = ctx.executor.planned().iter().map(|p| p.kind).collect();
    assert_eq!(kinds, vec![PlannedKind::CreateDir, PlannedKind::Write]);
    assert!(!kinds.contains(&PlannedKind::RemoveFile));
    std::fs::remove_dir_all(&root).unwrap();
}

#[tokio::test]
async fn a_dry_run_teardown_says_nothing_and_returns_zero() {
    let root = scratch("teardown-dry");
    let (ctx, seen) = dry_ctx(&root, StageOptions::default());
    let mut held = Guards::default();
    let mut sess = fresh(&root);
    let _g = session::lock_session_globals();
    let mut phase = RunPhaseScope::new(ctx.run_id, "Steam");
    let rc = teardown(
        &ctx,
        &bottle(&root),
        &mut held,
        &mut sess,
        &ctx.paths.session_state_path(),
        Ok(Reason::DryRun),
        &mut phase,
    )
    .await
    .unwrap();
    assert_eq!(rc, 0);
    assert!(seen.lock().unwrap().is_empty());
    assert!(ctx.executor.planned().is_empty());
    std::fs::remove_dir_all(&root).unwrap();
}

#[tokio::test]
async fn a_failure_propagates_the_original_error() {
    let root = scratch("teardown-failed");
    let (ctx, _) = dry_ctx(&root, StageOptions::default());
    let mut held = Guards::default();
    let mut sess = fresh(&root);
    let _g = session::lock_session_globals();
    let mut phase = RunPhaseScope::new(ctx.run_id, "Steam");
    let err = teardown(
        &ctx,
        &bottle(&root),
        &mut held,
        &mut sess,
        &ctx.paths.session_state_path(),
        Err(SabrageError::fatal_bare("goldberg install failed")),
        &mut phase,
    )
    .await
    .unwrap_err();
    assert_eq!(err.to_string(), "goldberg install failed");
    std::fs::remove_dir_all(&root).unwrap();
}

#[tokio::test]
async fn releasing_the_guards_consumes_both_slots_and_is_idempotent() {
    // Both guards are inert here, so this pins consumption and idempotence
    // only — not `Guards::release`'s trap order.
    let root = scratch("guard-order");
    let (ctx, seen) = dry_ctx(
        &root,
        StageOptions {
            no_audio: true,
            no_dashboard: true,
            ..Default::default()
        },
    );
    let mut sess = fresh(&root);
    let mut held = Guards {
        audio: Some(AudioGuard::arm(&ctx, &facts(), &mut sess).await.unwrap()),
        dashboard: Some(
            DashboardGuard::acquire(&ctx, &facts(), &mut sess)
                .await
                .unwrap(),
        ),
    };
    seen.lock().unwrap().clear();

    held.release(&ctx, &mut sess).await.unwrap();
    assert!(held.audio.is_none() && held.dashboard.is_none());
    // Both guards are inert (--no-audio/--no-dashboard) and there is no
    // staged helper under the fixture root, so a clean release is silent.
    assert!(rows(&seen.lock().unwrap()).is_empty());

    // Releasing twice is a no-op, which is what makes the error path safe.
    held.release(&ctx, &mut sess).await.unwrap();
    std::fs::remove_dir_all(&root).unwrap();
}

#[tokio::test]
async fn disarming_consumes_both_guard_slots() {
    let root = scratch("guard-disarm");
    let (ctx, _) = dry_ctx(
        &root,
        StageOptions {
            no_audio: true,
            no_dashboard: true,
            ..Default::default()
        },
    );
    let mut sess = fresh(&root);
    let mut held = Guards {
        audio: Some(AudioGuard::arm(&ctx, &facts(), &mut sess).await.unwrap()),
        dashboard: Some(
            DashboardGuard::acquire(&ctx, &facts(), &mut sess)
                .await
                .unwrap(),
        ),
    };
    held.disarm();
    assert!(held.audio.is_none() && held.dashboard.is_none());
    std::fs::remove_dir_all(&root).unwrap();
}

#[tokio::test]
async fn a_cancelled_token_short_circuits_before_any_launch_action() {
    let root = scratch("checkpoint");
    let (ctx, _) = dry_ctx(&root, StageOptions::default());
    assert!(checkpoint(&ctx).is_ok());
    ctx.cancel.cancel();
    assert!(matches!(
        checkpoint(&ctx).unwrap_err(),
        SabrageError::Cancelled
    ));
    std::fs::remove_dir_all(&root).unwrap();
}

#[tokio::test]
async fn the_guarded_region_reports_cancellation_before_it_launches() {
    let root = scratch("guarded-cancel");
    let (ctx, seen) = dry_ctx(
        &root,
        StageOptions {
            no_audio: true,
            no_dashboard: true,
            ..Default::default()
        },
    );
    ctx.cancel.cancel();
    let mut held = Guards::default();
    let mut sess = fresh(&root);
    let reason = guarded(
        &ctx,
        &bottle(&root),
        &facts(),
        &mut sess,
        &ctx.paths.session_state_path(),
        &mut held,
        None,
    )
    .await
    .unwrap();
    assert!(matches!(reason, Reason::Cancelled { child: None }));
    // The audio guard was acquired (inert) before the checkpoint fired …
    assert!(held.audio.is_some());
    // … and nothing was launched.
    assert!(!ctx
        .executor
        .planned()
        .iter()
        .any(|p| p.kind == PlannedKind::SpawnDetached));
    assert!(!session::live_session_is(ctx.run_id));
    assert!(matches!(
        seen.lock().unwrap().first(),
        Some(StageEvent::Line {
            severity: Severity::Info,
            ..
        })
    ));
    std::fs::remove_dir_all(&root).unwrap();
}

#[tokio::test]
async fn a_dry_run_plans_the_launch_supervises_nothing_and_owns_no_session() {
    let root = scratch("guarded-dry");
    let (mut ctx, seen) = dry_ctx(
        &root,
        StageOptions {
            no_audio: true,
            no_dashboard: true,
            dry_run: true,
            ..Default::default()
        },
    );
    ctx.paths.wine = Some(PathBuf::from("/cx/bin/wine"));
    let b = bottle(&root);
    let mut held = Guards::default();
    let mut sess = fresh(&root);

    let reason = guarded(
        &ctx,
        &b,
        &facts(),
        &mut sess,
        &ctx.paths.session_state_path(),
        &mut held,
        None,
    )
    .await
    .unwrap();
    assert!(matches!(reason, Reason::DryRun));

    // The banner was printed and the launch planned — nothing spawned.
    let printed = rows(&seen.lock().unwrap());
    assert!(printed
        .iter()
        .any(|l| l.starts_with("   log: ") && l.contains("beatsaber-")));
    let plan = ctx.executor.planned();
    assert!(plan.iter().any(|p| p.kind == PlannedKind::SpawnDetached));
    // A dry run never publishes a live session and never writes the log.
    assert!(!session::live_session_is(ctx.run_id));
    assert!(sess.wine.is_none());
    assert!(!ctx.paths.logs_dir().exists(), "no log file, no logs dir");
    std::fs::remove_dir_all(&root).unwrap();
}

/// The published run phase **at the moment each row was emitted**.
///
/// The sink runs synchronously inside `ctx.emit`, so this is a
/// deterministic probe of a value that is otherwise only observable from
/// inside the call — no polling task, no sleeping, no race.
type PhaseLog = Arc<Mutex<Vec<(String, Option<SessionPhase>)>>>;

fn observing_ctx(root: &Path, opts: StageOptions) -> (StageCtx, PhaseLog) {
    let seen: PhaseLog = Arc::new(Mutex::new(Vec::new()));
    let s = seen.clone();
    let sink: EventSink = Arc::new(move |ev: StageEvent| {
        let text = match &ev {
            StageEvent::Text { text, .. } => text.clone(),
            StageEvent::Section { title, .. } => format!("-- {title}"),
            StageEvent::Line { severity, text, .. } => format!("[{severity}] {text}"),
            StageEvent::Check { outcome, .. } => format!("check {}", outcome.slug),
            StageEvent::Fatal { message, .. } => format!("[fatal] {message}"),
            _ => String::new(),
        };
        s.lock()
            .unwrap()
            .push((text, session::run_phase().map(|i| i.phase)));
    });
    let run_id: RunId = Uuid::new_v4();
    let cancel = CancellationToken::new();
    let mut paths = Paths::new(root);
    paths.oxr_appsup = root.join("appsup-oxrsys");
    paths.sabrage_appsup = root.join("appsup-sabrage");
    paths.adb = None;
    paths.wine = None;
    paths.wineserver = None;
    let executor: Arc<dyn Executor> =
        Arc::new(DryRunExecutor::new(run_id, sink.clone(), cancel.clone()));
    let ctx = StageCtx::with_executor(paths, opts, sink, cancel, executor, run_id);
    (ctx, seen)
}

fn phases(log: &PhaseLog) -> Vec<Option<SessionPhase>> {
    log.lock().unwrap().iter().map(|(_, p)| *p).collect()
}

#[test]
fn the_scope_publishes_identity_and_drop_clears_only_its_own_run() {
    let _g = session::lock_session_globals();
    session::publish_run_phase(None);

    let a = Uuid::new_v4();
    {
        let scope = RunPhaseScope::new(a, "Steam");
        scope.publish(SessionPhase::Preflight);
        let info = session::run_phase().expect("published");
        assert_eq!(info.phase, SessionPhase::Preflight);
        assert_eq!(info.run_id, a);
        assert_eq!(info.bottle, "Steam", "#100: Stop needs a bottle name");
        assert!(info.exit_code.is_none());

        scope.publish(SessionPhase::Launching);
        assert_eq!(
            session::run_phase().map(|i| i.phase),
            Some(SessionPhase::Launching)
        );
    }
    assert!(
        session::run_phase().is_none(),
        "Drop must empty the slot on every exit path"
    );

    // A finalized scope's publication OUTLIVES it — that is what lets the
    // Session screen say "Exited (code N)" after `run` has returned.
    {
        let mut scope = RunPhaseScope::new(a, "Steam");
        scope.publish(SessionPhase::Stopping);
        scope.finalize_exited(3);
    }
    let info = session::run_phase().expect("Exited survives the scope");
    assert_eq!(info.phase, SessionPhase::Exited);
    assert_eq!(info.exit_code, Some(3), "#7: the code is carried");

    // A *different* run's scope must not clear it on the way out — `run`
    // releases the operation lock at the launch boundary, so two of these
    // genuinely overlap.
    {
        let _other = RunPhaseScope::new(Uuid::new_v4(), "Other");
    }
    assert_eq!(session::run_phase().map(|i| i.run_id), Some(a));

    session::publish_run_phase(None);
}

#[tokio::test]
async fn run_publishes_preflight_and_clears_it_when_the_preflight_fails() {
    let _g = session::lock_session_globals();
    session::publish_run_phase(None);

    let root = scratch("run-preflight-phase");
    let (mut ctx, log) = observing_ctx(
        &root,
        StageOptions {
            bottle_name: Some("Steam".to_string()),
            bs_dir_override: Some(root.join("BeatSaber")),
            dry_run: true,
            no_audio: true,
            no_dashboard: true,
            ..Default::default()
        },
    );
    ctx.bottle = Some(bottle(&root));

    // Nothing else in the fixture exists, so the preflight dies on its
    // first blocking row — the point is only that the phase was published
    // before it, and cleared after it.
    let err = run(&ctx, None).await.unwrap_err();
    assert!(!err.to_string().is_empty());

    let observed = phases(&log);
    assert!(!observed.is_empty(), "the preflight emits rows");
    assert_eq!(
        observed[0],
        Some(SessionPhase::Preflight),
        "#2: the very first row of a launch already reports Preflight, \
             not `No session`"
    );
    assert!(
        observed.iter().all(|p| *p == Some(SessionPhase::Preflight)),
        "a run that never reaches the guards never leaves Preflight: {observed:?}"
    );
    assert!(
        session::run_phase().is_none(),
        "a failing preflight leaves the slot empty — the RAII guard, on the `?`"
    );
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn a_missing_bottle_never_publishes_anything_at_all() {
    let _g = session::lock_session_globals();
    session::publish_run_phase(None);

    let root = scratch("run-no-bottle");
    let (ctx, _) = observing_ctx(&root, StageOptions::default());
    // `require_bottle` dies above the scope: nothing to report, nothing
    // published.
    assert!(run(&ctx, None).await.is_err());
    assert!(session::run_phase().is_none());
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn a_normal_teardown_reports_stopping_then_a_surviving_exited_code() {
    let _g = session::lock_session_globals();
    session::publish_run_phase(None);

    let root = scratch("phase-teardown-normal");
    let (ctx, log) = observing_ctx(&root, StageOptions::default());
    let mut held = Guards::default();
    let mut sess = fresh(&root);
    let log_path = root.join("logs/beatsaber-20260829-101112.log");

    {
        let mut phase = RunPhaseScope::new(ctx.run_id, "Steam");
        let rc = teardown(
            &ctx,
            &bottle(&root),
            &mut held,
            &mut sess,
            &ctx.paths.session_state_path(),
            Ok(Reason::Normal {
                rc: 7,
                log: log_path.clone(),
            }),
            &mut phase,
        )
        .await
        .unwrap();
        assert_eq!(rc, 7);
    }

    // The blank line still belongs to `Stopping`; the status line and the
    // phase that names its code are published together.
    assert_eq!(
        phases(&log),
        vec![Some(SessionPhase::Stopping), Some(SessionPhase::Exited)]
    );
    let info = session::run_phase().expect("#7: Exited outlives `run`");
    assert_eq!(info.phase, SessionPhase::Exited);
    assert_eq!(info.exit_code, Some(7));
    assert_eq!(info.bottle, "Steam");
    assert_eq!(info.run_id, ctx.run_id);

    session::publish_run_phase(None);
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn cancelled_failed_detached_and_dry_run_teardowns_end_at_idle() {
    let _g = session::lock_session_globals();

    for (label, outcome, want_stopping) in [
        ("cancelled", Err(SabrageError::Cancelled), true),
        (
            "failed",
            Err(SabrageError::fatal_bare("goldberg install failed")),
            true,
        ),
        (
            "detached",
            Ok(Reason::Detached {
                log: PathBuf::from("/l.log"),
            }),
            false,
        ),
        ("dry run", Ok(Reason::DryRun), false),
    ] {
        session::publish_run_phase(None);
        let root = scratch("phase-teardown-arms");
        let (ctx, log) = observing_ctx(&root, StageOptions::default());
        let mut held = Guards::default();
        let mut sess = fresh(&root);

        let observed;
        {
            let mut phase = RunPhaseScope::new(ctx.run_id, "Steam");
            let _ = teardown(
                &ctx,
                &bottle(&root),
                &mut held,
                &mut sess,
                &ctx.paths.session_state_path(),
                outcome,
                &mut phase,
            )
            .await;
            // Read the slot from *inside* the scope: `Failed` releases the
            // guards without emitting a single row, so the row log alone
            // cannot show whether teardown announced itself.
            observed = phases(&log);
            assert_eq!(
                session::run_phase().map(|i| i.phase),
                want_stopping.then_some(SessionPhase::Stopping),
                "{label}"
            );
        }

        assert!(
            !observed.contains(&Some(SessionPhase::Exited)),
            "{label}: only a normal exit has a code to report"
        );
        assert!(
            session::run_phase().is_none(),
            "{label}: Idle is the honest end state — the RAII guard clears it"
        );
        std::fs::remove_dir_all(&root).ok();
    }
}

#[tokio::test]
async fn the_detach_row_belongs_to_the_supervise_step() {
    // #13: `step::RUN_SUPERVISE` is the step that was running when detach
    // fired, and the announcement is the row that closes it.
    let root = scratch("detach-step");
    let (ctx, seen) = dry_ctx(&root, StageOptions::default());
    let mut held = Guards::default();
    let mut sess = fresh(&root);
    let _g = session::lock_session_globals();
    let mut phase = RunPhaseScope::new(ctx.run_id, "Steam");
    let log = root.join("l.log");

    teardown(
        &ctx,
        &bottle(&root),
        &mut held,
        &mut sess,
        &ctx.paths.session_state_path(),
        Ok(Reason::Detached { log: log.clone() }),
        &mut phase,
    )
    .await
    .unwrap();

    let evs = seen.lock().unwrap().clone();
    let steps: Vec<Option<&str>> = evs
        .iter()
        .filter(|e| matches!(e, StageEvent::Text { .. }))
        .map(|e| e.step())
        .collect();
    assert_eq!(steps, vec![Some(step::RUN_SUPERVISE)]);
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn a_failed_state_write_warns_instead_of_orphaning_the_running_game() {
    let root = scratch("record-session");
    let (mut ctx, seen) = dry_ctx(&root, StageOptions::default());
    // A real executor, so the write is genuinely attempted …
    ctx.executor = Arc::new(RealExecutor::new(
        ctx.run_id,
        ctx.sink.clone(),
        ctx.cancel.clone(),
    ));
    // … against a regular file where `session-state.json`'s directory
    // belongs, so `create_dir_all` cannot succeed.
    std::fs::write(&ctx.paths.sabrage_appsup, b"not a directory").unwrap();

    let sess = fresh(&root);
    let state_path = ctx.paths.session_state_path();
    // Returns `()` on purpose: by this point wine is spawned and published,
    // and an `Err` here would unwind out of `guarded` into teardown's
    // `Failed` arm — guards off, state cleared, and a running game with
    // `kill_on_drop(false)` that nothing can reach.
    record_launched_session(&ctx, &state_path, &sess).await;

    assert!(!state_path.exists());
    let printed = rows(&seen.lock().unwrap());
    assert_eq!(printed.len(), 1, "{printed:?}");
    assert!(
        printed[0].starts_with("[warn] could not record the session in "),
        "{}",
        printed[0]
    );
    assert!(
        printed[0]
            .ends_with("— Stop still works here, but a Sabrage restart will not find this session"),
        "{}",
        printed[0]
    );
    std::fs::remove_dir_all(&root).unwrap();
}
