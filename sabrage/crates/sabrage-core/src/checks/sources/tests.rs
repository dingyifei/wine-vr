use super::*;
use crate::checks::{CheckOptions, CheckStatus};
use crate::paths::Paths;
use std::fs;

fn ctx_for(root: &Path) -> CheckCtx {
    CheckCtx::new(Paths::new(root), CheckOptions::new())
}

#[test]
fn submodule_missing_fails_with_the_setup_remedy() {
    let ctx = ctx_for(Path::new("/nonexistent/sabrage-sources-probe"));
    let o = src_oxrsys(&ctx);
    assert_eq!(o.status, CheckStatus::Fail);
    assert_eq!(o.message, "submodule oxrsys not initialized");
    assert_eq!(o.remedy.as_deref(), Some("./demo.sh setup"));
}

#[test]
fn submodule_present_as_a_directory_passes() {
    let tmp =
        std::env::temp_dir().join(format!("sabrage-src-test-{}-{}", std::process::id(), "dir"));
    let sub = tmp.join("ext/wineopenxr");
    fs::create_dir_all(sub.join(".git")).unwrap();
    let ctx = ctx_for(&tmp);
    let o = src_wineopenxr(&ctx);
    assert_eq!(o.status, CheckStatus::Pass);
    assert_eq!(o.message, "submodule wineopenxr present");
    fs::remove_dir_all(&tmp).ok();
}

#[test]
fn submodule_present_as_a_gitlink_file_passes() {
    let tmp = std::env::temp_dir().join(format!(
        "sabrage-src-test-{}-{}",
        std::process::id(),
        "file"
    ));
    let sub = tmp.join("ext/ALVR");
    fs::create_dir_all(&sub).unwrap();
    fs::write(sub.join(".git"), "gitdir: ../../.git/modules/ext/ALVR\n").unwrap();
    let ctx = ctx_for(&tmp);
    let o = src_alvr(&ctx);
    assert_eq!(o.status, CheckStatus::Pass);
    assert_eq!(o.message, "submodule ALVR present");
    fs::remove_dir_all(&tmp).ok();
}

#[test]
fn patchset_check_greps_connection_rs() {
    let tmp = std::env::temp_dir().join(format!("sabrage-src-test-{}-patch", std::process::id()));
    let dir = tmp.join("ext/ALVR/alvr/server_core/src");
    fs::create_dir_all(&dir).unwrap();

    // Missing file -> fail.
    let ctx = ctx_for(&tmp);
    let o = src_alvr_patchset(&ctx);
    assert_eq!(o.status, CheckStatus::Fail);
    assert_eq!(o.message, "ALVR submodule missing the oxrsys patches");
    assert_eq!(
        o.remedy.as_deref(),
        Some("./demo.sh setup (checks out the pinned oxrsys-v20.14.1 branch)")
    );

    // Present but without the marker -> still fail.
    fs::write(dir.join("connection.rs"), "fn connect() {}\n").unwrap();
    let o = src_alvr_patchset(&ctx_for(&tmp));
    assert_eq!(o.status, CheckStatus::Fail);

    // Marker present -> pass.
    fs::write(
        dir.join("connection.rs"),
        "fn is_streaming_nonblocking() -> bool { true }\n",
    )
    .unwrap();
    let o = src_alvr_patchset(&ctx_for(&tmp));
    assert_eq!(o.status, CheckStatus::Pass);
    assert_eq!(o.message, "ALVR oxrsys patch set present");

    fs::remove_dir_all(&tmp).ok();
}
