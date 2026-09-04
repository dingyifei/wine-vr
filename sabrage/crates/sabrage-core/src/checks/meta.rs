//! Group `meta` — doctor.sh section 0: the generated shell contract mirror is in sync with contract/.
//!
//! Slugs owned here, in contract order:
//!
//! * `meta.contract-sync` — compares the sha256 recomputed from `contract/` on disk against
//!   the `# contract-sha256:` header of `scripts/demo/contract.gen.sh`
//!   (`util::contract_hash` / `util::contract_gen_recorded_hash`), and (Sabrage-only)
//!   against the contract this binary was compiled from.
//!
//! Every evaluator is `fn(&CheckCtx) -> CheckOutcome`: a read-only probe.
//! Message and remedy strings of the on-disk half must match `scripts/demo/doctor.sh`
//! verbatim; the compiled-vs-checkout half has no shell counterpart and is declared in
//! PARITY.md § Declared by the 2026-08-30 adversarial review (round 1 fixes),
//! "**Contract identity.**".

use super::Evaluator;
#[allow(unused_imports)]
use super::{CheckCtx, CheckOutcome, CheckStatus, SkipReason};

use crate::util;

/// Doctor row `meta.contract-sync`: Pass only when the sha256 recomputed from
/// `contract/` under `ctx.paths.root` equals both `contract.gen.sh`'s
/// `# contract-sha256:` header and this binary's compiled-in contract hash.
///
/// A missing or empty header is a Fail — doctor.sh's `[ -n "$_have" ]` guard;
/// only the missing case is pinned
/// (tests::fails_closed_when_the_repo_root_is_wrong). Neither side reads the
/// body of `contract.gen.sh`, so a hand-edited body under a current header
/// passes this row (A1-4); tier-1's `sabrage-contract-gen::generate() ==
/// include_str!` test catches that drift. The compiled-vs-checkout half is
/// Sabrage-only, finding A1-1
/// (tests::fails_when_the_binary_was_compiled_from_a_different_contract).
/// Reference: scripts/demo/doctor.sh section 0.
fn meta_contract_sync(ctx: &CheckCtx) -> CheckOutcome {
    let root = &ctx.paths.root;
    let have = util::contract_gen_recorded_hash(root);
    let want = util::contract_hash(root).ok();

    let in_sync = matches!(
        (&have, &want),
        (Some(h), Some(w)) if !h.is_empty() && h == w
    );

    if !in_sync {
        let outcome = CheckOutcome::fail(
            "meta.contract-sync",
            OUT_OF_SYNC_MESSAGE,
            OUT_OF_SYNC_REMEDY,
        );
        // Sabrage-only explainability: the two hashes doctor.sh only compares,
        // never shows.
        return match (want, have) {
            (Some(w), Some(h)) => outcome.with_detail(format!("want={w} have={h}")),
            (Some(w), None) => outcome.with_detail(format!("want={w} have=<no header>")),
            (None, _) => outcome.with_detail("could not recompute contract hash (unreadable file)"),
        };
    }

    // Checkout is internally consistent. Sabrage-only second half: is THIS
    // binary built from that same contract?
    let want = want.expect("in_sync requires want to be Some");
    let compiled = &*crate::contract::COMPILED_CONTRACT_SHA256;
    if &want != compiled {
        return CheckOutcome::fail(
            "meta.contract-sync",
            STALE_BINARY_MESSAGE,
            STALE_BINARY_REMEDY,
        )
        .with_detail(format!("checkout={want} binary={compiled}"));
    }

    CheckOutcome::pass(
        "meta.contract-sync",
        "contract/ in sync with scripts/demo/contract.gen.sh",
    )
    .with_detail(format!("want={want} have={want}"))
}

/// Message for the "checkout itself is out of sync" (or unreadable) case —
/// shared between the Doctor row and [`assert_binary_matches_checkout`] so a
/// mutating-stage refusal and the row that explains it always agree verbatim.
const OUT_OF_SYNC_MESSAGE: &str = "contract/ and scripts/demo/contract.gen.sh out of sync \
     (contract edited without regen, or the generated file was hand-edited)";
const OUT_OF_SYNC_REMEDY: &str = "scripts/dev/parity.sh --regen";

/// Message for the "checkout is self-consistent but this binary was compiled
/// from a different contract" case (round-1 finding A1-1).
const STALE_BINARY_MESSAGE: &str = "this Sabrage binary was built from a different contract \
     than the checkout it's pointed at";
const STALE_BINARY_REMEDY: &str = "rebuild it from this checkout (cd sabrage && cargo build) or \
     point Settings \u{203a} Repository at the checkout it was built from";

/// Sabrage-only compiled-vs-checkout identity guard, usable outside Doctor so
/// callers can refuse to act on a mismatch (round-1 finding A1-1).
///
/// Returns `Ok(())` only when `root`'s `contract/` hashes to exactly the
/// contract this binary was compiled from. Returns `(message, remedy)` rather
/// than `CheckOutcome` so callers outside `checks::` need no [`CheckCtx`];
/// the strings are the Doctor row's own, preventing drift.
///
/// # Errors
///
/// `Err((message, remedy))` when `contract/` under `root` cannot be read or
/// hashed (fails closed with the Doctor row's message), and when a
/// self-consistent checkout disagrees with
/// [`crate::contract::COMPILED_CONTRACT_SHA256`].
pub fn assert_binary_matches_checkout(root: &std::path::Path) -> Result<(), (String, String)> {
    let Some(want) = util::contract_hash(root).ok() else {
        return Err((
            OUT_OF_SYNC_MESSAGE.to_string(),
            OUT_OF_SYNC_REMEDY.to_string(),
        ));
    };
    let compiled = &*crate::contract::COMPILED_CONTRACT_SHA256;
    if &want != compiled {
        return Err((
            STALE_BINARY_MESSAGE.to_string(),
            STALE_BINARY_REMEDY.to_string(),
        ));
    }
    Ok(())
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

    /// The repo root: three levels above this crate's manifest directory.
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

    /// A checkout that is internally consistent (its `contract/` hashes to the
    /// same value `contract.gen.sh`'s header records) but whose contract bytes
    /// differ from the ones this test binary was compiled with — the "stale
    /// binary" case `contract::COMPILED_CONTRACT_SHA256` exists to catch. Round-1 finding
    /// A1-1.
    #[test]
    fn fails_when_the_binary_was_compiled_from_a_different_contract() {
        use crate::contract::{CONTRACT_FILES, CONTRACT_GEN_REL_PATH};

        let root = std::env::temp_dir().join(format!(
            "sabrage-meta-test-{}-stale-binary",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("contract")).expect("mkdir contract/");
        std::fs::create_dir_all(root.join("scripts/demo")).expect("mkdir scripts/demo/");

        // Deliberately not the real contract bytes this test binary was
        // compiled with — any content works, as long as the generated header
        // below is computed from these exact bytes (i.e. the checkout is
        // internally in sync with itself).
        for rel in CONTRACT_FILES {
            std::fs::write(
                root.join(rel),
                b"not-the-contract-this-binary-was-built-from\n",
            )
            .expect("write fake contract file");
        }
        let checkout_hash = util::contract_hash(&root).expect("just-written files are readable");
        std::fs::write(
            root.join(CONTRACT_GEN_REL_PATH),
            format!("# contract-sha256: {checkout_hash}\n"),
        )
        .expect("write fake contract.gen.sh");

        let ctx = CheckCtx::new(Paths::new(&root), CheckOptions::new());
        let o = meta_contract_sync(&ctx);

        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(o.status, CheckStatus::Fail);
        assert_eq!(
            o.message,
            "this Sabrage binary was built from a different contract than the checkout it's \
             pointed at"
        );
        assert_eq!(
            o.remedy.as_deref(),
            Some(
                "rebuild it from this checkout (cd sabrage && cargo build) or point Settings \
                 \u{203a} Repository at the checkout it was built from"
            )
        );
        assert_eq!(
            o.detail.as_deref(),
            Some(
                format!(
                    "checkout={checkout_hash} binary={}",
                    *crate::contract::COMPILED_CONTRACT_SHA256
                )
                .as_str()
            )
        );
    }

    /// [`assert_binary_matches_checkout`] returns `Ok(())` against the live
    /// checkout — the predicate behind `stages::deny_on_contract_skew`, run by
    /// every mutating door (`stages::run_stage`, `stages::run_stage_holding_lock`,
    /// `crate::fixes::apply`) before dispatch; Stop is ungated (round-1 finding A1-1).
    #[test]
    fn assert_binary_matches_checkout_passes_against_the_live_checkout() {
        assert!(assert_binary_matches_checkout(&repo_root()).is_ok());
    }

    #[test]
    fn assert_binary_matches_checkout_fails_closed_when_contract_is_unreadable() {
        let err =
            assert_binary_matches_checkout(std::path::Path::new("/nonexistent/sabrage-meta-probe"))
                .expect_err("unreadable contract/ must fail closed, not pass by omission");
        assert_eq!(err.0, OUT_OF_SYNC_MESSAGE);
        assert_eq!(err.1, OUT_OF_SYNC_REMEDY);
    }

    #[test]
    fn assert_binary_matches_checkout_fails_on_a_foreign_but_self_consistent_checkout() {
        use crate::contract::CONTRACT_FILES;

        let root = std::env::temp_dir().join(format!(
            "sabrage-meta-test-{}-assert-binary-matches",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("contract")).expect("mkdir contract/");
        for rel in CONTRACT_FILES {
            std::fs::write(
                root.join(rel),
                b"not-the-contract-this-binary-was-built-from\n",
            )
            .expect("write fake contract file");
        }

        let err = assert_binary_matches_checkout(&root)
            .expect_err("a self-consistent but foreign checkout must not be Ok");

        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(err.0, STALE_BINARY_MESSAGE);
        assert_eq!(err.1, STALE_BINARY_REMEDY);
    }
}
