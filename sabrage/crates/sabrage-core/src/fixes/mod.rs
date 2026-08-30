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
//! One contract fix id is deliberately **not** modelled yet — see
//! [`DEFERRED_CONTRACT_FIX_IDS`]. `from_contract_id` returns `None` for it
//! rather than pretending; a test in this module pins that the set of unmodelled
//! ids is exactly that one, so adding a fix to the contract without a variant
//! here fails the build's test run instead of silently disappearing.
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

/// Contract fix ids with no [`FixAction`] variant yet.
///
/// * `fix.create-z-drive` — creating `dosdevices/z:` in a bottle; only reachable
///   from `bottle.z-drive`, which no gate auto-fixes.
///
/// `fix.edit-protocol` left this list in Phase 4, when the comment-preserving
/// config editor it needed ([`crate::config::runtime_toml`], design-core §4.1)
/// landed with the Settings screen.
pub const DEFERRED_CONTRACT_FIX_IDS: [&str; 1] = ["fix.create-z-drive"];

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

    /// Parse a contract id (`"fix.restage-helper"`). `None` for an id this enum
    /// does not model — see [`DEFERRED_CONTRACT_FIX_IDS`].
    pub fn from_contract_id(id: &str) -> Option<FixAction> {
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
    /// game / runtime).
    pub forbidden_while_session_live: bool,
    /// Imperative one-liner for the button label.
    pub title: &'static str,
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
        },
        FixDef {
            action: FixAction::RestageHelper,
            needs_admin: false,
            destructive: false,
            forbidden_while_session_live: true,
            title: "restage the arm64 encoder helper",
        },
        FixDef {
            action: FixAction::RemoveAdbForwards,
            needs_admin: false,
            destructive: false,
            forbidden_while_session_live: true,
            title: "remove the stale adb port forwards",
        },
        FixDef {
            action: FixAction::DeleteSessionJson,
            needs_admin: false,
            destructive: true,
            forbidden_while_session_live: true,
            title: "delete ALVR's session.json (clears pinned client IPs)",
        },
        FixDef {
            action: FixAction::EditProtocol,
            needs_admin: false,
            destructive: false,
            forbidden_while_session_live: true,
            title: "set protocol = \"alvr\" in oxrsys-runtime.toml",
        },
        FixDef {
            action: FixAction::RunSetup,
            needs_admin: false,
            destructive: false,
            forbidden_while_session_live: true,
            title: "run setup",
        },
        FixDef {
            action: FixAction::RunBuild,
            needs_admin: false,
            destructive: false,
            forbidden_while_session_live: true,
            title: "run build",
        },
        FixDef {
            action: FixAction::RunInstall,
            needs_admin: true,
            destructive: false,
            forbidden_while_session_live: true,
            title: "run install",
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
pub async fn apply(action: FixAction, ctx: &StageCtx, sink: &EventSink) -> Result<FixReport> {
    let _guard = crate::stages::acquire_operation_lock().await;
    apply_holding_lock(action, ctx, sink).await
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
        let mut paths = Paths::new(&root);
        paths.oxr_appsup = root.join("OXRSys");
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
        let ctx = scratch_ctx("apply-blocks", None);
        let sink = null_sink();
        assert!(
            !ctx.paths.alvr_session_json().exists(),
            "fixture must not exist"
        );
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

    #[test]
    fn contract_ids_round_trip() {
        for a in FixAction::EVERY {
            let id = a.to_contract_id();
            assert!(id.starts_with(CONTRACT_FIX_PREFIX));
            assert_eq!(FixAction::from_contract_id(&id), Some(a));
            // The serde spelling is the bare id.
            assert_eq!(
                serde_json::to_string(&a).unwrap(),
                format!("\"{}\"", a.as_str())
            );
            // FromStr takes either spelling.
            assert_eq!(FixAction::from_str(a.as_str()).unwrap(), a);
            assert_eq!(FixAction::from_str(&id).unwrap(), a);
        }
        assert_eq!(FixAction::from_contract_id("set-graphics-backend"), None);
        assert!(FixAction::from_str("fix.nope").is_err());
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

    #[test]
    fn whole_stage_actions_map_to_their_stage() {
        assert_eq!(FixAction::RunSetup.as_stage(), Some(Stage::Setup));
        assert_eq!(FixAction::RunBuild.as_stage(), Some(Stage::Build));
        assert_eq!(FixAction::RunInstall.as_stage(), Some(Stage::Install));
        assert_eq!(FixAction::RestageHelper.as_stage(), None);
    }
}
