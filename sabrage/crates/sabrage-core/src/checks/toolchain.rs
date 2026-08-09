//! Group `toolchain` — doctor.sh section 4-5: build tools and the rustup cross target.
//!
//! Slugs owned here, in contract order:
//!
//! * `tool.cmake` — `command -v cmake`
//! * `tool.ninja` — `command -v ninja`
//! * `tool.git` — `command -v git`
//! * `tool.curl` — `command -v curl`
//! * `tool.mingw` — `command -v x86_64-w64-mingw32-gcc`
//! * `rust.x64-target` — `rustup` on PATH AND `rustup target list
//!   --installed` contains `x86_64-apple-darwin` (Homebrew cargo lacks the
//!   cross-target std)
//!
//! Every evaluator is `fn(&CheckCtx) -> CheckOutcome`: a **read-only probe**.
//! Message and remedy strings must match `scripts/demo/doctor.sh` verbatim.

use std::process::Command;

use super::Evaluator;
#[allow(unused_imports)]
use super::{CheckCtx, CheckOutcome, CheckStatus, SkipReason};
use crate::paths::which;

/// All five `tool.*` rows share the exact same remedy — the doctor.sh loop
/// prints it verbatim regardless of which binary in the list was missing.
const TOOLCHAIN_REMEDY: &str = "brew install cmake ninja git mingw-w64";

/// `command -v <bin> >/dev/null 2>&1` — pass message is the bare binary name
/// (`${_t#*:}`), fail message is `"<bin> missing"`, shared remedy.
fn tool_check(slug: &'static str, bin: &str) -> CheckOutcome {
    if which(bin).is_some() {
        CheckOutcome::pass(slug, bin)
    } else {
        CheckOutcome::fail(slug, format!("{bin} missing"), TOOLCHAIN_REMEDY)
    }
}

fn tool_cmake(_ctx: &CheckCtx) -> CheckOutcome {
    tool_check("tool.cmake", "cmake")
}

fn tool_ninja(_ctx: &CheckCtx) -> CheckOutcome {
    tool_check("tool.ninja", "ninja")
}

fn tool_git(_ctx: &CheckCtx) -> CheckOutcome {
    tool_check("tool.git", "git")
}

fn tool_curl(_ctx: &CheckCtx) -> CheckOutcome {
    tool_check("tool.curl", "curl")
}

fn tool_mingw(_ctx: &CheckCtx) -> CheckOutcome {
    tool_check("tool.mingw", "x86_64-w64-mingw32-gcc")
}

/// `rustup target list --installed 2>/dev/null | grep -q x86_64-apple-darwin`.
///
/// A pipeline's exit status is the last stage's (`grep -q`), so whether
/// `rustup` itself succeeded is irrelevant beyond what it printed to stdout —
/// this checks the same substring against whatever stdout was captured,
/// without gating on `rustup`'s own exit code.
fn rustup_has_x86_64_darwin() -> bool {
    match Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
    {
        Ok(o) => String::from_utf8_lossy(&o.stdout).contains("x86_64-apple-darwin"),
        Err(_) => false,
    }
}

fn rust_x64_target(_ctx: &CheckCtx) -> CheckOutcome {
    if which("rustup").is_some() && rustup_has_x86_64_darwin() {
        CheckOutcome::pass("rust.x64-target", "rustup with x86_64-apple-darwin target")
    } else {
        CheckOutcome::fail(
            "rust.x64-target",
            "rustup x86_64-apple-darwin target missing",
            "install rustup via https://rustup.rs and source ~/.cargo/env (brew's rustup is \
             keg-only/not on PATH), then: rustup toolchain install stable && rustup target add \
             x86_64-apple-darwin",
        )
    }
}

/// Evaluators this module binds, keyed by contract slug.
pub fn defs() -> Vec<(&'static str, Evaluator)> {
    vec![
        ("tool.cmake", tool_cmake as Evaluator),
        ("tool.ninja", tool_ninja as Evaluator),
        ("tool.git", tool_git as Evaluator),
        ("tool.curl", tool_curl as Evaluator),
        ("tool.mingw", tool_mingw as Evaluator),
        ("rust.x64-target", rust_x64_target as Evaluator),
    ]
}

#[cfg(test)]
mod tests {
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
        // `check_fn` is a `checks::Evaluator` function pointer (`fn(&CheckCtx)
        // -> CheckOutcome`), not JavaScript/Python `eval()` — no string is
        // ever interpreted as code here.
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

    #[test]
    fn defs_binds_all_six_slugs_in_contract_order() {
        let slugs: Vec<&str> = defs().into_iter().map(|(s, _)| s).collect();
        assert_eq!(
            slugs,
            vec![
                "tool.cmake",
                "tool.ninja",
                "tool.git",
                "tool.curl",
                "tool.mingw",
                "rust.x64-target",
            ]
        );
    }
}
