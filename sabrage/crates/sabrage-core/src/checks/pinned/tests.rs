use super::*;
use crate::checks::{CheckOptions, CheckStatus};
use crate::paths::Paths;
use std::fs;
use std::path::Path;

fn ctx_for(root: &Path) -> CheckCtx {
    CheckCtx::new(Paths::new(root), CheckOptions::new())
}

fn scratch(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("sabrage-pinned-test-{}-{tag}", std::process::id()))
}

#[test]
fn dxmt_missing_files_fails() {
    let tmp = scratch("dxmt-missing");
    let ctx = ctx_for(&tmp);
    let o = dep_dxmt(&ctx);
    assert_eq!(o.status, CheckStatus::Fail);
    assert_eq!(o.message, "ext/dxmt-artifacts missing or incomplete");
    assert_eq!(o.remedy.as_deref(), Some("./demo.sh setup"));
}

#[test]
fn dxmt_files_present_but_no_marker_warns() {
    let tmp = scratch("dxmt-nomarker");
    for f in &contract().dxmt.files {
        let p = tmp.join("ext/dxmt-artifacts").join(f);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, b"stub").unwrap();
    }
    let o = dep_dxmt(&ctx_for(&tmp));
    assert_eq!(o.status, CheckStatus::Warn);
    assert_eq!(
            o.message,
            "dxmt-artifacts present but provenance marker missing/stale — ./demo.sh setup re-fetches the pinned set"
        );
    fs::remove_dir_all(&tmp).ok();
}

#[test]
fn dxmt_files_and_current_marker_passes() {
    let tmp = scratch("dxmt-ok");
    for f in &contract().dxmt.files {
        let p = tmp.join("ext/dxmt-artifacts").join(f);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, b"stub").unwrap();
    }
    fs::write(
        tmp.join("ext/dxmt-artifacts/.sha256"),
        format!("{}\n", contract().deps.dxmt_tgz_sha256),
    )
    .unwrap();
    let o = dep_dxmt(&ctx_for(&tmp));
    assert_eq!(o.status, CheckStatus::Pass);
    assert_eq!(
        o.message,
        "dxmt-artifacts (monofunc fork) present, provenance verified"
    );
    fs::remove_dir_all(&tmp).ok();
}

#[test]
fn goldberg_missing_fails() {
    let tmp = scratch("gbe-missing");
    let o = dep_goldberg(&ctx_for(&tmp));
    assert_eq!(o.status, CheckStatus::Fail);
    assert_eq!(o.message, "Goldberg dll missing");
    assert_eq!(o.remedy.as_deref(), Some("./demo.sh setup"));
}

#[test]
fn goldberg_present_with_wrong_hash_warns_with_detail() {
    let tmp = scratch("gbe-wrong");
    let p = tmp.join("third_party/gbe/steam_api64.dll");
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(&p, b"not the pinned build").unwrap();
    let o = dep_goldberg(&ctx_for(&tmp));
    assert_eq!(o.status, CheckStatus::Warn);
    assert_eq!(
        o.message,
        "Goldberg dll present but hash differs from the pinned build"
    );
    assert!(o.detail.is_some());
    fs::remove_dir_all(&tmp).ok();
}
