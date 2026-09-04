use super::*;
use crate::checks::CheckOptions;
use crate::paths::Paths;

fn ctx() -> CheckCtx {
    CheckCtx::new(
        Paths::new("/nonexistent/sabrage-headset-probe"),
        CheckOptions::new(),
    )
}

#[test]
fn first_connected_serial_takes_the_first_device_row_or_none() {
    let cases: &[(&str, &str, Option<&str>)] = &[
        (
            "skips the header and an offline row",
            "List of devices attached\nemulator-5554\toffline\n1A2B3C4D\tdevice\n",
            Some("1A2B3C4D"),
        ),
        ("header only", "List of devices attached\n", None),
        (
            "unauthorized only",
            "List of devices attached\n1A2B3C4D\tunauthorized\n",
            None,
        ),
        ("empty output", "", None),
        (
            "first qualifying row wins",
            "List of devices attached\nAAA\tdevice\nBBB\tdevice\n",
            Some("AAA"),
        ),
    ];
    for (label, input, expected) in cases {
        let got = first_connected_serial(input);
        assert_eq!(got.as_deref(), *expected, "{label}");
    }
}

#[test]
fn probes_disabled_skips_both_slugs() {
    let opts = CheckOptions {
        allow_adb_probes: false,
        ..CheckOptions::new()
    };
    let ctx = CheckCtx::new(Paths::new("/nonexistent/sabrage-headset-probe"), opts);
    assert_eq!(hs_adb(&ctx).status, CheckStatus::Skipped);
    assert_eq!(hs_client(&ctx).status, CheckStatus::Skipped);
}

#[test]
fn no_adb_binary_warns_hs_adb_and_skips_hs_client() {
    // Paths::new probes the real machine, so this asserts the no-adb shape only
    // when the field genuinely came back None — the invariant, not a fixed
    // machine state.
    let c = ctx();
    if c.paths.adb.is_some() {
        return;
    }
    let adb = hs_adb(&c);
    assert_eq!(adb.status, CheckStatus::Warn);
    assert_eq!(
        adb.message,
        "no Quest over adb (fine for WiFi streaming; connect USB once to install the client)"
    );
    assert_eq!(hs_client(&c).status, CheckStatus::Skipped);
}
