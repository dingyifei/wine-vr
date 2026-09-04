use super::*;

/// The real checkout, three levels above this crate's manifest.
fn real_repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("repo root resolves")
}

#[test]
fn repo_root_walks_up_to_the_marker_pair() {
    let root = real_repo_root();
    assert_eq!(
        find_repo_root_from(&root.join("sabrage/target/debug/sabrage")),
        Some(root.clone())
    );
    assert_eq!(
        find_repo_root_from(&root.join("a/b/Sabrage.app/Contents/MacOS/sabrage-app")),
        Some(root)
    );
    assert_eq!(
        find_repo_root_from(Path::new("/nonexistent/sabrage/bin/sabrage")),
        None
    );
}

#[test]
fn explicit_override_wins_and_is_canonicalized() {
    let root = real_repo_root();
    let messy = format!("{}/sabrage/..", root.display());
    assert_eq!(resolve_repo_root(Some(&messy)).unwrap(), root);
    // An empty override falls through to the next source rather than
    // resolving to "".
    assert_ne!(
        resolve_repo_root(Some("")).ok(),
        Some(PathBuf::from("")),
        "empty override must not be taken literally"
    );
    // A non-existent explicit root is accepted verbatim (fixture roots).
    assert_eq!(
        resolve_repo_root(Some("/nonexistent/sabrage/fixture")).unwrap(),
        PathBuf::from("/nonexistent/sabrage/fixture")
    );
}

#[test]
fn a_relative_root_becomes_absolute_without_resolving_symlinks() {
    let cwd = std::env::current_dir().unwrap();
    assert_eq!(
        resolve_repo_root(Some("./fixtures/root")).unwrap(),
        cwd.join("fixtures/root")
    );
    // Lexical `..`, and `/..` is `/`.
    assert_eq!(
        resolve_repo_root(Some("/a/b/../c")).unwrap(),
        PathBuf::from("/a/c")
    );
    assert_eq!(resolve_repo_root(Some("/..")).unwrap(), PathBuf::from("/"));
}

#[test]
fn home_is_required_to_be_absolute_and_non_empty() {
    use std::ffi::OsString;
    // Unset and empty are both refused: an empty `$HOME` would make every
    // store path relative to the working directory.
    assert!(check_home(None).is_err());
    assert!(check_home(Some(OsString::from(""))).is_err());
    // Relative is refused too.
    let rel = check_home(Some(OsString::from("relative/home"))).unwrap_err();
    assert_eq!(rel.kind(), "fatal");
    assert!(rel.remedy().is_some(), "a refusal must carry a remedy");
    // Absolute is accepted verbatim.
    assert_eq!(
        check_home(Some(OsString::from("/Users/someone"))).unwrap(),
        PathBuf::from("/Users/someone")
    );
    // The checked constructor agrees with the unchecked one on a machine
    // whose HOME is fine (every test runner's).
    assert_eq!(Paths::new_checked("/repo").unwrap(), Paths::new("/repo"));
}

#[test]
fn paths_are_derived_from_the_explicit_root() {
    let p = Paths::new("/repo");
    assert_eq!(p.oxrsys, PathBuf::from("/repo/ext/oxrsys"));
    assert_eq!(p.oxr_build, PathBuf::from("/repo/ext/oxrsys/build-x64"));
    assert_eq!(
        p.oxr_dylib,
        PathBuf::from("/repo/ext/oxrsys/build-x64/runtime/liboxrsys-runtime.dylib")
    );
    assert_eq!(
        p.oxr_helper_built,
        PathBuf::from("/repo/ext/oxrsys/build-helper-arm64/runtime/oxrsys-encoder-helper")
    );
    assert_eq!(
        p.oxr_helper_staged,
        PathBuf::from("/repo/ext/oxrsys/build-x64/runtime/oxrsys-encoder-helper")
    );
    assert_eq!(
        p.woxr_dll,
        PathBuf::from("/repo/ext/wineopenxr/build/src/pe/wineopenxr.dll")
    );
    assert_eq!(
        p.woxr_so,
        PathBuf::from("/repo/ext/wineopenxr/build/src/unix/wineopenxr.so")
    );
    assert_eq!(
        p.alvr_dashboard,
        PathBuf::from("/repo/ext/ALVR/target/release/alvr_dashboard")
    );
    assert_eq!(
        p.gbe_dll,
        PathBuf::from("/repo/third_party/gbe/steam_api64.dll")
    );
    assert_eq!(
        p.host_xr_json,
        PathBuf::from("/usr/local/share/openxr/1/active_runtime.x86_64.json")
    );
    assert_eq!(p.logs_dir(), PathBuf::from("/repo/logs"));
    // Sabrage's own store is $HOME-derived, never repo-derived.
    assert!(p
        .sabrage_appsup
        .ends_with("Library/Application Support/Sabrage"));
    assert_eq!(
        p.session_state_path(),
        p.sabrage_appsup.join("session-state.json")
    );
    assert_eq!(
        p.session_state_lock_path(),
        p.sabrage_appsup.join("session-state.lock")
    );
    assert_eq!(
        p.toml_lock_path(),
        p.oxr_appsup.join(".oxrsys-runtime.toml.lock")
    );
    assert_eq!(p.rel_display(Path::new("/repo/ext/oxrsys")), "ext/oxrsys");
    assert_eq!(p.rel_display(Path::new("/elsewhere")), "/elsewhere");
}

#[test]
fn crossover_helpers_are_none_without_crossover() {
    // Whether CrossOver exists on this machine is machine state; assert the
    // invariant instead: cx/wine/wineserver are Some exactly when cx_app is.
    let p = Paths::new("/repo");
    assert_eq!(p.cx.is_some(), p.cx_app.is_some());
    assert_eq!(p.wine.is_some(), p.cx_app.is_some());
    assert_eq!(p.wineserver.is_some(), p.cx_app.is_some());
    assert_eq!(
        p.cx_dxmt("x86_64-unix/winemetal.so").is_some(),
        p.cx.is_some()
    );
    if let Some(cx) = &p.cx {
        assert!(cx.ends_with("Contents/SharedSupport/CrossOver"));
        assert_ne!(cx, Path::new("/Contents/SharedSupport/CrossOver"));
    }
}

#[test]
fn bottle_paths_match_lib_sh() {
    let b = Bottle::unvalidated("Steam");
    assert!(b
        .prefix
        .ends_with("Library/Application Support/CrossOver/Bottles/Steam"));
    assert!(b.sys32.ends_with("Steam/drive_c/windows/system32"));
    assert!(b.conf_path().ends_with("Steam/cxbottle.conf"));
    assert!(b.z_drive().ends_with("Steam/dosdevices/z:"));
    assert!(b
        .openxr_manifest()
        .ends_with("Steam/drive_c/openxr/wineopenxr64.json"));
}

#[test]
fn bs_dir_override_wins_and_default_uses_the_contract_leaf() {
    let b = Bottle::unvalidated("Steam");
    let over = PathBuf::from("/games/bs");
    assert_eq!(resolve_bs_dir(Some(&b), Some(&over)), over);
    assert_eq!(resolve_bs_dir(None, Some(&over)), over);

    let def = resolve_bs_dir(Some(&b), None);
    assert!(def.starts_with(&b.prefix));
    assert!(def.ends_with("drive_c/Program Files (x86)/Steam/steamapps/common/Beat Saber 1294"));

    // No bottle, no override: the empty-$PREFIX shell quirk, reproduced.
    assert_eq!(
        resolve_bs_dir(None, None),
        PathBuf::from("/drive_c/Program Files (x86)/Steam/steamapps/common/Beat Saber 1294")
    );
}
