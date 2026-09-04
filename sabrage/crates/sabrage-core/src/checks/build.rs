//! Group `build` — doctor.sh section 9, 9b: build outputs, including the native-arm64 encoder helper.
//!
//! Slugs owned here, in contract order: `build.oxr-dylib`, `build.alvr-core`,
//! `build.runtime-json`, `build.woxr-dll`, `build.woxr-so`, `build.dashboard`,
//! `build.helper-staged`, `build.helper-arm64`.
//!
//! Every evaluator is `fn(&CheckCtx) -> CheckOutcome`: a read-only probe whose
//! message and remedy strings must match `scripts/demo/doctor.sh` verbatim.
//!
//! `build.helper-arm64` must not accept `arm64e` alone: a wrong-arch binary
//! staged next to the runtime dylib shadows the good one and silently drops the
//! session to in-process H.264 (tests::helper_is_arm64_rejects_arm64e_only_binaries).

use std::path::Path;

use super::Evaluator;
use super::{CheckCtx, CheckOutcome, SkipReason};

/// Shared shape of the six `build.*` output-presence checks: passes with
/// `built: <relpath>`, fails with `missing build output: <relpath>` and the
/// `./demo.sh build` remedy, where `<relpath>` is `Paths::rel_display`.
fn built_output(ctx: &CheckCtx, slug: &'static str, path: &Path) -> CheckOutcome {
    let rel = ctx.paths.rel_display(path);
    if path.is_file() {
        CheckOutcome::pass(slug, format!("built: {rel}"))
    } else {
        CheckOutcome::fail(
            slug,
            format!("missing build output: {rel}"),
            "./demo.sh build",
        )
    }
}

fn oxr_dylib(ctx: &CheckCtx) -> CheckOutcome {
    built_output(ctx, "build.oxr-dylib", &ctx.paths.oxr_dylib)
}

fn alvr_core(ctx: &CheckCtx) -> CheckOutcome {
    built_output(ctx, "build.alvr-core", &ctx.paths.oxr_alvr_dylib)
}

fn runtime_json(ctx: &CheckCtx) -> CheckOutcome {
    built_output(ctx, "build.runtime-json", &ctx.paths.oxr_runtime_json)
}

fn woxr_dll(ctx: &CheckCtx) -> CheckOutcome {
    built_output(ctx, "build.woxr-dll", &ctx.paths.woxr_dll)
}

fn woxr_so(ctx: &CheckCtx) -> CheckOutcome {
    built_output(ctx, "build.woxr-so", &ctx.paths.woxr_so)
}

fn dashboard(ctx: &CheckCtx) -> CheckOutcome {
    built_output(ctx, "build.dashboard", &ctx.paths.alvr_dashboard)
}

/// True when `p` is a regular file with any execute bit set — `[ -x "$1" ]`
/// to the same approximation the `paths` module's `which()` uses (no
/// euid/egid resolution, which `lib.sh` never relied on either).
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// `lipo -archs <path>` stdout with trailing newlines stripped as `$(...)` does;
/// empty when `lipo` cannot run or writes nothing. Exit status is ignored; the
/// FAIL message of `build.helper-arm64` embeds this value.
pub fn lipo_archs_stdout(path: &Path) -> String {
    match std::process::Command::new("lipo")
        .arg("-archs")
        .arg(path)
        .output()
    {
        Ok(out) => {
            crate::util::strip_trailing_newlines(&String::from_utf8_lossy(&out.stdout)).to_string()
        }
        Err(_) => String::new(),
    }
}

/// True when `path` is executable and `lipo -archs` lists `arm64` as a whole
/// word. Single home of lib.sh's `helper_is_arm64()`; `crate::util` re-exports
/// it for the fix and stage layers.
///
/// A fat `x86_64 arm64e` binary must NOT match, while `x86_64 arm64` and thin
/// `arm64` must (tests::helper_is_arm64_rejects_arm64e_only_binaries,
/// tests::helper_is_arm64_is_true_for_the_thin_arm64_test_binary_itself).
pub fn helper_is_arm64(path: &Path) -> bool {
    if !is_executable(path) {
        return false;
    }
    lipo_archs_stdout(path)
        .split_ascii_whitespace()
        .any(|arch| arch == "arm64")
}

fn helper_staged(ctx: &CheckCtx) -> CheckOutcome {
    let bin = &ctx.paths.oxr_helper_staged;
    let rel = ctx.paths.rel_display(bin);
    if bin.is_file() {
        CheckOutcome::pass(
            "build.helper-staged",
            format!("built: {rel} (staged next to the runtime dylib)"),
        )
    } else {
        CheckOutcome::fail(
            "build.helper-staged",
            format!("encoder helper not staged: {rel}"),
            "./demo.sh build",
        )
    }
}

fn helper_arm64(ctx: &CheckCtx) -> CheckOutcome {
    let bin = &ctx.paths.oxr_helper_staged;
    if !bin.is_file() {
        // doctor.sh: `tap build.helper-arm64 skipped` in the helper-staged-FAIL
        // arm, with no explanatory text of its own; sabrage supplies one.
        return CheckOutcome::skipped(
            "build.helper-arm64",
            SkipReason::new(format!(
                "encoder helper not staged: {}",
                ctx.paths.rel_display(bin)
            )),
        );
    }
    if helper_is_arm64(bin) {
        CheckOutcome::pass("build.helper-arm64", "encoder helper is arm64")
    } else {
        let archs = lipo_archs_stdout(bin);
        CheckOutcome::fail(
            "build.helper-arm64",
            format!(
                "encoder helper is not an arm64 executable ({archs}) — a stale/wrong-arch binary here shadows the staged one"
            ),
            "./demo.sh build (restages the arm64 helper)",
        )
    }
}

/// Evaluators this module binds, keyed by contract slug.
pub fn defs() -> Vec<(&'static str, Evaluator)> {
    vec![
        ("build.oxr-dylib", oxr_dylib as Evaluator),
        ("build.alvr-core", alvr_core as Evaluator),
        ("build.runtime-json", runtime_json as Evaluator),
        ("build.woxr-dll", woxr_dll as Evaluator),
        ("build.woxr-so", woxr_so as Evaluator),
        ("build.dashboard", dashboard as Evaluator),
        ("build.helper-staged", helper_staged as Evaluator),
        ("build.helper-arm64", helper_arm64 as Evaluator),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::{CheckOptions, CheckStatus};
    use crate::paths::Paths;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    fn ctx_for(root: &Path) -> CheckCtx {
        CheckCtx::new(Paths::new(root), CheckOptions::new())
    }

    fn scratch(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("sabrage-build-test-{}-{tag}", std::process::id()))
    }

    #[test]
    fn missing_output_fails_with_the_build_remedy() {
        let ctx = ctx_for(Path::new("/nonexistent/sabrage-build-probe"));
        let o = oxr_dylib(&ctx);
        assert_eq!(o.status, CheckStatus::Fail);
        assert!(o.message.starts_with("missing build output: "));
        assert!(o.message.contains("liboxrsys-runtime.dylib"));
        assert_eq!(o.remedy.as_deref(), Some("./demo.sh build"));
    }

    #[test]
    fn present_output_passes_with_relative_path() {
        let tmp = scratch("dylib-present");
        let dylib = tmp.join("ext/oxrsys/build-x64/runtime/liboxrsys-runtime.dylib");
        fs::create_dir_all(dylib.parent().unwrap()).unwrap();
        fs::write(&dylib, b"stub").unwrap();
        let o = oxr_dylib(&ctx_for(&tmp));
        assert_eq!(o.status, CheckStatus::Pass);
        assert_eq!(
            o.message,
            "built: ext/oxrsys/build-x64/runtime/liboxrsys-runtime.dylib"
        );
        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn all_six_build_outputs_are_wired_to_their_own_paths() {
        let tmp = scratch("all-six");
        let ctx = ctx_for(&tmp);
        let cases: [(Evaluator, &Path); 6] = [
            (oxr_dylib, &ctx.paths.oxr_dylib),
            (alvr_core, &ctx.paths.oxr_alvr_dylib),
            (runtime_json, &ctx.paths.oxr_runtime_json),
            (woxr_dll, &ctx.paths.woxr_dll),
            (woxr_so, &ctx.paths.woxr_so),
            (dashboard, &ctx.paths.alvr_dashboard),
        ];
        for (eval, path) in cases {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"stub").unwrap();
            let o = eval(&ctx);
            assert_eq!(o.status, CheckStatus::Pass, "{path:?} should be present");
            fs::remove_file(path).unwrap();
            let o = eval(&ctx);
            assert_eq!(
                o.status,
                CheckStatus::Fail,
                "{path:?} should be missing again"
            );
        }
        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn helper_staged_missing_fails_and_arm64_check_is_skipped_with_a_reason() {
        let ctx = ctx_for(Path::new("/nonexistent/sabrage-build-probe"));
        let staged = helper_staged(&ctx);
        assert_eq!(staged.status, CheckStatus::Fail);
        assert!(staged.message.starts_with("encoder helper not staged: "));
        assert_eq!(staged.remedy.as_deref(), Some("./demo.sh build"));

        let arm64 = helper_arm64(&ctx);
        assert_eq!(arm64.status, CheckStatus::Skipped);
        assert!(arm64.message.starts_with("encoder helper not staged: "));
    }

    #[test]
    fn helper_staged_present_passes_with_the_staged_suffix() {
        let tmp = scratch("helper-staged");
        let bin = tmp.join("ext/oxrsys/build-x64/runtime/oxrsys-encoder-helper");
        fs::create_dir_all(bin.parent().unwrap()).unwrap();
        fs::write(&bin, b"stub").unwrap();
        let o = helper_staged(&ctx_for(&tmp));
        assert_eq!(o.status, CheckStatus::Pass);
        assert_eq!(
            o.message,
            "built: ext/oxrsys/build-x64/runtime/oxrsys-encoder-helper (staged next to the runtime dylib)"
        );
        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn helper_is_arm64_is_true_for_the_thin_arm64_test_binary_itself() {
        // The compiled test binary is itself a thin-arm64 Mach-O on this
        // repo's target machine: a real positive case with no compiler.
        // Skipped where that cannot hold (Intel Mac, Linux CI, no usable lipo).
        let exe = std::env::current_exe().expect("current_exe resolves");
        let archs = lipo_archs_stdout(&exe);
        if !archs.split_ascii_whitespace().any(|a| a == "arm64") {
            return;
        }
        assert!(is_executable(&exe));
        assert!(helper_is_arm64(&exe), "lipo -archs {exe:?} = {archs:?}");
    }

    #[test]
    fn helper_is_arm64_rejects_arm64e_only_binaries() {
        // /bin/ls ships as a universal `x86_64 arm64e` binary on every macOS
        // install — real-machine proof that `arm64e` alone must not satisfy
        // `grep -qw arm64` (the `e` makes it a different word).
        let ls = Path::new("/bin/ls");
        if !ls.is_file() {
            return;
        }
        let archs = lipo_archs_stdout(ls);
        if archs.split_ascii_whitespace().any(|a| a == "arm64") {
            return; // this machine's /bin/ls happens to carry a plain arm64 slice
        }
        assert!(!helper_is_arm64(ls), "archs were {archs:?}");
    }

    #[test]
    fn helper_is_arm64_false_for_a_non_executable_arm64_binary() {
        let exe = std::env::current_exe().expect("current_exe resolves");
        let tmp = scratch("non-exec-copy");
        fs::create_dir_all(&tmp).unwrap();
        let copy = tmp.join("oxrsys-encoder-helper");
        fs::copy(&exe, &copy).unwrap();
        let mut perms = fs::metadata(&copy).unwrap().permissions();
        perms.set_mode(0o644); // readable, not executable
        fs::set_permissions(&copy, perms).unwrap();

        assert!(!is_executable(&copy));
        assert!(!helper_is_arm64(&copy));
        // The FAIL message's lipo capture is independent of the -x gate: lipo
        // can still read the arch of a non-executable file.
        assert!(lipo_archs_stdout(&copy).contains("arm64"));
        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn helper_arm64_fails_with_the_lipo_output_embedded_when_wrong_arch() {
        let tmp = scratch("wrong-arch");
        let bin = tmp.join("ext/oxrsys/build-x64/runtime/oxrsys-encoder-helper");
        fs::create_dir_all(bin.parent().unwrap()).unwrap();
        // Not a Mach-O at all: lipo fails, stdout capture is empty, exactly
        // like the shell's `$(lipo -archs … 2>/dev/null)` on a bad file.
        fs::write(&bin, b"not a mach-o").unwrap();
        let mut perms = fs::metadata(&bin).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&bin, perms).unwrap();

        let o = helper_arm64(&ctx_for(&tmp));
        assert_eq!(o.status, CheckStatus::Fail);
        assert_eq!(
            o.message,
            "encoder helper is not an arm64 executable () — a stale/wrong-arch binary here shadows the staged one"
        );
        assert_eq!(
            o.remedy.as_deref(),
            Some("./demo.sh build (restages the arm64 helper)")
        );
        fs::remove_dir_all(&tmp).ok();
    }
}
