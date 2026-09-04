use super::*;
use crate::checks::CheckOptions;
use crate::paths::Paths;

fn ctx() -> CheckCtx {
    CheckCtx::new(Paths::new("/nonexistent/repo"), CheckOptions::new())
}

#[test]
fn tool_check_pass_and_fail_shapes() {
    // /bin/sh is on PATH on every macOS box this pipeline targets.
    let pass = tool_check("tool.cmake", "sh");
    assert_eq!(pass.status, CheckStatus::Pass);
    assert_eq!(pass.message, "sh");
    assert!(pass.remedy.is_none());

    let fail = tool_check("tool.cmake", "definitely-not-a-real-binary-xyz");
    assert_eq!(fail.status, CheckStatus::Fail);
    assert_eq!(fail.message, "definitely-not-a-real-binary-xyz missing");
    assert_eq!(fail.remedy.as_deref(), Some(TOOLCHAIN_REMEDY));
}

#[test]
fn every_tool_evaluator_matches_ground_truth_and_shares_the_remedy() {
    // Whether each tool is actually installed is machine state; assert
    // the shape is internally consistent with a direct `which` probe
    // rather than asserting a fixed pass/fail outcome.
    let c = ctx();
    for (check_fn, bin) in [
        (tool_cmake as Evaluator, "cmake"),
        (tool_ninja as Evaluator, "ninja"),
        (tool_git as Evaluator, "git"),
        (tool_curl as Evaluator, "curl"),
        (tool_mingw as Evaluator, "x86_64-w64-mingw32-gcc"),
    ] {
        let out = check_fn(&c);
        if which(bin).is_some() {
            assert_eq!(out.status, CheckStatus::Pass);
            assert_eq!(out.message, bin);
        } else {
            assert_eq!(out.status, CheckStatus::Fail);
            assert_eq!(out.message, format!("{bin} missing"));
            assert_eq!(out.remedy.as_deref(), Some(TOOLCHAIN_REMEDY));
        }
    }
}

#[test]
fn rust_x64_target_matches_ground_truth() {
    let out = rust_x64_target(&ctx());
    let want = which("rustup").is_some() && rustup_has_x86_64_darwin();
    if want {
        assert_eq!(out.status, CheckStatus::Pass);
        assert_eq!(out.message, "rustup with x86_64-apple-darwin target");
        assert!(out.remedy.is_none());
    } else {
        assert_eq!(out.status, CheckStatus::Fail);
        assert_eq!(out.message, "rustup x86_64-apple-darwin target missing");
        assert!(out.remedy.is_some());
    }
}
