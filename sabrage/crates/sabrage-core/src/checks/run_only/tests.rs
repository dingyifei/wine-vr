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

#[test]
fn wired_adb_is_skipped_unless_wired() {
    let root = scratch("wired-off");
    let ctx = fixture_ctx(&root, CheckOptions::new());
    let out = run_wired_adb(&ctx);
    assert_eq!(out.status, CheckStatus::Skipped);
    assert_eq!(out.message, "not --wired");
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

/// A7-4 regression: an unbounded probe let a wedged `adb` block inside the
/// evaluator, and with it the launch preflight, which holds the operation
/// lock and can only check for cancellation between evaluators. Pins that
/// the deadline fires and that the timed-out child is not left running.
#[test]
fn the_devices_probe_gives_up_on_a_wedged_adb_instead_of_blocking_forever() {
    let root = scratch("wired-wedged");
    let adb = root.join("platform-tools/adb");
    std::fs::create_dir_all(adb.parent().unwrap()).unwrap();
    // Prints its own pid, then never answers.
    std::fs::write(
        &adb,
        "#!/bin/sh\necho $$ > \"$(dirname \"$0\")/pid\"\nsleep 300\n",
    )
    .unwrap();
    std::fs::set_permissions(&adb, std::fs::Permissions::from_mode(0o755)).unwrap();

    let started = std::time::Instant::now();
    let out = adb_devices_output(&adb, Duration::from_millis(300));
    let waited = started.elapsed();

    assert_eq!(out, "", "a probe that never answered has no device list");
    assert!(
        waited < Duration::from_secs(10),
        "the probe must be bounded, waited {waited:?}"
    );
    // The killed child is gone: `kill -0` on its pid fails.
    let pid = std::fs::read_to_string(root.join("platform-tools/pid")).unwrap_or_default();
    let pid = pid.trim();
    if !pid.is_empty() {
        let alive = Command::new("/bin/kill")
            .args(["-0", pid])
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        assert!(!alive, "the timed-out probe left {pid} running");
    }
    std::fs::remove_dir_all(&root).ok();
}
