//! `sabrage-core`: the UI-agnostic native pipeline engine behind Sabrage.
//!
//! An independent implementation of the zsh pipeline (`demo.sh` +
//! `scripts/demo/*.sh`, which stays the reference). They meet at two places:
//!
//! 1. `contract/pipeline.toml` — pins, depot triple, port lists, DXMT artifact
//!    set, and ordered check/launch-action registries. Parsed directly here
//!    ([`contract`]); the shell reaches it via generated
//!    `scripts/demo/contract.gen.sh`. Registry order and slug coverage pinned by
//!    `checks::tests::registry_binds_in_contract_order_and_covers_every_slug`.
//! 2. Byte-shared on-disk artifacts — the host OpenXR manifest and the
//!    `oxrsys-runtime.toml` first-write template, rendered from
//!    `contract/*.template` by both sides ([`util`]). Manifest bytes pinned by
//!    `stages::install::tests::layer_four_stages_the_host_manifest_file_form_byte_for_byte`.
//!
//! Deliberate divergences live in `sabrage/PARITY.md`.
//!
//! Checks are read-only. Every mutation goes through [`executor::Executor`], so
//! `--dry-run` is the same code path with one implementation swapped rather than
//! a second, drifting one.

pub mod checks;
pub mod config;
pub mod contract;
pub mod error;
pub mod events;
pub mod executor;
pub mod fixes;
pub mod logs;
pub mod paths;
pub mod privilege;
pub mod process;
pub mod session;
pub mod stages;
pub mod store;
pub mod tap;
pub mod util;

pub use contract::{contract, Contract, Gate, CONTRACT};
pub use error::{Result, SabrageError};
pub use events::{step, Severity, Stage, StageEvent, StepId, Stream};
pub use executor::{
    dry_run_plan_body, BoxFuture, Copied, DetachedChild, DetachedStdio, DryRunExecutor, Executor,
    PlannedAction, RealExecutor, DRY_RUN_PLAN_EMPTY, DRY_RUN_PLAN_TITLE,
};
pub use fixes::{FixAction, FixDef, FixReport};
pub use logs::{LogBatch, LogSource, PastRun, Tailer};
pub use paths::{resolve_repo_root, Bottle, Paths};
pub use privilege::{AdminMethod, PrivilegedWrite};
pub use process::{capture, Captured, ChildSpec, ProcInfo};
pub use session::{
    live_session, EncoderInfo, LiveSessionHandle, SessionPhase, SessionStatus, LIVE_SESSION,
};
pub use stages::{
    null_sink, operation_in_progress, operation_in_progress_anywhere, require_bottle, run,
    run_stage, EventSink, StageCtx, StageOptions, StageOutcome, OPERATION_LOCK,
    RUN_WINESERVER_WAIT, STOP_WINESERVER_WAIT,
};
