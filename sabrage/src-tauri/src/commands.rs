//! Tauri commands for the Doctor screen, the pipeline stage runner, fixes, and
//! the sidebar's app-state footer.
//!
//! Bridges `sabrage-core`'s synchronous, read-only check engine
//! ([`sabrage_core::checks::run_doctor`]) and its mutating stage/fix layer
//! ([`sabrage_core::stages`], [`sabrage_core::fixes`]) to the frontend:
//!
//! * `run_doctor` streams one [`DoctorEvent`] per resolved `CheckOutcome` over
//!   an IPC [`Channel`] and resolves to the aggregate [`DoctorSummary`].
//! * `run_stage`, `fix`, and `stop_session` stream `sabrage_core::StageEvent`
//!   straight over the channel — it is already the wire shape design-core
//!   §3.1 specifies (internally tagged on `kind`, camelCase fields), so there
//!   is no second event type to keep in sync — and resolve once the
//!   stage/fix settles.
//! * `cancel_stage` interrupts an in-flight `run_stage`/`stop_session` by the
//!   `runId` its first `StageStarted` event carried.
//! * `get_app_state` is the small always-fresh snapshot the sidebar footer
//!   renders.
//!
//! `ui/src/ipc.ts` hand-mirrors every serde shape here 1:1 — keep both sides in
//! sync when either changes.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use sabrage_core::checks::{run_doctor as core_run_doctor, CheckCtx, CheckOptions, CheckStatus};
use sabrage_core::{
    contract, fixes, resolve_repo_root, EventSink, FixAction, FixReport, Paths, SabrageError,
    Stage, StageCtx, StageEvent, StageOptions, StageOutcome,
};
use serde::{Deserialize, Serialize};
use tauri::{ipc::Channel, State};

/// The ALVR client version this wine-vr checkout is pinned to (`CLAUDE.md`'s
/// submodule table). Not read from anywhere at runtime — like
/// `contract/pipeline.toml`'s baked-in pins, it is a fact about this checkout,
/// not machine state.
const ALVR_VERSION: &str = "v20.14.1";

// ── doctor ────────────────────────────────────────────────────────────────────

/// One streamed doctor row: a `CheckOutcome` plus the `group` the contract
/// attaches to its `slug`, and — when the contract names one — the `fix` id
/// (`CheckOutcome` itself carries neither; see `checks/mod.rs`'s doc comment
/// on the group → module mapping, and `fixes/mod.rs` for the id vocabulary).
/// `fix` is the bare contract id (`"fix.set-graphics-backend"`); the frontend
/// maps it to a [`FixAction`] wire value itself (`ipc.ts`'s
/// `contractFixIdToAction`, mirroring [`FixAction::from_contract_id`]) rather
/// than have this command do it, so the two deferred contract ids
/// (`fix.create-z-drive`, `fix.edit-protocol`) are a client-side "no button"
/// decision instead of a silently-dropped field.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorEvent {
    pub slug: String,
    pub group: String,
    pub status: CheckStatus,
    pub message: String,
    pub remedy: Option<String>,
    pub detail: Option<String>,
    pub fix: Option<String>,
}

/// The aggregate a `run_doctor` invocation resolves to, over the same
/// doctor-row set streamed on the channel (i.e. `run-only` excluded).
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorSummary {
    pub fail_count: usize,
    pub warn_count: usize,
    pub total: usize,
}

/// Sidebar footer snapshot.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppState {
    pub repo_root: Option<String>,
    pub bottles: Vec<String>,
    pub alvr_version: String,
}

/// Run every doctor check in contract order, streaming each resolved row to
/// `on_event` as it settles, and return the aggregate.
///
/// Runs on a blocking task: every evaluator is a synchronous, read-only probe
/// (a stat, a small read, a digest, or a short subprocess —
/// [`sabrage_core::checks::Evaluator`]'s own doc comment), several of which
/// shell out (`adb devices`, `SwitchAudioSource`), so this must not run
/// directly on an async-runtime worker.
#[tauri::command]
pub async fn run_doctor(
    bottle: Option<String>,
    bs_dir: Option<String>,
    on_event: Channel<DoctorEvent>,
) -> Result<DoctorSummary, String> {
    let repo_root = match resolve_repo_root(None) {
        Ok(p) => p,
        Err(err) => {
            // Surface the failure on the channel too, so a listener that only
            // watches the stream (not the invoke() rejection) still learns why
            // zero rows arrived.
            let message = err.to_string();
            let remedy = err.remedy().map(|s| s.to_string());
            let _ = on_event.send(DoctorEvent {
                slug: "meta.repo-root".to_string(),
                group: "meta".to_string(),
                status: CheckStatus::Fail,
                message: message.clone(),
                remedy,
                detail: None,
                fix: None,
            });
            return Err(message);
        }
    };

    // WINEVR_* env is the base (parity with the CLI and demo.sh precedence);
    // explicit GUI args override.
    let mut opts = CheckOptions::from_env();
    if let Some(b) = bottle {
        opts.bottle_name = Some(b);
    }
    if let Some(d) = bs_dir {
        opts.bs_dir_override = Some(PathBuf::from(d));
    }

    tauri::async_runtime::spawn_blocking(move || {
        let ctx = CheckCtx::new(Paths::new(&repo_root), opts);
        let mut fail_count = 0usize;
        let mut warn_count = 0usize;
        let mut total = 0usize;
        // run-only filtering lives in sabrage-core's run_doctor (the one policy
        // site) — every outcome that reaches this sink is a doctor row.
        core_run_doctor(&ctx, |outcome| {
            let spec = contract().check(&outcome.slug);
            let group = spec.map(|s| s.group.as_str()).unwrap_or("").to_string();
            let fix = spec.and_then(|s| s.fix.clone());
            total += 1;
            if outcome.status.counts_as_fail() {
                fail_count += 1;
            }
            if outcome.status.counts_as_warn() {
                warn_count += 1;
            }
            let _ = on_event.send(DoctorEvent {
                slug: outcome.slug,
                group,
                status: outcome.status,
                message: outcome.message,
                remedy: outcome.remedy,
                detail: outcome.detail,
                fix,
            });
        });
        DoctorSummary {
            fail_count,
            warn_count,
            total,
        }
    })
    .await
    .map_err(|e| format!("doctor check task did not complete: {e}"))
}

/// Sidebar footer snapshot: repo root (if resolvable), bottles present on this
/// machine, and the pinned ALVR client version.
#[tauri::command]
pub fn get_app_state() -> AppState {
    AppState {
        repo_root: resolve_repo_root(None)
            .ok()
            .map(|p| p.display().to_string()),
        bottles: sabrage_core::paths::list_bottles(),
        alvr_version: ALVR_VERSION.to_string(),
    }
}

// ── pipeline stages + fixes ──────────────────────────────────────────────────
//
// # Why no `tokio_util` / `uuid` appear here
//
// [`StageCtx::new`] wants a `tokio_util::sync::CancellationToken`, and its
// `run_id` is a `uuid::Uuid` — neither crate is a direct dependency of
// `sabrage-app` (only the Frame agent may edit a `Cargo.toml`; see this
// crate's task brief). Both are reached without ever naming them:
//
// * `Default::default()` builds the `CancellationToken` `StageCtx::new`
//   wants — the expected parameter type is enough for inference to pick the
//   right `Default` impl without a `use` for the crate that defines it.
//   `CancellationToken::cancel(&self)` is then called as an ordinary inherent
//   method, which Rust resolves on any value regardless of whether its
//   concrete type is nameable in the caller's module (inherent methods need
//   no `use`, only trait methods do).
// * The run id is threaded as its `.to_string()` form everywhere on this
//   side ([`RunRegistry`]'s key — the same bytes `StageEvent`'s `runId` field
//   serializes as), so `Uuid` itself is never named.
//
// [`RunRegistry`] therefore stores an opaque `Box<dyn Fn() + Send + Sync>`
// canceller per run rather than the concrete token.

/// Options shared by [`run_stage`] and [`stop_session`] — the stage-facing
/// slice of the `WINEVR_*` mirror ([`StageOptions`]) plus Sabrage's own
/// dry-run toggle.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageRunOpts {
    pub bottle: Option<String>,
    pub bs_dir: Option<String>,
    pub dry_run: Option<bool>,
}

/// Options for [`fix`] — no dry-run: a fix either changes something or it
/// doesn't ([`FixReport::changed`]).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FixRunOpts {
    pub bottle: Option<String>,
    pub bs_dir: Option<String>,
}

/// Tracks one cancellation handle per in-flight [`run_stage`]/[`stop_session`]
/// invocation, keyed by the run's id in its string form. Managed Tauri state
/// — see `lib.rs`'s `.manage(...)`.
#[derive(Default)]
pub struct RunRegistry {
    runs: Mutex<HashMap<String, Box<dyn Fn() + Send + Sync>>>,
}

impl RunRegistry {
    fn register(&self, run_id: String, canceller: Box<dyn Fn() + Send + Sync>) {
        self.runs
            .lock()
            .expect("RunRegistry mutex poisoned")
            .insert(run_id, canceller);
    }

    fn forget(&self, run_id: &str) {
        self.runs
            .lock()
            .expect("RunRegistry mutex poisoned")
            .remove(run_id);
    }

    /// Cancel the run named `run_id`, if it is still tracked. The entry is
    /// removed either way a cancel is attempted, so a doubled Cancel click
    /// (or a click after the run already finished) is a harmless no-op
    /// returning `false`.
    fn cancel(&self, run_id: &str) -> bool {
        match self
            .runs
            .lock()
            .expect("RunRegistry mutex poisoned")
            .remove(run_id)
        {
            Some(canceller) => {
                canceller();
                true
            }
            None => false,
        }
    }
}

/// Wrap a `Channel` as an [`EventSink`] — every stage/fix event forwarded
/// verbatim. A send failure (the webview navigated away, the channel closed)
/// is dropped: the stage keeps running either way, exactly as a dropped
/// `DoctorEvent` send is already handled above.
fn channel_sink(channel: Channel<StageEvent>) -> EventSink {
    Arc::new(move |ev: StageEvent| {
        let _ = channel.send(ev);
    })
}

/// Emit a well-formed `StageStarted`/`Fatal`/`StageFinished` bracket for a
/// failure that happens *before* a [`StageCtx`] can exist (repo-root
/// resolution). Keeps the invariant every stage's event stream relies on
/// (`stages::run_stage`'s own doc comment): a listener that only watches
/// events never sees a run that started and never ended.
fn emit_early_failure(on_event: &Channel<StageEvent>, stage: Stage, err: &SabrageError) {
    let run_id = Default::default();
    let _ = on_event.send(StageEvent::StageStarted { run_id, stage });
    let _ = on_event.send(StageEvent::Fatal {
        run_id,
        message: err.to_string(),
        remedy: err.remedy().map(|s| s.to_string()),
        fix: None,
    });
    let _ = on_event.send(StageEvent::StageFinished {
        run_id,
        stage,
        ok: false,
        exit_code_equiv: err.exit_code(),
    });
}

/// Merge GUI-supplied stage options onto the `WINEVR_*` environment — the
/// same precedence [`run_doctor`] already gives [`CheckOptions`] ("WINEVR_*
/// env is the base ... explicit GUI args override"): start from
/// [`StageOptions::from_env`], then let `bottle`/`bs_dir` override field-by-
/// field only when the caller actually supplied one. `dry_run` is the one
/// field this does not set — [`execute_stage`] and [`fix`] each decide it
/// themselves afterward, since a fix is never a dry run regardless of the
/// environment.
///
/// Before this existed, `execute_stage`/`fix` built a bare
/// `StageOptions { bottle_name: opts.bottle, .. }` from scratch, so a GUI
/// invocation with no bottle selected (`bottle: null`) silently ignored a
/// `WINEVR_BOTTLE` set in Sabrage's own environment — the CLI's `cmd_stage`
/// never had this bug (it already called `StageOptions::from_env()` first).
fn stage_options_from_env_and_gui(bottle: Option<String>, bs_dir: Option<String>) -> StageOptions {
    let mut opts = StageOptions::from_env();
    if let Some(b) = bottle {
        opts.bottle_name = Some(b);
    }
    if let Some(d) = bs_dir {
        opts.bs_dir_override = Some(PathBuf::from(d));
    }
    opts
}

/// Shared body of [`run_stage`] and [`stop_session`]: resolve the repo root,
/// build a [`StageCtx`], register its cancellation handle, run the stage, and
/// unregister on the way out (success or failure alike).
async fn execute_stage(
    stage: Stage,
    opts: StageRunOpts,
    on_event: Channel<StageEvent>,
    registry: &RunRegistry,
) -> Result<StageOutcome, String> {
    let repo_root = match resolve_repo_root(None) {
        Ok(p) => p,
        Err(err) => {
            emit_early_failure(&on_event, stage, &err);
            return Err(err.to_string());
        }
    };
    let mut stage_opts = stage_options_from_env_and_gui(opts.bottle, opts.bs_dir);
    stage_opts.dry_run = opts.dry_run.unwrap_or(false);
    let sink = channel_sink(on_event);
    let ctx = StageCtx::new(Paths::new(repo_root), stage_opts, sink, Default::default());
    let run_id = ctx.run_id.to_string();
    let cancel_handle = ctx.cancel.clone();
    registry.register(run_id.clone(), Box::new(move || cancel_handle.cancel()));
    let result = sabrage_core::run_stage(stage, &ctx).await;
    emit_dry_run_plan(&ctx);
    registry.forget(&run_id);
    result.map_err(|e| e.to_string())
}

/// Trailing "plan (dry run)" rows, so the GUI's Dry-run button delivers the
/// thing a plan exists for: which copies would happen and which would be
/// skipped because the bytes already match — a distinction the narrative rows
/// do not draw. Before this, `planned()` never left the backend and a GUI dry
/// run looked exactly like a real one.
///
/// Emitted as a [`StageEvent::Section`] plus one `info` row per action, using
/// `sabrage-core`'s shared [`sabrage_core::dry_run_plan_body`] — the same text
/// the CLI prints under its own `-- plan (dry run)` header, so the two
/// front-ends say the same thing word for word.
///
/// Runs on the failure path too (it is called before `result` is inspected),
/// matching the CLI, which prints the section after a `FATAL` as well —
/// `(nothing planned)` when the stage died before its first mutating step.
/// Keyed on `executor.is_dry_run()` rather than `opts.dry_run`, the
/// source-of-truth precedent the rest of the crate follows, so a real run's
/// event stream is untouched. These rows land after `StageFinished`, which is
/// exactly where the CLI prints them.
fn emit_dry_run_plan(ctx: &StageCtx) {
    if !ctx.executor.is_dry_run() {
        return;
    }
    ctx.section(sabrage_core::DRY_RUN_PLAN_TITLE);
    for line in sabrage_core::dry_run_plan_body(&ctx.executor.planned()) {
        ctx.info(line);
    }
}

/// Run one pipeline stage (`setup`/`build`/`install`/`stop`; `run` is Phase 3
/// and currently resolves to a `Fatal` — see `stages::dispatch`), streaming
/// every [`StageEvent`] to `on_event` as it happens.
///
/// The returned promise does not resolve until the stage finishes (or fails);
/// callers drive their UI off the event stream (in particular
/// `StageFinished`) and treat the resolved/rejected promise as a secondary
/// confirmation — the same shape `run_doctor` already uses for its channel
/// plus return value.
#[tauri::command]
pub async fn run_stage(
    stage: Stage,
    opts: StageRunOpts,
    on_event: Channel<StageEvent>,
    registry: State<'_, RunRegistry>,
) -> Result<StageOutcome, String> {
    execute_stage(stage, opts, on_event, &registry).await
}

/// Cancel the run named `run_id` (read off that run's first `StageStarted`
/// event), if it is still in flight. Returns `false` when no such run is
/// tracked — already finished, already cancelled, or the id was never valid.
#[tauri::command]
pub fn cancel_stage(run_id: String, registry: State<'_, RunRegistry>) -> bool {
    registry.cancel(&run_id)
}

/// Run the `stop` stage for `bottle` — the Session screen's Stop action.
#[tauri::command]
pub async fn stop_session(
    bottle: Option<String>,
    on_event: Channel<StageEvent>,
    registry: State<'_, RunRegistry>,
) -> Result<StageOutcome, String> {
    let opts = StageRunOpts {
        bottle,
        bs_dir: None,
        dry_run: None,
    };
    execute_stage(Stage::Stop, opts, on_event, &registry).await
}

/// Apply one fix ([`FixAction`]). Destructive fixes (currently only
/// `DeleteSessionJson` — [`FixAction::def`]) require `confirmed: true`; the
/// frontend shows its own in-app confirm dialog first (never
/// `window.confirm`, which blocks the webview) and this check is the
/// backend's half of that contract, not a substitute for it.
///
/// A fix whose [`FixAction::as_stage`] is `Some` (`RunSetup`/`RunBuild`/
/// `RunInstall`) still works through this command — it delegates to
/// [`sabrage_core::run_stage`] internally, exactly like a user-initiated
/// stage (see [`fixes::apply`]'s doc comment) — but the intended UI path for
/// those three is calling [`run_stage`] directly so the GateModal it opens is
/// the same one a plain stage run uses.
#[tauri::command]
pub async fn fix(
    action: FixAction,
    opts: FixRunOpts,
    confirmed: bool,
    on_event: Channel<StageEvent>,
) -> Result<FixReport, String> {
    if action.def().destructive && !confirmed {
        return Err(format!(
            "{action} is destructive and needs confirmation before it can run"
        ));
    }
    let repo_root = resolve_repo_root(None).map_err(|e| e.to_string())?;
    let mut stage_opts = stage_options_from_env_and_gui(opts.bottle, opts.bs_dir);
    stage_opts.dry_run = false;
    let sink = channel_sink(on_event);
    let ctx = StageCtx::new(Paths::new(repo_root), stage_opts, sink, Default::default());
    fixes::apply(action, &ctx, &ctx.sink)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// `std::env::set_var`/`remove_var` are process-global; serialize every
    /// test in this module that touches `WINEVR_BOTTLE` against each other
    /// (mirrors `fixes::session_json`'s `HOME_MUTEX` pattern for the same
    /// reason, one level up since these tests are synchronous).
    static WINEVR_BOTTLE_MUTEX: Mutex<()> = Mutex::new(());

    #[test]
    fn stage_options_from_env_and_gui_honours_winevr_bottle_when_the_gui_passes_none() {
        // Finding #4: `execute_stage`/`fix` used to build `StageOptions`
        // straight from the GUI's own args, so `WINEVR_BOTTLE` set in
        // Sabrage's environment was invisible to them (unlike `run_doctor`,
        // which already calls `CheckOptions::from_env()` first) — a Session
        // screen's `stop_session(None)` would die with "bottle name required"
        // even with `WINEVR_BOTTLE` set.
        let _guard = WINEVR_BOTTLE_MUTEX.lock().expect("mutex poisoned");
        let prev = std::env::var("WINEVR_BOTTLE").ok();

        // SAFETY: serialized by `WINEVR_BOTTLE_MUTEX` above; no other thread
        // reads/writes `WINEVR_BOTTLE` while this guard is held.
        unsafe { std::env::set_var("WINEVR_BOTTLE", "Steam") };

        // GUI passed `bottle: null` (`None`) — the env value must still win.
        let opts = stage_options_from_env_and_gui(None, None);
        assert_eq!(opts.bottle_name.as_deref(), Some("Steam"));

        // An explicit GUI value still overrides the env base.
        let opts = stage_options_from_env_and_gui(Some("Other".to_string()), None);
        assert_eq!(opts.bottle_name.as_deref(), Some("Other"));

        unsafe {
            match &prev {
                Some(v) => std::env::set_var("WINEVR_BOTTLE", v),
                None => std::env::remove_var("WINEVR_BOTTLE"),
            }
        }
    }

    #[test]
    fn a_dry_run_emits_the_shared_plan_rows_and_a_real_run_emits_none() {
        // Finding #13's GUI half: `planned()` never left the backend, so a
        // GUI dry run showed exactly the narrative rows a real run shows —
        // with no way to tell "would copy" from "would skip (bytes already
        // match)", the distinction the plan exists for.
        fn events_for(dry_run: bool) -> Vec<StageEvent> {
            let seen = Arc::new(Mutex::new(Vec::new()));
            let s = seen.clone();
            let sink: EventSink = Arc::new(move |ev| s.lock().unwrap().push(ev));
            let ctx = StageCtx::new(
                Paths::new("/nonexistent/sabrage-commands-test"),
                StageOptions {
                    dry_run,
                    ..StageOptions::default()
                },
                sink,
                Default::default(),
            );
            emit_dry_run_plan(&ctx);
            let evs = seen.lock().unwrap().clone();
            evs
        }

        // A real run's event stream is untouched.
        assert!(events_for(false).is_empty());

        // A dry run gets the section plus the shared body text — here the
        // empty-plan placeholder, since nothing ran.
        let evs = events_for(true);
        assert_eq!(evs.len(), 2, "{evs:?}");
        assert!(
            matches!(&evs[0], StageEvent::Section { title, .. } if title == sabrage_core::DRY_RUN_PLAN_TITLE),
            "{evs:?}"
        );
        assert!(
            matches!(
                &evs[1],
                StageEvent::Line { severity: sabrage_core::Severity::Info, text, .. }
                    if text == sabrage_core::DRY_RUN_PLAN_EMPTY
            ),
            "{evs:?}"
        );
    }

    #[test]
    fn no_doctor_row_group_matches_the_contracts_run_only_slugs() {
        use sabrage_core::checks::NO_DOCTOR_ROW_GROUP;
        let c = contract();
        assert_eq!(
            c.check("run.wine-exec").expect("slug present").group,
            NO_DOCTOR_ROW_GROUP
        );
        assert_eq!(
            c.check("run.bridge-built").expect("slug present").group,
            NO_DOCTOR_ROW_GROUP
        );
        // A doctor-visible slug, for contrast.
        assert_ne!(
            c.check("sys.arch").expect("slug present").group,
            NO_DOCTOR_ROW_GROUP
        );
    }

    #[test]
    fn run_registry_cancel_is_idempotent_and_reports_whether_a_run_was_found() {
        let registry = RunRegistry::default();
        let fired = Arc::new(AtomicBool::new(false));
        let f = fired.clone();
        registry.register(
            "abc".to_string(),
            Box::new(move || f.store(true, Ordering::SeqCst)),
        );
        assert!(!registry.cancel("does-not-exist"));
        assert!(registry.cancel("abc"));
        assert!(fired.load(Ordering::SeqCst));
        // Cancelling again finds nothing — the entry was removed.
        assert!(!registry.cancel("abc"));
    }

    #[test]
    fn forget_removes_without_firing_the_canceller() {
        let registry = RunRegistry::default();
        let fired = Arc::new(AtomicBool::new(false));
        let f = fired.clone();
        registry.register(
            "run".to_string(),
            Box::new(move || f.store(true, Ordering::SeqCst)),
        );
        registry.forget("run");
        assert!(!registry.cancel("run"));
        assert!(!fired.load(Ordering::SeqCst));
    }
}
