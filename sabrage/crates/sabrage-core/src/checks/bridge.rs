//! Group `bottle-bridge` — doctor.sh section 11: the per-bottle half of the bridge install.
//!
//! Slugs owned here, in contract order:
//!
//! * `bottle.woxr-dll` — built `wineopenxr.dll` == `<sys32>/wineopenxr.dll`
//! * `bottle.manifest` — `<prefix>/drive_c/openxr/wineopenxr64.json` exists
//! * `bottle.registry` — `<prefix>/system.reg` carries `ActiveRuntime`,
//!   `openxr`, and `wineopenxr64.json` in that order on one line — the exact
//!   shape of `grep -q 'ActiveRuntime.*openxr.*wineopenxr64.json'`, reproduced
//!   as an ordered substring scan (no regex metacharacters beyond `.*`, which
//!   `str::find` chained left-to-right already captures)
//!
//! Whole section: doctor.sh only reaches these three checks when `BOTTLE_OK`
//! is 1 (i.e. `CheckCtx::bottle` is `Some`); otherwise it prints one `info`
//! line and taps all three slugs `skipped` with no text of their own.
//! Following `checks::game`'s precedent (design-core §10 divergence 11),
//! sabrage carries that verbatim `info` text as the [`SkipReason`] on every
//! skipped slug instead of dropping it.
//!
//! Every evaluator is `fn(&CheckCtx) -> CheckOutcome`: a **read-only probe**.
//! Message and remedy strings must match `scripts/demo/doctor.sh` verbatim.

use super::Evaluator;
use super::{CheckCtx, CheckOutcome, SkipReason};
use crate::util::cmp_files;

/// The `info "per-bottle bridge checks skipped (no bottle)"` line, verbatim.
const SECTION_SKIP_REASON: &str = "per-bottle bridge checks skipped (no bottle)";

/// `grep -q 'ActiveRuntime.*openxr.*wineopenxr64.json' "$PREFIX/system.reg"`.
///
/// grep (no `-z`) matches per line, and `.` never spans a newline, so a match
/// requires all three literals on **one** line, in order. `.*` is greedy but
/// existence-only: the leftmost occurrence of each literal, searched after the
/// end of the previous one, is always the least constraining choice for the
/// remaining literals, so a simple left-to-right chained `find` decides
/// existence exactly like the regex engine would.
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
mod tests {
    use super::*;
    use crate::checks::{CheckOptions, CheckStatus};
    use crate::paths::{Bottle, Paths};
    use std::fs;
    use std::path::{Path, PathBuf};

    fn scratch(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("sabrage-bridge-test-{}-{tag}", std::process::id()))
    }

    #[test]
    fn ordered_substring_scan_matches_grep_semantics() {
        // A realistic wine system.reg line: "ActiveRuntime", then "openxr"
        // (from the `C:\openxr\` segment), then "wineopenxr64.json" — all on
        // one line, in order.
        assert!(registry_has_active_runtime(
            r#""ActiveRuntime"="C:\openxr\wineopenxr64.json""#
        ));
        // Out of order: openxr/wineopenxr64.json before ActiveRuntime -> no match.
        assert!(!registry_has_active_runtime(
            "openxr wineopenxr64.json ActiveRuntime"
        ));
        // Split across two lines: '.' never spans a newline in grep.
        assert!(!registry_has_active_runtime(
            "ActiveRuntime openxr\nwineopenxr64.json"
        ));
        // All three present, in order, on one line.
        assert!(registry_has_active_runtime(
            "junk ActiveRuntime=openxr/wineopenxr64.json trailing"
        ));
        // Missing the last literal entirely.
        assert!(!registry_has_active_runtime(
            "ActiveRuntime openxr wineopenxr.json"
        ));
        assert!(!registry_has_active_runtime(""));
    }

    /// A bottle rooted under `tmp` with the pieces the three checks read,
    /// without touching the real `~/Library/Application Support/CrossOver`.
    fn ctx_with_bottle(tmp: &Path) -> CheckCtx {
        let bottles_root = tmp.join("Bottles");
        fs::create_dir_all(bottles_root.join("Steam")).unwrap();
        fs::write(
            bottles_root.join("Steam/cxbottle.conf"),
            b"\"Template\" = \"win11_64\"\n",
        )
        .unwrap();
        // Bottle::exists()/resolve() walk the real bottles_root() (~/Library/...),
        // so build the Bottle by hand rather than through CheckCtx::new's
        // opts.bottle_name resolution path.
        let bottle = Bottle {
            name: "Steam".to_string(),
            prefix: bottles_root.join("Steam"),
            sys32: bottles_root.join("Steam/drive_c/windows/system32"),
        };
        fs::create_dir_all(&bottle.sys32).unwrap();
        let opts = CheckOptions {
            bottle_name: Some("Steam".to_string()),
            ..CheckOptions::new()
        };
        let mut ctx = CheckCtx::new(Paths::new(tmp), opts);
        ctx.bottle = Some(bottle);
        ctx
    }

    #[test]
    fn no_bottle_skips_all_three_with_the_verbatim_reason() {
        let ctx = CheckCtx::new(Paths::new(Path::new("/repo")), CheckOptions::new());
        for eval in [
            woxr_dll as Evaluator,
            manifest as Evaluator,
            registry as Evaluator,
        ] {
            let o = eval(&ctx);
            assert_eq!(o.status, CheckStatus::Skipped, "{}", o.slug);
            assert_eq!(o.message, SECTION_SKIP_REASON);
        }
    }

    #[test]
    fn matching_woxr_dll_passes() {
        let tmp = scratch("dll-match");
        let ctx = ctx_with_bottle(&tmp);
        fs::create_dir_all(ctx.paths.woxr_dll.parent().unwrap()).unwrap();
        fs::write(&ctx.paths.woxr_dll, b"same-bytes").unwrap();
        fs::write(
            ctx.bottle.as_ref().unwrap().sys32.join("wineopenxr.dll"),
            b"same-bytes",
        )
        .unwrap();

        let o = woxr_dll(&ctx);
        assert_eq!(o.status, CheckStatus::Pass);
        assert_eq!(o.message, "bottle system32/wineopenxr.dll current");
        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn stale_woxr_dll_fails_with_install_remedy() {
        let tmp = scratch("dll-stale");
        let ctx = ctx_with_bottle(&tmp);
        fs::create_dir_all(ctx.paths.woxr_dll.parent().unwrap()).unwrap();
        fs::write(&ctx.paths.woxr_dll, b"built-bytes").unwrap();
        // No file in sys32 at all -> cmp -s fails.
        let o = woxr_dll(&ctx);
        assert_eq!(o.status, CheckStatus::Fail);
        assert_eq!(o.message, "bottle wineopenxr.dll stale/missing");
        assert_eq!(
            o.remedy.as_deref(),
            Some("./demo.sh install --bottle Steam")
        );
        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn manifest_presence_and_absence() {
        let tmp = scratch("manifest");
        let ctx = ctx_with_bottle(&tmp);
        let missing = manifest(&ctx);
        assert_eq!(missing.status, CheckStatus::Fail);
        assert_eq!(missing.message, "bottle OpenXR manifest missing");
        assert_eq!(
            missing.remedy.as_deref(),
            Some("./demo.sh install --bottle Steam")
        );

        let manifest_path = ctx.bottle.as_ref().unwrap().openxr_manifest();
        fs::create_dir_all(manifest_path.parent().unwrap()).unwrap();
        fs::write(&manifest_path, b"{}").unwrap();
        let present = manifest(&ctx);
        assert_eq!(present.status, CheckStatus::Pass);
        assert_eq!(present.message, "bottle C:\\openxr\\wineopenxr64.json");
        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn registry_key_presence_and_absence() {
        let tmp = scratch("registry");
        let ctx = ctx_with_bottle(&tmp);
        let missing = registry(&ctx);
        assert_eq!(missing.status, CheckStatus::Fail);
        assert_eq!(missing.message, "bottle ActiveRuntime registry key missing");
        assert_eq!(
            missing.remedy.as_deref(),
            Some("./demo.sh install --bottle Steam")
        );

        let reg_path = ctx.bottle.as_ref().unwrap().system_reg();
        fs::write(
            &reg_path,
            "[Software\\\\Khronos\\\\OpenXR\\\\1] 1700000000\n\"ActiveRuntime\"=\"C:\\\\openxr\\\\wineopenxr64.json\"\n",
        )
        .unwrap();
        let present = registry(&ctx);
        assert_eq!(present.status, CheckStatus::Pass);
        assert_eq!(present.message, "bottle registry ActiveRuntime set");
        fs::remove_dir_all(&tmp).ok();
    }
}
