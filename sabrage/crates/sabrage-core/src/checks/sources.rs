//! Group `sources` — doctor.sh section 6: submodule checkouts and the ALVR patch set.
//!
//! Slugs, in contract order: `src.oxrsys`, `src.wineopenxr`, `src.alvr`,
//! `src.alvr-patchset` (`is_streaming_nonblocking` pin sanity-check for
//! oxrsys-v20.14.1).

use std::path::Path;

use super::Evaluator;
use super::{CheckCtx, CheckOutcome};

/// True when the submodule has a `.git` entry — a file (gitlink) or a
/// directory (nested clone).
fn git_marker_present(submodule_dir: &Path) -> bool {
    let marker = submodule_dir.join(".git");
    marker.is_file() || marker.is_dir()
}

fn basename(p: &Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Shared shape of the three `src.*` submodule-presence checks.
fn submodule_check(slug: &'static str, dir: &Path) -> CheckOutcome {
    let name = basename(dir);
    if git_marker_present(dir) {
        CheckOutcome::pass(slug, format!("submodule {name} present"))
    } else {
        CheckOutcome::fail(
            slug,
            format!("submodule {name} not initialized"),
            "./demo.sh setup",
        )
    }
}

fn src_oxrsys(ctx: &CheckCtx) -> CheckOutcome {
    submodule_check("src.oxrsys", &ctx.paths.oxrsys)
}

fn src_wineopenxr(ctx: &CheckCtx) -> CheckOutcome {
    submodule_check("src.wineopenxr", &ctx.paths.woxr)
}

fn src_alvr(ctx: &CheckCtx) -> CheckOutcome {
    submodule_check("src.alvr", &ctx.paths.alvr)
}

/// Passes when ALVR's `alvr/server_core/src/connection.rs` contains `is_streaming_nonblocking`;
/// a missing or unreadable file fails (`tests::patchset_check_greps_connection_rs`). No regex
/// metacharacters in the needle, so this substring search and doctor.sh's `grep -q` agree.
fn src_alvr_patchset(ctx: &CheckCtx) -> CheckOutcome {
    let path = ctx.paths.alvr.join("alvr/server_core/src/connection.rs");
    let present = std::fs::read(&path)
        .map(|bytes| String::from_utf8_lossy(&bytes).contains("is_streaming_nonblocking"))
        .unwrap_or(false);
    if present {
        CheckOutcome::pass("src.alvr-patchset", "ALVR oxrsys patch set present")
    } else {
        CheckOutcome::fail(
            "src.alvr-patchset",
            "ALVR submodule missing the oxrsys patches",
            "./demo.sh setup (checks out the pinned oxrsys-v20.14.1 branch)",
        )
    }
}

/// Evaluators this module binds, keyed by contract slug.
pub fn defs() -> Vec<(&'static str, Evaluator)> {
    vec![
        ("src.oxrsys", src_oxrsys as Evaluator),
        ("src.wineopenxr", src_wineopenxr as Evaluator),
        ("src.alvr", src_alvr as Evaluator),
        ("src.alvr-patchset", src_alvr_patchset as Evaluator),
    ]
}

#[cfg(test)]
mod tests;
