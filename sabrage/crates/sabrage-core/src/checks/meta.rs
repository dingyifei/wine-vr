//! Group `meta` — doctor.sh section 0: the generated shell contract mirror is in sync with contract/.
//!
//! Slugs owned here, in contract order:
//!
//! * `meta.contract-sync` — recompute the contract sha256 from `contract/` on
//!   disk and compare it against the `# contract-sha256:` header of
//!   `scripts/demo/contract.gen.sh` (`util::contract_hash` /
//!   `util::contract_gen_recorded_hash`)
//!
//! Every evaluator is `fn(&CheckCtx) -> CheckOutcome`: a **read-only probe**.
//! Message and remedy strings must match `scripts/demo/doctor.sh` verbatim.

use super::Evaluator;
#[allow(unused_imports)]
use super::{CheckCtx, CheckOutcome, CheckStatus, SkipReason};

use crate::util;

/// doctor.sh section 0:
/// ```sh
/// _want="$(cat "$ROOT/contract/pipeline.toml" \
///              "$ROOT/contract/oxrsys-runtime.toml.template" \
///              "$ROOT/contract/active_runtime.x86_64.json.template" \
///          2>/dev/null | shasum -a 256 | awk '{print $1}')"
/// _have="$(sed -n 's/^# contract-sha256: //p' "$ROOT/scripts/demo/contract.gen.sh" | head -1)"
/// if [ -n "$_have" ] && [ "$_want" = "$_have" ]; then chk ok …
/// else chk fail …
/// ```
///
/// `_have` empty (missing header, or the generated file missing entirely) is a
/// FAIL, same as a hash mismatch — the `[ -n "$_have" ]` guard is load-bearing.
fn meta_contract_sync(ctx: &CheckCtx) -> CheckOutcome {
    let root = &ctx.paths.root;
    let have = util::contract_gen_recorded_hash(root);
    let want = util::contract_hash(root).ok();

    let in_sync = matches!(
        (&have, &want),
        (Some(h), Some(w)) if !h.is_empty() && h == w
    );

    let outcome = if in_sync {
        CheckOutcome::pass(
            "meta.contract-sync",
            "contract/ in sync with scripts/demo/contract.gen.sh",
        )
    } else {
        CheckOutcome::fail(
            "meta.contract-sync",
            "contract/ and scripts/demo/contract.gen.sh out of sync (contract edited without \
             regen, or the generated file was hand-edited)",
            "scripts/dev/parity.sh --regen",
        )
    };

    // Sabrage-only explainability: the two hashes doctor.sh only compares,
    // never shows.
    match (want, have) {
        (Some(w), Some(h)) => outcome.with_detail(format!("want={w} have={h}")),
        (Some(w), None) => outcome.with_detail(format!("want={w} have=<no header>")),
        (None, _) => outcome.with_detail("could not recompute contract hash (unreadable file)"),
    }
}

/// Evaluators this module binds, keyed by contract slug.
pub fn defs() -> Vec<(&'static str, Evaluator)> {
    vec![("meta.contract-sync", meta_contract_sync as Evaluator)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::CheckOptions;
    use crate::paths::Paths;
    use std::path::PathBuf;

    /// The repo root, four levels above this crate's manifest — same recipe
    /// `util`'s own tests use.
    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .canonicalize()
            .expect("repo root resolves")
    }

    #[test]
    fn passes_against_the_live_checkout() {
        // The checked-in contract.gen.sh must already be in sync (util's own
        // test asserts the same thing at a lower level); this exercises the
        // full evaluator including the CheckOutcome shape.
        let ctx = CheckCtx::new(Paths::new(repo_root()), CheckOptions::new());
        let o = meta_contract_sync(&ctx);
        assert_eq!(o.status, CheckStatus::Pass);
        assert_eq!(
            o.message,
            "contract/ in sync with scripts/demo/contract.gen.sh"
        );
        assert!(o.remedy.is_none());
    }

    #[test]
    fn fails_closed_when_the_repo_root_is_wrong() {
        let ctx = CheckCtx::new(
            Paths::new("/nonexistent/sabrage-meta-probe"),
            CheckOptions::new(),
        );
        let o = meta_contract_sync(&ctx);
        assert_eq!(o.status, CheckStatus::Fail);
        assert_eq!(
            o.message,
            "contract/ and scripts/demo/contract.gen.sh out of sync (contract edited without \
             regen, or the generated file was hand-edited)"
        );
        assert_eq!(o.remedy.as_deref(), Some("scripts/dev/parity.sh --regen"));
    }
}
