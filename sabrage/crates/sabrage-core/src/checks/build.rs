//! Group `build` — doctor.sh section 9, 9b: build outputs, including the native-arm64 encoder helper.
//!
//! Slugs owned here, in contract order: `build.oxr-dylib`, `build.alvr-core`,
//! `build.runtime-json`, `build.woxr-dll`, `build.woxr-so`, `build.dashboard`,
//! `build.helper-staged`, `build.helper-arm64`.
//!
//! Every evaluator is `fn(&CheckCtx) -> CheckOutcome`: a read-only probe whose
//! message and remedy strings must match `scripts/demo/doctor.sh` verbatim.
//!
//! `build.helper-arm64` must not accept `arm64e` alone: a wrong-arch binary
//! staged next to the runtime dylib shadows the good one and silently drops the
//! session to in-process H.264 (tests::helper_is_arm64_rejects_arm64e_only_binaries).

use std::path::Path;

use super::Evaluator;
use super::{CheckCtx, CheckOutcome, SkipReason};

/// Shared shape of the six `build.*` output-presence checks: passes with
/// `built: <relpath>`, fails with `missing build output: <relpath>` and the
/// `./demo.sh build` remedy, where `<relpath>` is `Paths::rel_display`.
fn built_output(ctx: &CheckCtx, slug: &'static str, path: &Path) -> CheckOutcome {
    let rel = ctx.paths.rel_display(path);
    if path.is_file() {
        CheckOutcome::pass(slug, format!("built: {rel}"))
    } else {
        CheckOutcome::fail(
            slug,
            format!("missing build output: {rel}"),
            "./demo.sh build",
        )
    }
}

fn oxr_dylib(ctx: &CheckCtx) -> CheckOutcome {
    built_output(ctx, "build.oxr-dylib", &ctx.paths.oxr_dylib)
}

fn alvr_core(ctx: &CheckCtx) -> CheckOutcome {
    built_output(ctx, "build.alvr-core", &ctx.paths.oxr_alvr_dylib)
}

fn runtime_json(ctx: &CheckCtx) -> CheckOutcome {
    built_output(ctx, "build.runtime-json", &ctx.paths.oxr_runtime_json)
}

fn woxr_dll(ctx: &CheckCtx) -> CheckOutcome {
    built_output(ctx, "build.woxr-dll", &ctx.paths.woxr_dll)
}

fn woxr_so(ctx: &CheckCtx) -> CheckOutcome {
    built_output(ctx, "build.woxr-so", &ctx.paths.woxr_so)
}

fn dashboard(ctx: &CheckCtx) -> CheckOutcome {
    built_output(ctx, "build.dashboard", &ctx.paths.alvr_dashboard)
}

/// True when `p` is a regular file with any execute bit set — `[ -x "$1" ]`
/// to the same approximation the `paths` module's `which()` uses (no
/// euid/egid resolution, which `lib.sh` never relied on either).
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// `lipo -archs <path>` stdout with trailing newlines stripped as `$(...)` does;
/// empty when `lipo` cannot run or writes nothing. Exit status is ignored; the
/// FAIL message of `build.helper-arm64` embeds this value.
pub fn lipo_archs_stdout(path: &Path) -> String {
    match std::process::Command::new("lipo")
        .arg("-archs")
        .arg(path)
        .output()
    {
        Ok(out) => {
            crate::util::strip_trailing_newlines(&String::from_utf8_lossy(&out.stdout)).to_string()
        }
        Err(_) => String::new(),
    }
}

/// True when `path` is executable and `lipo -archs` lists `arm64` as a whole
/// word. Single home of lib.sh's `helper_is_arm64()`; `crate::util` re-exports
/// it for the fix and stage layers.
///
/// A fat `x86_64 arm64e` binary must NOT match, while `x86_64 arm64` and thin
/// `arm64` must (tests::helper_is_arm64_rejects_arm64e_only_binaries,
/// tests::helper_is_arm64_is_true_for_the_thin_arm64_test_binary_itself).
pub fn helper_is_arm64(path: &Path) -> bool {
    if !is_executable(path) {
        return false;
    }
    lipo_archs_stdout(path)
        .split_ascii_whitespace()
        .any(|arch| arch == "arm64")
}

fn helper_staged(ctx: &CheckCtx) -> CheckOutcome {
    let bin = &ctx.paths.oxr_helper_staged;
    let rel = ctx.paths.rel_display(bin);
    if bin.is_file() {
        CheckOutcome::pass(
            "build.helper-staged",
            format!("built: {rel} (staged next to the runtime dylib)"),
        )
    } else {
        CheckOutcome::fail(
            "build.helper-staged",
            format!("encoder helper not staged: {rel}"),
            "./demo.sh build",
        )
    }
}

fn helper_arm64(ctx: &CheckCtx) -> CheckOutcome {
    let bin = &ctx.paths.oxr_helper_staged;
    if !bin.is_file() {
        // doctor.sh: `tap build.helper-arm64 skipped` in the helper-staged-FAIL
        // arm, with no explanatory text of its own; sabrage supplies one.
        return CheckOutcome::skipped(
            "build.helper-arm64",
            SkipReason::new(format!(
                "encoder helper not staged: {}",
                ctx.paths.rel_display(bin)
            )),
        );
    }
    if helper_is_arm64(bin) {
        CheckOutcome::pass("build.helper-arm64", "encoder helper is arm64")
    } else {
        let archs = lipo_archs_stdout(bin);
        CheckOutcome::fail(
            "build.helper-arm64",
            format!(
                "encoder helper is not an arm64 executable ({archs}) — a stale/wrong-arch binary here shadows the staged one"
            ),
            "./demo.sh build (restages the arm64 helper)",
        )
    }
}

/// Evaluators this module binds, keyed by contract slug.
pub fn defs() -> Vec<(&'static str, Evaluator)> {
    vec![
        ("build.oxr-dylib", oxr_dylib as Evaluator),
        ("build.alvr-core", alvr_core as Evaluator),
        ("build.runtime-json", runtime_json as Evaluator),
        ("build.woxr-dll", woxr_dll as Evaluator),
        ("build.woxr-so", woxr_so as Evaluator),
        ("build.dashboard", dashboard as Evaluator),
        ("build.helper-staged", helper_staged as Evaluator),
        ("build.helper-arm64", helper_arm64 as Evaluator),
    ]
}

#[cfg(test)]
mod tests;
