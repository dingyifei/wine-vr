use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

/// Serializes every test in this module that touches a `WINEVR_*`
/// variable: `std::env::set_var`/`remove_var` are process-global. Named
/// for `WINEVR_BOTTLE` but not scoped to it —
/// `launch_stage_options_layers_the_launch_flags_with_gui_precedence`
/// holds it for the four launch flags too.
static WINEVR_BOTTLE_MUTEX: Mutex<()> = Mutex::new(());

#[test]
fn browse_start_prefers_the_field_then_the_bottle_derived_dir_then_home() {
    let home = Path::new("/Users/me");
    let existing = |p: &Path| -> Option<PathBuf> {
        // Only /Volumes/Games and the bottle's drive_c "exist".
        p.ancestors()
            .find(|a| {
                *a == Path::new("/Volumes/Games")
                    || a.to_string_lossy().ends_with("/Bottles/Steam/drive_c")
            })
            .map(Path::to_path_buf)
    };
    // Field set → its nearest existing ancestor wins even with a bottle.
    let s = suggest_bs_dir_with(
        Some("Steam"),
        Some("/Volumes/Games/Beat Saber 1294"),
        home,
        existing,
    );
    assert_eq!(s.browse_start, "/Volumes/Games");
    assert!(s.derived.ends_with(
        "/Bottles/Steam/drive_c/Program Files (x86)/Steam/steamapps/common/Beat Saber 1294"
    ));
    // Empty field → the bottle-derived dir's nearest existing ancestor.
    let s = suggest_bs_dir_with(Some("Steam"), Some("   "), home, existing);
    assert!(
        s.browse_start.ends_with("/Bottles/Steam/drive_c"),
        "{}",
        s.browse_start
    );
    // No bottle, nothing existing → $HOME, and no derived path at all.
    let s = suggest_bs_dir_with(None, None, home, |_| None);
    assert_eq!(s.derived, "");
    assert_eq!(s.browse_start, "/Users/me");
    let s = suggest_bs_dir_with(Some(""), Some(""), home, |_| None);
    assert_eq!(s.derived, "");
}

#[test]
fn settings_defaults_fill_only_what_env_and_gui_left_unset() {
    let settings = settings::Settings {
        default_bottle: Some("Steam".to_string()),
        default_bs_dir: Some("/Volumes/Games/Beat Saber 1294".to_string()),
        ..settings::Settings::default()
    };
    // Nothing set anywhere else → both defaults apply.
    let filled = fill_stage_options_from_settings(StageOptions::default(), &settings);
    assert_eq!(filled.bottle_name.as_deref(), Some("Steam"));
    assert_eq!(
        filled.bs_dir_override.as_deref(),
        Some(Path::new("/Volumes/Games/Beat Saber 1294"))
    );
    // An env/GUI-supplied value is never overridden by a default.
    let preset = StageOptions {
        bottle_name: Some("VR".to_string()),
        bs_dir_override: Some(PathBuf::from("/elsewhere")),
        ..StageOptions::default()
    };
    let kept = fill_stage_options_from_settings(preset.clone(), &settings);
    assert_eq!(kept, preset);
    // Empty strings in settings.json count as unset, not as "" paths.
    let blank = settings::Settings {
        default_bottle: Some(String::new()),
        default_bs_dir: Some(String::new()),
        ..settings::Settings::default()
    };
    let untouched = fill_stage_options_from_settings(StageOptions::default(), &blank);
    assert_eq!(untouched, StageOptions::default());
}

#[test]
fn stage_options_from_env_and_gui_honours_winevr_bottle_when_the_gui_passes_none() {
    // Finding #4: the env base is read before the GUI's own args, so a
    // GUI `None` still picks up `WINEVR_BOTTLE`; without it a Session
    // screen's `stop_session(None)` dies with "bottle name required".
    let _guard = WINEVR_BOTTLE_MUTEX.lock().expect("mutex poisoned");
    let prev = std::env::var("WINEVR_BOTTLE").ok();

    // SAFETY: serialized by `WINEVR_BOTTLE_MUTEX` above; no other thread
    // reads/writes `WINEVR_BOTTLE` while this guard is held.
    unsafe { std::env::set_var("WINEVR_BOTTLE", "Steam") };

    // GUI passed `bottle: null` (`None`) — the env value must still win.
    let opts = stage_options_from_env_and_gui(None, None);
    assert_eq!(opts.bottle_name.as_deref(), Some("Steam"));

    // An explicit GUI value still overrides the env base.
    let opts = stage_options_from_env_and_gui(Some("Other".to_string()), None);
    assert_eq!(opts.bottle_name.as_deref(), Some("Other"));

    unsafe {
        match &prev {
            Some(v) => std::env::set_var("WINEVR_BOTTLE", v),
            None => std::env::remove_var("WINEVR_BOTTLE"),
        }
    }
}

#[test]
fn launch_stage_options_layers_the_launch_flags_with_gui_precedence() {
    const VARS: [&str; 4] = [
        "WINEVR_NO_AUDIO",
        "WINEVR_NO_DASHBOARD",
        "WINEVR_WIRED",
        "WINEVR_VERBOSE",
    ];
    let _guard = WINEVR_BOTTLE_MUTEX.lock().expect("mutex poisoned");
    let saved: Vec<Option<String>> = VARS.iter().map(|v| std::env::var(v).ok()).collect();

    // SAFETY: serialized by `WINEVR_BOTTLE_MUTEX` above.
    unsafe {
        for v in VARS {
            std::env::remove_var(v);
        }
    }

    // Nothing supplied by the GUI: every flag falls back to its (now
    // cleared) env default, and `dry_run` — which has no env counterpart
    // at all — defaults to `false`.
    let stage_opts = launch_stage_options_from_env_and_gui(&LaunchOpts::default());
    assert!(!stage_opts.no_audio);
    assert!(!stage_opts.no_dashboard);
    assert!(!stage_opts.wired);
    assert!(!stage_opts.verbose);
    assert!(!stage_opts.dry_run);

    // The GUI supplies `Some(true)` for two of the four; the other two
    // must stay at the env default rather than being forced to `false`.
    let opts = LaunchOpts {
        no_audio: Some(true),
        wired: Some(true),
        dry_run: Some(true),
        ..LaunchOpts::default()
    };
    let stage_opts = launch_stage_options_from_env_and_gui(&opts);
    assert!(stage_opts.no_audio && stage_opts.wired);
    assert!(!stage_opts.no_dashboard && !stage_opts.verbose);
    assert!(stage_opts.dry_run);

    // SAFETY: serialized by `WINEVR_BOTTLE_MUTEX` above.
    unsafe {
        for (v, prev) in VARS.iter().zip(saved) {
            match prev {
                Some(val) => std::env::set_var(v, val),
                None => std::env::remove_var(v),
            }
        }
    }
}

#[test]
fn stop_targets_live_session_matches_none_or_the_same_bottle_only() {
    // Finding #8: the live branch is scoped by bottle, so stopping bottle
    // B can never tear down a live session on bottle A.
    assert!(
        stop_targets_live_session(None, "Steam"),
        "no bottle requested means \"stop whatever session is live\""
    );
    assert!(stop_targets_live_session(Some("Steam"), "Steam"));
    assert!(
        !stop_targets_live_session(Some("Other"), "Steam"),
        "a different bottle must fall through to the Stop stage instead"
    );
}

#[test]
fn wait_for_slot_clear_reports_the_deadline_it_hit() {
    // A11-1: `TeardownWait` separates a finished teardown from one still
    // running at the deadline, so `stop_session` cannot report both as
    // success.
    let flips = Arc::new(AtomicU64::new(0));
    let f = flips.clone();
    assert_eq!(
        wait_for_slot_clear(
            move || f.fetch_add(1, Ordering::SeqCst) >= 2,
            Duration::from_secs(5),
        ),
        TeardownWait::Cleared,
        "a predicate that flips mid-poll is a completed teardown"
    );
    assert!(flips.load(Ordering::SeqCst) >= 3);

    assert_eq!(
        wait_for_slot_clear(|| false, Duration::from_millis(0)),
        TeardownWait::TimedOut,
        "a slot that never clears must report the timeout, not success"
    );
}

#[test]
fn the_stop_and_quit_refusal_claims_only_what_happened() {
    // A11-1 (round 2): the stop has already fired the session's cancel
    // token and `reconcile::detach` returns `Ok(())` once it is set, so
    // the `TimedOut` message may not claim a detach it cannot back.
    assert_eq!(quit_stop_refusal(TeardownWait::NothingLive), None);
    assert_eq!(
        quit_stop_refusal(TeardownWait::Cleared),
        None,
        "a clean teardown is a silent, successful quit"
    );
    let timed_out = quit_stop_refusal(TeardownWait::TimedOut).expect("an unfinished teardown");
    assert!(
        !timed_out.contains("detach"),
        "must not claim a detach that provably did not happen: {timed_out}"
    );
    assert!(
        timed_out.contains("may still be running") && timed_out.contains("./demo.sh stop"),
        "must still say what is left and how to finish it: {timed_out}"
    );
    let detached = quit_stop_refusal(TeardownWait::Detached).expect("a detach is not a stop");
    assert!(
        detached.contains("detached instead of stopping"),
        "the one arm where a detach really did happen still names it: {detached}"
    );
}

#[test]
fn a_withheld_fix_reaches_no_doctor_row_and_no_fix_call() {
    // A4-2 / A12-1: `DEFERRED_CONTRACT_FIX_IDS` is enforced at both IPC
    // doors, not only in `FixAction::from_contract_id`, so a row such as
    // `cfg.session-pins` cannot render a Fix button for a withheld id.
    for id in sabrage_core::fixes::DEFERRED_CONTRACT_FIX_IDS {
        assert_eq!(
            offered_fix_id(Some(id)),
            None,
            "{id} is withheld and must never reach the client"
        );
    }
    assert_eq!(offered_fix_id(None), None);
    assert_eq!(
        offered_fix_id(Some("fix.set-graphics-backend")),
        Some("fix.set-graphics-backend".to_string()),
        "an offered id passes through unchanged"
    );
    assert_eq!(
        offered_fix_id(Some("fix.not-a-real-fix")),
        None,
        "an id no FixAction models is not offerable either"
    );

    // The contract's own `cfg.session-pins` row is the reachable case.
    let session_pins = contract().check("cfg.session-pins").expect("slug present");
    assert_eq!(
        session_pins.fix.as_deref(),
        Some("fix.delete-session-json"),
        "the contract still names the remedy; withholding it is Sabrage's decision"
    );
    assert_eq!(offered_fix_id(session_pins.fix.as_deref()), None);

    // …and the `fix` command refuses it even if a frontend asks anyway.
    let refusal = gui_fix_refusal(FixAction::DeleteSessionJson, true)
        .expect("a withheld fix is refused however it was reached");
    assert!(
        refusal.contains("not offered by this build"),
        "unexpected refusal: {refusal}"
    );
    assert!(
        gui_fix_refusal(FixAction::RestageHelper, false).is_none(),
        "an ordinary fix needs neither confirmation nor an exemption"
    );
}

#[test]
fn quit_is_intercepted_once_and_given_up_on_when_nobody_answers() {
    // A11-4: the dialog has exactly one responder (the webview's
    // `app://quit-requested` listener), so without the deadline a webview
    // that never answers leaves every Cmd-Q and window close prevented.
    assert_eq!(
        quit_intercept_decision(false, true, None),
        QuitIntercept::Ask,
        "the first request over a live session opens the dialog"
    );
    assert_eq!(
        quit_intercept_decision(false, true, Some(Duration::from_secs(1))),
        QuitIntercept::Ask,
        "a dialog the user is still reading keeps being asked"
    );
    assert_eq!(
        quit_intercept_decision(false, true, Some(QUIT_DIALOG_TIMEOUT)),
        QuitIntercept::GiveUp,
        "asking again after the deadline stops asking — the app must stay quittable"
    );
    assert_eq!(
        quit_intercept_decision(true, true, Some(Duration::from_secs(600))),
        QuitIntercept::PassThrough,
        "an approved quit is never intercepted, pending clock or not"
    );
    assert_eq!(
        quit_intercept_decision(false, false, Some(Duration::from_secs(600))),
        QuitIntercept::PassThrough,
        "nothing live to protect"
    );
    assert_eq!(
        quit_intercept_decision(true, false, None),
        QuitIntercept::PassThrough,
        "an approved quit with nothing live is never intercepted"
    );
}

#[test]
fn pending_quit_keeps_the_first_instant_and_clears() {
    let pending = PendingQuit::default();
    assert_eq!(pending.pending_for(), None);
    pending.mark();
    let first = pending.pending_for().expect("marked");
    pending.mark();
    assert!(
        pending.pending_for().expect("still marked") >= first,
        "a repeated request must not refresh the deadline it is meant to trip"
    );
    pending.clear();
    assert_eq!(pending.pending_for(), None);
}

#[test]
fn a_tail_unregisters_itself_when_its_task_ends() {
    // A11-5: a tail unregisters itself when its task ends, not only via
    // `stop_log_tail`, so `stop_log_tail` cannot answer `true` for a
    // dead task.
    let registry = TailRegistry::default();
    let stop = Arc::new(AtomicBool::new(false));
    let (id, guard) = registry.register(stop.clone());
    assert!(registry.stop(id), "a live tail is tracked");
    assert!(
        stop.load(Ordering::SeqCst),
        "stopping flips the task's flag"
    );
    drop(guard);

    // A second tail whose task ends on its own (the guard drops) must
    // leave nothing behind.
    let stop2 = Arc::new(AtomicBool::new(false));
    let (id2, guard2) = registry.register(stop2.clone());
    drop(guard2);
    assert!(
        !registry.stop(id2),
        "a tail whose task has ended is no longer tracked"
    );
    assert!(
        !stop2.load(Ordering::SeqCst),
        "and nothing is signalled — the task is already gone"
    );
}

#[test]
fn stop_all_stops_every_tracked_tail() {
    let registry = TailRegistry::default();
    let flags: Vec<Arc<AtomicBool>> = (0..3).map(|_| Arc::new(AtomicBool::new(false))).collect();
    let guards: Vec<TailGuard> = flags
        .iter()
        .map(|f| registry.register(f.clone()).1)
        .collect();
    registry.stop_all();
    assert!(flags.iter().all(|f| f.load(Ordering::SeqCst)));
    // Every id is gone, so a later `stop_log_tail` answers honestly.
    for g in &guards {
        assert!(!registry.stop(g.id));
    }
    drop(guards);
}

#[test]
fn the_status_broadcast_skips_repeats_but_never_the_first_one() {
    // E-A11: the 1 Hz broadcast drops repeats in the backend; the
    // frontend store is not relied on to dedup.
    let state = SessionMonitorState::default();
    let idle = SessionStatus::default();
    assert!(
        state.remember_broadcast(&idle),
        "the first one always emits"
    );
    assert!(
        !state.remember_broadcast(&idle),
        "an identical snapshot does not"
    );
    let changed = SessionStatus {
        phase: sabrage_core::session::SessionPhase::Running,
        ..SessionStatus::default()
    };
    assert!(state.remember_broadcast(&changed));
    assert!(!state.remember_broadcast(&changed));
    assert!(state.remember_broadcast(&idle), "and back again");
}

#[test]
fn a_dry_run_emits_the_shared_plan_rows_and_a_real_run_emits_none() {
    // Finding #13 (GUI half): `planned()` rows reach the GUI so a dry run
    // can tell "would copy" from "would skip (bytes already match)", the
    // distinction the plan exists for.
    fn events_for(dry_run: bool) -> Vec<StageEvent> {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        let sink: EventSink = Arc::new(move |ev| s.lock().unwrap().push(ev));
        let ctx = StageCtx::new(
            Paths::new("/nonexistent/sabrage-commands-test"),
            StageOptions {
                dry_run,
                ..StageOptions::default()
            },
            sink,
            Default::default(),
        );
        emit_dry_run_plan(&ctx);
        let evs = seen.lock().unwrap().clone();
        evs
    }

    // A real run's event stream is untouched.
    assert!(events_for(false).is_empty());

    // A dry run gets the section plus the shared body text — here the
    // empty-plan placeholder, since nothing ran.
    let evs = events_for(true);
    assert_eq!(evs.len(), 2, "{evs:?}");
    assert!(
        matches!(&evs[0], StageEvent::Section { title, .. } if title == sabrage_core::DRY_RUN_PLAN_TITLE),
        "{evs:?}"
    );
    assert!(
        matches!(
            &evs[1],
            StageEvent::Line { severity: sabrage_core::Severity::Info, text, .. }
                if text == sabrage_core::DRY_RUN_PLAN_EMPTY
        ),
        "{evs:?}"
    );
}

#[test]
fn run_registry_cancel_is_idempotent_and_reports_whether_a_run_was_found() {
    let registry = RunRegistry::default();
    let fired = Arc::new(AtomicBool::new(false));
    let f = fired.clone();
    registry.register(
        "abc".to_string(),
        Box::new(move || f.store(true, Ordering::SeqCst)),
    );
    assert!(!registry.cancel("does-not-exist"));
    assert!(registry.cancel("abc"));
    assert!(fired.load(Ordering::SeqCst));
    // Cancelling again finds nothing — the entry was removed.
    assert!(!registry.cancel("abc"));
}

#[test]
fn forget_removes_without_firing_the_canceller() {
    let registry = RunRegistry::default();
    let fired = Arc::new(AtomicBool::new(false));
    let f = fired.clone();
    registry.register(
        "run".to_string(),
        Box::new(move || f.store(true, Ordering::SeqCst)),
    );
    registry.forget("run");
    assert!(!registry.cancel("run"));
    assert!(!fired.load(Ordering::SeqCst));
}

// `sabrage-app` depends on no JSON crate, so these tests exercise the
// pure helpers; the serde wire-format round trips live in
// sabrage-core's store/config test modules.

#[test]
fn classify_repo_root_source_follows_resolve_repo_roots_own_precedence() {
    // `resolve_repo_root`'s explicit-override and env tiers cannot
    // themselves fail (`paths.rs`), so an explicit setting wins whether or
    // not the walk would also have succeeded, and env likewise over the walk.
    assert_eq!(
        classify_repo_root_source(true, true, true),
        RepoRootSource::Settings
    );
    assert_eq!(
        classify_repo_root_source(true, false, false),
        RepoRootSource::Settings
    );
    assert_eq!(
        classify_repo_root_source(false, true, true),
        RepoRootSource::Env
    );
    assert_eq!(
        classify_repo_root_source(false, true, false),
        RepoRootSource::Env
    );
    assert_eq!(
        classify_repo_root_source(false, false, true),
        RepoRootSource::Executable
    );
    assert_eq!(
        classify_repo_root_source(false, false, false),
        RepoRootSource::Unresolved
    );
}

#[test]
fn game_row_reflects_a_freshly_computed_validity_not_a_stored_one() {
    // Validity is recomputed, never trusted from the stored JSON: an
    // entry whose bs_dir and bottle do not exist comes back `NotFound`
    // whatever the entry itself claims.
    let paths = Paths::new("/nonexistent/sabrage-commands-game-row-test");
    let entry = library::GameEntry {
        name: "Beat Saber 1.29.4".to_string(),
        bs_dir: "/nonexistent/sabrage-commands-game-row-test/bs".to_string(),
        bottle: "NoSuchBottle".to_string(),
        ..library::GameEntry::default()
    };
    let row = game_row(&paths, entry.clone());
    assert_eq!(row.entry, entry);
    assert_eq!(row.validity.status, library::GameStatus::NotFound);
    assert!(!row.validity.exe_present);
}

#[test]
fn cache_hit_returns_the_stored_pair_without_reloading() {
    // E-C3-settings-paths-cache: seeds the cache directly rather than
    // through `snapshot`'s load-on-miss path, which would touch the real
    // `settings.json`; a populated cache must serve exactly what was stored.
    let settings = settings::Settings {
        default_bottle: Some("Steam".to_string()),
        ..Default::default()
    };
    let paths = Paths::new("/nonexistent/sabrage-settings-cache-test");
    let cache = SettingsPathsCache(std::sync::Mutex::new(Some((
        settings.clone(),
        paths.clone(),
    ))));
    assert_eq!(cache.settings(), settings);
    assert_eq!(cache.paths(), paths);
}

#[test]
fn invalidate_drops_the_cached_pair() {
    let paths = Paths::new("/nonexistent/sabrage-settings-cache-test-2");
    let cache = SettingsPathsCache(std::sync::Mutex::new(Some((
        settings::Settings::default(),
        paths,
    ))));
    cache.invalidate();
    assert!(
        cache.0.lock().unwrap().is_none(),
        "save_settings's invalidate() must leave the next read to reload, \
             not keep serving what a just-completed save made stale"
    );
}

#[test]
fn last_session_to_record_needs_a_game_id_a_launched_event_and_a_settled_outcome() {
    let launched = LaunchedInfo {
        started_at_unix_ms: 1_000,
        log_path: "logs/beatsaber-x.log".to_string(),
    };
    let outcome = StageOutcome {
        stage: Stage::Run,
        ok: true,
        exit_code_equiv: 0,
    };

    assert!(
        last_session_to_record(None, Some(&launched), Some(&outcome)).is_none(),
        "no gameId -> nothing to record (an ad hoc Session-screen launch)"
    );
    assert!(
        last_session_to_record(Some("abc"), None, Some(&outcome)).is_none(),
        "no Launched event observed -> nothing to record (died in preflight)"
    );
    assert!(
        last_session_to_record(Some("abc"), Some(&launched), None).is_none(),
        "no settled outcome -> nothing to record (no exit_code_equiv to attach)"
    );

    let (game_id, session) = last_session_to_record(Some("abc"), Some(&launched), Some(&outcome))
        .expect("all three present");
    assert_eq!(game_id, "abc");
    assert_eq!(session.started_at_unix_ms, 1_000);
    assert_eq!(session.exit_code, Some(0));
    assert_eq!(session.log_path.as_deref(), Some("logs/beatsaber-x.log"));
    assert!(
        session.ended_at_unix_ms >= session.started_at_unix_ms,
        "ended_at is \"now\", which is after the fixed started_at"
    );
}
