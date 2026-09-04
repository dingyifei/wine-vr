//! Parity tests between the native pipeline (`sabrage-core` / `sabrage-contract-gen`)
//! and the zsh reference implementation (`demo.sh`, `scripts/demo/*.sh`,
//! `contract/`).
//!
//! This crate carries **no runtime surface** — see `Cargo.toml`: every
//! dependency is a dev-dependency, and every test in `tests` is a tier-1 hermetic
//! `cargo test` per `docs/design/design-parity.md` §4 ("always-on pure tests,
//! no env gate, no machine state" beyond reading the repo tree the crate is
//! built from). Tier 2 (the live doctor diff) and the pre-push hook are
//! `scripts/dev/parity.sh`'s job, not this crate's.
//!
//! Every test reads its shell/contract inputs from the **working checkout on
//! disk** via [`tests::repo_root`], never from a compiled-in copy — with one
//! deliberate exception: the contract-gen parity test also compiles in its
//! own `include_str!` of the committed `scripts/demo/contract.gen.sh`, so the
//! compiled generator is compared against the checked-in bytes (the one place
//! those bytes are pinned). Everywhere else the point of this crate is to
//! catch "the checkout and the generated/compiled artifact disagree," which
//! comparing two compiled-in copies of the same `include_str!` would defeat
//! by construction.

#[cfg(test)]
mod tests;
