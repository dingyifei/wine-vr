//! Group `toolchain` — the `tool.*` and `rust.x64-target` doctor rows.
//!
//! Slug list and order live in `contract/pipeline.toml`; shell probes are in
//! `scripts/demo/doctor.sh` sections `4.` and `5.`.
//!
//! `rust.x64-target` requires a rustup toolchain because Homebrew's cargo
//! ships no std for `x86_64-apple-darwin`.
//!
//! Every evaluator is `fn(&CheckCtx) -> CheckOutcome`: a read-only probe.
//! Message and remedy strings must match `scripts/demo/doctor.sh` verbatim.

use std::process::Command;

use super::Evaluator;
#[allow(unused_imports)]
use super::{CheckCtx, CheckOutcome, CheckStatus, SkipReason};
use crate::paths::which;

/// All five `tool.*` rows share the exact same remedy — the doctor.sh loop
/// prints it verbatim regardless of which binary in the list was missing.
const TOOLCHAIN_REMEDY: &str = "brew install cmake ninja git mingw-w64";

/// Outcome for `slug` from whether `bin` resolves on `PATH`, carrying the
/// shared toolchain remedy when it does not.
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

/// Whether `rustup target list --installed` printed `x86_64-apple-darwin`.
///
/// `rustup`'s own exit status is ignored: the doctor.sh pipeline's status
/// is `grep -q`'s, so only stdout decides.
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
}
