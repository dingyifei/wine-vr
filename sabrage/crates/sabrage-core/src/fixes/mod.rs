//! The fix registry: the small set of mutations offered as remedies.
//!
//! A doctor row never mutates anything ([`crate::checks`]); when its remedy is
//! mechanically applicable the contract names a fix id, the GUI turns it into a
//! button, and the launch preflight applies it for `autofix`-gated checks.
//!
//! [`FixAction`]'s serde spelling is the contract id without the `fix.` prefix,
//! which lives in one constant ([`CONTRACT_FIX_PREFIX`]).
//! [`DEFERRED_CONTRACT_FIX_IDS`] are contract ids this crate never offers;
//! tests::every_contract_fix_id_is_modelled_or_explicitly_deferred pins that
//! set exactly, so a new contract fix cannot disappear silently.
//!
//! Every action runs behind [`crate::stages::OPERATION_LOCK`] and mutates state
//! a live session depends on. [`apply`] takes the lock and refuses a live
//! session both before and after the wait, using persistent identity rather than
//! this process's handle
//! (tests::a_queued_fix_is_refused_when_a_session_goes_live_during_the_wait).
//! [`apply_holding_lock`] is for callers that already hold the lock:
//! `tokio::sync::Mutex` is not reentrant (silent deadlock). It skips the
//! liveness check because the launch preflight edits while a stale wineserver
//! is still alive.

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
/// * `fix.create-z-drive` - creating `dosdevices/z:` in a bottle, reachable
///   only from `bottle.zdrive`, which no gate auto-fixes; no [`FixAction`]
///   variant models it.
/// * `fix.delete-session-json` - modelled ([`FixAction::DeleteSessionJson`])
///   but withheld: deleting the file leaves the client at an 800x900 black
///   screen ([`crate::fixes::session_json`]), and the working recovery is to
///   edit the pinned IP in place, which the Settings screen's config editor
///   does. Returning `None` here keeps the destructive button off the Doctor
///   row; no front-end offers it today — the Tauri `fix` command refuses a
///   deferred action outright ([`FixAction::is_deferred`]). The variant stays
///   modelled so [`FixDef::consequence`] keeps the outcome on record.
pub const DEFERRED_CONTRACT_FIX_IDS: [&str; 2] = ["fix.create-z-drive", "fix.delete-session-json"];

/// A mutation the pipeline can apply on the user's behalf.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FixAction {
    /// Force `"CX_GRAPHICS_BACKEND" = "dxmt"` in the bottle's `cxbottle.conf`
    /// (`bottle.gfx-dxmt`). CrossOver's "auto" silently breaks DXMT.
    SetGraphicsBackend,
    /// Copy the arm64 encoder helper from `build-helper-arm64` next to the
    /// runtime dylib (`build.helper-staged` / `build.helper-arm64`).
    RestageHelper,
    /// `adb forward --remove tcp:9943` + `tcp:9944`, per serial — never
    /// `--remove-all` (`net.adb-forwards`).
    RemoveAdbForwards,
    /// Delete ALVR's `session.json` to clear stale manual IP pins.
    ///
    /// **Known-bad remedy.** Deleting the file leaves the client at an 800x900
    /// black screen; editing the pinned IPs in place is the working recovery.
    /// Modelled because `cfg.session-pins` names it, marked `destructive` so
    /// it never runs unconfirmed
    /// (tests::the_known_bad_session_json_deletion_documents_its_outcome), and
    /// withheld from every Doctor button by [`DEFERRED_CONTRACT_FIX_IDS`].
    DeleteSessionJson,
    /// Set `protocol = "alvr"` in `oxrsys-runtime.toml`
    /// (`cfg.protocol.supported` / `cfg.protocol.legacy-oxrsys`).
    ///
    /// The only fix that writes the runtime config; delegates to
    /// [`crate::config::runtime_toml::write`].
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
    /// The [`FixAction`]-shaped form of the withheld set: the Tauri `fix`
    /// command needs it to refuse an action the registry withholds from the
    /// GUI, however the frontend arrived at it (A4-2 - the TypeScript mirror
    /// of the fix table can offer a button [`FixAction::from_contract_id`]
    /// would not). Pinned by tests::is_deferred_is_exactly_the_withheld_set.
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
/// The public door for the CLI and the Tauri `fix` command: a fix serializes
/// against stages and other Sabrage processes' operations. `sink` is a separate
/// parameter so a run preflight can stream fix rows into the run's event channel
/// while carrying the run's own [`StageCtx`].
///
/// The row carrying `ctx.run_id` is emitted before the cancellable wait, and
/// refusals are re-run once the lock is in hand: a `run` that won the lock race
/// publishes its live session and hands the lock back at its launch boundary.
/// See tests::{a_queued_fix_carries_its_run_id_and_cancels_out_of_the_wait,
/// a_queued_fix_is_refused_when_a_session_goes_live_during_the_wait}.
///
/// **A caller that already holds the lock must call [`apply_holding_lock`]** -
/// `tokio::sync::Mutex` is not reentrant.
///
/// # Errors
///
/// [`SabrageError::Cancelled`] when the wait is cancelled; a fatal error when a
/// session is live or the checkout is not the one this binary was built from;
/// otherwise whatever the action itself fails with.
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
/// on it would say nothing about whether a session is running.
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

/// [`apply`] for a caller that already holds [`crate::stages::OPERATION_LOCK`] -
/// the shape a launch preflight needs, taking the lock once and auto-fixing what
/// its `autofix`-gated checks reported.
///
/// `tokio::sync::Mutex` is not reentrant, so whole-stage fixes delegate to
/// [`crate::stages::run_stage_holding_lock`] (not [`crate::stages::run_stage`],
/// which would deadlock); they still stream like a user-initiated stage. See
/// tests::apply_holding_lock_runs_a_stage_fix_under_a_lock_the_caller_holds.
///
/// The delegation is boxed so the future's size stays independent of the stage
/// layer and a fix->stage->fix cycle is a runtime recursion rather than an
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
mod tests;
