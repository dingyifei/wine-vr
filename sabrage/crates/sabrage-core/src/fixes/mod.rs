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
//! one *another* front-end (or `./demo.sh run`) started.
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
pub async fn apply(action: FixAction, ctx: &StageCtx, sink: &EventSink) -> Result<FixReport> {
    deny_if_session_live(action, ctx)?;
    let _guard = crate::stages::acquire_operation_lock().await;
    apply_holding_lock(action, ctx, sink).await
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

    /// `forbidden_while_session_live` used to be metadata nothing read: every
    /// fix was applicable mid-session, including the one that removes the
    /// `--wired` forwards the stream is running over. The registry drives the
    /// test, so a new fix cannot slip through unenforced.
    #[tokio::test]
    async fn apply_refuses_every_session_forbidden_fix_while_a_session_is_live() {
        let _g = crate::session::lock_session_globals();
        let ctx = scratch_ctx("live-refusal", Some("FixtureBottle"));
        record_a_live_session(&ctx);
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
        assert!(!ctx.paths.alvr_session_json().exists());

        let guard = acquire_operation_lock().await;
        let report = apply_holding_lock(FixAction::DeleteSessionJson, &ctx, &sink)
            .await
            .expect("the preflight door is not gated");
        assert!(!report.changed);
        drop(guard);

        std::fs::remove_dir_all(&ctx.paths.root).ok();
    }

    /// The wired-session case the refusal exists for: the fix must not reach
    /// `adb` at all, so the stream's own forwards survive.
    #[tokio::test]
    async fn a_live_session_stops_the_adb_forward_removal_before_it_spawns_adb() {
        let _g = crate::session::lock_session_globals();
        let mut ctx = scratch_ctx("live-adb", Some("FixtureBottle"));
        record_a_live_session(&ctx);

        // A fake adb that records every invocation; the refusal means the log
        // is never created.
        let adb = ctx.paths.root.join("adb.sh");
        std::fs::create_dir_all(&ctx.paths.root).unwrap();
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
        let err = apply(FixAction::RemoveAdbForwards, &ctx, &sink)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("while a session is live"), "{err}");
        assert!(
            !log.exists(),
            "adb must never be spawned while a session runs"
        );

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

    /// The known-bad `session.json` deletion is withheld from the GUI (no
    /// button) but keeps honest metadata for the CLI door and for any
    /// confirmation dialog that grows one.
    #[test]
    fn the_known_bad_session_json_deletion_is_withheld_but_documented() {
        assert_eq!(
            FixAction::from_contract_id("fix.delete-session-json"),
            None,
            "a destructive remedy known to black-screen the client must render no Fix button"
        );
        assert!(DEFERRED_CONTRACT_FIX_IDS.contains(&"fix.delete-session-json"));

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

    #[test]
    fn whole_stage_actions_map_to_their_stage() {
        assert_eq!(FixAction::RunSetup.as_stage(), Some(Stage::Setup));
        assert_eq!(FixAction::RunBuild.as_stage(), Some(Stage::Build));
        assert_eq!(FixAction::RunInstall.as_stage(), Some(Stage::Install));
        assert_eq!(FixAction::RestageHelper.as_stage(), None);
    }
}
