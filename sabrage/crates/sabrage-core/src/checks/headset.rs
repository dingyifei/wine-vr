//! Group `headset` — doctor.sh section 14: headset-side state over adb (warnings only — WiFi streaming needs no USB).
//!
//! Slugs owned here, in contract order:
//!
//! * `hs.adb` — at least one `adb devices` row whose state is exactly
//!   `device`. Volatile
//! * `hs.client` — `pm list packages` on that serial mentions `alvr`.
//!   Volatile
//!
//! Every evaluator is `fn(&CheckCtx) -> CheckOutcome`: a **read-only probe**.
//! Message and remedy strings must match `scripts/demo/doctor.sh` verbatim.
//!
//! ## `allow_adb_probes`
//!
//! doctor.sh always shells out to `adb` when `$ADB` resolved to a binary —
//! there is no opt-out, and "no adb" and "adb present but no device" render
//! the *same* WARN. `CheckOptions::allow_adb_probes` is a Sabrage-only
//! extension (the GUI may not want to wake the adb daemon on every doctor
//! run); when it is `false` both slugs report [`CheckStatus::Skipped`]
//! instead of probing, which has no zsh counterpart and is not part of the
//! doctor parity contract — with the default (`true`), behavior is
//! byte-for-byte the shell's.

use std::path::Path;
use std::process::Command;

use super::Evaluator;
#[allow(unused_imports)]
use super::{CheckCtx, CheckOutcome, CheckStatus, SkipReason};

const PROBES_DISABLED: &str = "adb probing disabled (Sabrage setting)";

/// `"$ADB" devices` stdout, or empty when the binary is missing or fails to run.
fn adb_devices_output(adb: &Path) -> String {
    match Command::new(adb).arg("devices").output() {
        Ok(out) => String::from_utf8_lossy(&out.stdout).into_owned(),
        Err(_) => String::new(),
    }
}

/// `awk 'NR>1 && $2=="device"{print $1; exit}'` over `adb devices` output:
/// the serial of the first row (skipping the `List of devices attached`
/// header) whose state field is exactly `device` (not `offline`,
/// `unauthorized`, `no permissions`, …).
fn first_connected_serial(devices_output: &str) -> Option<String> {
    for line in devices_output.lines().skip(1) {
        let mut fields = line.split_whitespace();
        let Some(serial) = fields.next() else {
            continue;
        };
        if fields.next() == Some("device") {
            return Some(serial.to_string());
        }
    }
    None
}

fn connected_serial(ctx: &CheckCtx) -> Option<String> {
    let adb = ctx.paths.adb.as_deref()?;
    first_connected_serial(&adb_devices_output(adb))
}

/// doctor.sh section 14, the `hs.adb` half:
/// ```sh
/// if [ -n "$ADB" ] && "$ADB" devices … | awk … | grep -q .; then
///   SER="$(…)"; chk ok hs.adb "Quest connected via adb ($SER)"
/// else chk warn hs.adb "no Quest over adb (fine for WiFi streaming; connect USB once to install the client)"; fi
/// ```
fn hs_adb(ctx: &CheckCtx) -> CheckOutcome {
    if !ctx.opts.allow_adb_probes {
        return CheckOutcome::skipped("hs.adb", SkipReason::new(PROBES_DISABLED));
    }
    match connected_serial(ctx) {
        Some(ser) => CheckOutcome::pass("hs.adb", format!("Quest connected via adb ({ser})")),
        None => CheckOutcome::warn(
            "hs.adb",
            "no Quest over adb (fine for WiFi streaming; connect USB once to install the client)",
        ),
    }
}

/// `"$ADB" -s "$SER" shell pm list packages 2>/dev/null | grep -q alvr` — a
/// substring match against the whole package-list stdout (case-sensitive,
/// same as bare `grep`).
fn client_installed(adb: &Path, serial: &str) -> bool {
    Command::new(adb)
        .args(["-s", serial, "shell", "pm", "list", "packages"])
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).contains("alvr"))
        .unwrap_or(false)
}

/// doctor.sh section 14, the `hs.client` half — only reached inside the
/// `hs.adb` OK branch; otherwise `tap hs.client skipped`.
fn hs_client(ctx: &CheckCtx) -> CheckOutcome {
    if !ctx.opts.allow_adb_probes {
        return CheckOutcome::skipped("hs.client", SkipReason::new(PROBES_DISABLED));
    }
    let (Some(adb), Some(serial)) = (ctx.paths.adb.as_deref(), connected_serial(ctx)) else {
        return CheckOutcome::skipped("hs.client", SkipReason::new("no Quest connected over adb"));
    };
    if client_installed(adb, &serial) {
        CheckOutcome::pass("hs.client", "ALVR client installed on the Quest")
    } else {
        CheckOutcome::warn(
            "hs.client",
            "ALVR client not detected on the Quest — install ALVR v20.14.1 client APK",
        )
    }
}

/// Evaluators this module binds, keyed by contract slug.
pub fn defs() -> Vec<(&'static str, Evaluator)> {
    vec![
        ("hs.adb", hs_adb as Evaluator),
        ("hs.client", hs_client as Evaluator),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::CheckOptions;
    use crate::paths::Paths;

    fn ctx() -> CheckCtx {
        CheckCtx::new(
            Paths::new("/nonexistent/sabrage-headset-probe"),
            CheckOptions::new(),
        )
    }

    // ── pure parsing ─────────────────────────────────────────────────────────

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

    // ── evaluator shape (machine-independent) ──────────────────────────────

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
        // Paths::new probed the real machine for adb; this only asserts the
        // *no-adb* shape when the field genuinely came back None, mirroring
        // paths.rs's own "assert the invariant, not a fixed machine state".
        let c = ctx();
        if c.paths.adb.is_some() {
            return; // this machine has adb on PATH; nothing to assert here
        }
        let adb = hs_adb(&c);
        assert_eq!(adb.status, CheckStatus::Warn);
        assert_eq!(
            adb.message,
            "no Quest over adb (fine for WiFi streaming; connect USB once to install the client)"
        );
        assert_eq!(hs_client(&c).status, CheckStatus::Skipped);
    }
}
