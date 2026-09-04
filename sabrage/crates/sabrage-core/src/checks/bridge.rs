//! Group `bottle-bridge` (doctor.sh section 11): the per-bottle half of the
//! bridge install. Binds `bottle.woxr-dll`, `bottle.manifest` and
//! `bottle.registry` in contract order; every evaluator is a read-only probe.
//! With no bottle all three return `skipped` carrying doctor.sh's verbatim
//! info line as the [`SkipReason`]. Message and remedy prose must match
//! `scripts/demo/doctor.sh` verbatim.
//!
//! See tests::{ordered_substring_scan_matches_grep_semantics,
//! no_bottle_skips_all_three_with_the_verbatim_reason}.

use super::Evaluator;
use super::{CheckCtx, CheckOutcome, SkipReason};
use crate::util::cmp_files;

/// The `info "per-bottle bridge checks skipped (no bottle)"` line, verbatim.
const SECTION_SKIP_REASON: &str = "per-bottle bridge checks skipped (no bottle)";

/// Whether `text` carries `ActiveRuntime`, `openxr` and `wineopenxr64.json`
/// in that order on one line: the semantics of doctor.sh's
/// `grep -q 'ActiveRuntime.*openxr.*wineopenxr64.json' "$PREFIX/system.reg"`.
///
/// A left-to-right chained `find` decides that existence question exactly:
/// `.*` is greedy but existence-only, and `.` never spans a newline.
/// See tests::ordered_substring_scan_matches_grep_semantics.
fn registry_has_active_runtime(text: &str) -> bool {
    text.lines().any(|line| {
        let needles = ["ActiveRuntime", "openxr", "wineopenxr64.json"];
        let mut pos = 0usize;
        needles.iter().all(|needle| match line[pos..].find(needle) {
            Some(off) => {
                pos += off + needle.len();
                true
            }
            None => false,
        })
    })
}

fn woxr_dll(ctx: &CheckCtx) -> CheckOutcome {
    let Some(bottle) = &ctx.bottle else {
        return CheckOutcome::skipped("bottle.woxr-dll", SkipReason::new(SECTION_SKIP_REASON));
    };
    let dst = bottle.sys32.join("wineopenxr.dll");
    if cmp_files(&ctx.paths.woxr_dll, &dst) {
        CheckOutcome::pass("bottle.woxr-dll", "bottle system32/wineopenxr.dll current")
    } else {
        CheckOutcome::fail(
            "bottle.woxr-dll",
            "bottle wineopenxr.dll stale/missing",
            format!("./demo.sh install --bottle {}", ctx.bottle_label()),
        )
    }
}

fn manifest(ctx: &CheckCtx) -> CheckOutcome {
    let Some(bottle) = &ctx.bottle else {
        return CheckOutcome::skipped("bottle.manifest", SkipReason::new(SECTION_SKIP_REASON));
    };
    if bottle.openxr_manifest().is_file() {
        CheckOutcome::pass("bottle.manifest", r"bottle C:\openxr\wineopenxr64.json")
    } else {
        CheckOutcome::fail(
            "bottle.manifest",
            "bottle OpenXR manifest missing",
            format!("./demo.sh install --bottle {}", ctx.bottle_label()),
        )
    }
}

fn registry(ctx: &CheckCtx) -> CheckOutcome {
    let Some(bottle) = &ctx.bottle else {
        return CheckOutcome::skipped("bottle.registry", SkipReason::new(SECTION_SKIP_REASON));
    };
    let found = std::fs::read(bottle.system_reg())
        .map(|bytes| registry_has_active_runtime(&String::from_utf8_lossy(&bytes)))
        .unwrap_or(false);
    if found {
        CheckOutcome::pass("bottle.registry", "bottle registry ActiveRuntime set")
    } else {
        CheckOutcome::fail(
            "bottle.registry",
            "bottle ActiveRuntime registry key missing",
            format!("./demo.sh install --bottle {}", ctx.bottle_label()),
        )
    }
}

/// Evaluators this module binds, keyed by contract slug.
pub fn defs() -> Vec<(&'static str, Evaluator)> {
    vec![
        ("bottle.woxr-dll", woxr_dll as Evaluator),
        ("bottle.manifest", manifest as Evaluator),
        ("bottle.registry", registry as Evaluator),
    ]
}

#[cfg(test)]
mod tests;
