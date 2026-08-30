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
//! Phase 3 (session/run) adds:
//!
//! * `launch` — `run_stage(Stage::Run)` under a name that says what it does;
//!   its promise does not resolve until the session ends (design-core §3.2's
//!   state machine runs to Teardown), which can be hours.
//! * `get_session_status` / the 1 Hz `session://status` broadcaster
//!   ([`spawn_session_status_broadcaster`]) — both read the one managed
//!   [`SessionMonitorState`].
//! * `stop_session` grows a second branch: a session *this process*
//!   supervises ([`sabrage_core::live_session`]) is stopped by firing its
//!   `cancel` token rather than by running the `stop` stage over it.
//! * `detach_session` / `resolve_quit` are critique.md's "app-quit semantics
//!   for a live session" answer — see `lib.rs`'s `ExitRequested`/
//!   `CloseRequested` handlers, which open the dialog these resolve.
//! * `reconcile_session` runs [`sabrage_core::session::reconcile::reconcile`]
//!   over a `Vec<String>`-collecting sink instead of a `Channel` — a
//!   request/response call, not a stream.
//! * `start_log_tail`/`stop_log_tail`/`list_past_runs`/`get_log_source_path`
//!   back the Logs screen; tails run on blocking tasks, exactly like
//!   `run_doctor`'s evaluators, because [`sabrage_core::logs::Tailer`] is
//!   synchronous file I/O.
//!
//! `ui/src/ipc.ts` hand-mirrors every serde shape here 1:1 — keep both sides in
//! sync when either changes.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sabrage_core::checks::{run_doctor as core_run_doctor, CheckCtx, CheckOptions, CheckStatus};
use sabrage_core::session::reconcile::Reconciled;
use sabrage_core::session::watcher::SessionMonitor;
use sabrage_core::{
    contract, fixes, live_session, resolve_repo_root, EventSink, FixAction, FixReport, LogBatch,
    LogSource, PastRun, Paths, SabrageError, SessionStatus, Stage, StageCtx, StageEvent,
    StageOptions, StageOutcome, Tailer,
};
use serde::{Deserialize, Serialize};
use tauri::{ipc::Channel, AppHandle, Emitter, Manager, State};

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

/// Options for [`launch`] — [`StageRunOpts`]'s bottle/bs-dir/dry-run plus the
/// four flags `run.sh` reads only inside itself
/// (`WINEVR_NO_AUDIO`/`_NO_DASHBOARD`/`_WIRED`/`_VERBOSE`), which is why they
/// have no home on [`StageRunOpts`] — every other stage ignores them.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchOpts {
    pub bottle: Option<String>,
    pub bs_dir: Option<String>,
    pub no_audio: Option<bool>,
    pub no_dashboard: Option<bool>,
    pub wired: Option<bool>,
    pub verbose: Option<bool>,
    pub dry_run: Option<bool>,
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

/// [`launch`]'s options, merged the same way [`stage_options_from_env_and_gui`]
/// merges bottle/bs-dir: `WINEVR_*` env is the base, and a GUI-supplied `Some`
/// overrides field-by-field. `dry_run` has no environment counterpart (same
/// note as [`execute_stage`]'s call site) and is applied unconditionally.
fn launch_stage_options_from_env_and_gui(opts: &LaunchOpts) -> StageOptions {
    let mut stage_opts = stage_options_from_env_and_gui(opts.bottle.clone(), opts.bs_dir.clone());
    if let Some(v) = opts.no_audio {
        stage_opts.no_audio = v;
    }
    if let Some(v) = opts.no_dashboard {
        stage_opts.no_dashboard = v;
    }
    if let Some(v) = opts.wired {
        stage_opts.wired = v;
    }
    if let Some(v) = opts.verbose {
        stage_opts.verbose = v;
    }
    stage_opts.dry_run = opts.dry_run.unwrap_or(false);
    stage_opts
}

/// Shared body of [`execute_stage`] and [`launch`]: resolve the repo root,
/// build a [`StageCtx`] from an already-merged [`StageOptions`], register its
/// cancellation handle, run the stage, and unregister on the way out (success
/// or failure alike).
async fn execute_stage_with_options(
    stage: Stage,
    stage_opts: StageOptions,
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

/// [`run_stage`]/[`stop_session`]'s body: merge [`StageRunOpts`] the usual way
/// and hand off to [`execute_stage_with_options`].
async fn execute_stage(
    stage: Stage,
    opts: StageRunOpts,
    on_event: Channel<StageEvent>,
    registry: &RunRegistry,
) -> Result<StageOutcome, String> {
    let mut stage_opts = stage_options_from_env_and_gui(opts.bottle, opts.bs_dir);
    stage_opts.dry_run = opts.dry_run.unwrap_or(false);
    execute_stage_with_options(stage, stage_opts, on_event, registry).await
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

/// Run one pipeline stage (`setup`/`build`/`install`/`stop`/`run`), streaming
/// every [`StageEvent`] to `on_event` as it happens.
///
/// `run` is fully dispatched here too (`stages::dispatch` -> `stages::run::run`
/// — there is no `Fatal` shortcut, that note is Phase-1-era and stale) and
/// behaves identically to calling [`launch`] below with the same options: both
/// ultimately call [`sabrage_core::run_stage`] through
/// [`execute_stage_with_options`]. [`launch`] is still the intended UI entry
/// point for `run` — it is named for what it does, and the doc comment a
/// caller actually reads before awaiting something that can take hours should
/// say so up front.
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

/// Launch Beat Saber through the bridge — `Stage::Run`.
///
/// **The returned promise does not resolve until the session ends.** `run`'s
/// state machine (design-core §3.2) runs Preflight → Prepare → Guards →
/// Launch → Supervise → Teardown in one call, and Supervise waits on the wine
/// child for as long as the game stays open — routinely hours. Drive UI off
/// `on_event`: `StageEvent::Launched` marks "the game is up", `StageFinished`
/// marks "the session is over" (with the exit-code-equivalent `run_stage`'s
/// own doc comment already describes for `run`). Treat this promise the way
/// every other stage command's promise is already treated here — a secondary
/// confirmation, never the liveness signal a screen renders off of.
#[tauri::command]
pub async fn launch(
    opts: LaunchOpts,
    on_event: Channel<StageEvent>,
    registry: State<'_, RunRegistry>,
) -> Result<StageOutcome, String> {
    let stage_opts = launch_stage_options_from_env_and_gui(&opts);
    execute_stage_with_options(Stage::Run, stage_opts, on_event, &registry).await
}

/// Cancel the run named `run_id` (read off that run's first `StageStarted`
/// event), if it is still in flight. Returns `false` when no such run is
/// tracked — already finished, already cancelled, or the id was never valid.
#[tauri::command]
pub fn cancel_stage(run_id: String, registry: State<'_, RunRegistry>) -> bool {
    registry.cancel(&run_id)
}

/// Bound on [`stop_live_session_and_wait`]'s poll. Generous enough to cover
/// `wineserver -k`'s own 5 s fatal budget ([`sabrage_core::RUN_WINESERVER_WAIT`])
/// plus guard teardown; a hang past this simply returns control to the caller
/// — the point was firing the cancel token, and the caller only needed to
/// *try* waiting for it to finish.
const LIVE_SESSION_STOP_TIMEOUT: Duration = Duration::from_secs(30);

/// Fire the live session's `cancel` token — the INT path: stop wine, then
/// restore every guard (see [`sabrage_core::session`]'s module doc) — and
/// wait, bounded at [`LIVE_SESSION_STOP_TIMEOUT`], for
/// [`sabrage_core::live_session`] naming that same run to go back to `None`.
/// A no-op when nothing is live.
///
/// Shared by [`stop_session`]'s live-session branch and [`resolve_quit`]'s
/// `Stop` arm, so the two can never disagree on what "stop the session"
/// means. Runs its wait inside [`tauri::async_runtime::spawn_blocking`]: this
/// crate has no `tokio::time` re-export to `.await` a sleep with (only
/// `tokio::sync` items reach it, via [`tauri::async_runtime`] — see that
/// module's own doc comment), and only the Frame agent may add a direct
/// `tokio` dependency to pull one in.
async fn stop_live_session_and_wait() {
    let Some(handle) = live_session() else {
        return;
    };
    handle.cancel.cancel();
    let run_id = handle.run_id;
    let _ = tauri::async_runtime::spawn_blocking(move || {
        let deadline = std::time::Instant::now() + LIVE_SESSION_STOP_TIMEOUT;
        while live_session().is_some_and(|h| h.run_id == run_id)
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(100));
        }
    })
    .await;
}

/// Detach from the live session, if any — [`sabrage_core::session::reconcile::detach`]:
/// mark `session-state.json` `detached`, fire the handle's `detach` token
/// (stop supervising, leak every guard on purpose), and leave the session
/// running. A no-op, `Ok(())`, when nothing is live.
///
/// Shared by [`detach_session`] and [`resolve_quit`]'s `Keep` arm.
/// `RunEvent::Exit` for a quit that was never asked about (AppKit
/// `terminate:` — Dock-menu Quit, logout, AppleScript `quit` — which tao
/// cannot intercept): if a session this process supervises is still live and
/// no dialog answer approved this exit, apply the "keep running" answer
/// synchronously — [`detach_live_session`] fires the handle's detach token and
/// waits (bounded, inside `reconcile::detach`) for the supervise loop to disarm
/// its guards and mark the record `detached`. Best-effort: an error here must
/// not stop the process from exiting, and there is nobody left to show it to.
pub(crate) fn detach_on_terminate(quit_approved: bool) {
    if quit_approved || live_session().is_none() {
        return;
    }
    let _ = tauri::async_runtime::block_on(detach_live_session());
}

async fn detach_live_session() -> Result<(), String> {
    let Some(handle) = live_session() else {
        return Ok(());
    };
    let repo_root = resolve_repo_root(None).map_err(|e| e.to_string())?;
    let paths = Paths::new(repo_root);
    sabrage_core::session::reconcile::detach(&paths, &handle)
        .await
        .map_err(|e| e.to_string())
}

/// Whether [`stop_session`]'s live-session branch applies to the bottle it
/// was asked to stop.
///
/// `None` — the caller didn't name a bottle, "stop whatever session is
/// showing" — always matches, so the live session (if any) is always the
/// target when unspecified. A specific `requested_bottle` must equal the live
/// handle's own bottle: without this, `stop_session(Some("B"))` while this
/// process supervises a live session on bottle `"A"` would fire `"A"`'s
/// cancel token — the wrong bottle's session torn down — instead of falling
/// through to run the `Stop` stage for `"B"` as it should.
///
/// A pure fn so the scoping rule is unit-testable without a live
/// [`sabrage_core::session::LiveSessionHandle`].
pub(crate) fn stop_targets_live_session(requested_bottle: Option<&str>, live_bottle: &str) -> bool {
    match requested_bottle {
        None => true,
        Some(b) => b == live_bottle,
    }
}

/// Run the `stop` stage for `bottle` — the Session screen's Stop action.
///
/// When [`sabrage_core::live_session`] names a session **this process**
/// supervises *and* [`stop_targets_live_session`] says the requested `bottle`
/// is that same session (or none was requested), stopping it means firing its
/// own `cancel` token ([`stop_live_session_and_wait`]) rather than running the
/// `stop` stage over it from outside: the supervising `launch` call's own
/// event channel already carries every teardown row, so `on_event` here
/// streams nothing new. The synthetic outcome (`exit_code_equiv: 130`) mirrors
/// INT parity, the same convention `SabrageError::Cancelled` already uses
/// elsewhere.
///
/// Otherwise — no session this process supervises, or one on a *different*
/// bottle than was asked for, exactly demo.sh's own `stop.sh` situation, which
/// only ever finds a *bottle name* — this runs the `Stop` stage as before
/// (which also restores any guards a crashed session left pending in
/// `session-state.json`).
#[tauri::command]
pub async fn stop_session(
    bottle: Option<String>,
    on_event: Channel<StageEvent>,
    registry: State<'_, RunRegistry>,
) -> Result<StageOutcome, String> {
    if let Some(handle) = live_session() {
        if stop_targets_live_session(bottle.as_deref(), &handle.bottle) {
            stop_live_session_and_wait().await;
            drop(on_event);
            return Ok(StageOutcome {
                stage: Stage::Run,
                ok: true,
                exit_code_equiv: 130,
            });
        }
    }
    let opts = StageRunOpts {
        bottle,
        bs_dir: None,
        dry_run: None,
    };
    execute_stage(Stage::Stop, opts, on_event, &registry).await
}

/// Detach from the live session — the app-quit "leave it running" answer
/// (critique.md, "app-quit semantics for a live session"). `Ok(())` and a
/// no-op when nothing is live: detaching from nothing is not an error.
#[tauri::command]
pub async fn detach_session() -> Result<(), String> {
    detach_live_session().await
}

/// The three answers to "a session is still running — quit anyway?" — the
/// dialog `lib.rs`'s `ExitRequested`/`CloseRequested` handlers open by
/// emitting `app://quit-requested`; the frontend calls [`resolve_quit`] once
/// picked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum QuitChoice {
    /// Run the INT path (stop wine, restore every guard), then exit.
    Stop,
    /// Detach — leave the session running unsupervised — then exit.
    Keep,
    /// Do nothing; the pending quit is abandoned and the app/window stays
    /// open (it was already kept open by `lib.rs` calling
    /// `prevent_exit`/`prevent_close` before this was ever offered).
    Cancel,
}

/// Whether an `ExitRequested`/`CloseRequested` should be intercepted with the
/// "stop / keep / cancel" dialog rather than let straight through.
///
/// A pure function so the gating rule itself — not yet-answered vs. a session
/// worth protecting — is unit-testable without any Tauri machinery. `lib.rs`
/// calls this with `!QuitApproved` already inverted into `quit_approved` and
/// `live_session().is_some()`.
pub(crate) fn should_intercept_quit(quit_approved: bool, session_is_live: bool) -> bool {
    !quit_approved && session_is_live
}

/// Managed: has this run of the app already been told "go ahead and exit"?
/// Set by [`resolve_quit`]'s `Stop`/`Keep` arms, right before their own
/// `app.exit(0)` — that call re-enters `ExitRequested`, and this flag is what
/// lets the *second* one pass through [`should_intercept_quit`] undisturbed.
#[derive(Default)]
pub struct QuitApproved(pub AtomicBool);

/// Resolve the pending "quit while a session is live?" dialog.
#[tauri::command]
pub async fn resolve_quit(
    choice: QuitChoice,
    app: AppHandle,
    quit_approved: State<'_, QuitApproved>,
) -> Result<(), String> {
    match choice {
        QuitChoice::Cancel => {
            quit_approved.0.store(false, Ordering::SeqCst);
        }
        QuitChoice::Stop => {
            stop_live_session_and_wait().await;
            quit_approved.0.store(true, Ordering::SeqCst);
            app.exit(0);
        }
        QuitChoice::Keep => {
            // Best-effort: a quit the user already approved must not hang
            // (or refuse) over a transient repo-root/IO failure on the way
            // out — detaching is the point, not a precondition for exiting.
            let _ = detach_live_session().await;
            quit_approved.0.store(true, Ordering::SeqCst);
            app.exit(0);
        }
    }
    Ok(())
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

// ── session status ────────────────────────────────────────────────────────────

/// Managed: the one [`SessionMonitor`] this process polls from, behind a
/// tokio [`Mutex`][tauri::async_runtime::Mutex] — [`get_session_status`] and
/// [`spawn_session_status_broadcaster`] share it rather than each keeping (and
/// re-deriving the tail cursors of) their own. Built lazily on first use,
/// because building one needs a resolved repo root and `.setup()` must not
/// fail app startup over that.
#[derive(Default)]
pub struct SessionMonitorState(tauri::async_runtime::Mutex<Option<SessionMonitor>>);

/// Get-or-build the managed monitor. `resolve_repo_root` failing here is the
/// same "no checkout found" condition every other command already surfaces.
fn ensure_monitor(guard: &mut Option<SessionMonitor>) -> Result<&mut SessionMonitor, String> {
    if guard.is_none() {
        let repo_root = resolve_repo_root(None).map_err(|e| e.to_string())?;
        *guard = Some(SessionMonitor::new(Paths::new(repo_root)));
    }
    Ok(guard.as_mut().expect("just initialized above"))
}

/// One [`SessionStatus`] snapshot — the sidebar dot, the Session pill, and
/// every Stop button's poll fallback for the `session://status` broadcast
/// below.
#[tauri::command]
pub async fn get_session_status(
    monitor: State<'_, SessionMonitorState>,
) -> Result<SessionStatus, String> {
    let mut guard = monitor.0.lock().await;
    let m = ensure_monitor(&mut guard)?;
    Ok(m.snapshot().await)
}

/// Started once from `lib.rs`'s `.setup()`: snapshot the session every second
/// and broadcast it on `session://status`, unconditionally — the frontend
/// store (`stores/session.svelte.ts`) is the one that dedups, so a listener
/// attaching mid-session still gets a real value within a second instead of
/// waiting for the next state change. Runs for the app's lifetime; there is
/// nothing to stop it with.
///
/// The 1 s sleep runs on a blocking task with a synchronous `block_on` of the
/// lock+snapshot, for the same reason [`stop_live_session_and_wait`]'s wait
/// does: no `tokio::time` re-export is reachable from this crate without a
/// direct `tokio` dependency (Frame-agent-only), and
/// [`tauri::async_runtime::spawn_blocking`] plus [`tauri::async_runtime::block_on`]
/// is the primitive that is.
pub fn spawn_session_status_broadcaster(app: AppHandle) {
    tauri::async_runtime::spawn_blocking(move || loop {
        std::thread::sleep(Duration::from_secs(1));
        let state = app.state::<SessionMonitorState>();
        let status = tauri::async_runtime::block_on(async {
            let mut guard = state.0.lock().await;
            match ensure_monitor(&mut guard) {
                Ok(m) => m.snapshot().await,
                Err(_) => SessionStatus::default(),
            }
        });
        let _ = app.emit("session://status", &status);
    });
}

// ── reconcile ─────────────────────────────────────────────────────────────────

/// Extract one human line from a [`StageEvent`] emitted during
/// [`reconcile_session`]'s pass. [`sabrage_core::session::reconcile::reconcile`]'s
/// own doc comment says it "emits nothing on the happy path" — so on a clean
/// startup this yields nothing, and only fires when something was actually
/// restored or reported. Deliberately uncoloured/unmarked, unlike the CLI's
/// renderer (`sabrage-cli/src/main.rs`): the frontend draws its own banner
/// around these rows rather than needing shell styling baked in.
fn reconcile_row_text(ev: &StageEvent) -> Option<String> {
    match ev {
        StageEvent::Section { title, .. } => Some(format!("-- {title}")),
        StageEvent::Text { text, .. } => Some(text.clone()),
        StageEvent::Line {
            severity,
            text,
            remedy,
            ..
        } => {
            let mut line = format!("{} {text}", severity.as_str().to_uppercase());
            if let Some(r) = remedy {
                line.push_str(&format!(" (remedy: {r})"));
            }
            Some(line)
        }
        StageEvent::Fatal {
            message, remedy, ..
        } => {
            let mut line = format!("FATAL {message}");
            if let Some(r) = remedy {
                line.push_str(&format!(" (remedy: {r})"));
            }
            Some(line)
        }
        _ => None,
    }
}

/// [`reconcile_session`]'s return: the classification plus every human line
/// emitted while producing it.
///
/// The field really is named `kind` and really does hold a [`Reconciled`],
/// whose own serde tag field is *also* named `kind`
/// (`{"kind":{"kind":"dead", …},"rows":[…]}`) — [`Reconciled`] is internally
/// tagged for its own reasons (`session/reconcile.rs`'s doc comment) and this
/// wrapper's field name is unrelated, just coincidentally the same word.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconcileReport {
    pub kind: Reconciled,
    pub rows: Vec<String>,
}

/// Reconcile whatever `session-state.json` says on disk against what is
/// actually running.
///
/// Deliberately request/response, not a [`Channel`] stream: this is a one-shot
/// call the frontend makes at startup and again before showing the Launch
/// button, not a stage the user watches live. The sink collects each emitted
/// row into a plain `Vec<String>` (via [`reconcile_row_text`]) instead of
/// forwarding to a channel.
#[tauri::command]
pub async fn reconcile_session(bottle: Option<String>) -> Result<ReconcileReport, String> {
    let repo_root = resolve_repo_root(None).map_err(|e| e.to_string())?;
    let stage_opts = stage_options_from_env_and_gui(bottle, None);
    let rows: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let sink_rows = rows.clone();
    let sink: EventSink = Arc::new(move |ev: StageEvent| {
        if let Some(line) = reconcile_row_text(&ev) {
            sink_rows.lock().expect("rows mutex poisoned").push(line);
        }
    });
    let ctx = StageCtx::new(Paths::new(repo_root), stage_opts, sink, Default::default());
    let kind = sabrage_core::session::reconcile::reconcile(&ctx)
        .await
        .map_err(|e| e.to_string())?;
    let rows = Arc::try_unwrap(rows)
        .map(|m| m.into_inner().expect("rows mutex poisoned"))
        .unwrap_or_default();
    Ok(ReconcileReport { kind, rows })
}

// ── logs ──────────────────────────────────────────────────────────────────────

/// How often a live tail re-polls its file.
const LOG_TAIL_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Trailing lines preloaded when opening a from-the-end-only source
/// ([`LogSource::AlvrSession`] — design-core §7: the file is unbounded, so a
/// full read from the start is never correct).
const LOG_TAIL_PRELOAD_LINES: usize = 200;

/// Tracks in-flight [`start_log_tail`] pollers by an opaque id. Stopping one
/// flips its [`AtomicBool`] rather than aborting the task outright: the task
/// notices on its own next wake (at most [`LOG_TAIL_POLL_INTERVAL`] later) and
/// exits between polls instead of being cut off mid-read.
#[derive(Default)]
pub struct TailRegistry {
    next_id: AtomicU64,
    tails: Mutex<HashMap<u64, Arc<AtomicBool>>>,
}

impl TailRegistry {
    fn register(&self, stop: Arc<AtomicBool>) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        self.tails
            .lock()
            .expect("TailRegistry mutex poisoned")
            .insert(id, stop);
        id
    }

    fn stop(&self, id: u64) -> bool {
        match self
            .tails
            .lock()
            .expect("TailRegistry mutex poisoned")
            .remove(&id)
        {
            Some(stop) => {
                stop.store(true, Ordering::SeqCst);
                true
            }
            None => false,
        }
    }
}

/// Start tailing `source`, streaming each non-empty [`LogBatch`] to
/// `on_batch` every [`LOG_TAIL_POLL_INTERVAL`] until [`stop_log_tail`] is
/// called for the returned id.
///
/// Runs on a blocking task ([`tauri::async_runtime::spawn_blocking`]): every
/// [`Tailer`] method is synchronous file I/O — the same reason [`run_doctor`]
/// isolates its evaluators the same way. A batch with no new lines and no
/// rotation is not sent, to avoid four sends a second of "nothing happened"
/// noise on an idle log; [`LogBatch::rotated`]/`truncated` still reach the UI
/// the moment either is true.
#[tauri::command]
pub fn start_log_tail(
    source: LogSource,
    on_batch: Channel<LogBatch>,
    registry: State<'_, TailRegistry>,
) -> Result<u64, String> {
    let repo_root = resolve_repo_root(None).map_err(|e| e.to_string())?;
    let paths = Paths::new(repo_root);
    let path = sabrage_core::logs::resolve_source(&paths, &source)
        .ok_or_else(|| "no log file exists for this source yet".to_string())?;
    let from_end = matches!(source, LogSource::AlvrSession);
    let mut tailer = Tailer::open(
        &path,
        from_end,
        if from_end { LOG_TAIL_PRELOAD_LINES } else { 0 },
    )
    .map_err(|e| e.to_string())?;

    let stop = Arc::new(AtomicBool::new(false));
    let id = registry.register(stop.clone());
    tauri::async_runtime::spawn_blocking(move || {
        while !stop.load(Ordering::SeqCst) {
            match tailer.poll() {
                Ok(batch) if batch.rotated || batch.truncated || !batch.lines.is_empty() => {
                    if on_batch.send(batch).is_err() {
                        break; // the webview navigated away / the channel closed
                    }
                }
                Ok(_) => {}
                Err(_) => break, // the file became unreadable; nothing left to tail
            }
            std::thread::sleep(LOG_TAIL_POLL_INTERVAL);
        }
    });
    Ok(id)
}

/// Stop a tail started by [`start_log_tail`]. `false` when `id` is not (or is
/// no longer) tracked — already stopped, or never valid.
#[tauri::command]
pub fn stop_log_tail(id: u64, registry: State<'_, TailRegistry>) -> bool {
    registry.stop(id)
}

/// Every `logs/beatsaber-*.log` on disk, newest first — both front-ends'
/// runs, since they share the directory. Empty (never an error) when the repo
/// root cannot be resolved: no root means no `logs/` to list.
#[tauri::command]
pub fn list_past_runs() -> Vec<PastRun> {
    match resolve_repo_root(None) {
        Ok(root) => sabrage_core::logs::list_past_runs(&Paths::new(root).logs_dir()),
        Err(_) => Vec::new(),
    }
}

/// Resolve `source` to a path on this machine, or `None` when nothing matches
/// yet (an empty `logs/` on a fresh checkout, no session ever run).
#[tauri::command]
pub fn get_log_source_path(source: LogSource) -> Option<String> {
    let repo_root = resolve_repo_root(None).ok()?;
    let paths = Paths::new(repo_root);
    sabrage_core::logs::resolve_source(&paths, &source).map(|p| p.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// `std::env::set_var`/`remove_var` are process-global; serialize every
    /// test in this module that touches any `WINEVR_*` variable against each
    /// other (mirrors `fixes::session_json`'s `HOME_MUTEX` pattern for the
    /// same reason, one level up since these tests are synchronous). Named
    /// for `WINEVR_BOTTLE` (the first test to need it) but not scoped to it —
    /// [`launch_stage_options_layers_the_launch_flags_with_gui_precedence`]
    /// below holds it too.
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
    fn launch_stage_options_layers_the_launch_flags_with_gui_precedence() {
        const VARS: [&str; 4] = [
            "WINEVR_NO_AUDIO",
            "WINEVR_NO_DASHBOARD",
            "WINEVR_WIRED",
            "WINEVR_VERBOSE",
        ];
        let _guard = WINEVR_BOTTLE_MUTEX.lock().expect("mutex poisoned");
        let saved: Vec<Option<String>> = VARS.iter().map(|v| std::env::var(v).ok()).collect();

        // SAFETY: serialized by `WINEVR_BOTTLE_MUTEX` above.
        unsafe {
            for v in VARS {
                std::env::remove_var(v);
            }
        }

        // Nothing supplied by the GUI: every flag falls back to its (now
        // cleared) env default, and `dry_run` — which has no env counterpart
        // at all — defaults to `false`.
        let stage_opts = launch_stage_options_from_env_and_gui(&LaunchOpts::default());
        assert!(!stage_opts.no_audio);
        assert!(!stage_opts.no_dashboard);
        assert!(!stage_opts.wired);
        assert!(!stage_opts.verbose);
        assert!(!stage_opts.dry_run);

        // The GUI supplies `Some(true)` for two of the four; the other two
        // must stay at the env default rather than being forced to `false`.
        let opts = LaunchOpts {
            no_audio: Some(true),
            wired: Some(true),
            dry_run: Some(true),
            ..LaunchOpts::default()
        };
        let stage_opts = launch_stage_options_from_env_and_gui(&opts);
        assert!(stage_opts.no_audio && stage_opts.wired);
        assert!(!stage_opts.no_dashboard && !stage_opts.verbose);
        assert!(stage_opts.dry_run);

        // SAFETY: serialized by `WINEVR_BOTTLE_MUTEX` above.
        unsafe {
            for (v, prev) in VARS.iter().zip(saved) {
                match prev {
                    Some(val) => std::env::set_var(v, val),
                    None => std::env::remove_var(v),
                }
            }
        }
    }

    #[test]
    fn stop_targets_live_session_matches_none_or_the_same_bottle_only() {
        // Finding #8: `stop_session`'s live branch used to be unscoped by
        // bottle — stopping bottle B tore down a live session on bottle A.
        assert!(
            stop_targets_live_session(None, "Steam"),
            "no bottle requested means \"stop whatever session is live\""
        );
        assert!(stop_targets_live_session(Some("Steam"), "Steam"));
        assert!(
            !stop_targets_live_session(Some("Other"), "Steam"),
            "a different bottle must fall through to the Stop stage instead"
        );
    }

    #[test]
    fn should_intercept_quit_only_when_unapproved_and_live() {
        assert!(should_intercept_quit(false, true));
        assert!(!should_intercept_quit(true, true), "already approved");
        assert!(
            !should_intercept_quit(false, false),
            "nothing live to protect"
        );
        assert!(!should_intercept_quit(true, false));
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
