use super::*;
use crate::checks::{CheckOptions, CheckStatus};
use crate::paths::Paths;
use std::fs;
use std::path::Path;

fn scratch(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("sabrage-game-test-{}-{tag}", std::process::id()))
}

#[test]
fn no_bottle_no_override_skips_both_slugs_with_the_verbatim_reason() {
    let ctx = CheckCtx::new(Paths::new(Path::new("/repo")), CheckOptions::new());

    let p = game_present(&ctx);
    assert_eq!(p.status, CheckStatus::Skipped);
    assert_eq!(p.message, SECTION_SKIP_REASON);

    let v = game_version(&ctx);
    assert_eq!(v.status, CheckStatus::Skipped);
    assert_eq!(v.message, SECTION_SKIP_REASON);
}

#[test]
fn bs_dir_override_without_a_bottle_still_runs_the_section() {
    let tmp = scratch("override-only");
    let opts = CheckOptions {
        bs_dir_override: Some(tmp.clone()),
        ..CheckOptions::new()
    };
    let ctx = CheckCtx::new(Paths::new(Path::new("/repo")), opts);

    let o = game_present(&ctx);
    assert_eq!(o.status, CheckStatus::Fail);
    assert_eq!(
        o.message,
        format!("Beat Saber 1.29.4 not found at {}", tmp.display())
    );
    assert!(o
        .remedy
        .as_deref()
        .unwrap()
        .ends_with("  (or set WINEVR_BS_DIR)"));

    let v = game_version(&ctx);
    assert_eq!(v.status, CheckStatus::Skipped);
}

#[test]
fn exe_present_with_matching_marker_passes_both() {
    let tmp = scratch("versioned");
    fs::create_dir_all(&tmp).unwrap();
    fs::write(tmp.join("Beat Saber.exe"), b"stub").unwrap();
    fs::write(tmp.join("BeatSaberVersion.txt"), "1.29.4_4575554838\n").unwrap();
    let opts = CheckOptions {
        bs_dir_override: Some(tmp.clone()),
        ..CheckOptions::new()
    };
    let ctx = CheckCtx::new(Paths::new(Path::new("/repo")), opts);

    let p = game_present(&ctx);
    assert_eq!(p.status, CheckStatus::Pass);

    let v = game_version(&ctx);
    assert_eq!(v.status, CheckStatus::Pass);
    assert_eq!(
        v.message,
        format!("Beat Saber 1.29.4_4575554838 at {}", tmp.display())
    );
    fs::remove_dir_all(&tmp).ok();
}

#[test]
fn exe_present_with_other_version_warns() {
    let tmp = scratch("wrong-version");
    fs::create_dir_all(&tmp).unwrap();
    fs::write(tmp.join("Beat Saber.exe"), b"stub").unwrap();
    fs::write(tmp.join("BeatSaberVersion.txt"), "1.34.2_9999999999\n").unwrap();
    let opts = CheckOptions {
        bs_dir_override: Some(tmp.clone()),
        ..CheckOptions::new()
    };
    let ctx = CheckCtx::new(Paths::new(Path::new("/repo")), opts);

    let v = game_version(&ctx);
    assert_eq!(v.status, CheckStatus::Warn);
    assert_eq!(
        v.message,
        "Beat Saber version '1.34.2_9999999999' is not 1.29.4 — the Meta account gate may block it"
    );
    fs::remove_dir_all(&tmp).ok();
}
