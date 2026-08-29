//! Group `run-only` — doctor.sh section n/a: preflights that exist only in the launch path (no doctor row).
//!
//! Slugs owned here, in contract order:
//!
//! * `run.wine-exec` — the CrossOver `wine` binary is present and executable
//! * `run.bridge-built` — both bridge build outputs exist — run covers the
//!   `build.woxr-dll`/`build.woxr-so` pair with this single gate
//! * `run.wired-adb` — only evaluated for `--wired`: an adb device is
//!   connected so the `tcp:9943`/`tcp:9944` forwards can be created
//!
//! Every evaluator is `fn(&CheckCtx) -> CheckOutcome`: a **read-only probe**.
//!
//! # These three have no doctor row, so their prose comes from `run.sh`
//!
//! The module-level rule elsewhere in [`crate::checks`] is "message and remedy
//! strings must match `scripts/demo/doctor.sh` verbatim". doctor.sh never
//! prints these slugs at all ([`super::NO_DOCTOR_ROW_GROUP`]), so the text to
//! match is `run.sh`'s `die` string instead — and it is carried whole in
//! `message`, with no separate `remedy`, because that is the shape `run.sh`
//! prints:
//!
//! ```zsh
//! # preflight: run.wine-exec
//! [ -x "$WINE" ] || die "CrossOver wine not found at $WINE — is CrossOver installed?"
//! # preflight: run.bridge-built
//! [ -f "$OXR_DYLIB" ] && [ -f "$WOXR_DLL" ] || die "bridge not built — ./demo.sh build"
//! # preflight: run.wired-adb
//! [ -n "$ADB" ] || die "--wired needs adb (Android platform-tools) on PATH or under ~/Library/Android/sdk"
//! [ -n "$WIRED_SER" ] || die "--wired: no Quest over adb — connect USB and check 'adb devices'"
//! ```
//!
//! The launch preflight ([`crate::stages::run::preflight`]) turns a FAIL here
//! into [`crate::error::SabrageError::Fatal`] carrying exactly that text, so
//! the two front-ends abort with the same sentence.

use std::path::Path;
use std::process::Command;

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

// ── run.wine-exec ────────────────────────────────────────────────────────────

/// `run.sh` line 17: `[ -x "$WINE" ] || die "CrossOver wine not found at $WINE
/// — is CrossOver installed?"`.
///
/// **Declared divergence — the CrossOver-absent message.** lib.sh builds
/// `CX="${CX_APP:-}/Contents/SharedSupport/CrossOver"` unconditionally, so with
/// no CrossOver.app installed the shell's `$WINE` is the bogus absolute path
/// `/Contents/SharedSupport/CrossOver/bin/wine` and the die reads "CrossOver
/// wine not found at /Contents/SharedSupport/CrossOver/bin/wine". [`crate::paths`]
/// deliberately models that case as `wine: None` rather than reproducing a path
/// that looks real (design-core §1), so there is no path to interpolate. Rather
/// than fabricate the shell's misleading one, this branch says what is actually
/// true — `CrossOver.app not found` — inside the same sentence. The
/// CrossOver-**present** branch (a `wine` that exists but is not executable, or
/// is missing from an installed CrossOver) is `run.sh`'s string verbatim, which
/// is the case that actually happens.
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

// ── run.bridge-built ─────────────────────────────────────────────────────────

/// `run.sh` line 19: `[ -f "$OXR_DYLIB" ] && [ -f "$WOXR_DLL" ] || die "bridge
/// not built — ./demo.sh build"`.
///
/// One gate over the pair doctor splits across `build.oxr-dylib` and
/// `build.woxr-dll`; the `detail` names whichever half is actually missing,
/// which the shell's single message cannot.
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

// ── run.wired-adb ────────────────────────────────────────────────────────────

/// `"$ADB" devices` stdout, or empty when the binary is missing or fails to
/// run. Same probe `checks::headset` makes; duplicated rather than shared
/// because that one is private to a module this one does not own.
fn adb_devices_output(adb: &Path) -> String {
    match Command::new(adb).arg("devices").output() {
        Ok(out) => String::from_utf8_lossy(&out.stdout).into_owned(),
        Err(_) => String::new(),
    }
}

/// `awk 'NR>1 && $2=="device"{print $1; exit}'` over `adb devices` output —
/// `run.sh` line 102's `WIRED_SER`.
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

/// `run.sh` lines 103–105, the two `--wired` preconditions:
///
/// ```zsh
/// if [ -n "${WINEVR_WIRED:-}" ]; then
///   [ -n "$ADB" ] || die "--wired needs adb (Android platform-tools) on PATH or under ~/Library/Android/sdk"
///   [ -n "$WIRED_SER" ] || die "--wired: no Quest over adb — connect USB and check 'adb devices'"
/// ```
///
/// Without `--wired` the shell evaluates neither, so this reports
/// [`CheckStatus::Skipped`] — "not applicable", which the launch preflight
/// treats as a non-blocking row rather than as an unverifiable gate.
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
    match first_connected_serial(&adb_devices_output(adb)) {
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
mod tests {
    use super::*;
    use crate::checks::CheckOptions;
    use crate::paths::Paths;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    fn scratch(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("sabrage-run-only-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    /// A context over a fixture root, never the real machine's CrossOver or
    /// adb: both are `None` unless a test sets them.
    fn fixture_ctx(root: &Path, opts: CheckOptions) -> CheckCtx {
        let mut paths = Paths::new(root);
        paths.cx_app = None;
        paths.cx = None;
        paths.wine = None;
        paths.wineserver = None;
        paths.adb = None;
        paths.oxr_appsup = root.join("OXRSys");
        paths.toml_path = paths.oxr_appsup.join("oxrsys-runtime.toml");
        paths.sabrage_appsup = root.join("Sabrage");
        CheckCtx::new(paths, opts)
    }

    fn touch_exec(p: &Path) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn touch(p: &Path) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"x").unwrap();
    }

    // ── run.wine-exec ────────────────────────────────────────────────────────

    #[test]
    fn wine_exec_passes_for_an_executable_wine() {
        let root = scratch("wine-ok");
        let mut ctx = fixture_ctx(&root, CheckOptions::new());
        let wine = root.join("CrossOver/bin/wine");
        touch_exec(&wine);
        ctx.paths.wine = Some(wine);

        let out = run_wine_exec(&ctx);
        assert_eq!(out.status, CheckStatus::Pass);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn wine_exec_fails_with_run_shs_verbatim_die_text_when_the_file_is_not_executable() {
        let root = scratch("wine-noexec");
        let mut ctx = fixture_ctx(&root, CheckOptions::new());
        let wine = root.join("CrossOver/bin/wine");
        touch(&wine); // present, mode 0644
        ctx.paths.wine = Some(wine.clone());

        let out = run_wine_exec(&ctx);
        assert_eq!(out.status, CheckStatus::Fail);
        assert_eq!(
            out.message,
            format!(
                "CrossOver wine not found at {} — is CrossOver installed?",
                wine.display()
            )
        );
        assert_eq!(out.remedy, None, "run.sh's die carries no remedy line");
        std::fs::remove_dir_all(&root).ok();
    }

    /// The declared divergence: no CrossOver.app means no path to interpolate,
    /// so the sentence says so instead of quoting lib.sh's bogus
    /// `/Contents/SharedSupport/CrossOver/bin/wine`.
    #[test]
    fn wine_exec_says_crossover_app_not_found_when_there_is_no_path_at_all() {
        let root = scratch("wine-none");
        let ctx = fixture_ctx(&root, CheckOptions::new());
        let out = run_wine_exec(&ctx);
        assert_eq!(out.status, CheckStatus::Fail);
        assert_eq!(
            out.message,
            "CrossOver wine not found (CrossOver.app not found) — is CrossOver installed?"
        );
        assert!(
            !out.message
                .contains("/Contents/SharedSupport/CrossOver/bin/wine"),
            "must not fabricate lib.sh's bogus path"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    // ── run.bridge-built ─────────────────────────────────────────────────────

    #[test]
    fn bridge_built_needs_both_halves() {
        let root = scratch("bridge");
        let ctx = fixture_ctx(&root, CheckOptions::new());

        // Neither.
        let out = run_bridge_built(&ctx);
        assert_eq!(out.status, CheckStatus::Fail);
        assert_eq!(out.message, "bridge not built — ./demo.sh build");
        let detail = out.detail.clone().unwrap();
        assert!(detail.contains("liboxrsys-runtime.dylib"), "{detail}");
        assert!(detail.contains("wineopenxr.dll"), "{detail}");

        // Only the dylib.
        touch(&ctx.paths.oxr_dylib);
        let out = run_bridge_built(&ctx);
        assert_eq!(out.status, CheckStatus::Fail);
        assert_eq!(out.message, "bridge not built — ./demo.sh build");
        assert!(!out.detail.clone().unwrap().contains("liboxrsys-runtime"));

        // Both.
        touch(&ctx.paths.woxr_dll);
        assert_eq!(run_bridge_built(&ctx).status, CheckStatus::Pass);

        std::fs::remove_dir_all(&root).ok();
    }

    // ── run.wired-adb ────────────────────────────────────────────────────────

    #[test]
    fn wired_adb_is_skipped_unless_wired() {
        let root = scratch("wired-off");
        let ctx = fixture_ctx(&root, CheckOptions::new());
        let out = run_wired_adb(&ctx);
        assert_eq!(out.status, CheckStatus::Skipped);
        assert_eq!(out.message, "not --wired");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn wired_adb_fails_verbatim_when_adb_is_absent() {
        let root = scratch("wired-noadb");
        let ctx = fixture_ctx(
            &root,
            CheckOptions {
                wired: true,
                ..CheckOptions::new()
            },
        );
        let out = run_wired_adb(&ctx);
        assert_eq!(out.status, CheckStatus::Fail);
        assert_eq!(
            out.message,
            "--wired needs adb (Android platform-tools) on PATH or under ~/Library/Android/sdk"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    /// With probing disabled the row is honestly unverifiable rather than a
    /// pass — the preflight turns this into a Fatal, never a launch.
    #[test]
    fn wired_adb_is_skipped_with_a_reason_when_probing_is_disabled() {
        let root = scratch("wired-noprobe");
        let mut ctx = fixture_ctx(
            &root,
            CheckOptions {
                wired: true,
                allow_adb_probes: false,
                ..CheckOptions::new()
            },
        );
        let adb = root.join("platform-tools/adb");
        touch_exec(&adb);
        ctx.paths.adb = Some(adb);

        let out = run_wired_adb(&ctx);
        assert_eq!(out.status, CheckStatus::Skipped);
        assert_eq!(out.message, PROBES_DISABLED);
        std::fs::remove_dir_all(&root).ok();
    }

    /// A fake `adb` that reports one connected Quest: the row passes and
    /// carries the serial as `detail`, which is what the `--wired` launch
    /// action forwards ports on. Still no real adb daemon.
    #[test]
    fn wired_adb_passes_with_the_serial_as_detail() {
        let root = scratch("wired-dev");
        let mut ctx = fixture_ctx(
            &root,
            CheckOptions {
                wired: true,
                ..CheckOptions::new()
            },
        );
        let adb = root.join("platform-tools/adb");
        std::fs::create_dir_all(adb.parent().unwrap()).unwrap();
        std::fs::write(
            &adb,
            "#!/bin/sh\nprintf 'List of devices attached\\nBAD1\\toffline\\n1WMHH000X0\\tdevice\\n'\n",
        )
        .unwrap();
        std::fs::set_permissions(&adb, std::fs::Permissions::from_mode(0o755)).unwrap();
        ctx.paths.adb = Some(adb);

        let out = run_wired_adb(&ctx);
        assert_eq!(out.status, CheckStatus::Pass);
        assert_eq!(out.detail.as_deref(), Some("1WMHH000X0"));
        std::fs::remove_dir_all(&root).ok();
    }

    /// A fake `adb` that prints nothing: the `awk` finds no `device` row, so
    /// the second `--wired` die fires. No real adb daemon is touched.
    #[test]
    fn wired_adb_fails_verbatim_when_no_device_is_connected() {
        let root = scratch("wired-nodev");
        let mut ctx = fixture_ctx(
            &root,
            CheckOptions {
                wired: true,
                ..CheckOptions::new()
            },
        );
        let adb = root.join("platform-tools/adb");
        std::fs::create_dir_all(adb.parent().unwrap()).unwrap();
        std::fs::write(&adb, "#!/bin/sh\necho 'List of devices attached'\n").unwrap();
        std::fs::set_permissions(&adb, std::fs::Permissions::from_mode(0o755)).unwrap();
        ctx.paths.adb = Some(adb);

        let out = run_wired_adb(&ctx);
        assert_eq!(out.status, CheckStatus::Fail);
        assert_eq!(
            out.message,
            "--wired: no Quest over adb — connect USB and check 'adb devices'"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn first_connected_serial_skips_the_header_and_non_device_states() {
        let out = "List of devices attached\nemulator-5554\toffline\n1A2B3C4D\tdevice\n";
        assert_eq!(first_connected_serial(out).as_deref(), Some("1A2B3C4D"));
        assert_eq!(first_connected_serial("List of devices attached\n"), None);
        // The header row itself never counts, even if it somehow parsed.
        assert_eq!(first_connected_serial("only-a-header\n"), None);
    }

    #[test]
    fn defs_cover_exactly_the_contracts_run_only_group() {
        let bound: Vec<&str> = defs().into_iter().map(|(s, _)| s).collect();
        let declared: Vec<&str> = crate::contract::contract()
            .checks
            .iter()
            .filter(|c| c.group == crate::checks::NO_DOCTOR_ROW_GROUP)
            .map(|c| c.slug.as_str())
            .collect();
        assert_eq!(bound, declared);
    }
}
