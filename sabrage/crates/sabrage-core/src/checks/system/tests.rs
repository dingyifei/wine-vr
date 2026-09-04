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
            Some("upgrade to macOS 27+, or pin ext/oxrsys back to the NV12-era revision cf5f926")
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
