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
//! Message and remedy strings must match `scripts/demo/doctor.sh` verbatim.

use super::Evaluator;
#[allow(unused_imports)]
use super::{CheckCtx, CheckOutcome, CheckStatus, SkipReason};

/// Evaluators this module binds, keyed by contract slug.
pub fn defs() -> Vec<(&'static str, Evaluator)> {
    // FILLED BY PHASE 1 EVALUATOR AGENT
    vec![]
}
