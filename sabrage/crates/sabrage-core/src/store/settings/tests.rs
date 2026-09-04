use super::*;
use crate::executor::{DryRunExecutor, PlannedKind, RealExecutor};
use crate::stages::null_sink;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "sabrage-store-settings-test-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn real() -> RealExecutor {
    RealExecutor::new(Uuid::nil(), null_sink(), CancellationToken::new())
}

#[test]
fn settings_path_is_the_json_file_under_appsup() {
    assert_eq!(
        settings_path(Path::new("/x/Sabrage")),
        PathBuf::from("/x/Sabrage/settings.json")
    );
}

#[test]
fn missing_file_loads_as_default() {
    let s = load(Path::new("/nonexistent/sabrage/settings.json")).unwrap();
    assert_eq!(s, Settings::default());
    assert!(s.allow_adb_probes, "doctor-parity default is true");
    assert!(!s.runtime_config_edit_acknowledged);
    assert!(s.repo_root.is_none() && s.default_bottle.is_none());
}

#[test]
fn a_corrupt_file_is_an_error_never_a_silent_reset() {
    let dir = scratch("corrupt");
    let path = dir.join("settings.json");
    std::fs::write(&path, b"{not json at all").unwrap();
    let err = load(&path).unwrap_err();
    assert_eq!(err.kind(), "io");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn unknown_fields_survive_a_load_save_round_trip() {
    // The downgrade case: an older binary loads a newer file, the user
    // flips one toggle, and the whole object is autosaved back.
    let dir = scratch("downgrade");
    let path = dir.join("settings.json");
    std::fs::write(
        &path,
        r#"{"repoRoot":"/repo","futureSetting":{"nested":true},"futureFlag":false}"#,
    )
    .unwrap();

    let mut s = load(&path).unwrap();
    assert_eq!(
        s.version, SETTINGS_VERSION,
        "a file with no `version` key reads as the current one"
    );
    assert_eq!(s.repo_root.as_deref(), Some("/repo"));
    assert_eq!(s.extra.len(), 2, "{:?}", s.extra);
    assert_eq!(s.extra["futureFlag"], serde_json::json!(false));
    s.allow_adb_probes = false; // the one control the old build knows
    save(&real(), &path, &s).await.unwrap();

    let reread = load(&path).unwrap();
    assert!(!reread.allow_adb_probes);
    assert_eq!(reread.extra, s.extra);

    std::fs::remove_dir_all(&dir).unwrap();
}

/// A13a-3 / A13b-4 regression: a key nested inside `launch` survives a
/// load/save round trip on a downgrade (the version half of the pair is
/// pinned by tests::a_newer_version_is_refused_and_its_bytes_left_alone).
#[tokio::test]
async fn unknown_nested_launch_keys_survive_a_load_save_round_trip() {
    let dir = scratch("nested-downgrade");
    let path = dir.join("settings.json");
    std::fs::write(
        &path,
        r#"{"defaultBottle":"bs","launch":{"noAudio":true,"futureFlag":"keep-me"}}"#,
    )
    .unwrap();

    let mut s = load(&path).unwrap();
    assert!(s.launch.no_audio);
    assert_eq!(
        s.launch.extra["futureFlag"],
        serde_json::json!("keep-me"),
        "a key inside `launch` is preserved, not dropped: {:?}",
        s.launch.extra
    );

    // The autosave shape: one known toggle flipped, whole object written.
    s.launch.wired = true;
    save(&real(), &path, &s).await.unwrap();

    let text = std::fs::read_to_string(&path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed["launch"]["futureFlag"], serde_json::json!("keep-me"));
    assert_eq!(parsed["launch"]["wired"], serde_json::json!(true));
    assert_eq!(load(&path).unwrap(), s);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_newer_version_is_refused_and_its_bytes_left_alone() {
    let dir = scratch("newer-version");
    let path = dir.join("settings.json");
    let bytes = format!(
        r#"{{"version":{},"defaultBottle":"bs","launch":{{"noAudio":true,"futureFlag":"keep-me"}}}}"#,
        SETTINGS_VERSION + 1
    );
    std::fs::write(&path, &bytes).unwrap();

    let err = load(&path).unwrap_err();
    let msg = err.to_string();
    assert_eq!(err.kind(), "fatal", "{msg}");
    assert!(msg.contains("is version 2"), "{msg}");
    assert!(
        err.remedy().unwrap_or_default().contains("update Sabrage"),
        "{:?}",
        err.remedy()
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        bytes,
        "a refusal never rewrites the file"
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn an_ordinary_settings_file_carries_no_extra_keys() {
    let dir = scratch("no-extra");
    let path = dir.join("settings.json");
    save(&real(), &path, &Settings::default()).await.unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    let obj = parsed.as_object().unwrap();
    for key in [
        "version",
        "repoRoot",
        "defaultBottle",
        "defaultBsDir",
        "launch",
        "allowAdbProbes",
        "runtimeConfigEditAcknowledged",
    ] {
        assert!(
            obj.contains_key(key),
            "a defaults-only file must still carry `{key}`: {text}"
        );
    }
    assert_eq!(
        obj.len(),
        7,
        "an empty `extra` must add nothing to the file: {text}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn round_trips_camel_case_through_the_file() {
    let dir = scratch("roundtrip");
    let path = dir.join("nested/settings.json");
    let s = Settings {
        repo_root: Some("/repo".into()),
        default_bottle: Some("Steam".into()),
        default_bs_dir: Some("/games/bs".into()),
        launch: LaunchDefaults {
            no_audio: true,
            no_dashboard: false,
            wired: true,
            verbose: false,
            ..LaunchDefaults::default()
        },
        allow_adb_probes: false,
        runtime_config_edit_acknowledged: true,
        ..Settings::default()
    };
    save(&real(), &path, &s).await.unwrap();

    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.ends_with("}\n"), "pretty JSON plus one newline");
    assert!(text.contains("\"repoRoot\""));
    assert!(text.contains("\"defaultBottle\""));
    assert!(text.contains("\"noAudio\""));
    assert!(text.contains("\"allowAdbProbes\""));
    assert!(text.contains("\"runtimeConfigEditAcknowledged\""));
    assert_eq!(load(&path).unwrap(), s);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_minimal_file_loads_on_defaults() {
    let json = r#"{}"#;
    let s: Settings = serde_json::from_str(json).unwrap();
    assert_eq!(s, Settings::default());
}

#[test]
fn effective_stage_options_carries_bottle_dir_and_flags_with_dry_run_false() {
    let s = Settings {
        default_bottle: Some("Steam".into()),
        default_bs_dir: Some("/games/bs".into()),
        launch: LaunchDefaults {
            no_audio: true,
            no_dashboard: true,
            wired: false,
            verbose: true,
            ..LaunchDefaults::default()
        },
        ..Settings::default()
    };
    let opts = s.effective_stage_options();
    assert_eq!(opts.bottle_name.as_deref(), Some("Steam"));
    assert_eq!(opts.bs_dir_override, Some(PathBuf::from("/games/bs")));
    assert!(!opts.dry_run);
    assert!(opts.no_audio && opts.no_dashboard && opts.verbose && !opts.wired);
}

#[tokio::test]
async fn a_dry_run_executor_plans_the_write_instead_of_performing_it() {
    let dir = scratch("dry");
    let path = dir.join("settings.json");
    let ex = DryRunExecutor::new(Uuid::nil(), null_sink(), CancellationToken::new());
    save(&ex, &path, &Settings::default()).await.unwrap();
    assert!(!path.exists(), "dry run wrote the settings file");
    let kinds: Vec<PlannedKind> = ex.planned().iter().map(|p| p.kind).collect();
    assert_eq!(kinds, vec![PlannedKind::CreateDir, PlannedKind::Write]);
    std::fs::remove_dir_all(&dir).unwrap();
}
