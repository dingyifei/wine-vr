//! The fix registry: the small set of *mutations offered as remedies*.
//!
//! A doctor row never mutates anything (see [`crate::checks`]); when its remedy
//! is mechanically applicable, the contract names a fix id and the GUI turns the
//! remedy into a button. The launch preflight applies the same fixes
//! automatically for the checks whose gate is `autofix`.
//!
//! # Contract ids vs. this enum
//!
//! `contract/pipeline.toml` spells fixes `"fix.set-graphics-backend"`; this enum
//! carries the id **without** the `fix.` prefix as its serde representation, so
//! the wire format is `"set-graphics-backend"` and
//! [`FixAction::to_contract_id`] / [`FixAction::from_contract_id`] bridge the
//! two. The prefix lives in exactly one constant ([`CONTRACT_FIX_PREFIX`]).
//!
//! Two contract fix ids are deliberately **not** offered — see
//! [`DEFERRED_CONTRACT_FIX_IDS`]. `from_contract_id` returns `None` for them
//! rather than pretending; a test in this module pins that the set of unoffered
//! ids is exactly those, so adding a fix to the contract without a variant here
//! fails the build's test run instead of silently disappearing.
//!
//! # Serialization guarantee
//!
//! Every action runs behind the same operation lock as a stage
//! ([`crate::stages::OPERATION_LOCK`]) and every one of them mutates state a
//! live session depends on, which is why `forbidden_while_session_live` is true
//! for all of them. [`apply`] takes that lock; [`apply_holding_lock`] is the
//! same dispatch for a caller that already holds it (a launch preflight), since
//! `tokio::sync::Mutex` is not reentrant and taking it twice on one task
//! deadlocks in silence.
//!
//! # `forbidden_while_session_live` is enforced, not advertised
//!
//! [`apply`] — the GUI/CLI door — refuses every such action while
//! [`crate::stages::live_session_block`] can see a session, using persistent
//! identity (`session-state.json`, `runtime_status.json`) rather than only this
//! process's in-memory handle: the session a Doctor button would break is often
//! one *another* front-end (or `./demo.sh run`) started. It refuses **twice** —
//! before the operation-lock wait and again with the lock held — because a
//! `run` that won the lock race publishes its session and then releases the
//! lock at its launch boundary.
//!
//! [`apply_holding_lock`] deliberately does **not** check: it is the launch
//! preflight's door, reached from inside `run` before the live handle is
//! published, and one of the fixes it applies
//! ([`crate::fixes::backend::set_graphics_backend_for_launch`]) exists precisely
//! to edit while a stale wineserver is still alive.

pub mod adb;
pub mod backend;
pub mod helper;
pub mod session_json;

use serde::{Deserialize, Serialize};

use crate::error::{Result, SabrageError};
use crate::events::{Stage, StageEvent};
use crate::stages::{EventSink, StageCtx};

/// The prefix the contract puts in front of every fix id.
pub const CONTRACT_FIX_PREFIX: &str = "fix.";

/// Contract fix ids this crate does not offer as a button, sorted.
///
/// * `fix.create-z-drive` — creating `dosdevices/z:` in a bottle; only reachable
///   from `bottle.z-drive`, which no gate auto-fixes. No [`FixAction`] variant
///   yet.
/// * `fix.delete-session-json` — modelled ([`FixAction::DeleteSessionJson`]) but
///   **withheld**: the remedy is known to leave the client at an 800x900 black
///   screen ([`crate::fixes::session_json`]), and the working recovery is to
///   edit the pinned IP in place, which the Settings screen's config editor now
///   does. Returning `None` here is what keeps the destructive button off the
///   Doctor row; the action itself stays reachable from `sabrage fix
///   delete-session-json` for a user who has read [`FixDef::consequence`].
///
/// `fix.edit-protocol` left this list in Phase 4, when the comment-preserving
/// config editor it needed ([`crate::config::runtime_toml`], design-core §4.1)
/// landed with the Settings screen.
pub const DEFERRED_CONTRACT_FIX_IDS: [&str; 2] = ["fix.create-z-drive", "fix.delete-session-json"];

/// A mutation the pipeline can apply on the user's behalf.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FixAction {
    /// Force `"CX_GRAPHICS_BACKEND" = "dxmt"` in the bottle's `cxbottle.conf`
    /// (`bottle.graphics-backend`). CrossOver's "auto" silently breaks DXMT.
    SetGraphicsBackend,
    /// Copy the arm64 encoder helper from `build-helper-arm64` next to the
    /// runtime dylib (`build.helper-staged` / `build.helper-arm64`).
    RestageHelper,
    /// `adb forward --remove tcp:9943` + `tcp:9944`, per serial — never
    /// `--remove-all` (`net.adb-forwards`).
    RemoveAdbForwards,
    /// Delete ALVR's `session.json` to clear stale manual IP pins.
    ///
    /// **Known-bad remedy.** Deleting the file has been observed to leave the
    /// client at an 800x900 black screen; editing the pinned IPs in place is the
    /// working recovery. Kept because the contract's `cfg.session-pins` row
    /// still names it, marked `destructive` so it can never run unconfirmed, and
    /// superseded once the in-place config editor lands.
    DeleteSessionJson,
    /// Set `protocol = "alvr"` in `oxrsys-runtime.toml`
    /// (`cfg.protocol.supported` / `cfg.protocol.legacy-oxrsys`).
    ///
    /// The one fix that writes the runtime config. It goes through
    /// [`crate::config::runtime_toml::write`], so it inherits that module's
    /// rules: create-if-absent from the shared template, an in-place value edit
    /// that moves no other byte, and a rolling backup under Sabrage's own
    /// `backups/`.
    EditProtocol,
    /// Run the whole `setup` stage.
    RunSetup,
    /// Run the whole `build` stage.
    RunBuild,
    /// Run the whole `install` stage (the one action that can prompt for
    /// administrator authorization).
    RunInstall,
}

impl FixAction {
    /// Every action, in registry order.
    pub const EVERY: [FixAction; 8] = [
        FixAction::SetGraphicsBackend,
        FixAction::RestageHelper,
        FixAction::RemoveAdbForwards,
        FixAction::DeleteSessionJson,
        FixAction::EditProtocol,
        FixAction::RunSetup,
        FixAction::RunBuild,
        FixAction::RunInstall,
    ];

    /// The contract id **without** the `fix.` prefix — also the serde spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            FixAction::SetGraphicsBackend => "set-graphics-backend",
            FixAction::RestageHelper => "restage-helper",
            FixAction::RemoveAdbForwards => "remove-adb-forwards",
            FixAction::DeleteSessionJson => "delete-session-json",
            FixAction::EditProtocol => "edit-protocol",
            FixAction::RunSetup => "run-setup",
            FixAction::RunBuild => "run-build",
            FixAction::RunInstall => "run-install",
        }
    }

    /// The full contract id, e.g. `"fix.set-graphics-backend"`.
    pub fn to_contract_id(self) -> String {
        format!("{CONTRACT_FIX_PREFIX}{}", self.as_str())
    }

    /// [`to_contract_id`](Self::to_contract_id) as a `&'static str`, for the
    /// rows this fix emits when it runs *as a fix* rather than as a step of a
    /// stage ([`crate::events::StepId`] is `&'static str`, so a `String` id
    /// cannot be used).
    ///
    /// `step_ids_are_the_contract_ids` pins the two spellings equal.
    pub fn step_id(self) -> crate::events::StepId {
        match self {
            FixAction::SetGraphicsBackend => "fix.set-graphics-backend",
            FixAction::RestageHelper => "fix.restage-helper",
            FixAction::RemoveAdbForwards => "fix.remove-adb-forwards",
            FixAction::DeleteSessionJson => "fix.delete-session-json",
            FixAction::EditProtocol => "fix.edit-protocol",
            FixAction::RunSetup => "fix.run-setup",
            FixAction::RunBuild => "fix.run-build",
            FixAction::RunInstall => "fix.run-install",
        }
    }

    /// Is this action's contract id one of the deliberately withheld ones
    /// ([`DEFERRED_CONTRACT_FIX_IDS`])?
    ///
    /// The named form of `DEFERRED_CONTRACT_FIX_IDS.contains(&id)` for callers
    /// that hold a [`FixAction`] rather than a contract id — the Tauri `fix`
    /// command needs exactly this to refuse an action the registry withholds
    /// from the GUI, however the frontend arrived at it (A4-2: the TypeScript
    /// mirror of the fix table can offer a button
    /// [`FixAction::from_contract_id`] would not).
    pub fn is_deferred(self) -> bool {
        DEFERRED_CONTRACT_FIX_IDS.contains(&self.to_contract_id().as_str())
    }

    /// Parse a contract id (`"fix.restage-helper"`). `None` for an id this enum
    /// does not model **and** for one it models but does not offer — see
    /// [`DEFERRED_CONTRACT_FIX_IDS`]. The GUI renders a Fix button only where
    /// this returns `Some`, so a withheld remedy has no button anywhere.
    pub fn from_contract_id(id: &str) -> Option<FixAction> {
        if DEFERRED_CONTRACT_FIX_IDS.contains(&id) {
            return None;
        }
        let bare = id.strip_prefix(CONTRACT_FIX_PREFIX)?;
        FixAction::EVERY.into_iter().find(|a| a.as_str() == bare)
    }

    /// The stage this action delegates to, for the three whole-stage fixes.
    pub fn as_stage(self) -> Option<Stage> {
        match self {
            FixAction::RunSetup => Some(Stage::Setup),
            FixAction::RunBuild => Some(Stage::Build),
            FixAction::RunInstall => Some(Stage::Install),
            _ => None,
        }
    }

    /// This action's registry entry.
    pub fn def(self) -> &'static FixDef {
        fix_defs()
            .iter()
            .find(|d| d.action == self)
            .expect("every FixAction has a FixDef")
    }
}

impl std::fmt::Display for FixAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for FixAction {
    type Err = SabrageError;

    /// Accepts both spellings — `"restage-helper"` and `"fix.restage-helper"` —
    /// so `sabrage fix <id>` works with whichever form the user copied.
    fn from_str(s: &str) -> Result<FixAction> {
        let bare = s.strip_prefix(CONTRACT_FIX_PREFIX).unwrap_or(s);
        FixAction::EVERY
            .into_iter()
            .find(|a| a.as_str() == bare)
            .ok_or_else(|| SabrageError::InvalidInput(format!("unknown fix '{s}'")))
    }
}

/// Static metadata about one fix: what the UI must know before offering it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixDef {
    pub action: FixAction,
    /// Can prompt for administrator authorization (install layer 4 only).
    pub needs_admin: bool,
    /// Deletes or overwrites user state — the GUI must confirm first.
    pub destructive: bool,
    /// Must not run while a session is live (`SessionMonitor` reports a running
    /// game / runtime). Enforced by [`apply`], not merely advertised.
    pub forbidden_while_session_live: bool,
    /// Imperative one-liner for the button label.
    pub title: &'static str,
    /// What the user must know *before* confirming, for a remedy whose known
    /// outcome is worse than the row it fixes.
    ///
    /// `None` for every ordinary fix — the title says enough. `Some` is a
    /// sentence a confirmation dialog shows **instead of** a generic "this
    /// cannot be undone", so the disclosure happens before the mutation rather
    /// than in the report after it.
    pub consequence: Option<&'static str>,
}

/// The fix registry.
pub fn fix_defs() -> &'static [FixDef] {
    const DEFS: &[FixDef] = &[
        FixDef {
            action: FixAction::SetGraphicsBackend,
            needs_admin: false,
            destructive: false,
            forbidden_while_session_live: true,
            title: "force the bottle's graphics backend to dxmt",
            consequence: None,
        },
        FixDef {
            action: FixAction::RestageHelper,
            needs_admin: false,
            destructive: false,
            forbidden_while_session_live: true,
            title: "restage the arm64 encoder helper",
            consequence: None,
        },
        FixDef {
            action: FixAction::RemoveAdbForwards,
            needs_admin: false,
            destructive: false,
            forbidden_while_session_live: true,
            title: "remove the stale adb port forwards",
            consequence: None,
        },
        FixDef {
            action: FixAction::DeleteSessionJson,
            needs_admin: false,
            destructive: true,
            forbidden_while_session_live: true,
            title: "delete ALVR's session.json (clears pinned client IPs)",
            consequence: Some(
                "Known-bad remedy: deleting this file has been observed to leave the client at \
                 an 800x900 black screen. The file is copied to Application \
                 Support/Sabrage/backups first, and editing the pinned IP in place is the \
                 recovery that works.",
            ),
        },
        FixDef {
            action: FixAction::EditProtocol,
            needs_admin: false,
            destructive: false,
            forbidden_while_session_live: true,
            title: "set protocol = \"alvr\" in oxrsys-runtime.toml",
            consequence: None,
        },
        FixDef {
            action: FixAction::RunSetup,
            needs_admin: false,
            destructive: false,
            forbidden_while_session_live: true,
            title: "run setup",
            consequence: None,
        },
        FixDef {
            action: FixAction::RunBuild,
            needs_admin: false,
            destructive: false,
            forbidden_while_session_live: true,
            title: "run build",
            consequence: None,
        },
        FixDef {
            action: FixAction::RunInstall,
            needs_admin: true,
            destructive: false,
            forbidden_while_session_live: true,
            title: "run install",
            consequence: None,
        },
    ];
    DEFS
}

/// What a fix did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FixReport {
    pub action: FixAction,
    /// False when the fix found nothing to do (already current, nothing stale) —
    /// the preflight then emits no [`StageEvent::AutoFixed`].
    pub changed: bool,
    /// Human-facing summary; becomes `AutoFixed.description`.
    pub description: String,
}

impl FixReport {
    /// A fix that changed something.
    pub fn changed(action: FixAction, description: impl Into<String>) -> FixReport {
        FixReport {
            action,
            changed: true,
            description: description.into(),
        }
    }

    /// A fix that found nothing to do.
    pub fn unchanged(action: FixAction, description: impl Into<String>) -> FixReport {
        FixReport {
            action,
            changed: false,
            description: description.into(),
        }
    }
}

/// Apply one fix, taking [`crate::stages::OPERATION_LOCK`] for its duration.
///
/// The public entry point for the CLI and the Tauri `fix` command: a Doctor
/// "Fix" button must not rewrite `cxbottle.conf` or restage the helper while a
/// build or install is halfway through, and a whole-stage fix must serialize
/// against a user-initiated stage exactly as one stage serializes against
/// another.
///
/// `sink` is normally `&ctx.sink`; it is a separate parameter so a fix invoked
/// from a *run* preflight can stream its rows into the run's event channel
/// while carrying the run's own [`StageCtx`].
///
/// **A caller that already holds the lock must call [`apply_holding_lock`]
/// instead** — `tokio::sync::Mutex` is not reentrant.
///
/// # Waiting is visible, cancellable, and re-judged
///
/// Mirrors [`crate::stages::run_stage`] exactly, and for the same reasons:
///
/// * a row carrying `ctx.run_id` is emitted **before** the wait, because that
///   id is the front-end's only handle on the operation — a fix queued behind
///   another Sabrage process's build otherwise had nothing to name and no way
///   to be cancelled, while still mutating the machine when its turn came;
/// * the wait itself goes through
///   [`crate::stages::acquire_operation_lock_cancellable`], so Cancel ends it as
///   [`SabrageError::Cancelled`] with nothing touched;
/// * the refusals are re-run **after** the lock is in hand. A `run` that won the
///   lock race publishes its live session and then gives the lock back at its
///   launch boundary, so a fix admitted while the machine was idle could
///   otherwise remove the very `--wired` forwards the stream is running over.
pub async fn apply(action: FixAction, ctx: &StageCtx, sink: &EventSink) -> Result<FixReport> {
    deny_before_apply(action, ctx)?;
    sink(StageEvent::info(
        ctx.run_id,
        Some(action.step_id()),
        format!("applying fix '{action}'"),
    ));
    if crate::stages::operation_in_progress_anywhere() {
        sink(StageEvent::info(
            ctx.run_id,
            Some(action.step_id()),
            "waiting for another Sabrage operation to finish",
        ));
    }
    let Some(_guard) = crate::stages::acquire_operation_lock_cancellable(&ctx.cancel).await else {
        return Err(SabrageError::Cancelled);
    };
    deny_before_apply(action, ctx)?;
    apply_holding_lock(action, ctx, sink).await
}

/// Every refusal the GUI/CLI fix door owes before it may mutate: a live
/// session, then contract skew.
///
/// One function because [`apply`] applies it twice — before the operation-lock
/// wait and again with the lock held (see [`apply`]).
fn deny_before_apply(action: FixAction, ctx: &StageCtx) -> Result<()> {
    deny_if_session_live(action, ctx)?;
    // Same guard every mutating stage takes: a fix run by a binary built from
    // another checkout writes that binary's ports, pins and templates.
    crate::stages::deny_on_contract_skew(ctx)
}

/// Enforce [`FixDef::forbidden_while_session_live`] for the GUI/CLI door.
///
/// Checked **before** the lock, because the operation lock is deliberately free
/// for the whole of a live session (`stages`' "Lock policy for `run`"): waiting
/// on it would tell us nothing about whether a session is running.
///
/// TODO: call `session::ensure_idle` here once it lands — the liveness policy
/// belongs to the session layer; [`crate::stages::live_session_block`] is the
/// same rule, parked next to the operation lock it complements.
fn deny_if_session_live(action: FixAction, ctx: &StageCtx) -> Result<()> {
    if !action.def().forbidden_while_session_live {
        return Ok(());
    }
    match crate::stages::live_session_block(&ctx.paths) {
        None => Ok(()),
        Some(reason) => Err(ctx.fatal(
            format!(
                "refusing to apply '{action}' while a session is live — {reason}; stop the \
                 session first"
            ),
            Some(format!(
                "./demo.sh stop --bottle {}",
                ctx.opts.bottle_name.as_deref().unwrap_or("<name>")
            )),
        )),
    }
}

/// [`apply`] for a caller that already holds [`crate::stages::OPERATION_LOCK`].
///
/// This is the shape a launch preflight needs: it takes the lock once for the
/// whole launch and then auto-fixes whatever its `autofix`-gated checks
/// reported. Whole-stage fixes therefore delegate to
/// [`crate::stages::run_stage_holding_lock`] rather than
/// [`crate::stages::run_stage`], which would deadlock on the second acquire.
///
/// Whole-stage fixes (`RunSetup`/`RunBuild`/`RunInstall`) still stream exactly
/// like a user-initiated stage — no second, quieter code path. The delegation is
/// boxed: it keeps the future's size independent of the stage layer and makes
/// any future fix→stage→fix cycle a runtime recursion rather than an
/// unsized-future compile error.
pub async fn apply_holding_lock(
    action: FixAction,
    ctx: &StageCtx,
    sink: &EventSink,
) -> Result<FixReport> {
    if let Some(stage) = action.as_stage() {
        let outcome = Box::pin(crate::stages::run_stage_holding_lock(stage, ctx)).await?;
        return Ok(FixReport::changed(
            action,
            format!("{} stage completed", outcome.stage),
        ));
    }
    match action {
        FixAction::SetGraphicsBackend => backend::set_graphics_backend(ctx, sink).await,
        FixAction::RestageHelper => helper::restage_helper(ctx, sink).await,
        FixAction::RemoveAdbForwards => adb::remove_adb_forwards(ctx, sink).await,
        FixAction::DeleteSessionJson => session_json::delete_session_json(ctx, sink).await,
        FixAction::EditProtocol => crate::config::runtime_toml::edit_protocol(ctx, sink).await,
        // Unreachable: as_stage() handled these above.
        FixAction::RunSetup | FixAction::RunBuild | FixAction::RunInstall => {
            unreachable!("whole-stage fixes are dispatched by as_stage()")
        }
    }
}

/// Emit the [`StageEvent::AutoFixed`] row for a report that changed something.
///
/// Shared by the preflight and by any GUI-initiated fix, so the event shape is
/// produced in exactly one place.
pub fn emit_auto_fixed(ctx: &StageCtx, sink: &EventSink, step: &str, report: &FixReport) {
    if report.changed {
        sink(StageEvent::AutoFixed {
            run_id: ctx.run_id,
            step: step.to_string(),
            fix: report.action,
            description: report.description.clone(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::contract;
    use crate::paths::Paths;
    use crate::stages::{acquire_operation_lock, null_sink, StageCtx, StageOptions};
    use std::str::FromStr;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    /// A context pointed at a scratch root, with `oxr_appsup` redirected so
    /// `alvr_session_json()` can never resolve to the real
    /// `~/Library/Application Support/OXRSys/alvr/session.json`. Nothing is
    /// created: the file's absence is what makes `DeleteSessionJson` a fast,
    /// side-effect-free no-op, which is all these lock tests need it to be.
    fn scratch_ctx(tag: &str, bottle: Option<&str>) -> StageCtx {
        let root =
            std::env::temp_dir().join(format!("sabrage-fixes-lock-{tag}-{}", std::process::id()));
        // The scratch root is a checkout as far as `apply` is concerned, and
        // `apply` refuses one this binary was not built from — so give it this
        // binary's own contract.
        crate::stages::materialize_compiled_contract(&root);
        let mut paths = Paths::new(&root);
        paths.oxr_appsup = root.join("OXRSys");
        // …and `sabrage_appsup` too, so the live-session policy reads a scratch
        // `session-state.json` rather than the real machine's.
        paths.sabrage_appsup = root.join("Sabrage");
        let opts = StageOptions {
            bottle_name: bottle.map(str::to_string),
            ..StageOptions::default()
        };
        StageCtx::new(paths, opts, null_sink(), CancellationToken::new())
    }

    /// `apply` is the public entry point, so it must serialize against a stage:
    /// a Doctor "Fix" button cannot be allowed to rewrite `cxbottle.conf` or
    /// restage the helper while an install is halfway through. Before this, the
    /// four non-stage actions took no lock at all.
    #[tokio::test]
    async fn apply_waits_for_the_operation_lock_then_proceeds() {
        // `apply` now also consults the live-session policy, which reads
        // process-global slots a sibling test may publish into.
        let _g = crate::session::lock_session_globals();
        let ctx = scratch_ctx("apply-blocks", None);
        let sink = null_sink();
        let guard = acquire_operation_lock().await;
        let mut task =
            tokio::spawn(async move { apply(FixAction::DeleteSessionJson, &ctx, &sink).await });

        assert!(
            tokio::time::timeout(Duration::from_millis(250), &mut task)
                .await
                .is_err(),
            "apply did not wait for the lock"
        );
        drop(guard);
        let report = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("apply must proceed once the lock is free")
            .expect("task not panicked")
            .expect("an absent session.json is a no-op, not an error");
        assert!(!report.changed);
        assert_eq!(report.action, FixAction::DeleteSessionJson);
    }

    /// `tokio::sync::Mutex` is not reentrant: a preflight that holds the lock
    /// and auto-fixes with a whole-stage action would deadlock in silence on
    /// `apply`. `apply_holding_lock` routes the stage through
    /// [`crate::stages::run_stage_holding_lock`] instead.
    ///
    /// `RunInstall` on a context naming a bottle that does not exist aborts in
    /// `require_bottle`, the first line of the stage — far enough in to prove
    /// the dispatch happened, and short of anything that mutates.
    #[tokio::test]
    async fn apply_holding_lock_runs_a_stage_fix_under_a_lock_the_caller_holds() {
        let ctx = scratch_ctx("holding", Some("NoSuchBottleSabrageTest"));
        let sink = null_sink();
        let guard = acquire_operation_lock().await;

        let err = tokio::time::timeout(
            Duration::from_secs(5),
            apply_holding_lock(FixAction::RunInstall, &ctx, &sink),
        )
        .await
        .expect("apply_holding_lock must not wait on the lock its caller holds")
        .expect_err("the bottle does not exist");
        assert!(
            err.to_string()
                .starts_with("bottle 'NoSuchBottleSabrageTest' not found at "),
            "{err}"
        );

        drop(guard);
    }

    /// Write a `session-state.json` under `ctx`'s (scratch) Sabrage store whose
    /// recorded wine identity is **this** process: alive, reporting the start
    /// time recorded for it — what `session::reconcile::classify` calls `Live`.
    fn record_a_live_session(ctx: &StageCtx) {
        let path = ctx.paths.session_state_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut state = crate::session::state::SessionState::new(
            uuid::Uuid::new_v4(),
            "FixtureBottle",
            "/bs",
            "/log",
            0,
        );
        state.wine = crate::process::ProcInfo::observe(std::process::id());
        assert!(state.wine.is_some(), "this process must be observable");
        std::fs::write(&path, serde_json::to_string(&state).unwrap()).unwrap();
    }

    /// r1:A4-1 regression: `forbidden_while_session_live` used to be metadata
    /// nothing read: every fix was applicable mid-session, including the one
    /// that removes the `--wired` forwards the stream is running over. The
    /// registry drives the test, so a new fix cannot slip through unenforced,
    /// and the recording fake `adb` proves the refusal lands before the fix
    /// ever spawns `adb`.
    #[tokio::test]
    async fn apply_refuses_every_session_forbidden_fix_while_a_session_is_live() {
        let _g = crate::session::lock_session_globals();
        let mut ctx = scratch_ctx("live-refusal", Some("FixtureBottle"));
        record_a_live_session(&ctx);

        // A fake adb that records every invocation; the refusal means the log
        // is never created.
        let adb = ctx.paths.root.join("adb.sh");
        let log = ctx.paths.root.join("adb-invoked.log");
        std::fs::write(
            &adb,
            format!("#!/bin/sh\necho \"$@\" >> {}\nexit 0\n", log.display()),
        )
        .unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&adb).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&adb, perms).unwrap();
        }
        ctx.paths.adb = Some(adb);

        let sink = null_sink();

        for action in FixAction::EVERY {
            assert!(
                action.def().forbidden_while_session_live,
                "{action} is not marked forbidden — this test needs updating"
            );
            let err = apply(action, &ctx, &sink)
                .await
                .expect_err("a live session must refuse every fix");
            let msg = err.to_string();
            assert!(
                msg.starts_with(&format!(
                    "refusing to apply '{action}' while a session is live"
                )),
                "{msg}"
            );
            assert!(msg.contains("FixtureBottle"), "{msg}");
        }

        assert!(
            !log.exists(),
            "adb must never be spawned while a session runs"
        );

        std::fs::remove_dir_all(&ctx.paths.root).ok();
    }

    /// The launch preflight's door keeps working on the same state: `run`
    /// applies auto-fixes *before* it publishes a live session, and one of them
    /// (`set_graphics_backend_for_launch`) exists to edit while a stale
    /// wineserver is alive. Gating there would make a relaunch impossible.
    #[tokio::test]
    async fn apply_holding_lock_is_not_gated_by_a_live_session() {
        let _g = crate::session::lock_session_globals();
        let ctx = scratch_ctx("live-preflight", None);
        record_a_live_session(&ctx);
        let sink = null_sink();

        let guard = acquire_operation_lock().await;
        let report = apply_holding_lock(FixAction::DeleteSessionJson, &ctx, &sink)
            .await
            .expect("the preflight door is not gated");
        assert!(!report.changed);
        drop(guard);

        std::fs::remove_dir_all(&ctx.paths.root).ok();
    }

    #[test]
    fn contract_ids_round_trip() {
        for a in FixAction::EVERY {
            let id = a.to_contract_id();
            assert!(id.starts_with(CONTRACT_FIX_PREFIX));
            // A withheld remedy is the one case where the round trip stops: no
            // `from_contract_id`, therefore no button anywhere in the GUI.
            let expected = if DEFERRED_CONTRACT_FIX_IDS.contains(&id.as_str()) {
                None
            } else {
                Some(a)
            };
            assert_eq!(FixAction::from_contract_id(&id), expected);
            // The serde spelling is the bare id.
            assert_eq!(
                serde_json::to_string(&a).unwrap(),
                format!("\"{}\"", a.as_str())
            );
            // FromStr takes either spelling — including for a withheld action,
            // which stays reachable from `sabrage fix <id>`.
            assert_eq!(FixAction::from_str(a.as_str()).unwrap(), a);
            assert_eq!(FixAction::from_str(&id).unwrap(), a);
        }
        assert_eq!(FixAction::from_contract_id("set-graphics-backend"), None);
        assert!(FixAction::from_str("fix.nope").is_err());
    }

    /// r1:A12-1 regression: the withheld `session.json` deletion must state its
    /// known outcome before anything can run it — `destructive`, plus a
    /// `consequence` naming the 800x900 black screen, the backup it takes
    /// first and the in-place recovery. The no-button half of the withholding
    /// is pinned by `is_deferred_is_exactly_the_withheld_set`.
    #[test]
    fn the_known_bad_session_json_deletion_documents_its_outcome() {
        let def = FixAction::DeleteSessionJson.def();
        assert!(def.destructive);
        let consequence = def
            .consequence
            .expect("the known outcome must be stated before the mutation, not after it");
        assert!(consequence.contains("800x900"), "{consequence}");
        assert!(consequence.contains("backups"), "{consequence}");
        assert!(consequence.contains("in place"), "{consequence}");

        // Every other fix says nothing extra: `consequence` is for remedies
        // whose outcome is worse than the row they fix, not a description field.
        for a in FixAction::EVERY {
            assert_eq!(
                a.def().consequence.is_some(),
                a == FixAction::DeleteSessionJson,
                "{a} carries an unexpected consequence string"
            );
        }
    }

    #[test]
    fn every_contract_fix_id_is_modelled_or_explicitly_deferred() {
        let mut unmodelled: Vec<&str> = contract()
            .checks
            .iter()
            .filter_map(|c| c.fix.as_deref())
            .filter(|id| FixAction::from_contract_id(id).is_none())
            .collect();
        unmodelled.sort_unstable();
        unmodelled.dedup();
        assert_eq!(
            unmodelled, DEFERRED_CONTRACT_FIX_IDS,
            "a contract fix id is neither modelled as a FixAction nor listed in \
             DEFERRED_CONTRACT_FIX_IDS"
        );
    }

    #[test]
    fn every_autofix_gate_maps_to_a_modelled_action() {
        use crate::contract::Gate;
        for check in &contract().checks {
            if check.native_gate == Gate::Autofix || check.shell_gate == Gate::Autofix {
                let id = check.fix.as_deref().expect("autofix gate declares a fix");
                assert!(
                    FixAction::from_contract_id(id).is_some(),
                    "{}: autofix gate names unmodelled fix {id}",
                    check.slug
                );
            }
        }
    }

    #[test]
    fn registry_covers_every_action_exactly_once() {
        assert_eq!(fix_defs().len(), FixAction::EVERY.len());
        for a in FixAction::EVERY {
            assert_eq!(a.def().action, a);
        }
        // Only install can prompt for admin (design-core §5: exactly one
        // privileged write in the whole pipeline).
        let admin: Vec<FixAction> = fix_defs()
            .iter()
            .filter(|d| d.needs_admin)
            .map(|d| d.action)
            .collect();
        assert_eq!(admin, vec![FixAction::RunInstall]);
        // Only the session.json deletion destroys user state.
        let destructive: Vec<FixAction> = fix_defs()
            .iter()
            .filter(|d| d.destructive)
            .map(|d| d.action)
            .collect();
        assert_eq!(destructive, vec![FixAction::DeleteSessionJson]);
        assert!(fix_defs().iter().all(|d| d.forbidden_while_session_live));
    }

    /// The refusal is re-run with the operation lock in hand.
    ///
    /// The window: a Doctor fix admitted while the machine was idle waits
    /// behind another operation; a `run` acquires first, publishes its live
    /// session and hands the lock back at its launch boundary. The fix used to
    /// then mutate — for `remove-adb-forwards`, by deleting the very forwards
    /// the `--wired` stream was running over.
    #[tokio::test]
    async fn a_queued_fix_is_refused_when_a_session_goes_live_during_the_wait() {
        let _g = crate::session::lock_session_globals();
        let ctx = scratch_ctx("queued-live", Some("FixtureBottle"));
        let oxr = ctx.paths.oxr_appsup.clone();
        let root = ctx.paths.root.clone();
        assert_eq!(
            crate::stages::live_session_block(&ctx.paths),
            None,
            "the fixture must be idle at admission"
        );

        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let s = seen.clone();
        let sink: EventSink = std::sync::Arc::new(move |ev| s.lock().unwrap().push(ev));

        let held = acquire_operation_lock().await;
        let task = {
            let sink = sink.clone();
            tokio::spawn(async move { apply(FixAction::RestageHelper, &ctx, &sink).await })
        };

        // Admitted: the announcement row proves it got past the pre-lock door.
        let announced = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let has = seen.lock().unwrap().iter().any(|ev| {
                    matches!(ev, StageEvent::Line { text, .. } if text.contains("applying fix"))
                });
                if has {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await;
        assert!(announced.is_ok(), "a queued fix must announce itself");

        // A session starts while it waits — the `run` that won the lock race.
        // Fresh *and* naming a live pid (this process): both halves are what
        // `session::watcher::runtime_status_live` requires.
        std::fs::create_dir_all(&oxr).unwrap();
        let now = crate::session::now_unix_ms();
        let pid = std::process::id();
        std::fs::write(
            oxr.join("runtime_status.json"),
            format!(r#"{{"state":"streaming","process_id":{pid},"updated_at_unix_ms":{now}}}"#),
        )
        .unwrap();
        drop(held);

        let err = tokio::time::timeout(Duration::from_secs(10), task)
            .await
            .expect("the queued fix must settle once the lock is free")
            .expect("the fix task must not panic")
            .expect_err("a fix that acquires the lock under a live session is refused");
        assert!(
            err.to_string()
                .starts_with("refusing to apply 'restage-helper' while a session is live"),
            "{err}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// The wait is named and cancellable: the first event carries `ctx.run_id`
    /// (the front-end's only handle on the operation) and Cancel ends the wait
    /// without applying anything.
    #[tokio::test]
    async fn a_queued_fix_carries_its_run_id_and_cancels_out_of_the_wait() {
        let _g = crate::session::lock_session_globals();
        let ctx = scratch_ctx("queued-cancel", None);
        let root = ctx.paths.root.clone();
        let run_id = ctx.run_id;
        let cancel = ctx.cancel.clone();

        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let s = seen.clone();
        let sink: EventSink = std::sync::Arc::new(move |ev| s.lock().unwrap().push(ev));

        let held = acquire_operation_lock().await;
        let task = {
            let sink = sink.clone();
            tokio::spawn(async move { apply(FixAction::DeleteSessionJson, &ctx, &sink).await })
        };

        let announced = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let found = seen.lock().unwrap().iter().any(|ev| {
                    matches!(ev, StageEvent::Line { run_id: r, step, text, .. }
                        if *r == run_id
                            && step.as_deref() == Some("fix.delete-session-json")
                            && text.contains("applying fix"))
                });
                if found {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await;
        assert!(
            announced.is_ok(),
            "the run id must be emitted before the wait: {:?}",
            seen.lock().unwrap()
        );
        assert!(seen.lock().unwrap().iter().any(|ev| matches!(
            ev,
            StageEvent::Line { text, .. } if text.contains("waiting for another Sabrage operation")
        )));

        cancel.cancel();
        let err = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("cancelling must end the wait")
            .expect("the fix task must not panic")
            .expect_err("a cancelled fix does not report a FixReport");
        assert!(matches!(err, SabrageError::Cancelled), "{err:?}");
        assert_eq!(err.exit_code(), 130);

        drop(held);
        std::fs::remove_dir_all(&root).ok();
    }

    /// A binary pointed at a checkout it was not built from must not mutate
    /// through the fix door either — it would write its *own* ports, pins and
    /// templates into someone else's tree.
    #[tokio::test]
    async fn apply_refuses_a_checkout_the_binary_was_not_built_from() {
        let _g = crate::session::lock_session_globals();
        let ctx = scratch_ctx("contract-skew", Some("FixtureBottle"));
        // Overwrite the materialized contract with foreign bytes.
        for rel in crate::contract::CONTRACT_FILES {
            std::fs::write(
                ctx.paths.root.join(rel),
                b"not-the-contract-this-binary-was-built-from\n",
            )
            .unwrap();
        }
        let sink = null_sink();
        // The refusal is `checks::meta`'s wording — the same sentence the
        // `meta.contract-sync` doctor row shows.
        let (expected, _) = crate::checks::meta::assert_binary_matches_checkout(&ctx.paths.root)
            .expect_err("the fixture must be a skewed checkout");

        for action in FixAction::EVERY {
            let err = apply(action, &ctx, &sink)
                .await
                .expect_err("a foreign checkout must refuse every fix");
            assert_eq!(err.to_string(), expected, "{action}");
        }

        std::fs::remove_dir_all(&ctx.paths.root).ok();
    }

    /// The step a fix's own rows are attributed to is its contract id — the
    /// `&'static str` form of it, since [`crate::events::StepId`] cannot be a
    /// `String`.
    #[test]
    fn step_ids_are_the_contract_ids() {
        for a in FixAction::EVERY {
            assert_eq!(a.step_id(), a.to_contract_id());
        }
        // The one module that hard-codes its own copy agrees.
        assert_eq!(
            FixAction::RemoveAdbForwards.step_id(),
            "fix.remove-adb-forwards"
        );
    }

    /// r1:A4-2 regression: a known-broken destructive remedy renders no Fix button.
    /// `is_deferred` is the [`FixAction`]-shaped form of the withheld set: the
    /// Tauri `fix` command needs it to refuse an action the GUI should never
    /// have offered (its TypeScript mirror of the fix table can render a button
    /// `from_contract_id` would not).
    #[test]
    fn is_deferred_is_exactly_the_withheld_set() {
        let deferred: Vec<String> = FixAction::EVERY
            .into_iter()
            .filter(|a| a.is_deferred())
            .map(|a| a.to_contract_id())
            .collect();
        assert_eq!(deferred, vec!["fix.delete-session-json".to_string()]);
        for a in FixAction::EVERY {
            assert_eq!(
                a.is_deferred(),
                FixAction::from_contract_id(&a.to_contract_id()).is_none(),
                "{a}: is_deferred must agree with the no-button rule"
            );
        }
    }

    #[test]
    fn whole_stage_actions_map_to_their_stage() {
        assert_eq!(FixAction::RunSetup.as_stage(), Some(Stage::Setup));
        assert_eq!(FixAction::RunBuild.as_stage(), Some(Stage::Build));
        assert_eq!(FixAction::RunInstall.as_stage(), Some(Stage::Install));
        assert_eq!(FixAction::RestageHelper.as_stage(), None);
    }
}
