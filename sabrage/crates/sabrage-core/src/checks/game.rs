//! Group `game` — doctor.sh section 8: the Beat Saber 1.29.4 install.
//!
//! Binds `game.present` and `game.version` in contract order; every evaluator
//! is `fn(&CheckCtx) -> CheckOutcome`, a read-only probe.
//!
//! Printed strings are reproduced verbatim (`docs/troubleshooting.md` quotes
//! them and nothing tests it). Tap-only strings are impl-owned prose, marked
//! at their site.

use std::path::PathBuf;

use super::Evaluator;
use super::{CheckCtx, CheckOutcome, SkipReason};
use crate::contract::contract;
use crate::util::bs_version;

/// True when doctor skips the whole Beat Saber section: neither a resolved
/// bottle nor an explicit `--bs-dir` override.
fn section_skipped(ctx: &CheckCtx) -> bool {
    ctx.bottle.is_none() && ctx.opts.bs_dir_override.is_none()
}

/// doctor's skip line, verbatim (doctor.sh section 8). doctor taps both
/// `game.*` slugs skipped with no per-slug text; sabrage carries this text as
/// the [`SkipReason`] on both (tests::no_bottle_no_override_skips_both_slugs_with_the_verbatim_reason).
const SECTION_SKIP_REASON: &str = "Beat Saber check skipped (needs --bottle or --bs-dir)";

fn exe_path(ctx: &CheckCtx) -> PathBuf {
    ctx.bs_dir.join("Beat Saber.exe")
}

fn game_present(ctx: &CheckCtx) -> CheckOutcome {
    if section_skipped(ctx) {
        return CheckOutcome::skipped("game.present", SkipReason::new(SECTION_SKIP_REASON));
    }
    if exe_path(ctx).is_file() {
        // doctor.sh taps `game.present ok` and prints no row for the found
        // case; the tap channel carries slug+status only, so this message is
        // impl-owned prose, not a verbatim doctor.sh string.
        CheckOutcome::silent_pass(
            "game.present",
            format!("Beat Saber.exe found at {}", ctx.bs_dir.display()),
        )
    } else {
        CheckOutcome::fail(
            "game.present",
            format!("Beat Saber 1.29.4 not found at {}", ctx.bs_dir.display()),
            format!(
                "{}  (or set WINEVR_BS_DIR)",
                contract().depot_command(&ctx.bs_dir)
            ),
        )
    }
}

fn game_version(ctx: &CheckCtx) -> CheckOutcome {
    if section_skipped(ctx) {
        return CheckOutcome::skipped("game.version", SkipReason::new(SECTION_SKIP_REASON));
    }
    if !exe_path(ctx).is_file() {
        // doctor.sh: `tap game.version skipped` in the game.present-FAIL arm,
        // with no explanatory text of its own; sabrage supplies one.
        return CheckOutcome::skipped(
            "game.version",
            SkipReason::new(format!(
                "Beat Saber 1.29.4 not found at {}",
                ctx.bs_dir.display()
            )),
        );
    }
    let ver = bs_version(&ctx.bs_dir);
    if ver.starts_with("1.29.4") {
        CheckOutcome::pass(
            "game.version",
            format!("Beat Saber {ver} at {}", ctx.bs_dir.display()),
        )
    } else {
        CheckOutcome::warn(
            "game.version",
            format!(
                "Beat Saber version '{ver}' is not 1.29.4 — the Meta account gate may block it"
            ),
        )
    }
}

/// Evaluators this module binds, keyed by contract slug.
pub fn defs() -> Vec<(&'static str, Evaluator)> {
    vec![
        ("game.present", game_present as Evaluator),
        ("game.version", game_version as Evaluator),
    ]
}

#[cfg(test)]
mod tests;
