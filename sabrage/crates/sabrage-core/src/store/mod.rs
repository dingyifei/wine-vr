//! Sabrage's own persistent store under `~/Library/Application Support/Sabrage/`
//! (Phase 4).
//!
//! demo.sh has no counterpart to any of this — every file here is GUI-only
//! state (CLAUDE.md, "Sabrage ⇄ demo.sh parity": *"GUI-only state lives under
//! `~/Library/Application Support/Sabrage/`"*), never a parity artifact, never
//! read by the shell pipeline. Each submodule follows the same read/write
//! convention as [`crate::session::state`] (its `session-state.json`): a
//! missing file loads as the type's default, a present-but-corrupt file is a
//! hard [`crate::error::SabrageError`] rather than a silent reset, and every
//! write goes through the [`crate::executor::Executor`] so `--dry-run` plans
//! instead of mutating.
//!
//! * [`settings`] — `settings.json`: repo root, default bottle/dir, global
//!   launch-flag defaults, the adb-probing and runtime-config-edit
//!   acknowledgement toggles (design-core §4.2).
//! * [`library`] — `library.json`: the game registry demo.sh never had
//!   (design-core §4.3) — [`library::GameEntry`]/[`library::Library`] CRUD
//!   (every writer through [`library::transact`], which holds one lock across
//!   the whole load→mutate→save), [`library::effective_options`]'s
//!   settings⊕override merge — the single home of that precedence rule,
//!   reached by id through [`library::Library::launch_options_for`] — and
//!   [`library::validate`]'s read-only install-health snapshot
//!   ([`library::GameValidity`]).
//! * [`goldberg`] — the restore-the-`.orig-steam`-backup action
//!   ([`goldberg::revert_original_steam_dll`]), Sabrage's one deliberate
//!   divergence from `run.sh`'s Goldberg step (`PARITY.md`). It refuses rather
//!   than claim to have restored an "original" it cannot authenticate.

pub mod goldberg;
pub mod library;
pub mod settings;
