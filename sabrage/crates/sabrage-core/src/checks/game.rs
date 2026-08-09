//! Group `game` — doctor.sh section 8: the Beat Saber 1.29.4 install.
//!
//! Slugs owned here, in contract order:
//!
//! * `game.present` — `<bs_dir>/Beat Saber.exe` exists; the remedy is the
//!   pinned `DepotDownloader` line (`Contract::depot_command`). Whole section
//!   skipped when there is neither a bottle nor `--bs-dir`
//! * `game.version` — `util::bs_version` starts with `1.29.4` — WARN
//!   otherwise (newer builds hit the Meta account gate)
//!
//! Every evaluator is `fn(&CheckCtx) -> CheckOutcome`: a **read-only probe**.
//! Message and remedy strings must match `scripts/demo/doctor.sh` verbatim.

use std::path::PathBuf;

use super::Evaluator;
use super::{CheckCtx, CheckOutcome, SkipReason};
use crate::contract::contract;
use crate::util::bs_version;

/// doctor.sh's `[ "$BOTTLE_OK" = 0 ] && [ -z "${WINEVR_BS_DIR:-}" ]` — the
/// whole section is skipped only when there is neither a resolved bottle nor
/// an explicit `--bs-dir` override.
fn section_skipped(ctx: &CheckCtx) -> bool {
    ctx.bottle.is_none() && ctx.opts.bs_dir_override.is_none()
}

/// The `info "Beat Saber check skipped (needs --bottle or --bs-dir)"` line,
/// verbatim. doctor prints it once (no slug) and then taps both `game.*`
/// slugs `skipped` with no text of their own; sabrage carries the same text
/// as the [`SkipReason`] on both outcomes instead of dropping it entirely
/// (design-core §10 divergence 11 — sabrage always says why a check was
/// skipped, even where doctor stays silent per-slug).
const SECTION_SKIP_REASON: &str = "Beat Saber check skipped (needs --bottle or --bs-dir)";

fn exe_path(ctx: &CheckCtx) -> PathBuf {
    ctx.bs_dir.join("Beat Saber.exe")
}

fn game_present(ctx: &CheckCtx) -> CheckOutcome {
    if section_skipped(ctx) {
        return CheckOutcome::skipped("game.present", SkipReason::new(SECTION_SKIP_REASON));
    }
    if exe_path(ctx).is_file() {
        // doctor.sh: `tap game.present ok` — no row is printed for the found
        // case; the visible row is game.version's instead. The tap channel
        // carries slug+status only (see `crate::tap`), so this message is
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
mod tests {
    use super::*;
    use crate::checks::{CheckOptions, CheckStatus};
    use crate::paths::Paths;
    use std::fs;
    use std::path::Path;

    fn scratch(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("sabrage-game-test-{}-{tag}", std::process::id()))
    }

    #[test]
    fn no_bottle_no_override_skips_both_slugs_with_the_verbatim_reason() {
        let ctx = CheckCtx::new(Paths::new(Path::new("/repo")), CheckOptions::new());
        assert!(section_skipped(&ctx));

        let p = game_present(&ctx);
        assert_eq!(p.status, CheckStatus::Skipped);
        assert_eq!(p.message, SECTION_SKIP_REASON);

        let v = game_version(&ctx);
        assert_eq!(v.status, CheckStatus::Skipped);
        assert_eq!(v.message, SECTION_SKIP_REASON);
    }

    #[test]
    fn bs_dir_override_without_a_bottle_still_runs_the_section() {
        let tmp = scratch("override-only");
        let opts = CheckOptions {
            bs_dir_override: Some(tmp.clone()),
            ..CheckOptions::new()
        };
        let ctx = CheckCtx::new(Paths::new(Path::new("/repo")), opts);
        assert!(!section_skipped(&ctx));

        let o = game_present(&ctx);
        assert_eq!(o.status, CheckStatus::Fail);
        assert_eq!(
            o.message,
            format!("Beat Saber 1.29.4 not found at {}", tmp.display())
        );
        assert!(o
            .remedy
            .as_deref()
            .unwrap()
            .ends_with("  (or set WINEVR_BS_DIR)"));

        let v = game_version(&ctx);
        assert_eq!(v.status, CheckStatus::Skipped);
    }

    #[test]
    fn exe_present_with_matching_marker_passes_both() {
        let tmp = scratch("versioned");
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join("Beat Saber.exe"), b"stub").unwrap();
        fs::write(tmp.join("BeatSaberVersion.txt"), "1.29.4_4575554838\n").unwrap();
        let opts = CheckOptions {
            bs_dir_override: Some(tmp.clone()),
            ..CheckOptions::new()
        };
        let ctx = CheckCtx::new(Paths::new(Path::new("/repo")), opts);

        let p = game_present(&ctx);
        assert_eq!(p.status, CheckStatus::Pass);

        let v = game_version(&ctx);
        assert_eq!(v.status, CheckStatus::Pass);
        assert_eq!(
            v.message,
            format!("Beat Saber 1.29.4_4575554838 at {}", tmp.display())
        );
        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn exe_present_with_other_version_warns() {
        let tmp = scratch("wrong-version");
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join("Beat Saber.exe"), b"stub").unwrap();
        fs::write(tmp.join("BeatSaberVersion.txt"), "1.34.2_9999999999\n").unwrap();
        let opts = CheckOptions {
            bs_dir_override: Some(tmp.clone()),
            ..CheckOptions::new()
        };
        let ctx = CheckCtx::new(Paths::new(Path::new("/repo")), opts);

        let v = game_version(&ctx);
        assert_eq!(v.status, CheckStatus::Warn);
        assert_eq!(
            v.message,
            "Beat Saber version '1.34.2_9999999999' is not 1.29.4 — the Meta account gate may block it"
        );
        fs::remove_dir_all(&tmp).ok();
    }
}
