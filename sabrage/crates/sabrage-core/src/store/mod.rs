//! Sabrage's own persistent store under `~/Library/Application Support/Sabrage/`:
//! GUI-only state, never a parity artifact and never read by the shell pipeline
//! (CLAUDE.md, "Sabrage ⇄ demo.sh parity").
//!
//! A missing file loads as the type's default, a corrupt or newer-schema file is a
//! hard [`crate::error::SabrageError`] rather than a silent reset, and every write
//! goes through the [`crate::executor::Executor`] so `--dry-run` plans instead of
//! mutating — the same convention as [`crate::session::state`]. Pinned by
//! settings::tests::a_corrupt_file_is_an_error_never_a_silent_reset,
//! library::tests::a_newer_schema_version_is_refused_not_silently_rewritten, and
//! settings::tests::a_dry_run_executor_plans_the_write_instead_of_performing_it.
//!
//! [`goldberg::revert_original_steam_dll`] refuses rather than claim to have restored
//! an "original" it cannot authenticate — Sabrage's one deliberate divergence from
//! `run.sh`'s Goldberg step (PARITY.md § Planned for later phases (declared now),
//! "Revert-original-`steam_api64.dll` action").

pub mod goldberg;
pub mod library;
pub mod settings;
