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
mod tests;
