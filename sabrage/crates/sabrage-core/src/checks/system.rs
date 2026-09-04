//! Group `system, crossover` — hardware, OS version, and the CrossOver install.
//!
//! Slugs owned here, in contract order: `sys.arch`, `sys.macos27`,
//! `cx.present` (silent when found), `cx.version`.
//!
//! `sys.macos27` is a hard FAIL below macOS 27 even on the native-arm64
//! encoder path, so the in-process fallback stays viable.
//!
//! Every evaluator is `fn(&CheckCtx) -> CheckOutcome`: a read-only probe.
//! Message and remedy strings must match scripts/demo/doctor.sh verbatim;
//! `cx.present` is silent in doctor.sh, so its message is Sabrage-only.

use std::path::Path;
use std::process::{Command, Output};

use super::Evaluator;
#[allow(unused_imports)]
use super::{CheckCtx, CheckOutcome, CheckStatus, SkipReason};

fn run(cmd: &str, args: &[&str]) -> Option<Output> {
    Command::new(cmd).args(args).output().ok()
}

fn stdout_trimmed(o: &Output) -> String {
    crate::util::strip_trailing_newlines(&String::from_utf8_lossy(&o.stdout)).to_string()
}

fn uname_m() -> String {
    run("uname", &["-m"])
        .as_ref()
        .map(stdout_trimmed)
        .unwrap_or_default()
}

/// The CPU brand string, empty when the probe fails: doctor.sh discards stderr
/// and has no `|| echo` fallback here.
fn cpu_brand_string() -> String {
    match run("sysctl", &["-n", "machdep.cpu.brand_string"]) {
        Some(o) if o.status.success() => stdout_trimmed(&o),
        _ => String::new(),
    }
}

/// The macOS product version, or `"0"` when the probe fails, matching
/// doctor.sh's `|| echo 0` fallback.
fn macos_product_version() -> String {
    match run("sw_vers", &["-productVersion"]) {
        Some(o) if o.status.success() => stdout_trimmed(&o),
        _ => "0".to_string(),
    }
}

/// `CFBundleShortVersionString` from `plist`, or `"0"` when the read fails,
/// matching doctor.sh's `|| echo 0` fallback.
fn cf_bundle_short_version(plist: &Path) -> String {
    match run(
        "defaults",
        &[
            "read",
            &plist.to_string_lossy(),
            "CFBundleShortVersionString",
        ],
    ) {
        Some(o) if o.status.success() => stdout_trimmed(&o),
        _ => "0".to_string(),
    }
}

/// Each dot-separated component's leading digit run, parsed as `u64` (`0` for
/// a component with no leading digits — e.g. a non-numeric build suffix, or
/// the shell's `echo 0` fallback for a failed probe).
fn dotted_components(ver: &str) -> Vec<u64> {
    ver.split('.')
        .map(|part| {
            let digits: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
            digits.parse().unwrap_or(0)
        })
        .collect()
}

/// True iff `a`'s dotted version is `>=` `b`'s, treating a missing trailing
/// component on either side as `0`; a single-component `b` (e.g. `"27"`)
/// therefore pins only the major version, as doctor.sh's `${OSVER%%.*}`
/// truncation does for `sys.macos27`.
///
/// design-core divergence 10: this is a numeric compare, not doctor.sh's
/// string ordering, and the two must agree on every observable version —
/// tests::dotted_ge_table.
fn dotted_ge(a: &str, b: &str) -> bool {
    let ca = dotted_components(a);
    let cb = dotted_components(b);
    let len = ca.len().max(cb.len());
    for i in 0..len {
        let xa = ca.get(i).copied().unwrap_or(0);
        let xb = cb.get(i).copied().unwrap_or(0);
        match xa.cmp(&xb) {
            std::cmp::Ordering::Less => return false,
            std::cmp::Ordering::Greater => return true,
            std::cmp::Ordering::Equal => {}
        }
    }
    true
}

fn sys_arch(_ctx: &CheckCtx) -> CheckOutcome {
    let arch = uname_m();
    if arch == "arm64" {
        CheckOutcome::pass(
            "sys.arch",
            format!("Apple Silicon ({})", cpu_brand_string()),
        )
    } else {
        CheckOutcome::fail(
            "sys.arch",
            format!("not an Apple Silicon Mac ({arch})"),
            "this demo requires an arm64 Mac",
        )
    }
}

fn sys_macos27(_ctx: &CheckCtx) -> CheckOutcome {
    let osver = macos_product_version();
    if dotted_ge(&osver, "27") {
        CheckOutcome::pass(
            "sys.macos27",
            format!("macOS {osver} (>= 27: VT encodes BGRA directly under Rosetta)"),
        )
    } else {
        CheckOutcome::fail(
            "sys.macos27",
            format!(
                "macOS {osver} < 27 — in-process BGRA-direct encode produces green video (VT \
                 zero-chroma bug); the in-process fallback needs macOS 27+ even with the native \
                 helper (native helper path unaffected)"
            ),
            "upgrade to macOS 27+, or pin ext/oxrsys back to the NV12-era revision cf5f926",
        )
    }
}

fn cx_present(ctx: &CheckCtx) -> CheckOutcome {
    match &ctx.paths.cx_app {
        // doctor.sh is silent when clean here (`tap cx.present ok`, no console
        // line) — there is no verbatim string to match, so this message is
        // Sabrage-only and the CLI console suppresses the row.
        Some(app) => CheckOutcome::silent_pass(
            "cx.present",
            format!("CrossOver.app found at {}", app.display()),
        ),
        None => CheckOutcome::fail(
            "cx.present",
            "CrossOver.app not found",
            "install CrossOver into ~/Applications or /Applications",
        ),
    }
}

fn cx_version(ctx: &CheckCtx) -> CheckOutcome {
    let Some(cx_app) = &ctx.paths.cx_app else {
        return CheckOutcome::skipped("cx.version", "CrossOver.app not found".into());
    };
    let cxver = cf_bundle_short_version(&cx_app.join("Contents/Info.plist"));
    if dotted_ge(&cxver, "26.2") {
        CheckOutcome::pass(
            "cx.version",
            format!("CrossOver {cxver} at {}", cx_app.display()),
        )
    } else {
        CheckOutcome::fail(
            "cx.version",
            format!("CrossOver {cxver} < 26.2"),
            "upgrade CrossOver to 26.2+",
        )
    }
}

/// Evaluators this module binds, keyed by contract slug.
pub fn defs() -> Vec<(&'static str, Evaluator)> {
    vec![
        ("sys.arch", sys_arch as Evaluator),
        ("sys.macos27", sys_macos27 as Evaluator),
        ("cx.present", cx_present as Evaluator),
        ("cx.version", cx_version as Evaluator),
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
    fn dotted_ge_table() {
        assert!(dotted_ge("27", "27"));
        assert!(dotted_ge("28", "27"));
        assert!(!dotted_ge("26", "27"));

        // A single-component threshold only pins the major version — matches
        // doctor.sh's `${OSVER%%.*}` truncation for sys.macos27.
        assert!(dotted_ge("27.0", "27"));
        assert!(dotted_ge("27.99.1", "27"));
        assert!(!dotted_ge("26.99", "27"));

        // Multi-component compare is genuinely numeric, not lexicographic:
        // "26.10" must beat "26.2" even though "1" < "2" as a character.
        assert!(dotted_ge("26.10", "26.2"));
        assert!(dotted_ge("26.2", "26.2"));
        assert!(dotted_ge("26.3", "26.2"));
        assert!(!dotted_ge("26.1", "26.2"));

        // Trailing components on either side default to 0.
        assert!(dotted_ge("26.2.1", "26.2"));
        assert!(!dotted_ge("26.2.0", "26.2.1"));

        // The shell's `echo 0` failure fallback and a genuinely empty string.
        assert!(!dotted_ge("0", "27"));
        assert!(!dotted_ge("", "27"));
    }

    #[test]
    fn sys_arch_reports_this_machine_truthfully() {
        let out = sys_arch(&ctx());
        let real_arch = uname_m();
        if real_arch == "arm64" {
            assert_eq!(out.status, CheckStatus::Pass);
            assert!(out.message.starts_with("Apple Silicon ("));
            assert!(out.remedy.is_none());
        } else {
            assert_eq!(out.status, CheckStatus::Fail);
            assert_eq!(
                out.message,
                format!("not an Apple Silicon Mac ({real_arch})")
            );
            assert_eq!(
                out.remedy.as_deref(),
                Some("this demo requires an arm64 Mac")
            );
        }
    }

    #[test]
    fn sys_macos27_matches_dotted_ge_of_the_real_version() {
        let out = sys_macos27(&ctx());
        let osver = macos_product_version();
        if dotted_ge(&osver, "27") {
            assert_eq!(out.status, CheckStatus::Pass);
            assert_eq!(
                out.message,
                format!("macOS {osver} (>= 27: VT encodes BGRA directly under Rosetta)")
            );
        } else {
            assert_eq!(out.status, CheckStatus::Fail);
            assert_eq!(
                out.remedy.as_deref(),
                Some(
                    "upgrade to macOS 27+, or pin ext/oxrsys back to the NV12-era revision cf5f926"
                )
            );
        }
    }

    #[test]
    fn cx_present_and_version_agree_with_paths() {
        let c = ctx();
        let present = cx_present(&c);
        let version = cx_version(&c);
        match &c.paths.cx_app {
            Some(_) => {
                assert_eq!(present.status, CheckStatus::Pass);
                assert_ne!(version.status, CheckStatus::Skipped);
            }
            None => {
                assert_eq!(present.status, CheckStatus::Fail);
                assert_eq!(present.message, "CrossOver.app not found");
                assert_eq!(
                    present.remedy.as_deref(),
                    Some("install CrossOver into ~/Applications or /Applications")
                );
                assert_eq!(version.status, CheckStatus::Skipped);
            }
        }
    }
}
