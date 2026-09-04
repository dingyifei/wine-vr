//! Group `headset` — scripts/demo/doctor.sh section 14: `hs.adb` and
//! `hs.client`, both warning-only (WiFi streaming needs no USB). Every
//! evaluator is a read-only probe over `adb`.
//!
//! `CheckOptions::allow_adb_probes = false` is a Sabrage-only state with no zsh
//! counterpart: both slugs report `Skipped` instead of probing, which is outside
//! the doctor parity contract (tests::probes_disabled_skips_both_slugs).

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

/// Serial of the first `adb devices` row after the header whose state field is
/// exactly `device` (not `offline`, `unauthorized`, …); `None` when none
/// qualifies.
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

/// `hs.adb`: Pass with the Quest's serial, Warn when adb is missing or no
/// device is connected (tests::no_adb_binary_warns_hs_adb_and_skips_hs_client),
/// Skipped when probing is disabled. Reference: scripts/demo/doctor.sh, section 14.
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

/// True when the package list on `serial` mentions `alvr` anywhere in stdout
/// (case-sensitive substring, matching the shell's bare `grep`); false when the
/// command fails to run.
fn client_installed(adb: &Path, serial: &str) -> bool {
    Command::new(adb)
        .args(["-s", serial, "shell", "pm", "list", "packages"])
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).contains("alvr"))
        .unwrap_or(false)
}

/// `hs.client`: Pass when the ALVR client is installed on the connected Quest,
/// Warn when it is not, Skipped when no Quest is connected or probing is
/// disabled.
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
mod tests;
