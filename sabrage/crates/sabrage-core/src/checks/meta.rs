//! Group `meta` — doctor.sh section 0: the generated shell contract mirror is in sync with contract/.
//!
//! Slugs owned here, in contract order:
//!
//! * `meta.contract-sync` — recompute the contract sha256 from `contract/` on
//!   disk and compare it against the `# contract-sha256:` header of
//!   `scripts/demo/contract.gen.sh` (`util::contract_hash` /
//!   `util::contract_gen_recorded_hash`), **and** (Sabrage-only, see below)
//!   against the contract this binary was compiled from.
//!
//! Every evaluator is `fn(&CheckCtx) -> CheckOutcome`: a **read-only probe**.
//! Message and remedy strings must match `scripts/demo/doctor.sh` verbatim —
//! this holds for the on-disk-vs-generated-header comparison, which is the
//! only half of this check the shell side can perform (doctor.sh has no
//! compiled-in contract to compare against). The compiled-vs-checkout half
//! below is Sabrage-only; its message/remedy prose has no shell counterpart
//! and is declared as an intentional divergence in `sabrage/PARITY.md`.

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
///
/// # A1-4: what this catches, and what it does not
///
/// `have` is the `# contract-sha256:` **header line**, never the generated
/// file's body — this evaluator (like doctor.sh's own `_have` capture) reads
/// one `sed -n 's/^# contract-sha256: //p'` line and nothing else. A
/// `contract.gen.sh` whose header is current but whose *body* was hand-edited
/// (or regenerated from a different `contract/` than the header names) is
/// therefore invisible to this check at runtime: `have == want` still holds,
/// because `want` is recomputed from `contract/` on disk and never reads
/// `contract.gen.sh`'s body either. That drift is caught only by tier-1's
/// `sabrage-contract-gen::generate() == include_str!("contract.gen.sh")`
/// test (`scripts/dev/parity.sh`, `.github/workflows/parity.yml`) — a
/// hand-edited body is a red CI run, not a red doctor row.
///
/// This on-disk comparison only verifies that `contract.gen.sh`'s header is
/// **fresh relative to the checkout** — it says nothing about whether *this
/// binary* was itself compiled from that same checkout. A Sabrage binary
/// embeds its contract at compile time ([`crate::contract`]'s `include_str!`s);
/// a build from an older/newer checkout than the one `repo_root` now points at
/// would pass this half of the check while still running stale check logic
/// against a contract it silently disagrees with. So, Sabrage-only (the shell
/// has no compiled-in contract to compare against — see `sabrage/PARITY.md`),
/// once the checkout is internally consistent this evaluator additionally
/// compares the checkout's hash against [`crate::contract::COMPILED_CONTRACT_SHA256`].
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

/// Sabrage-only compiled-vs-checkout identity guard, factored out of
/// [`meta_contract_sync`] so that callers *other* than Doctor can refuse to
/// act on a mismatch instead of merely reporting it. Round-1 finding A1-1: the
/// Doctor row for this existed, but Setup/Build/Install/Run dispatched
/// regardless of it — only the launch preflight (which runs this whole check
/// group) actually stopped anything. This packet's cross-area half
/// (`stages::run_stage` / `run_stage_holding_lock` calling this before
/// dispatch, owned by area A4) closes that gap; this function is the reusable
/// predicate those call sites need, kept here so its message/remedy strings
/// can never drift from the Doctor row's.
///
/// Returns `Err((message, remedy))` — never a `CheckOutcome`, so this has no
/// [`CheckCtx`] dependency and callers outside `checks::` don't need one just
/// to ask "is it safe to mutate?". Fails closed (`Err`) when `contract/`
/// itself can't be read/hashed under `root`, using the same message the
/// Doctor row would show in that case; a self-consistent checkout that
/// disagrees with [`crate::contract::COMPILED_CONTRACT_SHA256`] is the other
/// `Err` case. `Ok(())` only when `root`'s `contract/` hashes to exactly what
/// this binary was compiled from.
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

    /// Equal-hash case stays on the existing pass path — same assertions as
    /// `passes_against_the_live_checkout`, restated here so the "unchanged
    /// behaviour" half of the A1-1 regression test lives next to the mismatch
    /// case above rather than only implicitly relying on the older test.
    #[test]
    fn compiled_hash_matches_the_live_checkout() {
        let root = repo_root();
        let checkout_hash = util::contract_hash(&root).expect("contract files readable");
        assert_eq!(checkout_hash, *crate::contract::COMPILED_CONTRACT_SHA256);
    }

    /// [`assert_binary_matches_checkout`] is the reusable predicate area A4's
    /// `stages::run_stage` / `run_stage_holding_lock` call before dispatching a
    /// mutating stage (packet counterpart of A1-1). `Ok(())` against the live
    /// checkout, with the same message/remedy strings `meta_contract_sync`
    /// uses for its two `Err` shapes.
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
