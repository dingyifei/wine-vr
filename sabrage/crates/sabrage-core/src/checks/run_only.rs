//! Group `run-only` — preflights that exist only in the launch path, so
//! doctor prints no row for them ([`super::NO_DOCTOR_ROW_GROUP`]).
//!
//! Slugs owned here, in contract order: `run.wine-exec`, `run.bridge-built`
//! (one gate over the pair doctor splits across `build.oxr-dylib` and
//! `build.woxr-dll`), `run.wired-adb` (evaluated only for `--wired`, which
//! needs a connected device for the `tcp:9943`/`tcp:9944` forwards). Every
//! evaluator is a read-only `fn(&CheckCtx) -> CheckOutcome`.
//!
//! Reference: `scripts/demo/run.sh`, the `# preflight: run.*` tags. With no
//! doctor row, each `message` carries run.sh's whole `die` sentence and no
//! `remedy`; the launch preflight turns a FAIL into
//! [`crate::error::SabrageError::Fatal`] with that text. See
//! `checks::tests::registry_binds_in_contract_order_and_covers_every_slug` and
//! tests::wine_exec_fails_with_run_shs_verbatim_die_text_when_the_file_is_not_executable.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::Evaluator;
#[allow(unused_imports)]
use super::{CheckCtx, CheckOutcome, CheckStatus, SkipReason};

/// Sabrage-only: the GUI may switch adb probing off so a preflight never wakes
/// the adb daemon. Same constant text `checks::headset` uses.
const PROBES_DISABLED: &str = "adb probing disabled (Sabrage setting)";

/// `[ -x "$1" ]`: file exists and has *some* execute bit set. Same
/// (deliberately not euid/egid-aware) approximation `checks::build` and
/// `Paths::which` use; kept local because that one is private to a module this
/// one does not own.
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// `run.wine-exec`: passes when `ctx.paths.wine` exists and is executable;
/// otherwise fails with run.sh's `# preflight: run.wine-exec` die sentence,
/// except in the declared divergence below.
///
/// Declared divergence — no CrossOver.app: there is no path to interpolate
/// ([`crate::paths`] models that case as `wine: None` rather than reproducing
/// lib.sh's bogus `/Contents/SharedSupport/CrossOver/bin/wine`), so the
/// sentence names the missing app instead. See
/// tests::wine_exec_says_crossover_app_not_found_when_there_is_no_path_at_all.
fn run_wine_exec(ctx: &CheckCtx) -> CheckOutcome {
    match ctx.paths.wine.as_deref() {
        Some(wine) if is_executable(wine) => CheckOutcome::pass(
            "run.wine-exec",
            format!("CrossOver wine: {}", wine.display()),
        ),
        Some(wine) => CheckOutcome::fail_bare(
            "run.wine-exec",
            format!(
                "CrossOver wine not found at {} — is CrossOver installed?",
                wine.display()
            ),
        ),
        None => CheckOutcome::fail_bare(
            "run.wine-exec",
            "CrossOver wine not found (CrossOver.app not found) — is CrossOver installed?",
        ),
    }
}

/// `run.bridge-built`: passes when both bridge outputs exist, otherwise fails
/// with run.sh's `# preflight: run.bridge-built` die sentence. One gate over
/// the pair doctor splits across `build.oxr-dylib` and `build.woxr-dll`; the
/// `detail` names whichever half is missing, which the shell's single message
/// cannot. See tests::bridge_built_needs_both_halves.
fn run_bridge_built(ctx: &CheckCtx) -> CheckOutcome {
    let dylib = &ctx.paths.oxr_dylib;
    let dll = &ctx.paths.woxr_dll;
    if dylib.is_file() && dll.is_file() {
        return CheckOutcome::pass(
            "run.bridge-built",
            format!(
                "bridge built: {} + {}",
                ctx.paths.rel_display(dylib),
                ctx.paths.rel_display(dll)
            ),
        );
    }
    let mut missing: Vec<String> = Vec::new();
    if !dylib.is_file() {
        missing.push(ctx.paths.rel_display(dylib));
    }
    if !dll.is_file() {
        missing.push(ctx.paths.rel_display(dll));
    }
    CheckOutcome::fail_bare("run.bridge-built", "bridge not built — ./demo.sh build")
        .with_detail(format!("missing: {}", missing.join(", ")))
}

/// Deadline for the one evaluator in this crate's check layer that spawns a
/// child — the same bound the launch action's twin probe applies
/// ([`crate::process::DEFAULT_PROBE_TIMEOUT`], via
/// [`crate::process::capture_with`]).
const ADB_PROBE_TIMEOUT: Duration = crate::process::DEFAULT_PROBE_TIMEOUT;

/// How often [`adb_devices_output`] asks whether the child is done. Small
/// enough to be invisible on the healthy path (adb answers in milliseconds),
/// large enough not to spin a core while waiting out a wedged one.
const ADB_PROBE_POLL: Duration = Duration::from_millis(20);

/// `"$ADB" devices` stdout, or empty when the binary is missing, fails to run,
/// or does not answer within `timeout`. Same probe `checks::headset` makes;
/// duplicated rather than shared because that probe is private to a module
/// this one does not own.
///
/// The bound is load-bearing (A7-4): evaluators are synchronous, so a wedged
/// `adb` would hold the launch's operation lock past every cancellation
/// checkpoint. tests::the_devices_probe_gives_up_on_a_wedged_adb_instead_of_blocking_forever.
///
/// `timeout` is a parameter so that test can pin the deadline in milliseconds;
/// the one production call site passes [`ADB_PROBE_TIMEOUT`].
fn adb_devices_output(adb: &Path, timeout: Duration) -> String {
    let mut child = match Command::new(adb)
        .arg("devices")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return String::new(),
    };

    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(ADB_PROBE_POLL),
            // Expired, or the wait itself failed: leave nothing behind, and
            // report the probe the way a missing binary reports.
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return String::new();
            }
        }
    }

    // The child has exited, so its stdout is closed and complete: reading it
    // now cannot block. (`adb devices` prints a handful of lines — far under
    // the pipe buffer — and a child that somehow filled the buffer instead
    // never exits, and is killed on the deadline above.)
    let mut out = Vec::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.read_to_end(&mut out);
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The first serial marked `device` in `adb devices` output, or `None` —
/// run.sh's `WIRED_SER` awk (`NR>1 && $2=="device"`) under
/// `# preflight: run.wired-adb`.
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

/// `run.wired-adb`: with `--wired`, fails with run.sh's two
/// `# preflight: run.wired-adb` die sentences when adb is absent or no device
/// answers. Without `--wired` the shell evaluates neither, so this reports
/// [`CheckStatus::Skipped`] — "not applicable", which the launch preflight
/// treats as a non-blocking row rather than as an unverifiable gate. See
/// tests::wired_adb_is_skipped_unless_wired.
fn run_wired_adb(ctx: &CheckCtx) -> CheckOutcome {
    if !ctx.opts.wired {
        return CheckOutcome::skipped("run.wired-adb", SkipReason::new("not --wired"));
    }
    let Some(adb) = ctx.paths.adb.as_deref() else {
        return CheckOutcome::fail_bare(
            "run.wired-adb",
            "--wired needs adb (Android platform-tools) on PATH or under ~/Library/Android/sdk",
        );
    };
    // Sabrage-only escape hatch; `--wired` with probing off cannot be verified,
    // and the preflight refuses to launch on an unverifiable gate (S11).
    if !ctx.opts.allow_adb_probes {
        return CheckOutcome::skipped("run.wired-adb", SkipReason::new(PROBES_DISABLED));
    }
    match first_connected_serial(&adb_devices_output(adb, ADB_PROBE_TIMEOUT)) {
        Some(serial) => CheckOutcome::pass(
            "run.wired-adb",
            format!("Quest over adb for --wired ({serial})"),
        )
        .with_detail(serial),
        None => CheckOutcome::fail_bare(
            "run.wired-adb",
            "--wired: no Quest over adb — connect USB and check 'adb devices'",
        ),
    }
}

/// Evaluators this module binds, keyed by contract slug.
pub fn defs() -> Vec<(&'static str, Evaluator)> {
    vec![
        ("run.wine-exec", run_wine_exec as Evaluator),
        ("run.bridge-built", run_bridge_built as Evaluator),
        ("run.wired-adb", run_wired_adb as Evaluator),
    ]
}

#[cfg(test)]
mod tests;
