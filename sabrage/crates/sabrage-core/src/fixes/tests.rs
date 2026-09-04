use super::*;
use crate::contract::contract;
use crate::paths::Paths;
use crate::stages::{acquire_operation_lock, null_sink, StageCtx, StageOptions};
use std::str::FromStr;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// A context pointed at a scratch root, with `oxr_appsup` redirected so
/// `alvr_session_json()` can never resolve to the real
/// `~/Library/Application Support/OXRSys/alvr/session.json`. Nothing is
/// created: the file's absence is what makes `DeleteSessionJson` a fast,
/// side-effect-free no-op, which is all these lock tests need it to be.
fn scratch_ctx(tag: &str, bottle: Option<&str>) -> StageCtx {
    let root =
        std::env::temp_dir().join(format!("sabrage-fixes-lock-{tag}-{}", std::process::id()));
    // The scratch root is a checkout as far as `apply` is concerned, and
    // `apply` refuses one this binary was not built from — so give it this
    // binary's own contract.
    crate::stages::materialize_compiled_contract(&root);
    let mut paths = Paths::new(&root);
    paths.oxr_appsup = root.join("OXRSys");
    // …and `sabrage_appsup` too, so the live-session policy reads a scratch
    // `session-state.json` rather than the real machine's.
    paths.sabrage_appsup = root.join("Sabrage");
    let opts = StageOptions {
        bottle_name: bottle.map(str::to_string),
        ..StageOptions::default()
    };
    StageCtx::new(paths, opts, null_sink(), CancellationToken::new())
}

/// `apply` is the public entry point, so it must serialize against a stage:
/// a Doctor "Fix" button cannot be allowed to rewrite `cxbottle.conf` or
/// restage the helper while an install is halfway through.
#[tokio::test]
async fn apply_waits_for_the_operation_lock_then_proceeds() {
    // `apply` now also consults the live-session policy, which reads
    // process-global slots a sibling test may publish into.
    let _g = crate::session::lock_session_globals();
    let ctx = scratch_ctx("apply-blocks", None);
    let sink = null_sink();
    let guard = acquire_operation_lock().await;
    let mut task =
        tokio::spawn(async move { apply(FixAction::DeleteSessionJson, &ctx, &sink).await });

    assert!(
        tokio::time::timeout(Duration::from_millis(250), &mut task)
            .await
            .is_err(),
        "apply did not wait for the lock"
    );
    drop(guard);
    let report = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("apply must proceed once the lock is free")
        .expect("task not panicked")
        .expect("an absent session.json is a no-op, not an error");
    assert!(!report.changed);
    assert_eq!(report.action, FixAction::DeleteSessionJson);
}

/// `tokio::sync::Mutex` is not reentrant: a preflight that holds the lock
/// and auto-fixes with a whole-stage action would deadlock in silence on
/// `apply`. `apply_holding_lock` routes the stage through
/// [`crate::stages::run_stage_holding_lock`] instead.
///
/// `RunInstall` on a context naming a bottle that does not exist aborts in
/// `require_bottle`, the first line of the stage — far enough in to prove
/// the dispatch happened, and short of anything that mutates.
#[tokio::test]
async fn apply_holding_lock_runs_a_stage_fix_under_a_lock_the_caller_holds() {
    let ctx = scratch_ctx("holding", Some("NoSuchBottleSabrageTest"));
    let sink = null_sink();
    let guard = acquire_operation_lock().await;

    let err = tokio::time::timeout(
        Duration::from_secs(5),
        apply_holding_lock(FixAction::RunInstall, &ctx, &sink),
    )
    .await
    .expect("apply_holding_lock must not wait on the lock its caller holds")
    .expect_err("the bottle does not exist");
    assert!(
        err.to_string()
            .starts_with("bottle 'NoSuchBottleSabrageTest' not found at "),
        "{err}"
    );

    drop(guard);
}

/// Write a `session-state.json` under `ctx`'s (scratch) Sabrage store whose
/// recorded wine identity is **this** process: alive, reporting the start
/// time recorded for it — what `session::reconcile::classify` calls `Live`.
fn record_a_live_session(ctx: &StageCtx) {
    let path = ctx.paths.session_state_path();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut state = crate::session::state::SessionState::new(
        uuid::Uuid::new_v4(),
        "FixtureBottle",
        "/bs",
        "/log",
        0,
    );
    state.wine = crate::process::ProcInfo::observe(std::process::id());
    assert!(state.wine.is_some(), "this process must be observable");
    std::fs::write(&path, serde_json::to_string(&state).unwrap()).unwrap();
}

/// r1:A4-1 regression: `forbidden_while_session_live` used to be metadata
/// nothing read: every fix was applicable mid-session, including the one
/// that removes the `--wired` forwards the stream is running over. The
/// registry drives the test, so a new fix cannot slip through unenforced,
/// and the recording fake `adb` proves the refusal lands before the fix
/// ever spawns `adb`.
#[tokio::test]
async fn apply_refuses_every_session_forbidden_fix_while_a_session_is_live() {
    let _g = crate::session::lock_session_globals();
    let mut ctx = scratch_ctx("live-refusal", Some("FixtureBottle"));
    record_a_live_session(&ctx);

    // A fake adb that records every invocation; the refusal means the log
    // is never created.
    let adb = ctx.paths.root.join("adb.sh");
    let log = ctx.paths.root.join("adb-invoked.log");
    std::fs::write(
        &adb,
        format!("#!/bin/sh\necho \"$@\" >> {}\nexit 0\n", log.display()),
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&adb).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&adb, perms).unwrap();
    }
    ctx.paths.adb = Some(adb);

    let sink = null_sink();

    for action in FixAction::EVERY {
        assert!(
            action.def().forbidden_while_session_live,
            "{action} is not marked forbidden — this test needs updating"
        );
        let err = apply(action, &ctx, &sink)
            .await
            .expect_err("a live session must refuse every fix");
        let msg = err.to_string();
        assert!(
            msg.starts_with(&format!(
                "refusing to apply '{action}' while a session is live"
            )),
            "{msg}"
        );
        assert!(msg.contains("FixtureBottle"), "{msg}");
    }

    assert!(
        !log.exists(),
        "adb must never be spawned while a session runs"
    );

    std::fs::remove_dir_all(&ctx.paths.root).ok();
}

/// The launch preflight's door keeps working on the same state: `run`
/// applies auto-fixes *before* it publishes a live session, and one of them
/// (`set_graphics_backend_for_launch`) exists to edit while a stale
/// wineserver is alive. Gating there would make a relaunch impossible.
#[tokio::test]
async fn apply_holding_lock_is_not_gated_by_a_live_session() {
    let _g = crate::session::lock_session_globals();
    let ctx = scratch_ctx("live-preflight", None);
    record_a_live_session(&ctx);
    let sink = null_sink();

    let guard = acquire_operation_lock().await;
    let report = apply_holding_lock(FixAction::DeleteSessionJson, &ctx, &sink)
        .await
        .expect("the preflight door is not gated");
    assert!(!report.changed);
    drop(guard);

    std::fs::remove_dir_all(&ctx.paths.root).ok();
}

#[test]
fn contract_ids_round_trip() {
    for a in FixAction::EVERY {
        let id = a.to_contract_id();
        assert!(id.starts_with(CONTRACT_FIX_PREFIX));
        // A withheld remedy is the one case where the round trip stops: no
        // `from_contract_id`, therefore no button anywhere in the GUI.
        let expected = if DEFERRED_CONTRACT_FIX_IDS.contains(&id.as_str()) {
            None
        } else {
            Some(a)
        };
        assert_eq!(FixAction::from_contract_id(&id), expected);
        // The serde spelling is the bare id.
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            format!("\"{}\"", a.as_str())
        );
        // FromStr takes either spelling — including for a withheld action,
        // which stays reachable from `sabrage fix <id>`.
        assert_eq!(FixAction::from_str(a.as_str()).unwrap(), a);
        assert_eq!(FixAction::from_str(&id).unwrap(), a);
    }
    assert_eq!(FixAction::from_contract_id("set-graphics-backend"), None);
    assert!(FixAction::from_str("fix.nope").is_err());
}

/// r1:A12-1 regression: the withheld `session.json` deletion must state its
/// known outcome before anything can run it — `destructive`, plus a
/// `consequence` naming the 800x900 black screen, the backup it takes
/// first and the in-place recovery. The no-button half of the withholding
/// is pinned by `is_deferred_is_exactly_the_withheld_set`.
#[test]
fn the_known_bad_session_json_deletion_documents_its_outcome() {
    let def = FixAction::DeleteSessionJson.def();
    assert!(def.destructive);
    let consequence = def
        .consequence
        .expect("the known outcome must be stated before the mutation, not after it");
    assert!(consequence.contains("800x900"), "{consequence}");
    assert!(consequence.contains("backups"), "{consequence}");
    assert!(consequence.contains("in place"), "{consequence}");

    // Every other fix says nothing extra: `consequence` is for remedies
    // whose outcome is worse than the row they fix, not a description field.
    for a in FixAction::EVERY {
        assert_eq!(
            a.def().consequence.is_some(),
            a == FixAction::DeleteSessionJson,
            "{a} carries an unexpected consequence string"
        );
    }
}

#[test]
fn every_contract_fix_id_is_modelled_or_explicitly_deferred() {
    let mut unmodelled: Vec<&str> = contract()
        .checks
        .iter()
        .filter_map(|c| c.fix.as_deref())
        .filter(|id| FixAction::from_contract_id(id).is_none())
        .collect();
    unmodelled.sort_unstable();
    unmodelled.dedup();
    assert_eq!(
        unmodelled, DEFERRED_CONTRACT_FIX_IDS,
        "a contract fix id is neither modelled as a FixAction nor listed in \
             DEFERRED_CONTRACT_FIX_IDS"
    );
}

#[test]
fn every_autofix_gate_maps_to_a_modelled_action() {
    use crate::contract::Gate;
    for check in &contract().checks {
        if check.native_gate == Gate::Autofix || check.shell_gate == Gate::Autofix {
            let id = check.fix.as_deref().expect("autofix gate declares a fix");
            assert!(
                FixAction::from_contract_id(id).is_some(),
                "{}: autofix gate names unmodelled fix {id}",
                check.slug
            );
        }
    }
}

#[test]
fn registry_covers_every_action_exactly_once() {
    assert_eq!(fix_defs().len(), FixAction::EVERY.len());
    for a in FixAction::EVERY {
        assert_eq!(a.def().action, a);
    }
    // Only install can prompt for admin (design-core §5: exactly one
    // privileged write in the whole pipeline).
    let admin: Vec<FixAction> = fix_defs()
        .iter()
        .filter(|d| d.needs_admin)
        .map(|d| d.action)
        .collect();
    assert_eq!(admin, vec![FixAction::RunInstall]);
    // Only the session.json deletion destroys user state.
    let destructive: Vec<FixAction> = fix_defs()
        .iter()
        .filter(|d| d.destructive)
        .map(|d| d.action)
        .collect();
    assert_eq!(destructive, vec![FixAction::DeleteSessionJson]);
    assert!(fix_defs().iter().all(|d| d.forbidden_while_session_live));
}

/// The refusal is re-run with the operation lock in hand.
///
/// The window: a fix admitted while idle waits behind another operation;
/// a `run` acquires first, publishes its live session and hands the lock
/// back at its launch boundary - leaving `remove-adb-forwards` free to
/// delete the very forwards the `--wired` stream is running over.
#[tokio::test]
async fn a_queued_fix_is_refused_when_a_session_goes_live_during_the_wait() {
    let _g = crate::session::lock_session_globals();
    let ctx = scratch_ctx("queued-live", Some("FixtureBottle"));
    let oxr = ctx.paths.oxr_appsup.clone();
    let root = ctx.paths.root.clone();
    assert_eq!(
        crate::stages::live_session_block(&ctx.paths),
        None,
        "the fixture must be idle at admission"
    );

    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let s = seen.clone();
    let sink: EventSink = std::sync::Arc::new(move |ev| s.lock().unwrap().push(ev));

    let held = acquire_operation_lock().await;
    let task = {
        let sink = sink.clone();
        tokio::spawn(async move { apply(FixAction::RestageHelper, &ctx, &sink).await })
    };

    // Admitted: the announcement row proves it got past the pre-lock door.
    let announced = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let has = seen.lock().unwrap().iter().any(
                |ev| matches!(ev, StageEvent::Line { text, .. } if text.contains("applying fix")),
            );
            if has {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    assert!(announced.is_ok(), "a queued fix must announce itself");

    // A session starts while it waits — the `run` that won the lock race.
    // Fresh *and* naming a live pid (this process): both halves are what
    // `session::watcher::runtime_status_live` requires.
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
        .expect("the queued fix must settle once the lock is free")
        .expect("the fix task must not panic")
        .expect_err("a fix that acquires the lock under a live session is refused");
    assert!(
        err.to_string()
            .starts_with("refusing to apply 'restage-helper' while a session is live"),
        "{err}"
    );

    std::fs::remove_dir_all(&root).ok();
}

/// The wait is named and cancellable: the first event carries `ctx.run_id`
/// (the front-end's only handle on the operation) and Cancel ends the wait
/// without applying anything.
#[tokio::test]
async fn a_queued_fix_carries_its_run_id_and_cancels_out_of_the_wait() {
    let _g = crate::session::lock_session_globals();
    let ctx = scratch_ctx("queued-cancel", None);
    let root = ctx.paths.root.clone();
    let run_id = ctx.run_id;
    let cancel = ctx.cancel.clone();

    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let s = seen.clone();
    let sink: EventSink = std::sync::Arc::new(move |ev| s.lock().unwrap().push(ev));

    let held = acquire_operation_lock().await;
    let task = {
        let sink = sink.clone();
        tokio::spawn(async move { apply(FixAction::DeleteSessionJson, &ctx, &sink).await })
    };

    let announced = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let found = seen.lock().unwrap().iter().any(|ev| {
                matches!(ev, StageEvent::Line { run_id: r, step, text, .. }
                        if *r == run_id
                            && step.as_deref() == Some("fix.delete-session-json")
                            && text.contains("applying fix"))
            });
            if found {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    assert!(
        announced.is_ok(),
        "the run id must be emitted before the wait: {:?}",
        seen.lock().unwrap()
    );
    assert!(seen.lock().unwrap().iter().any(|ev| matches!(
        ev,
        StageEvent::Line { text, .. } if text.contains("waiting for another Sabrage operation")
    )));

    cancel.cancel();
    let err = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("cancelling must end the wait")
        .expect("the fix task must not panic")
        .expect_err("a cancelled fix does not report a FixReport");
    assert!(matches!(err, SabrageError::Cancelled), "{err:?}");
    assert_eq!(err.exit_code(), 130);

    drop(held);
    std::fs::remove_dir_all(&root).ok();
}

/// A binary pointed at a checkout it was not built from must not mutate
/// through the fix door either — it would write its *own* ports, pins and
/// templates into someone else's tree.
#[tokio::test]
async fn apply_refuses_a_checkout_the_binary_was_not_built_from() {
    let _g = crate::session::lock_session_globals();
    let ctx = scratch_ctx("contract-skew", Some("FixtureBottle"));
    // Overwrite the materialized contract with foreign bytes.
    for rel in crate::contract::CONTRACT_FILES {
        std::fs::write(
            ctx.paths.root.join(rel),
            b"not-the-contract-this-binary-was-built-from\n",
        )
        .unwrap();
    }
    let sink = null_sink();
    // The refusal is `checks::meta`'s wording — the same sentence the
    // `meta.contract-sync` doctor row shows.
    let (expected, _) = crate::checks::meta::assert_binary_matches_checkout(&ctx.paths.root)
        .expect_err("the fixture must be a skewed checkout");

    for action in FixAction::EVERY {
        let err = apply(action, &ctx, &sink)
            .await
            .expect_err("a foreign checkout must refuse every fix");
        assert_eq!(err.to_string(), expected, "{action}");
    }

    std::fs::remove_dir_all(&ctx.paths.root).ok();
}

/// The step a fix's own rows are attributed to is its contract id — the
/// `&'static str` form of it, since [`crate::events::StepId`] cannot be a
/// `String`.
#[test]
fn step_ids_are_the_contract_ids() {
    for a in FixAction::EVERY {
        assert_eq!(a.step_id(), a.to_contract_id());
    }
    // The one module that hard-codes its own copy agrees.
    assert_eq!(
        FixAction::RemoveAdbForwards.step_id(),
        "fix.remove-adb-forwards"
    );
}

/// r1:A4-2 regression: a known-broken destructive remedy renders no Fix button.
/// `is_deferred` is the [`FixAction`]-shaped form of the withheld set: the
/// Tauri `fix` command needs it to refuse an action the GUI should never
/// have offered (its TypeScript mirror of the fix table can render a button
/// `from_contract_id` would not).
#[test]
fn is_deferred_is_exactly_the_withheld_set() {
    let deferred: Vec<String> = FixAction::EVERY
        .into_iter()
        .filter(|a| a.is_deferred())
        .map(|a| a.to_contract_id())
        .collect();
    assert_eq!(deferred, vec!["fix.delete-session-json".to_string()]);
    for a in FixAction::EVERY {
        assert_eq!(
            a.is_deferred(),
            FixAction::from_contract_id(&a.to_contract_id()).is_none(),
            "{a}: is_deferred must agree with the no-button rule"
        );
    }
}

#[test]
fn whole_stage_actions_map_to_their_stage() {
    assert_eq!(FixAction::RunSetup.as_stage(), Some(Stage::Setup));
    assert_eq!(FixAction::RunBuild.as_stage(), Some(Stage::Build));
    assert_eq!(FixAction::RunInstall.as_stage(), Some(Stage::Install));
    assert_eq!(FixAction::RestageHelper.as_stage(), None);
}
