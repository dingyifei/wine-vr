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
