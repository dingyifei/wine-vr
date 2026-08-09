//! `sabrage-core` — the UI-agnostic native pipeline engine behind Sabrage.
//!
//! This crate is the Rust half of a two-implementation system. The zsh pipeline
//! (`demo.sh` + `scripts/demo/*.sh`) stays the reference; sabrage-core is an
//! independent implementation that meets it at exactly two places:
//!
//! 1. **`contract/pipeline.toml`** — pins, the depot triple, port lists, the DXMT
//!    artifact set, and the ordered check/launch-action registries. The shell
//!    consumes it through the GENERATED `scripts/demo/contract.gen.sh`; this
//!    crate parses it directly ([`contract`]).
//! 2. **Byte-shared on-disk artifacts** — the host OpenXR manifest and the
//!    `oxrsys-runtime.toml` first-write template, rendered from
//!    `contract/*.template` by both sides ([`util`]). install.sh does literal
//!    string equality on the manifest, so a single differing byte makes the two
//!    front-ends thrash each other with sudo prompts.
//!
//! Console text is *not* a shared contract — but check message and remedy
//! strings still track `scripts/demo/doctor.sh` verbatim, because
//! `docs/troubleshooting.md` quotes them.
//!
//! # Phase 0 (this frame)
//!
//! The API skeleton every later agent codes against: the contract types, the
//! typed lib.sh path port, the check registry and its outcome vocabulary, the
//! shell-idiom primitives, and the parity tap renderer. Check evaluators are
//! declared but unbound — see [`checks::build_registry`] for how that state is
//! made explicit rather than silent.
//!
//! # Module map
//!
//! * [`contract`] — the compiled-in `contract/` and its types
//! * [`paths`] — [`paths::Paths`] / [`paths::Bottle`], the typed lib.sh port
//! * [`checks`] — check registry, [`checks::CheckOutcome`], [`checks::run_doctor`]
//! * [`util`] — `cmp -s`, sha256, `win_path`, `bs_version`, template rendering,
//!   the `meta.contract-sync` hash recipe
//! * [`tap`] — the `"<slug> <status>"` parity channel
//! * [`error`] — [`error::SabrageError`] and demo.sh exit-code mapping

pub mod checks;
pub mod contract;
pub mod error;
pub mod paths;
pub mod tap;
pub mod util;

pub use contract::{contract, Contract, Gate, CONTRACT};
pub use error::{Result, SabrageError};
pub use paths::{Bottle, Paths};
