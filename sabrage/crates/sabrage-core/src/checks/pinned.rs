//! Group `pinned` — doctor.sh section 7: the sha256-pinned binary dependencies.
//!
//! Slugs owned here, in contract order:
//!
//! * `dep.dxmt` — all five `[dxmt] files` present, plus the `.sha256`
//!   provenance marker equal to `deps.dxmt_tgz_sha256` (marker missing/stale
//!   = WARN, files missing = FAIL)
//! * `dep.goldberg` — `third_party/gbe/steam_api64.dll` matches
//!   `deps.gbe_dll_sha256`; present-but-different is WARN, absent is FAIL
//!   (run tolerates a hash mismatch, dies only when the file is gone)
//!
//! Every evaluator is `fn(&CheckCtx) -> CheckOutcome`: a **read-only probe**.
//! Message and remedy strings must match `scripts/demo/doctor.sh` verbatim.

use super::Evaluator;
use super::{CheckCtx, CheckOutcome};
use crate::contract::contract;
use crate::util::{sha256_file, strip_trailing_newlines};

/// lib.sh's `dxmt_files_ok()`: every `[dxmt] files` entry present under
/// `ext/dxmt-artifacts/`.
fn dxmt_files_ok(ctx: &CheckCtx) -> bool {
    contract()
        .dxmt
        .files
        .iter()
        .all(|f| ctx.paths.dxmt_art.join(f).is_file())
}

/// The marker half of lib.sh's `dxmt_ok()`: `.sha256` equals the pinned
/// tarball hash. `$(cat file)` strips all trailing newlines before the
/// comparison, so the on-disk marker may or may not carry one.
fn dxmt_marker_current(ctx: &CheckCtx) -> bool {
    match std::fs::read_to_string(ctx.paths.dxmt_art.join(".sha256")) {
        Ok(text) => strip_trailing_newlines(&text) == contract().deps.dxmt_tgz_sha256,
        Err(_) => false,
    }
}

fn dep_dxmt(ctx: &CheckCtx) -> CheckOutcome {
    if !dxmt_files_ok(ctx) {
        return CheckOutcome::fail(
            "dep.dxmt",
            "ext/dxmt-artifacts missing or incomplete",
            "./demo.sh setup",
        );
    }
    if dxmt_marker_current(ctx) {
        CheckOutcome::pass(
            "dep.dxmt",
            "dxmt-artifacts (monofunc fork) present, provenance verified",
        )
    } else {
        CheckOutcome::warn(
            "dep.dxmt",
            "dxmt-artifacts present but provenance marker missing/stale — ./demo.sh setup re-fetches the pinned set",
        )
    }
}

fn dep_goldberg(ctx: &CheckCtx) -> CheckOutcome {
    let gbe = &ctx.paths.gbe_dll;
    let expected = &contract().deps.gbe_dll_sha256;
    // One hash pass: the warn detail reuses this digest rather than re-hashing a
    // multi-megabyte dll on every doctor/preflight run; no test catches the added cost.
    match sha256_file(gbe) {
        Ok(got) if got.eq_ignore_ascii_case(expected) => {
            CheckOutcome::pass("dep.goldberg", "Goldberg steam_api64.dll (sha256 verified)")
        }
        Ok(got) => CheckOutcome::warn(
            "dep.goldberg",
            "Goldberg dll present but hash differs from the pinned build",
        )
        .with_detail(format!("expected sha256 {expected}, got {got}")),
        Err(_) if !gbe.is_file() => {
            CheckOutcome::fail("dep.goldberg", "Goldberg dll missing", "./demo.sh setup")
        }
        Err(e) => CheckOutcome::warn(
            "dep.goldberg",
            "Goldberg dll present but hash differs from the pinned build",
        )
        .with_detail(format!(
            "expected sha256 {expected}, but could not re-hash: {e}"
        )),
    }
}

/// Evaluators this module binds, keyed by contract slug.
pub fn defs() -> Vec<(&'static str, Evaluator)> {
    vec![
        ("dep.dxmt", dep_dxmt as Evaluator),
        ("dep.goldberg", dep_goldberg as Evaluator),
    ]
}

#[cfg(test)]
mod tests;
