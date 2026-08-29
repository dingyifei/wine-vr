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
//! # Layers
//!
//! Phase 1 shipped the read-only half: the contract types, the typed lib.sh path
//! port, the check registry, the shell-idiom primitives, and the parity tap.
//! Phase 2 adds the mutating half — the stage layer — around a single rule:
//! **every mutation goes through [`executor::Executor`]**, so `--dry-run` is the
//! same code path with one implementation swapped rather than a second, drifting
//! one.
//!
//! # Module map
//!
//! Read-only:
//! * [`contract`] — the compiled-in `contract/` and its types
//! * [`paths`] — [`paths::Paths`] / [`paths::Bottle`], the typed lib.sh port,
//!   and [`paths::resolve_repo_root`]
//! * [`checks`] — check registry, [`checks::CheckOutcome`], [`checks::run_doctor`]
//! * [`util`] — `cmp -s`, sha256, `win_path`, `bs_version`, the DXMT artifact
//!   predicates, template rendering, the `meta.contract-sync` hash recipe
//! * [`tap`] — the `"<slug> <status>"` parity channel
//!
//! Mutating:
//! * [`events`] — [`events::StageEvent`], [`events::Stage`], the step ids
//! * [`stages`] — [`stages::StageCtx`], the operation lock, [`stages::run_stage`]
//! * [`executor`] — every mutating primitive, real and dry-run
//! * [`process`] — child spawn/stream/cancel and the exec-path reap primitive
//! * [`fixes`] — the remedy actions doctor rows and the launch preflight offer
//! * [`privilege`] — the pipeline's one privileged write
//! * [`error`] — [`error::SabrageError`] and demo.sh exit-code mapping

pub mod checks;
pub mod contract;
pub mod error;
pub mod events;
pub mod executor;
pub mod fixes;
pub mod paths;
pub mod privilege;
pub mod process;
pub mod stages;
pub mod tap;
pub mod util;

pub use contract::{contract, Contract, Gate, CONTRACT};
pub use error::{Result, SabrageError};
pub use events::{step, Severity, Stage, StageEvent, StepId, Stream};
pub use executor::{
    dry_run_plan_body, BoxFuture, Copied, DryRunExecutor, Executor, PlannedAction, RealExecutor,
    DRY_RUN_PLAN_EMPTY, DRY_RUN_PLAN_TITLE,
};
pub use fixes::{FixAction, FixDef, FixReport};
pub use paths::{resolve_repo_root, Bottle, Paths};
pub use privilege::{AdminMethod, PrivilegedWrite};
pub use process::{ChildSpec, ProcInfo};
pub use stages::{
    null_sink, operation_in_progress, require_bottle, run_stage, EventSink, StageCtx, StageOptions,
    StageOutcome, OPERATION_LOCK,
};
