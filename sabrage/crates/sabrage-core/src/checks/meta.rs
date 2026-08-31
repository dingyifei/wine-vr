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
/// compares the checkout's hash against [`compiled_contract_hash`].
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
            "contract/ and scripts/demo/contract.gen.sh out of sync (contract edited without \
             regen, or the generated file was hand-edited)",
            "scripts/dev/parity.sh --regen",
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
    let compiled = compiled_contract_hash();
    if want != compiled {
        return CheckOutcome::fail(
            "meta.contract-sync",
            "this Sabrage binary was built from a different contract than the checkout it's \
             pointed at",
            "rebuild it from this checkout (cd sabrage && cargo build) or point Settings \u{203a} \
             Repository at the checkout it was built from",
        )
        .with_detail(format!("checkout={want} binary={compiled}"));
    }

    CheckOutcome::pass(
        "meta.contract-sync",
        "contract/ in sync with scripts/demo/contract.gen.sh",
    )
    .with_detail(format!("want={want} have={want}"))
}

/// The contract hash of the bytes THIS binary was compiled with — same recipe
/// as [`util::contract_hash`] (sha256 of the three contract files concatenated
/// in [`crate::contract::CONTRACT_FILES`] order), but over the `include_str!`
/// constants in [`crate::contract`] rather than files read at runtime. A
/// mismatch against the checkout's on-disk hash means this binary predates (or
/// postdates) a contract edit in the checkout `repo_root` points at.
fn compiled_contract_hash() -> String {
    let mut concatenated = String::with_capacity(
        crate::contract::PIPELINE_TOML.len()
            + crate::contract::RUNTIME_TOML_TEMPLATE.len()
            + crate::contract::HOST_MANIFEST_TEMPLATE.len(),
    );
    concatenated.push_str(crate::contract::PIPELINE_TOML);
    concatenated.push_str(crate::contract::RUNTIME_TOML_TEMPLATE);
    concatenated.push_str(crate::contract::HOST_MANIFEST_TEMPLATE);
    crate::util::sha256_bytes(concatenated.as_bytes())
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
    /// binary" case `compiled_contract_hash` exists to catch. Round-1 finding
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
                    compiled_contract_hash()
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
        assert_eq!(checkout_hash, compiled_contract_hash());
    }
}
