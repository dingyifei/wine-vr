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
mod tests;
