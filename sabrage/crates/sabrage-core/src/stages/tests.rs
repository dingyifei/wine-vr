use super::*;
use std::sync::Mutex as StdMutex;

fn ctx_with(opts: StageOptions) -> (StageCtx, Arc<StdMutex<Vec<StageEvent>>>) {
    let seen = Arc::new(StdMutex::new(Vec::new()));
    let s = seen.clone();
    let sink: EventSink = Arc::new(move |ev| s.lock().unwrap().push(ev));
    let ctx = StageCtx::new(
        Paths::new("/nonexistent/sabrage/repo"),
        opts,
        sink,
        CancellationToken::new(),
    );
    (ctx, seen)
}

#[test]
fn dry_run_selects_the_recording_executor() {
    let (real, _) = ctx_with(StageOptions::default());
    assert!(!real.executor.is_dry_run());
    let (dry, _) = ctx_with(StageOptions {
        dry_run: true,
        ..Default::default()
    });
    assert!(dry.executor.is_dry_run());
}

#[test]
fn require_bottle_reproduces_lib_sh_die_text() {
    let (ctx, seen) = ctx_with(StageOptions::default());
    let err = require_bottle(&ctx).unwrap_err();
    let msg = err.to_string();
    let (first, second) = msg.split_once('\n').expect("two-line message");
    assert_eq!(
        first,
        "CrossOver bottle name required: pass --bottle <name> or set WINEVR_BOTTLE."
    );
    assert!(
        second.starts_with("       Existing bottles: "),
        "second line was {second:?}"
    );
    // Each listed bottle is followed by a space (tr '\n' ' '), so the line
    // either ends in a space or lists nothing at all.
    let listed = second.trim_start_matches("       Existing bottles: ");
    assert!(listed.is_empty() || listed.ends_with(' '));
    // A die always announces itself.
    assert!(matches!(
        seen.lock().unwrap().last(),
        Some(StageEvent::Fatal { .. })
    ));

    // Named but absent bottle: the other die string.
    let (ctx, _) = ctx_with(StageOptions {
        bottle_name: Some("NoSuchBottle".into()),
        ..Default::default()
    });
    let msg = require_bottle(&ctx).unwrap_err().to_string();
    assert!(
        msg.starts_with("bottle 'NoSuchBottle' not found at ")
            && msg.ends_with(" — create it in CrossOver (win11_64) first"),
        "{msg}"
    );
}

#[tokio::test]
async fn run_stage_brackets_the_stage_with_events_even_when_it_fails() {
    // `stop` with no bottle: `require_bottle` dies before touching the
    // machine, which is the cheapest real failure any stage can have.
    let (ctx, seen) = ctx_with(StageOptions::default());
    run_stage(Stage::Stop, &ctx).await.unwrap_err();

    let evs = seen.lock().unwrap().clone();
    assert!(matches!(
        evs.first(),
        Some(StageEvent::StageStarted {
            stage: Stage::Stop,
            ..
        })
    ));
    assert!(matches!(
        evs.last(),
        Some(StageEvent::StageFinished {
            stage: Stage::Stop,
            ok: false,
            exit_code_equiv: 1,
            ..
        })
    ));
}

#[test]
fn stage_outcome_from_code_is_ok_only_for_zero() {
    // `run` propagates wine's status: a non-zero code is a stage that
    // finished, not-ok — not an error.
    assert_eq!(
        StageOutcome::from_code(Stage::Run, 0),
        StageOutcome::success(Stage::Run)
    );
    let crashed = StageOutcome::from_code(Stage::Run, 3);
    assert!(!crashed.ok);
    assert_eq!(crashed.exit_code_equiv, 3);
}

#[test]
fn the_two_wineserver_budgets_stay_distinct() {
    // PARITY.md § Invariants that must NOT change (byte/behavior parity),
    // "wineserver budgets (5 s fatal / 4 s soft)": 5 s fatal (run) vs
    // 4 s soft (stop). Never unify.
    assert_eq!(RUN_WINESERVER_WAIT, Duration::from_secs(5));
    assert_eq!(STOP_WINESERVER_WAIT, Duration::from_secs(4));
    assert_ne!(RUN_WINESERVER_WAIT, STOP_WINESERVER_WAIT);
}

#[test]
fn check_ctx_forwards_every_launch_flag() {
    let (ctx, _) = ctx_with(StageOptions {
        bottle_name: Some("Steam".into()),
        verbose: true,
        no_audio: true,
        no_dashboard: true,
        wired: true,
        ..Default::default()
    });
    let cc = ctx.check_ctx();
    assert!(cc.opts.verbose && cc.opts.no_audio && cc.opts.no_dashboard && cc.opts.wired);
    assert_eq!(cc.opts.bottle_name.as_deref(), Some("Steam"));
    // doctor parity: adb probing stays on unless a caller turns it off.
    assert!(cc.opts.allow_adb_probes);
}

#[tokio::test]
async fn the_operation_lock_admits_one_holder() {
    // Only the "held ⇒ reported" direction is deterministic: the test
    // binary runs tests in parallel, and a sibling test taking the lock
    // would make the converse flaky.
    let guard = acquire_operation_lock().await;
    assert!(operation_in_progress());
    assert!(OPERATION_LOCK.try_lock().is_err());
    drop(guard);
}

/// The advisory file lock, not `OPERATION_LOCK`, excludes a second Sabrage
/// process. `flock` is per open file description, so a second `File` on the
/// same path in this process sees exactly what another process sees.
#[tokio::test]
async fn the_advisory_file_lock_excludes_a_second_holder_and_releases_on_drop() {
    let path = std::env::temp_dir().join(format!(
        "sabrage-oplock-test-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::remove_file(&path).ok();

    let FileLock::Held(held) = acquire_lock_file(&path, &CancellationToken::new()).await else {
        panic!("lock acquired");
    };
    let other = open_lock_file(&path).expect("second handle opens");
    assert!(
        matches!(other.try_lock(), Err(std::fs::TryLockError::WouldBlock)),
        "a second process must not be able to take the lock"
    );
    // The pid is written for a diagnostic "held by …" message.
    assert_eq!(
        std::fs::read_to_string(&path).unwrap().trim(),
        std::process::id().to_string()
    );

    drop(held);
    assert!(
        other.try_lock().is_ok(),
        "dropping the guard must release the file lock"
    );
    drop(other);
    std::fs::remove_file(&path).ok();
}

/// The real guard takes both halves. Only the "held ⇒ locked" direction is
/// deterministic here (a sibling test may take the lock the instant this
/// one lets go), the same asymmetry `the_operation_lock_admits_one_holder`
/// documents.
#[tokio::test]
async fn acquire_operation_lock_takes_the_file_lock_too() {
    let guard = acquire_operation_lock().await;
    let probe = open_lock_file(&operation_lock_path()).expect("lock file opens");
    assert!(matches!(
        probe.try_lock(),
        Err(std::fs::TryLockError::WouldBlock)
    ));
    drop(probe);
    drop(guard);
}

/// The advisory-lock wait is cancellable: without this, Stop could not
/// reach a stage queued behind another Sabrage process (the poll loop only
/// ever woke for the lock becoming free).
#[tokio::test]
async fn the_file_lock_wait_gives_up_when_the_token_fires() {
    let path = std::env::temp_dir().join(format!(
        "sabrage-oplock-cancel-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::remove_file(&path).ok();
    // Stand in for the other process: a second open file description on the
    // same path is exactly what `flock` treats as a foreign holder.
    let blocker = open_lock_file(&path).expect("blocker opens");
    blocker.try_lock().expect("blocker takes the lock");

    // Already cancelled before the first probe.
    let precancelled = CancellationToken::new();
    precancelled.cancel();
    assert!(matches!(
        acquire_lock_file(&path, &precancelled).await,
        FileLock::Cancelled
    ));

    // Cancelled from another task while the poll loop is asleep.
    let cancel = CancellationToken::new();
    let c = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        c.cancel();
    });
    let waited = tokio::time::timeout(Duration::from_secs(5), acquire_lock_file(&path, &cancel))
        .await
        .expect("the wait must end when the token fires");
    assert!(matches!(waited, FileLock::Cancelled));

    drop(blocker);
    std::fs::remove_file(&path).ok();
}

/// A queued stage is visible (`StageStarted` before the wait, so the run id
/// — and Cancel — exist during it) and cancellable, and it never reaches
/// its dispatch.
#[tokio::test]
async fn a_queued_stage_announces_itself_and_cancels_out_of_the_wait() {
    let held = acquire_operation_lock().await;
    let (ctx, seen) = ctx_with(StageOptions {
        dry_run: true,
        ..Default::default()
    });
    let cancel = ctx.cancel.clone();
    let run_id = ctx.run_id;
    let task = tokio::spawn(async move { run_stage(Stage::Stop, &ctx).await });

    // The started event must arrive while the lock is still held elsewhere.
    let started = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let has = seen.lock().unwrap().iter().any(
                |ev| matches!(ev, StageEvent::StageStarted { stage, .. } if *stage == Stage::Stop),
            );
            if has {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    assert!(
        started.is_ok(),
        "StageStarted must be emitted before the operation lock is taken"
    );

    cancel.cancel();
    let err = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("cancelling must end the wait")
        .expect("the stage task must not panic")
        .expect_err("a cancelled stage fails");
    assert!(matches!(err, SabrageError::Cancelled), "{err:?}");
    assert_eq!(err.exit_code(), 130);

    let events = seen.lock().unwrap().clone();
    assert!(events.iter().any(|ev| matches!(
        ev,
        StageEvent::StageFinished { run_id: r, ok: false, exit_code_equiv: 130, .. } if *r == run_id
    )));
    // The queue notice, and nothing from `stop` itself.
    assert!(events.iter().any(|ev| matches!(
        ev,
        StageEvent::Line { text, .. } if text.contains("waiting for another Sabrage operation")
    )));
    assert!(
        !events
            .iter()
            .any(|ev| matches!(ev, StageEvent::Section { .. })),
        "a cancelled stage must not have run: {events:?}"
    );
    drop(held);
}

/// The queue probe sees the cross-process half, which is the wait that
/// actually lasts minutes (a `sabrage` CLI build).
#[tokio::test]
async fn operation_in_progress_anywhere_sees_the_advisory_file_lock() {
    let foreign = open_lock_file(&operation_lock_path()).expect("lock file opens");
    // A sibling test may hold the lock; then there is nothing to prove here
    // (and `_anywhere` is already true for the in-process reason).
    if foreign.try_lock().is_err() {
        assert!(operation_in_progress_anywhere());
        return;
    }
    assert!(operation_lock_file_busy());
    assert!(operation_in_progress_anywhere());
    drop(foreign);
}

/// A test-run lock file never lands in the user's real Sabrage store.
#[test]
fn the_test_lock_file_is_not_in_the_user_support_directory() {
    let path = operation_lock_path();
    assert!(
        !path.starts_with(crate::paths::sabrage_support_dir()),
        "{} must not be under the real support directory during tests",
        path.display()
    );
}

/// A ctx whose session-state and OXRSys stores are scratch directories, so
/// `live_session_block` reads fixtures rather than the real machine — and
/// whose scratch root carries this binary's own contract, so the identity
/// guard is satisfied and the *other* refusals are what the tests observe.
fn ctx_at(root: &std::path::Path, bottle: Option<&str>) -> StageCtx {
    materialize_compiled_contract(root);
    let mut paths = Paths::new(root);
    paths.sabrage_appsup = root.join("Sabrage");
    paths.oxr_appsup = root.join("OXRSys");
    let opts = StageOptions {
        bottle_name: bottle.map(str::to_string),
        ..StageOptions::default()
    };
    StageCtx::new(paths, opts, null_sink(), CancellationToken::new())
}

/// Write a `session-state.json` whose recorded wine identity is **this**
/// process: alive, and reporting the start time recorded for it, which is
/// exactly what `classify` calls `Live`.
fn write_live_session_state(ctx: &StageCtx) {
    let path = ctx.paths.session_state_path();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut state =
        crate::session::state::SessionState::new(Uuid::new_v4(), "FixtureBottle", "/bs", "/log", 0);
    state.wine = crate::process::ProcInfo::observe(std::process::id());
    assert!(state.wine.is_some(), "this process must be observable");
    std::fs::write(&path, serde_json::to_string(&state).unwrap()).unwrap();
}

/// setup/build/install replace what a running session has open, so they are
/// refused outright; `stop` (the way out) and `run` (which reconciles for
/// itself) are not.
#[tokio::test]
async fn run_stage_refuses_setup_build_and_install_while_a_session_is_live() {
    let _g = crate::session::lock_session_globals();
    let root = std::env::temp_dir().join(format!("sabrage-live-stage-{}", std::process::id()));
    let ctx = ctx_at(&root, Some("FixtureBottle"));
    write_live_session_state(&ctx);

    for stage in [Stage::Setup, Stage::Build, Stage::Install] {
        let err = run_stage(stage, &ctx).await.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.starts_with(&format!("refusing to run {stage} while a session is live")),
            "{msg}"
        );
    }
    assert!(!stage_is_forbidden_while_session_live(Stage::Run));
    assert!(!stage_is_forbidden_while_session_live(Stage::Stop));
    // `stop` still gets as far as its own bottle check, not the refusal.
    let stop_err = run_stage(Stage::Stop, &ctx_at(&root, None))
        .await
        .unwrap_err();
    assert!(stop_err
        .to_string()
        .starts_with("CrossOver bottle name required"));

    std::fs::remove_dir_all(&root).ok();
}

/// The live-session refusal is checked before the operation lock **and
/// again after it**: a stage admitted while idle can wait minutes behind
/// another process's build, and a `run` that wins the lock race publishes
/// its session and releases the lock at launch — so the queued stage must
/// be re-refused or it replaces the artifacts of a streaming game.
#[tokio::test]
async fn a_queued_stage_is_refused_when_a_session_goes_live_during_the_wait() {
    let _g = crate::session::lock_session_globals();
    let root = std::env::temp_dir().join(format!("sabrage-queued-live-{}", std::process::id()));
    std::fs::remove_dir_all(&root).ok();
    materialize_compiled_contract(&root);

    let mut paths = Paths::new(&root);
    paths.sabrage_appsup = root.join("Sabrage");
    paths.oxr_appsup = root.join("OXRSys");
    let oxr = paths.oxr_appsup.clone();
    let seen = Arc::new(StdMutex::new(Vec::new()));
    let s = seen.clone();
    let sink: EventSink = Arc::new(move |ev| s.lock().unwrap().push(ev));
    let ctx = StageCtx::new(
        paths,
        StageOptions {
            // Nothing here may mutate even if the guard were to fail open.
            dry_run: true,
            bottle_name: Some("FixtureBottle".into()),
            ..StageOptions::default()
        },
        sink,
        CancellationToken::new(),
    );
    assert_eq!(
        live_session_block(&ctx.paths),
        None,
        "the fixture must be idle at admission"
    );

    let held = acquire_operation_lock().await;
    let task = tokio::spawn(async move { run_stage(Stage::Build, &ctx).await });

    // Admitted: the stage got past the pre-lock refusal and is now waiting.
    let started = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let has = seen.lock().unwrap().iter().any(
                |ev| matches!(ev, StageEvent::StageStarted { stage, .. } if *stage == Stage::Build),
            );
            if has {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    assert!(started.is_ok(), "the queued stage must announce itself");

    // A session starts while it waits — the `run` that won the lock race.
    std::fs::create_dir_all(&oxr).unwrap();
    let now = crate::session::now_unix_ms();
    let pid = std::process::id();
    std::fs::write(
        oxr.join("runtime_status.json"),
        format!(r#"{{"state":"streaming","process_id":{pid},"updated_at_unix_ms":{now}}}"#),
    )
    .unwrap();
    drop(held);

    let err = tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .expect("the queued stage must settle once the lock is free")
        .expect("the stage task must not panic")
        .expect_err("a stage that acquires the lock under a live session is refused");
    assert!(
        err.to_string()
            .starts_with("refusing to run build while a session is live"),
        "{err}"
    );

    let events = seen.lock().unwrap().clone();
    assert!(
        !events
            .iter()
            .any(|ev| matches!(ev, StageEvent::Section { .. })),
        "the refused stage must never have reached its dispatch: {events:?}"
    );
    // The bracket still closes: StageStarted was emitted before the wait.
    assert!(matches!(
        events.last(),
        Some(StageEvent::StageFinished {
            stage: Stage::Build,
            ok: false,
            ..
        })
    ));

    std::fs::remove_dir_all(&root).ok();
}

/// A scratch checkout whose `contract/` is *not* the one this binary was
/// compiled from — the X-binary/Y-checkout skew `meta.contract-sync`
/// reports and this guard refuses.
fn skewed_checkout(tag: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("sabrage-skew-{tag}-{}", std::process::id()));
    std::fs::remove_dir_all(&root).ok();
    std::fs::create_dir_all(root.join("contract")).unwrap();
    for rel in crate::contract::CONTRACT_FILES {
        std::fs::write(
            root.join(rel),
            b"not-the-contract-this-binary-was-built-from\n",
        )
        .unwrap();
    }
    root
}

/// Every mutating stage refuses a checkout this binary does not describe,
/// through both doors, before any event or executor call. `stop` stays open
/// — it is the way out.
#[tokio::test]
async fn every_mutating_stage_refuses_a_checkout_the_binary_was_not_built_from() {
    let _g = crate::session::lock_session_globals();
    let root = skewed_checkout("stages");
    // The abort says exactly what the `meta.contract-sync` row says: both
    // read it from `checks::meta`, which owns the wording.
    let (expected_message, expected_remedy) =
        crate::checks::meta::assert_binary_matches_checkout(&root)
            .expect_err("the fixture must be a skewed checkout");
    for stage in [Stage::Setup, Stage::Build, Stage::Install, Stage::Run] {
        let (ctx, seen) = {
            let mut paths = Paths::new(&root);
            paths.sabrage_appsup = root.join("Sabrage");
            paths.oxr_appsup = root.join("OXRSys");
            let seen = Arc::new(StdMutex::new(Vec::new()));
            let s = seen.clone();
            let sink: EventSink = Arc::new(move |ev| s.lock().unwrap().push(ev));
            (
                StageCtx::new(
                    paths,
                    StageOptions {
                        dry_run: true,
                        bottle_name: Some("FixtureBottle".into()),
                        ..StageOptions::default()
                    },
                    sink,
                    CancellationToken::new(),
                ),
                seen,
            )
        };

        let err = run_stage(stage, &ctx).await.unwrap_err();
        assert_eq!(err.to_string(), expected_message, "{stage}");
        assert!(
            matches!(&err, SabrageError::Fatal { remedy, .. }
                    if remedy.as_deref() == Some(expected_remedy.as_str())),
            "{stage}: the abort must carry the row's remedy: {err:?}"
        );
        let events = seen.lock().unwrap().clone();
        assert!(
            !events
                .iter()
                .any(|ev| matches!(ev, StageEvent::StageStarted { .. })),
            "the refusal precedes the stage banner: {events:?}"
        );

        // The holding-lock door is gated too — it is reached from the
        // launch preflight's whole-stage auto-fixes.
        let guard = acquire_operation_lock().await;
        let err = run_stage_holding_lock(stage, &ctx).await.unwrap_err();
        assert_eq!(err.to_string(), expected_message, "{stage}");
        drop(guard);
    }

    // `stop` is never gated: it gets as far as its own bottle check.
    let mut paths = Paths::new(&root);
    paths.sabrage_appsup = root.join("Sabrage");
    paths.oxr_appsup = root.join("OXRSys");
    let ctx = StageCtx::new(
        paths,
        StageOptions::default(),
        null_sink(),
        CancellationToken::new(),
    );
    let stop_err = run_stage(Stage::Stop, &ctx).await.unwrap_err();
    assert!(stop_err
        .to_string()
        .starts_with("CrossOver bottle name required"));

    std::fs::remove_dir_all(&root).ok();
}

/// The abort and the `meta.contract-sync` row say the same thing because
/// both read it from `checks::meta`: a self-consistent-but-foreign checkout
/// makes the doctor row and the stage refusal agree word for word.
#[test]
fn the_contract_skew_die_is_the_meta_row_verbatim() {
    let root = skewed_checkout("meta-text");
    std::fs::create_dir_all(root.join("scripts/demo")).unwrap();
    let checkout = crate::util::contract_hash(&root).expect("just-written files are readable");
    std::fs::write(
        root.join(crate::contract::CONTRACT_GEN_REL_PATH),
        format!("# contract-sha256: {checkout}\n"),
    )
    .unwrap();

    let outcome = crate::checks::registry()
        .get("meta.contract-sync")
        .expect("the slug is bound")
        .evaluate(&CheckCtx::new(Paths::new(&root), CheckOptions::new()));
    let (message, remedy) = crate::checks::meta::assert_binary_matches_checkout(&root)
        .expect_err("a foreign checkout must not pass the identity guard");
    assert_eq!(outcome.message, message);
    assert_eq!(outcome.remedy.as_deref(), Some(remedy.as_str()));

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn step_emitter_attributes_rows() {
    let (ctx, seen) = ctx_with(StageOptions::default());
    let st = ctx.step(crate::events::step::INSTALL_BOTTLE);
    st.ok("ActiveRuntime registered");
    ctx.ok("no step");
    let evs = seen.lock().unwrap().clone();
    assert_eq!(evs[0].step(), Some("install.3.bottle"));
    assert_eq!(evs[1].step(), None);
}
