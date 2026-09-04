//! Group `overlay` — the global bridge overlay inside CrossOver.app: the DXMT
//! artifacts and the built `wineopenxr` binaries must match the copies under
//! `$CX/lib` (a CrossOver update silently reverts them).
//!
//! Owns `overlay.dxmt-d3d11`, `overlay.dxmt-winemetal`, `overlay.woxr-dll`,
//! and `overlay.woxr-so` in contract order. Mirrors doctor.sh section 10.

use std::path::Path;

use super::Evaluator;
use super::{CheckCtx, CheckOutcome, SkipReason};
use crate::util::cmp_files;

fn basename(p: &Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Compares one overlay source against its destination under `$CX/lib`.
///
/// Passes when byte-identical; fails with the `./demo.sh install` remedy
/// when they differ or either file is missing; skipped only when `dst` is
/// `None` (no CrossOver.app).
fn overlay_check(
    ctx: &CheckCtx,
    slug: &'static str,
    src: &Path,
    dst: Option<&Path>,
) -> CheckOutcome {
    // Skip reason is ours — doctor.sh section 10 taps bare `… skipped` with no `info` line.
    let Some(dst) = dst else {
        return CheckOutcome::skipped(slug, SkipReason::new("CrossOver.app not found"));
    };
    let base = basename(dst);
    if cmp_files(src, dst) {
        CheckOutcome::pass(slug, format!("global overlay current: {base}"))
    } else {
        CheckOutcome::fail(
            slug,
            format!("global overlay stale/missing: {base}"),
            format!("./demo.sh install --bottle {}", ctx.bottle_label()),
        )
    }
}

fn dxmt_d3d11(ctx: &CheckCtx) -> CheckOutcome {
    let src = ctx.paths.dxmt_art.join("x86_64-windows/d3d11.dll");
    let dst = ctx.paths.cx_dxmt("x86_64-windows/d3d11.dll");
    overlay_check(ctx, "overlay.dxmt-d3d11", &src, dst.as_deref())
}

fn dxmt_winemetal(ctx: &CheckCtx) -> CheckOutcome {
    let src = ctx.paths.dxmt_art.join("x86_64-unix/winemetal.so");
    let dst = ctx.paths.cx_dxmt("x86_64-unix/winemetal.so");
    overlay_check(ctx, "overlay.dxmt-winemetal", &src, dst.as_deref())
}

fn woxr_dll(ctx: &CheckCtx) -> CheckOutcome {
    let dst = ctx.paths.cx_wine_lib("x86_64-windows/wineopenxr.dll");
    overlay_check(ctx, "overlay.woxr-dll", &ctx.paths.woxr_dll, dst.as_deref())
}

fn woxr_so(ctx: &CheckCtx) -> CheckOutcome {
    let dst = ctx.paths.cx_wine_lib("x86_64-unix/wineopenxr.so");
    overlay_check(ctx, "overlay.woxr-so", &ctx.paths.woxr_so, dst.as_deref())
}

/// Evaluators this module binds, keyed by contract slug.
pub fn defs() -> Vec<(&'static str, Evaluator)> {
    vec![
        ("overlay.dxmt-d3d11", dxmt_d3d11 as Evaluator),
        ("overlay.dxmt-winemetal", dxmt_winemetal as Evaluator),
        ("overlay.woxr-dll", woxr_dll as Evaluator),
        ("overlay.woxr-so", woxr_so as Evaluator),
    ]
}

#[cfg(test)]
mod tests;
