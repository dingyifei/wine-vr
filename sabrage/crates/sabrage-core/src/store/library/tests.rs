use super::*;
use crate::executor::{DryRunExecutor, PlannedKind, RealExecutor};
use crate::stages::null_sink;
use std::fs;
use tokio_util::sync::CancellationToken;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "sabrage-store-library-test-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn real() -> RealExecutor {
    RealExecutor::new(Uuid::nil(), null_sink(), CancellationToken::new())
}

fn entry(name: &str) -> GameEntry {
    GameEntry {
        id: Uuid::new_v4(),
        name: name.to_string(),
        bs_dir: "/games/bs".to_string(),
        bottle: "Steam".to_string(),
        appid: 620980,
        added_at_unix_ms: 1786300214181,
        launch_overrides: LaunchOverrides::default(),
        last_session: None,
    }
}

#[test]
fn library_path_is_the_json_file_under_appsup() {
    assert_eq!(
        library_path(Path::new("/x/Sabrage")),
        PathBuf::from("/x/Sabrage/library.json")
    );
}

#[test]
fn missing_file_loads_as_default() {
    let lib = load(Path::new("/nonexistent/sabrage/library.json")).unwrap();
    assert_eq!(lib, Library::default());
}

#[test]
fn a_corrupt_file_is_an_error_never_a_silent_reset() {
    let dir = scratch("corrupt");
    let path = dir.join("library.json");
    fs::write(&path, b"{not json").unwrap();
    let err = load(&path).unwrap_err();
    assert_eq!(err.kind(), "io");
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_newer_schema_version_is_refused_not_silently_rewritten() {
    let dir = scratch("newer-version");
    let path = dir.join("library.json");
    let text = format!(
        r#"{{"version":{},"games":[],"futureTopLevel":"keep-me"}}"#,
        LIBRARY_VERSION + 1
    );
    fs::write(&path, &text).unwrap();

    let err = load(&path).unwrap_err();
    assert!(err.to_string().contains("version"), "{err}");
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        text,
        "a refused load never touches the file"
    );

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn unknown_fields_are_ignored_on_load() {
    let dir = scratch("unknown-fields");
    let path = dir.join("library.json");
    fs::write(
        &path,
        r#"{"version":1,"games":[],"futureField":{"nested":true}}"#,
    )
    .unwrap();
    let lib = load(&path).unwrap();
    assert_eq!(lib, Library::default());
    fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn round_trips_camel_case_through_the_file() {
    let dir = scratch("roundtrip");
    let path = dir.join("nested/library.json");
    let mut lib = Library::default();
    let mut e = entry("Beat Saber 1.29.4");
    e.launch_overrides.wired = Some(true);
    e.last_session = Some(LastSession {
        started_at_unix_ms: 1,
        ended_at_unix_ms: 2,
        exit_code: Some(0),
        log_path: Some("/repo/logs/x.log".into()),
    });
    lib.upsert(e);

    save(&real(), &path, &lib).await.unwrap();
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.ends_with("}\n"));
    assert!(text.contains("\"bsDir\""));
    assert!(text.contains("\"launchOverrides\""));
    assert!(text.contains("\"startedAtUnixMs\""));
    assert_eq!(load(&path).unwrap(), lib);

    fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn a_dry_run_executor_plans_the_write_instead_of_performing_it() {
    let dir = scratch("dry");
    let path = dir.join("library.json");
    let ex = DryRunExecutor::new(Uuid::nil(), null_sink(), CancellationToken::new());
    save(&ex, &path, &Library::default()).await.unwrap();
    assert!(!path.exists());
    let kinds: Vec<PlannedKind> = ex.planned().iter().map(|p| p.kind).collect();
    assert_eq!(kinds, vec![PlannedKind::CreateDir, PlannedKind::Write]);
    fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn transact_writes_only_when_the_library_actually_changed() {
    let dir = scratch("transact-noop");
    let path = dir.join("library.json");

    // A removal that finds nothing must not mint a library.json.
    let removed = transact(&real(), &path, |lib| lib.remove(Uuid::new_v4()))
        .await
        .unwrap();
    assert!(!removed);
    assert!(!path.exists(), "no change, no write");

    let e = entry("A");
    let id = e.id;
    transact(&real(), &path, |lib| {
        lib.upsert(e);
    })
    .await
    .unwrap();
    assert!(load(&path).unwrap().get(id).is_some());

    fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn interleaved_transactions_do_not_resurrect_a_removed_game() {
    // The shape this pins: the Library screen removes a game while the
    // post-launch task records that same game's last session. Without
    // `transact`, each loads its own snapshot and saves the whole file
    // back, and whichever renames last wins outright.
    let dir = scratch("transact-race");
    let path = dir.join("library.json");
    let a = entry("A");
    let b = entry("B");
    let (id_a, id_b) = (a.id, b.id);
    let mut seed = Library::default();
    seed.upsert(a);
    seed.upsert(b);
    save(&real(), &path, &seed).await.unwrap();

    let session = LastSession {
        started_at_unix_ms: 1,
        ended_at_unix_ms: 2,
        exit_code: Some(0),
        log_path: None,
    };
    let (ex_a, ex_b) = (real(), real());
    let (removal, record) = tokio::join!(
        transact(&ex_a, &path, |lib| lib.remove(id_a)),
        transact(&ex_b, &path, {
            let session = session.clone();
            move |lib| lib.record_last_session(id_a, session)
        }),
    );
    assert!(removal.unwrap(), "the removal found the entry");
    let _ = record.unwrap(); // may or may not have found it — order decides

    let after = load(&path).unwrap();
    assert!(
        after.get(id_a).is_none(),
        "a removed game must never come back: {:?}",
        after.games.iter().map(|g| &g.name).collect::<Vec<_>>()
    );
    assert!(after.get(id_b).is_some(), "the other entry is untouched");

    fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn an_edit_racing_a_recorded_session_keeps_both() {
    // A13b-5's half: the editor submits a whole entry it cloned before the
    // session was recorded. `upsert_editable` keeps the server-owned
    // fields, `transact` keeps the two writes from clobbering each other.
    let dir = scratch("transact-edit");
    let path = dir.join("library.json");
    let e = entry("A");
    let id = e.id;
    let mut seed = Library::default();
    seed.upsert(e.clone());
    save(&real(), &path, &seed).await.unwrap();

    let session = LastSession {
        started_at_unix_ms: 10,
        ended_at_unix_ms: 20,
        exit_code: Some(0),
        log_path: Some("/repo/logs/x.log".into()),
    };
    transact(&real(), &path, |lib| {
        lib.record_last_session(id, session.clone())
    })
    .await
    .unwrap();

    // The editor's stale clone: renamed, and still carrying no session.
    let mut stale = e;
    stale.name = "A renamed".to_string();
    transact(&real(), &path, |lib| {
        lib.upsert_editable(stale);
    })
    .await
    .unwrap();

    let after = load(&path).unwrap();
    let stored = after.get(id).unwrap();
    assert_eq!(stored.name, "A renamed", "the edit landed");
    assert_eq!(
        stored.last_session.as_ref(),
        Some(&session),
        "the session recorded while the form was open survived"
    );

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn upsert_inserts_then_replaces_by_id() {
    let mut lib = Library::default();
    let e = entry("A");
    let stored = lib.upsert(e.clone());
    assert_eq!(stored, &e);
    assert_eq!(lib.games.len(), 1);

    let mut replacement = e.clone();
    replacement.name = "A renamed".to_string();
    let stored = lib.upsert(replacement.clone());
    assert_eq!(stored.name, "A renamed");
    assert_eq!(lib.games.len(), 1, "same id replaces, does not append");
}

#[test]
fn remove_reports_whether_it_found_something() {
    let mut lib = Library::default();
    let e = entry("A");
    let id = e.id;
    lib.upsert(e);
    assert!(lib.remove(id));
    assert!(lib.games.is_empty());
    assert!(
        !lib.remove(id),
        "removing twice finds nothing the second time"
    );
}

#[test]
fn get_finds_by_id_and_nothing_else() {
    let mut lib = Library::default();
    let e = entry("A");
    let id = e.id;
    lib.upsert(e);
    assert!(lib.get(id).is_some());
    assert!(lib.get(Uuid::new_v4()).is_none());
}

#[test]
fn upsert_editable_keeps_the_server_owned_fields_of_the_stored_entry() {
    let mut lib = Library::default();
    let mut stored = entry("A");
    stored.added_at_unix_ms = 1;
    stored.appid = 620980;
    stored.last_session = Some(LastSession {
        started_at_unix_ms: 5,
        ended_at_unix_ms: 6,
        exit_code: Some(0),
        log_path: None,
    });
    let expected_session = stored.last_session.clone();
    lib.upsert(stored.clone());

    // What the Edit-game form submits: the editable fields it changed,
    // plus whatever the clone happened to carry for the rest.
    let mut incoming = stored.clone();
    incoming.name = "A renamed".to_string();
    incoming.bottle = "Other".to_string();
    incoming.launch_overrides.wired = Some(true);
    incoming.last_session = None;
    incoming.added_at_unix_ms = 999;
    incoming.appid = 1;

    let saved = lib.upsert_editable(incoming).clone();
    assert_eq!(saved.name, "A renamed");
    assert_eq!(saved.bottle, "Other");
    assert_eq!(saved.launch_overrides.wired, Some(true));
    assert_eq!(saved.last_session, expected_session, "server-owned");
    assert_eq!(saved.added_at_unix_ms, 1, "server-owned");
    assert_eq!(saved.appid, 620980, "server-owned");
    assert_eq!(lib.games.len(), 1);

    // An id the library does not know is a plain insert.
    let fresh = entry("B");
    let fresh_id = fresh.id;
    lib.upsert_editable(fresh);
    assert!(lib.get(fresh_id).is_some());
}

#[test]
fn record_last_session_updates_the_matching_entry_only() {
    let mut lib = Library::default();
    let a = entry("A");
    let b = entry("B");
    let (id_a, id_b) = (a.id, b.id);
    lib.upsert(a);
    lib.upsert(b);

    let session = LastSession {
        started_at_unix_ms: 100,
        ended_at_unix_ms: 200,
        exit_code: Some(0),
        log_path: Some("/repo/logs/beatsaber-x.log".into()),
    };
    assert!(lib.record_last_session(id_a, session.clone()));
    assert_eq!(lib.get(id_a).unwrap().last_session.as_ref(), Some(&session));
    assert!(lib.get(id_b).unwrap().last_session.is_none());

    assert!(!lib.record_last_session(Uuid::new_v4(), session));
}

#[test]
fn template_prefers_settings_default_bottle_over_the_bottle_list() {
    let settings = Settings {
        default_bottle: Some("Preferred".into()),
        ..Settings::default()
    };
    let e = new_entry_template(&settings, &["Other".into()], None);
    assert_eq!(e.bottle, "Preferred");
    assert_eq!(e.name, "Beat Saber 1.29.4");
    assert_eq!(e.appid, 620980);
    assert!(e.last_session.is_none());
    assert_eq!(e.launch_overrides, LaunchOverrides::default());
}

#[test]
fn template_falls_back_to_the_first_bottle_then_empty_string() {
    let e = new_entry_template(
        &Settings::default(),
        &["First".into(), "Second".into()],
        None,
    );
    assert_eq!(e.bottle, "First");

    let e = new_entry_template(&Settings::default(), &[], None);
    assert_eq!(e.bottle, "");
}

#[test]
fn template_bs_dir_precedence_settings_then_env_then_resolved_default() {
    let settings_dir = Settings {
        default_bottle: Some("Steam".into()),
        default_bs_dir: Some("/from/settings".into()),
        ..Settings::default()
    };
    let e = new_entry_template(&settings_dir, &[], Some("/from/env"));
    assert_eq!(e.bs_dir, "/from/settings", "settings wins over env");

    let settings_no_dir = Settings {
        default_bottle: Some("Steam".into()),
        ..Settings::default()
    };
    let e = new_entry_template(&settings_no_dir, &[], Some("/from/env"));
    assert_eq!(e.bs_dir, "/from/env", "env wins when settings has none");

    let e = new_entry_template(&settings_no_dir, &[], None);
    assert!(
        e.bs_dir
            .ends_with("Steam/drive_c/Program Files (x86)/Steam/steamapps/common/Beat Saber 1294"),
        "falls back to resolve_bs_dir's default: {}",
        e.bs_dir
    );
}

#[test]
fn effective_options_merges_overrides_over_settings_and_takes_identity_from_the_entry() {
    let settings = Settings {
        launch: crate::store::settings::LaunchDefaults {
            no_audio: false,
            no_dashboard: true,
            wired: false,
            verbose: false,
            ..Default::default()
        },
        ..Settings::default()
    };
    let mut e = entry("A");
    e.launch_overrides = LaunchOverrides {
        no_audio: Some(true), // override flips the global default
        no_dashboard: None,   // falls through to settings (true)
        wired: None,          // falls through to settings (false)
        verbose: Some(true),  // override sets what settings left false
    };

    let opts = effective_options(&settings, &e);
    assert_eq!(opts.bottle_name.as_deref(), Some("Steam"));
    assert_eq!(opts.bs_dir_override, Some(PathBuf::from("/games/bs")));
    assert!(!opts.dry_run);
    assert!(opts.no_audio, "override Some(true) beats settings false");
    assert!(opts.no_dashboard, "None falls through to settings true");
    assert!(!opts.wired, "None falls through to settings false");
    assert!(opts.verbose, "override Some(true) beats settings false");
}

#[test]
fn launch_options_for_resolves_the_merge_by_id_and_is_none_for_a_stranger() {
    let settings = Settings {
        launch: crate::store::settings::LaunchDefaults {
            no_audio: false,
            no_dashboard: true,
            wired: false,
            verbose: false,
            ..Default::default()
        },
        ..Settings::default()
    };
    let mut e = entry("A");
    e.launch_overrides.no_audio = Some(true);
    let id = e.id;
    let mut lib = Library::default();
    lib.upsert(e.clone());

    let opts = lib.launch_options_for(id, &settings).unwrap();
    assert_eq!(
        opts,
        effective_options(&settings, &e),
        "one merge, one home"
    );
    assert!(lib.launch_options_for(Uuid::new_v4(), &settings).is_none());
}

fn fake_bottle(label: &str) -> Bottle {
    let dir = std::env::temp_dir().join(format!(
        "sabrage-library-test-bottle-{label}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    Bottle {
        name: "TestBottle".to_string(),
        sys32: dir.join("drive_c/windows/system32"),
        prefix: dir,
    }
}

fn paths() -> Paths {
    Paths::new("/nonexistent/sabrage/repo")
}

#[test]
fn no_bottle_no_exe_is_not_found_with_a_says_why_problem() {
    let bs_dir = scratch("validate-notfound");
    let v = validate(&paths(), &bs_dir, "");
    assert!(!v.exe_present);
    assert!(!v.bottle_exists, "empty bottle name never exists");
    assert_eq!(v.status, GameStatus::NotFound);
    assert!(v.problems.iter().any(|p| p.contains("Beat Saber.exe")));
    fs::remove_dir_all(&bs_dir).unwrap();
}

#[test]
fn exe_present_but_bottle_missing_needs_setup() {
    let bs_dir = scratch("validate-needssetup");
    fs::write(bs_dir.join("Beat Saber.exe"), b"stub").unwrap();
    fs::write(bs_dir.join("BeatSaberVersion.txt"), "1.29.4_4575554838\n").unwrap();
    let v = validate(&paths(), &bs_dir, "NoSuchBottle");
    assert!(v.exe_present);
    assert!(v.version_ok);
    assert!(!v.bottle_exists);
    assert_eq!(v.status, GameStatus::NeedsSetup);
    assert!(v
        .problems
        .iter()
        .any(|p| p.contains("CrossOver bottle 'NoSuchBottle' not found")));
    fs::remove_dir_all(&bs_dir).unwrap();
}

#[test]
fn wrong_version_needs_attention() {
    let bs_dir = scratch("validate-wrongversion");
    fs::write(bs_dir.join("Beat Saber.exe"), b"stub").unwrap();
    fs::write(bs_dir.join("BeatSaberVersion.txt"), "1.34.2_9999999999\n").unwrap();
    let b = fake_bottle("wrongversion");
    fs::write(
        b.conf_path(),
        "\"Template\" = \"win11_64\"\n\"CX_GRAPHICS_BACKEND\" = \"dxmt\"\n",
    )
    .unwrap();

    let v = validate_with_bottle(&paths().gbe_dll, &bs_dir, &b.name, &b);
    assert_eq!(v.detected_version.as_deref(), Some("1.34.2_9999999999"));
    assert!(!v.version_ok);
    assert!(v.bottle_exists);
    assert_eq!(v.status, GameStatus::NeedsAttention);
    assert!(v.problems.iter().any(|p| p.contains("is not 1.29.4")));

    fs::remove_dir_all(&bs_dir).unwrap();
    fs::remove_dir_all(&b.prefix).unwrap();
}

#[test]
fn outside_drive_c_without_z_drive_needs_attention() {
    let b = fake_bottle("nozdrive");
    fs::write(
        b.conf_path(),
        "\"Template\" = \"win11_64\"\n\"CX_GRAPHICS_BACKEND\" = \"dxmt\"\n",
    )
    .unwrap();
    let bs_dir = scratch("validate-outside");
    fs::write(bs_dir.join("Beat Saber.exe"), b"stub").unwrap();
    fs::write(bs_dir.join("BeatSaberVersion.txt"), "1.29.4_4575554838\n").unwrap();

    let v = validate_with_bottle(&paths().gbe_dll, &bs_dir, &b.name, &b);
    assert!(
        v.outside_drive_c,
        "scratch dir is not under the bottle's drive_c"
    );
    assert_eq!(v.z_drive_ok, Some(false));
    assert_eq!(v.status, GameStatus::NeedsAttention);
    assert!(v.problems.iter().any(|p| p.contains("no z: drive")));

    fs::remove_dir_all(&bs_dir).unwrap();
    fs::remove_dir_all(&b.prefix).unwrap();
}

#[test]
fn a_fully_healthy_game_is_ready_with_no_problems() {
    let b = fake_bottle("ready");
    fs::create_dir_all(b.prefix.join("drive_c")).unwrap();
    fs::write(
        b.conf_path(),
        "\"Template\" = \"win11_64\"\n\"CX_GRAPHICS_BACKEND\" = \"dxmt\"\n",
    )
    .unwrap();
    let bs_dir = b
        .prefix
        .join("drive_c/Program Files (x86)/Steam/steamapps/common/Beat Saber 1294");
    fs::create_dir_all(&bs_dir).unwrap();
    fs::write(bs_dir.join("Beat Saber.exe"), b"stub").unwrap();
    fs::write(bs_dir.join("BeatSaberVersion.txt"), "1.29.4_4575554838\n").unwrap();
    // Ready requires the dll run.sh's `# launch-action: goldberg-stage`
    // would otherwise die on — see
    // `healthy_game_without_steam_dll_is_not_ready`.
    fs::write(bs_dir.join("steam_api64.dll"), b"REAL-STEAM").unwrap();

    let v = validate_with_bottle(&paths().gbe_dll, &bs_dir, &b.name, &b);
    assert!(v.exe_present && v.version_ok && v.bottle_exists && v.bottle_backend_dxmt);
    assert!(
        !v.outside_drive_c,
        "install lives under the bottle's drive_c"
    );
    assert_eq!(
        v.z_drive_ok, None,
        "z: is irrelevant when not outside drive_c"
    );
    assert_eq!(v.bottle_template.as_deref(), Some("win11_64"));
    assert_eq!(v.status, GameStatus::Ready);
    assert!(v.problems.is_empty(), "{:?}", v.problems);

    fs::remove_dir_all(&b.prefix).unwrap();
}

#[test]
fn bottle_template_and_backend_mismatches_surface_as_problems_without_forcing_needs_attention() {
    // Template and backend are detail-row facts, not launch gates: a
    // wrong value surfaces as a problem but never moves `status` off
    // Ready.
    let b = fake_bottle("mismatch");
    fs::create_dir_all(b.prefix.join("drive_c")).unwrap();
    fs::write(
        b.conf_path(),
        "\"Template\" = \"win10_64\"\n\"CX_GRAPHICS_BACKEND\" = \"auto\"\n",
    )
    .unwrap();
    let bs_dir = b.prefix.join("drive_c/bs");
    fs::create_dir_all(&bs_dir).unwrap();
    fs::write(bs_dir.join("Beat Saber.exe"), b"stub").unwrap();
    fs::write(bs_dir.join("BeatSaberVersion.txt"), "1.29.4_4575554838\n").unwrap();
    fs::write(bs_dir.join("steam_api64.dll"), b"REAL-STEAM").unwrap();

    let v = validate_with_bottle(&paths().gbe_dll, &bs_dir, &b.name, &b);
    assert_eq!(v.bottle_template.as_deref(), Some("win10_64"));
    assert!(!v.bottle_backend_dxmt);
    assert_eq!(v.status, GameStatus::Ready);
    assert!(v.problems.iter().any(|p| p.contains("win11_64")));
    assert!(v.problems.iter().any(|p| p.contains("not dxmt")));

    fs::remove_dir_all(&b.prefix).unwrap();
}

fn plugin_dir(bs_dir: &Path) -> PathBuf {
    bs_dir.join("Beat Saber_Data/Plugins/x86_64")
}

#[test]
fn goldberg_state_covers_all_five_variants() {
    let bs_dir = scratch("validate-goldberg");
    let dir = plugin_dir(&bs_dir);
    fs::create_dir_all(&dir).unwrap();
    let bottle = fake_bottle("goldberg-matrix");
    // The pin the fixture's "Goldberg" bytes actually hash to — no test
    // can fabricate bytes matching the contract's real digest, which is
    // exactly what `validate_pinned` exists for.
    let pin = crate::util::sha256_bytes(b"GOLDBERG-EMULATOR-BYTES");
    // The checkout's staged Goldberg payload — the *other* way a dll is
    // known to be Goldberg. Absent for every case below except the last.
    let payload = bs_dir.join("third_party-gbe-steam_api64.dll");
    let v = |pin: &str| validate_pinned(&payload, &bs_dir, &bottle.name, &bottle, pin);

    // No dll at all.
    assert_eq!(v(&pin).goldberg, GoldbergState::NoDll);
    assert!(!v(&pin).orig_steam_present);

    // A dll that is not Goldberg, no backup: never Goldberg'd.
    fs::write(dir.join("steam_api64.dll"), b"REAL-STEAM").unwrap();
    assert_eq!(v(&pin).goldberg, GoldbergState::Original);
    assert!(!v(&pin).orig_steam_present);

    // The Goldberg dll with **no** backup: not `Original` — the bytes
    // prove otherwise (an install that arrived already Goldberg'd).
    fs::write(dir.join("steam_api64.dll"), b"GOLDBERG-EMULATOR-BYTES").unwrap();
    let got = v(&pin);
    assert_eq!(got.goldberg, GoldbergState::AppliedUnverified);
    assert!(!got.orig_steam_present);

    // Backup present, live dll does not match the pin: Modified.
    fs::write(dir.join("steam_api64.dll.orig-steam"), b"REAL-STEAM").unwrap();
    fs::write(dir.join("steam_api64.dll"), b"SOME-OTHER-BYTES").unwrap();
    let got = v(&pin);
    assert_eq!(got.goldberg, GoldbergState::Modified);
    assert!(got.orig_steam_present);

    // Backup present, live dll matches the pin: Applied.
    fs::write(dir.join("steam_api64.dll"), b"GOLDBERG-EMULATOR-BYTES").unwrap();
    assert_eq!(v(&pin).goldberg, GoldbergState::Applied);

    // A13a-1: a Goldberg build that is **not** the pin — an older or
    // hand-swapped `third_party/gbe/steam_api64.dll`, or a dll installed
    // before a pin bump — with no backup. Byte-identical to the payload
    // this checkout installs, so it is not the untouched Steam original,
    // and the revert door must not offer to "restore" it.
    fs::remove_file(dir.join("steam_api64.dll.orig-steam")).unwrap();
    fs::write(&payload, b"CUSTOM-GOLDBERG-BUILD").unwrap();
    fs::write(dir.join("steam_api64.dll"), b"CUSTOM-GOLDBERG-BUILD").unwrap();
    let got = v(&pin); // `pin` matches the *other* fixture bytes, never these
    assert_eq!(got.goldberg, GoldbergState::AppliedUnverified);
    // …and with a backup alongside it, the ordinary applied state.
    fs::write(dir.join("steam_api64.dll.orig-steam"), b"REAL-STEAM").unwrap();
    assert_eq!(v(&pin).goldberg, GoldbergState::Applied);
    // Restore the matrix's tail state for the contract-pin assertion below.
    fs::remove_file(&payload).unwrap();
    fs::write(dir.join("steam_api64.dll"), b"GOLDBERG-EMULATOR-BYTES").unwrap();

    // And the real contract pin still flows through `validate` itself:
    // these fixture bytes are not it, so the same tree reads as Modified.
    assert_eq!(
        validate(&paths(), &bs_dir, "").goldberg,
        GoldbergState::Modified
    );

    fs::remove_dir_all(&bs_dir).unwrap();
    fs::remove_dir_all(&bottle.prefix).unwrap();
}

#[test]
fn healthy_game_without_steam_dll_is_not_ready() {
    // Everything run.sh checks except the dll its
    // `# launch-action: goldberg-stage` block dies on.
    let b = fake_bottle("nodll");
    fs::create_dir_all(b.prefix.join("drive_c")).unwrap();
    fs::write(
        b.conf_path(),
        "\"Template\" = \"win11_64\"\n\"CX_GRAPHICS_BACKEND\" = \"dxmt\"\n",
    )
    .unwrap();
    let bs_dir = b.prefix.join("drive_c/bs");
    fs::create_dir_all(&bs_dir).unwrap();
    fs::write(bs_dir.join("Beat Saber.exe"), b"stub").unwrap();
    fs::write(bs_dir.join("BeatSaberVersion.txt"), "1.29.4_4575554838\n").unwrap();

    let v = validate_with_bottle(&paths().gbe_dll, &bs_dir, &b.name, &b);
    assert_eq!(v.goldberg, GoldbergState::NoDll);
    assert_ne!(
        v.status,
        GameStatus::Ready,
        "the launch would die on the missing dll"
    );
    assert_eq!(v.status, GameStatus::NeedsAttention);
    assert!(
        v.problems.iter().any(|p| p.contains("steam_api64.dll")),
        "{:?}",
        v.problems
    );

    fs::remove_dir_all(&b.prefix).unwrap();
}
