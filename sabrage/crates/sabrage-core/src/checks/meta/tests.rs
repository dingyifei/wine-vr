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
