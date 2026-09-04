//! Group `host` — doctor.sh section 12: the root-owned host OpenXR registration.
//!
//! Slugs owned here, in contract order:
//!
//! * `host.manifest` — `/usr/local/share/openxr/1/active_runtime.x86_64.json`
//!   exists and its parsed `runtime.library_path` equals the expected
//!   `oxr_dylib` and that file exists. Wine's secure-exec ignores
//!   `XR_RUNTIME_JSON`, so this file is the only thing routing the game to
//!   oxrsys — and it embeds an ABSOLUTE path, so moving the repo breaks it.
//!   Pointing at a different but existing dylib is WARN; pointing at a
//!   missing one is FAIL
//!
//! Every evaluator is `fn(&CheckCtx) -> CheckOutcome`: a **read-only probe**.
//! Message and remedy strings must match `scripts/demo/doctor.sh` verbatim.

use std::path::Path;

use super::Evaluator;
#[allow(unused_imports)]
use super::{CheckCtx, CheckOutcome, CheckStatus, SkipReason};

/// `host.manifest`: the root-owned host OpenXR manifest exists and its
/// `runtime.library_path` routes to the expected `oxr_dylib`.
///
/// Reference: scripts/demo/doctor.sh `# 12. host loader registration`
fn host_manifest(ctx: &CheckCtx) -> CheckOutcome {
    let host_json = &ctx.paths.host_xr_json;
    if !host_json.is_file() {
        return CheckOutcome::fail(
            "host.manifest",
            format!("{} missing", host_json.display()),
            format!(
                "./demo.sh install --bottle {} (sudo writes it)",
                ctx.bottle_label()
            ),
        );
    }

    let Some(lp) = host_manifest_library_path(host_json) else {
        return CheckOutcome::fail(
            "host.manifest",
            format!(
                "cannot parse {} (broken python3 or malformed JSON)",
                host_json.display()
            ),
            "check 'python3 -V' works (xcode-select --install), then inspect the file",
        );
    };

    let expected = &ctx.paths.oxr_dylib;
    let lp_path = Path::new(&lp);
    let outcome = if lp == expected.to_string_lossy() && lp_path.is_file() {
        CheckOutcome::pass("host.manifest", format!("host OpenXR registration -> {lp}"))
    } else if !lp.is_empty() && lp_path.is_file() {
        CheckOutcome::warn(
            "host.manifest",
            format!(
                "host registration points at {lp} (expected {})",
                expected.display()
            ),
        )
    } else {
        CheckOutcome::fail(
            "host.manifest",
            "host registration points at a missing dylib",
            format!(
                "./demo.sh install --bottle {} (sudo rewrites {})",
                ctx.bottle_label(),
                host_json.display()
            ),
        )
    };
    outcome.with_detail(format!("parsed library_path = {lp:?}"))
}

/// Parses `runtime.library_path` out of the host OpenXR manifest at `path`.
///
/// Returns `None` for every way the shell's `PYRC != 0` branch is reachable:
/// unreadable file, malformed JSON, missing or non-object `"runtime"`, or
/// missing `"library_path"`. A non-string `library_path` also yields `None`
/// (real Python would stringify it); `contract/active_runtime.x86_64.json.template`
/// never writes one, and both routes end in FAIL.
///
/// `pub` because `src-tauri/src/commands.rs`'s `get_repo_info` reuses this
/// parse for its `hostManifestLibraryPath`/`hostManifestPointsHere` fields
/// rather than poking the JSON a second time.
pub fn host_manifest_library_path(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value
        .get("runtime")?
        .get("library_path")?
        .as_str()
        .map(str::to_string)
}

/// Evaluators this module binds, keyed by contract slug.
pub fn defs() -> Vec<(&'static str, Evaluator)> {
    vec![("host.manifest", host_manifest as Evaluator)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::CheckOptions;
    use crate::paths::Paths;
    use std::fs;
    use std::path::PathBuf;

    fn scratch(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("sabrage-host-test-{}-{tag}", std::process::id()))
    }

    fn ctx_with(tmp: &Path, host_json: PathBuf, oxr_dylib: PathBuf) -> CheckCtx {
        let mut paths = Paths::new(tmp);
        paths.host_xr_json = host_json;
        paths.oxr_dylib = oxr_dylib;
        CheckCtx::new(paths, CheckOptions::new())
    }

    fn write_manifest(path: &Path, library_path: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            path,
            format!(r#"{{"runtime":{{"name":"oxrsys","library_path":"{library_path}"}}}}"#),
        )
        .unwrap();
    }

    #[test]
    fn missing_manifest_fails_with_the_install_remedy() {
        let tmp = scratch("missing");
        let host_json = tmp.join("host/active_runtime.x86_64.json");
        let ctx = ctx_with(&tmp, host_json.clone(), tmp.join("dylib"));
        let o = host_manifest(&ctx);
        assert_eq!(o.status, CheckStatus::Fail);
        assert_eq!(o.message, format!("{} missing", host_json.display()));
        assert_eq!(
            o.remedy.as_deref(),
            Some("./demo.sh install --bottle <name> (sudo writes it)")
        );
    }

    #[test]
    fn invalid_manifest_bytes_fail_with_the_parse_remedy() {
        // Both rows reach the same None arm by different hops: bytes serde
        // cannot parse, and valid JSON with no "runtime" key.
        const CASES: &[(&str, &[u8])] = &[
            ("malformed json", b"{not json"),
            ("missing runtime key", b"{}"),
        ];
        for &(label, bytes) in CASES {
            let tmp = scratch(&label.replace(' ', "-"));
            let host_json = tmp.join("host/active_runtime.x86_64.json");
            fs::create_dir_all(host_json.parent().unwrap()).unwrap();
            fs::write(&host_json, bytes).unwrap();
            let ctx = ctx_with(&tmp, host_json.clone(), tmp.join("dylib"));
            let o = host_manifest(&ctx);
            assert_eq!(o.status, CheckStatus::Fail, "{label}");
            assert_eq!(
                o.message,
                format!(
                    "cannot parse {} (broken python3 or malformed JSON)",
                    host_json.display()
                ),
                "{label}"
            );
            assert_eq!(
                o.remedy.as_deref(),
                Some("check 'python3 -V' works (xcode-select --install), then inspect the file"),
                "{label}"
            );
            fs::remove_dir_all(&tmp).ok();
        }
    }

    #[test]
    fn matching_and_present_dylib_passes() {
        let tmp = scratch("match");
        let host_json = tmp.join("host/active_runtime.x86_64.json");
        let dylib = tmp.join("ext/oxrsys/build-x64/runtime/liboxrsys-runtime.dylib");
        fs::create_dir_all(dylib.parent().unwrap()).unwrap();
        fs::write(&dylib, b"stub").unwrap();
        write_manifest(&host_json, &dylib.to_string_lossy());
        let ctx = ctx_with(&tmp, host_json, dylib.clone());
        let o = host_manifest(&ctx);
        assert_eq!(o.status, CheckStatus::Pass);
        assert_eq!(
            o.message,
            format!("host OpenXR registration -> {}", dylib.display())
        );
        assert!(o.remedy.is_none());
        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn different_but_existing_dylib_warns() {
        let tmp = scratch("different");
        let host_json = tmp.join("host/active_runtime.x86_64.json");
        let actual = tmp.join("elsewhere/lib.dylib");
        let expected = tmp.join("ext/oxrsys/build-x64/runtime/liboxrsys-runtime.dylib");
        fs::create_dir_all(actual.parent().unwrap()).unwrap();
        fs::write(&actual, b"stub").unwrap();
        write_manifest(&host_json, &actual.to_string_lossy());
        let ctx = ctx_with(&tmp, host_json, expected.clone());
        let o = host_manifest(&ctx);
        assert_eq!(o.status, CheckStatus::Warn);
        assert_eq!(
            o.message,
            format!(
                "host registration points at {} (expected {})",
                actual.display(),
                expected.display()
            )
        );
        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn pointing_at_a_missing_dylib_fails() {
        let tmp = scratch("dangling");
        let host_json = tmp.join("host/active_runtime.x86_64.json");
        let expected = tmp.join("ext/oxrsys/build-x64/runtime/liboxrsys-runtime.dylib");
        write_manifest(&host_json, "/nowhere/liboxrsys-runtime.dylib");
        let ctx = ctx_with(&tmp, host_json.clone(), expected);
        let o = host_manifest(&ctx);
        assert_eq!(o.status, CheckStatus::Fail);
        assert_eq!(o.message, "host registration points at a missing dylib");
        assert_eq!(
            o.remedy.as_deref(),
            Some(
                format!(
                    "./demo.sh install --bottle <name> (sudo rewrites {})",
                    host_json.display()
                )
                .as_str()
            )
        );
        fs::remove_dir_all(&tmp).ok();
    }
}
