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
//! Phase 4 (settings/library/config — the section above `mod tests`) adds
//! eleven more: `read_runtime_config`/`write_runtime_config` over
//! [`sabrage_core::config`], `get_settings`/`save_settings`/`get_repo_info`
//! over [`sabrage_core::store::settings`], and
//! `get_library`/`new_game_template`/`save_game`/`remove_game`/
//! `validate_game`/`revert_original_steam_dll` over
//! [`sabrage_core::store::library`] and [`sabrage_core::store::goldberg`].
//! None of the eleven stream over an IPC `Channel` (no `on_event` in the
//! brief's IPC contract table), so their mutations go through a bare
//! [`RealExecutor`] ([`real_executor`]) rather than a [`StageCtx`] — see that
//! section's own module note. `launch` additionally grows a `gameId` and,
//! when one is supplied and the run actually reaches
//! [`StageEvent::Launched`], records a [`sabrage_core::store::library::LastSession`]
//! into `library.json` once the run settles ([`last_session_to_record`]).
//!
//! `ui/src/ipc.ts` hand-mirrors every serde shape here 1:1 — keep both sides in
//! sync when either changes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use sabrage_core::checks::{
    host, run_doctor as core_run_doctor, CheckCtx, CheckOptions, CheckStatus,
};
use sabrage_core::config as runtime_config;
use sabrage_core::session::reconcile::Reconciled;
use sabrage_core::session::watcher::SessionMonitor;
use sabrage_core::store::{goldberg, library, settings};
use sabrage_core::{
    contract, fixes, live_session, null_sink, resolve_repo_root, util, EventSink, FixAction,
    FixReport, LogBatch, LogSource, PastRun, Paths, RealExecutor, SabrageError, SessionStatus,
    Stage, StageCtx, StageEvent, StageOptions, StageOutcome, Tailer,
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
/// `fix` is the bare contract id (`"fix.set-graphics-backend"`) — and only
/// ever one this build actually offers: every id is projected through
/// [`offered_fix_id`] before it goes on the wire, so a deliberately withheld
/// one ([`sabrage_core::fixes::DEFERRED_CONTRACT_FIX_IDS`]:
/// `fix.create-z-drive`, and the known-bad `fix.delete-session-json`) reaches
/// no client at all. The frontend still maps the id it *does* receive to a
/// [`FixAction`] wire value itself (`ipc.ts`'s `contractFixIdToAction`,
/// mirroring [`FixAction::from_contract_id`], deferred set included), but that
/// mirror is now defence in depth rather than the policy: a hand-maintained
/// TypeScript table could otherwise render a Fix button for a remedy Rust
/// withholds, which is exactly how `cfg.session-pins` kept offering the
/// black-screen `delete-session-json` button. `fix.edit-protocol` is no longer
/// deferred (Phase 4, `sabrage_core::fixes::FixAction::EditProtocol`): it
/// round-trips through `from_contract_id` like every other fix.
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

/// The fix id a [`DoctorEvent`] may carry, given the one the contract names
/// for that check (`None` when it names none).
///
/// [`FixAction::from_contract_id`] is the single source of truth for "does
/// this build offer that remedy as a button": it returns `None` both for an
/// id no [`FixAction`] models and for one that is modelled but withheld
/// ([`sabrage_core::fixes::DEFERRED_CONTRACT_FIX_IDS`]). Projecting through it
/// here keeps a withheld id off the wire entirely, so no frontend — however
/// its own fix table drifted — can offer the button.
///
/// The id is round-tripped back out of the parsed action rather than passed
/// through verbatim, so what the client receives is by construction a
/// spelling [`FixAction::from_contract_id`] accepts.
fn offered_fix_id(contract_fix: Option<&str>) -> Option<String> {
    FixAction::from_contract_id(contract_fix?).map(FixAction::to_contract_id)
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
///
/// `default_bottle`/`default_bs_dir` (Phase 4) are `settings.json`'s stored
/// defaults, straight through — letting the Sidebar/Session screens prefill
/// without a second `get_settings` round trip. `None` means "nothing
/// configured yet", same as on [`sabrage_core::store::settings::Settings`]
/// itself.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppState {
    pub repo_root: Option<String>,
    pub bottles: Vec<String>,
    pub alvr_version: String,
    pub default_bottle: Option<String>,
    pub default_bs_dir: Option<String>,
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
    // settings.repo_root plumbing (Phase 4): a persisted override — or a
    // corrupt/missing settings file, which [`load_settings`] already
    // degrades to `None` for, logged rather than failing doctor outright.
    let settings = load_settings();
    let repo_root = match resolve_repo_root(settings.repo_root.as_deref()) {
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

    // Precedence, highest first: explicit GUI args > `WINEVR_*` env (parity
    // with the CLI and demo.sh) > the persisted `settings.json` defaults
    // (Phase 4 — a Finder-launched .app has no environment at all, so without
    // this tier the Doctor screen could never find a Beat Saber dir the user
    // had set on the Settings screen).
    let mut opts = CheckOptions::from_env();
    // settings.allow_adb_probes (Phase 4): this used to be hard-coded `true`
    // regardless of the toggle on the Settings screen.
    opts.allow_adb_probes = settings.allow_adb_probes;
    if opts.bottle_name.is_none() {
        opts.bottle_name = settings.default_bottle.clone().filter(|s| !s.is_empty());
    }
    if opts.bs_dir_override.is_none() {
        opts.bs_dir_override = settings
            .default_bs_dir
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);
    }
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
            let fix = offered_fix_id(spec.and_then(|s| s.fix.as_deref()));
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
/// machine, the pinned ALVR client version, and (Phase 4) `settings.json`'s
/// default bottle/Beat Saber dir.
#[tauri::command]
pub fn get_app_state() -> AppState {
    let settings = load_settings();
    AppState {
        repo_root: resolve_repo_root(settings.repo_root.as_deref())
            .ok()
            .map(|p| p.display().to_string()),
        bottles: sabrage_core::paths::list_bottles(),
        alvr_version: ALVR_VERSION.to_string(),
        default_bottle: settings.default_bottle,
        default_bs_dir: settings.default_bs_dir,
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
///
/// `game_id` (Phase 4) is the library entry this launch came from, if any —
/// the Library screen's "Run through bridge" sets it, the Session screen's ad
/// hoc launch leaves it `None`. See [`last_session_to_record`] for what it
/// unlocks.
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
    pub game_id: Option<String>,
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

/// [`channel_sink`], plus a `tap` called with every event before it is
/// forwarded — [`launch`]'s way of observing whether `StageEvent::Launched`
/// fired for this run (Phase 4's last-session recording,
/// [`last_session_to_record`]) without a second event type or a second
/// subscription: `StageEvent` is already the wire shape design-core §3.1
/// specifies (this module's own doc comment).
fn channel_sink_tee(
    channel: Channel<StageEvent>,
    tap: impl Fn(&StageEvent) + Send + Sync + 'static,
) -> EventSink {
    Arc::new(move |ev: StageEvent| {
        tap(&ev);
        let _ = channel.send(ev);
    })
}

/// Emit a well-formed `StageStarted`/`Fatal`/`StageFinished` bracket for a
/// failure that happens *before* a [`StageCtx`] can exist (repo-root
/// resolution). Keeps the invariant every stage's event stream relies on
/// (`stages::run_stage`'s own doc comment): a listener that only watches
/// events never sees a run that started and never ended.
///
/// Takes the already-built [`EventSink`] rather than a raw `Channel` —
/// [`execute_stage_with_sink`]'s failure branch runs before it knows whether
/// its caller wrapped a plain [`channel_sink`] or [`launch`]'s tapped one, and
/// either way the sink is called exactly like [`sabrage_core::fixes`]'s own
/// `sink(StageEvent::…)` call sites already do.
fn emit_early_failure(sink: &EventSink, stage: Stage, err: &SabrageError) {
    let run_id = Default::default();
    sink(StageEvent::StageStarted { run_id, stage });
    sink(StageEvent::Fatal {
        run_id,
        message: err.to_string(),
        remedy: err.remedy().map(|s| s.to_string()),
        fix: None,
    });
    sink(StageEvent::StageFinished {
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
    let mut opts = fill_stage_options_from_settings(StageOptions::from_env(), &load_settings());
    if let Some(b) = bottle {
        opts.bottle_name = Some(b);
    }
    if let Some(d) = bs_dir {
        opts.bs_dir_override = Some(PathBuf::from(d));
    }
    opts
}

/// The lowest precedence tier of [`stage_options_from_env_and_gui`] (and of
/// [`run_doctor`]'s `CheckOptions`): `settings.json`'s `default_bottle` /
/// `default_bs_dir` fill a bottle or Beat Saber dir that neither the
/// environment nor the caller supplied. Phase 4 — a Finder-launched .app has
/// no `WINEVR_*` environment, so this tier is what makes the Settings
/// screen's "Paths" card actually reach setup/build/install/doctor/stop.
/// Pure (settings passed in) so it is testable without touching `$HOME`.
fn fill_stage_options_from_settings(
    mut opts: StageOptions,
    settings: &settings::Settings,
) -> StageOptions {
    if opts.bottle_name.is_none() {
        opts.bottle_name = settings.default_bottle.clone().filter(|s| !s.is_empty());
    }
    if opts.bs_dir_override.is_none() {
        opts.bs_dir_override = settings
            .default_bs_dir
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);
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

/// The `stage://queued` payload: a run that has been assigned an id and a
/// cancellation handle but is still waiting for the operation lock, so the
/// frontend can offer Cancel (and name the run) during the wait.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedStage {
    pub run_id: String,
    pub stage: Stage,
}

/// Announce a run that is about to queue behind another operation.
///
/// A stage that has to wait for the operation lock reaches the frontend twice
/// over: `sabrage_core::run_stage` now emits its `StageStarted` *before* the
/// wait (so the run id — and with it Cancel — exists for the whole of it), and
/// this event names the wait as a wait. The run is already registered with
/// [`RunRegistry`] by this point ([`sabrage_core::executor::RealExecutor`]
/// fails every filesystem primitive with `Cancelled` once its token fires, and
/// the lock wait itself is cancellable), so Cancel during the queue is real.
///
/// The probe is [`sabrage_core::stages::operation_in_progress_anywhere`], not
/// the in-process half alone: the wait this notice exists for is most often the
/// one behind a `sabrage` CLI build in **another process**, which the
/// in-process mutex cannot see at all.
///
/// Emitted as an app event rather than on the stage channel because
/// `StageEvent::StageStarted` is documented as "always the first event of a
/// run" — a queue notice is not part of the run's own event stream.
/// Best-effort in both directions: the probe is racy (it can only over- or
/// under-report a lock that is about to change hands), and a failed emit is
/// dropped exactly as a failed channel send is.
fn announce_if_queued(app: &AppHandle, run_id: &str, stage: Stage) {
    if !sabrage_core::stages::operation_in_progress_anywhere() {
        return;
    }
    let _ = app.emit(
        "stage://queued",
        QueuedStage {
            run_id: run_id.to_string(),
            stage,
        },
    );
}

/// [`execute_stage`]/[`launch`]'s shared body, taking an already-built
/// [`EventSink`] rather than a raw `Channel` — the seam [`launch`] uses to
/// pass a tapped sink ([`channel_sink_tee`]) instead of a plain forwarding
/// one, so it can observe `StageEvent::Launched` without a second
/// subscription. Resolves the repo root, builds a [`StageCtx`] from an
/// already-merged [`StageOptions`], registers its cancellation handle, runs
/// the stage, and unregisters on the way out (success or failure alike).
///
/// settings.repo_root plumbing (Phase 4): resolves through
/// [`resolve_repo_root_via_settings`] rather than a bare `resolve_repo_root(None)`
/// — see that function's doc comment.
async fn execute_stage_with_sink(
    stage: Stage,
    stage_opts: StageOptions,
    sink: EventSink,
    registry: &RunRegistry,
    app: &AppHandle,
) -> Result<StageOutcome, String> {
    let repo_root = match resolve_repo_root_via_settings() {
        Ok(p) => p,
        Err(err) => {
            emit_early_failure(&sink, stage, &err);
            return Err(err.to_string());
        }
    };
    let paths = match Paths::new_checked(repo_root) {
        Ok(p) => p,
        Err(err) => {
            emit_early_failure(&sink, stage, &err);
            return Err(err.to_string());
        }
    };
    let ctx = StageCtx::new(paths, stage_opts, sink, Default::default());
    let run_id = ctx.run_id.to_string();
    let cancel_handle = ctx.cancel.clone();
    registry.register(run_id.clone(), Box::new(move || cancel_handle.cancel()));
    announce_if_queued(app, &run_id, stage);
    let result = sabrage_core::run_stage(stage, &ctx).await;
    emit_dry_run_plan(&ctx);
    registry.forget(&run_id);
    result.map_err(|e| e.to_string())
}

/// [`run_stage`]/[`stop_session`]'s body: merge [`StageRunOpts`] the usual way
/// and hand off to [`execute_stage_with_sink`], wrapping `on_event` as a plain
/// [`channel_sink`] (no Launched-tapping needed, unlike [`launch`]).
async fn execute_stage(
    stage: Stage,
    opts: StageRunOpts,
    on_event: Channel<StageEvent>,
    registry: &RunRegistry,
    app: &AppHandle,
) -> Result<StageOutcome, String> {
    let mut stage_opts = stage_options_from_env_and_gui(opts.bottle, opts.bs_dir);
    stage_opts.dry_run = opts.dry_run.unwrap_or(false);
    execute_stage_with_sink(stage, stage_opts, channel_sink(on_event), registry, app).await
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
/// [`execute_stage_with_sink`]. [`launch`] is still the intended UI entry
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
    app: AppHandle,
) -> Result<StageOutcome, String> {
    execute_stage(stage, opts, on_event, &registry, &app).await
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
    app: AppHandle,
) -> Result<StageOutcome, String> {
    let stage_opts = launch_stage_options_from_env_and_gui(&opts);

    // Phase 4: tee the channel so a `StageEvent::Launched` for *this* run is
    // observed, win or lose — `last_session_to_record` below is the pure
    // decision of whether that (plus `game_id`, plus a settled outcome) adds
    // up to something worth writing into `library.json`.
    let launched: Arc<Mutex<Option<LaunchedInfo>>> = Arc::new(Mutex::new(None));
    let tap_launched = launched.clone();
    let sink = channel_sink_tee(on_event, move |ev| {
        if let StageEvent::Launched {
            started_at_unix_ms,
            log_path,
            ..
        } = ev
        {
            *tap_launched.lock().expect("launched-info mutex poisoned") = Some(LaunchedInfo {
                started_at_unix_ms: *started_at_unix_ms,
                log_path: log_path.clone(),
            });
        }
    });

    let result = execute_stage_with_sink(Stage::Run, stage_opts, sink, &registry, &app).await;

    let launched_info = launched
        .lock()
        .expect("launched-info mutex poisoned")
        .clone();
    if let Some((game_id, session)) = last_session_to_record(
        opts.game_id.as_deref(),
        launched_info.as_ref(),
        result.as_ref().ok(),
    ) {
        record_last_session(&game_id, session, &app.state::<SettingsPathsCache>()).await;
    }

    result
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

/// What [`stop_live_session_and_wait`]'s bounded wait actually observed.
///
/// The distinction is the whole point: before this existed the wait returned
/// `()`, so a teardown that hung past [`LIVE_SESSION_STOP_TIMEOUT`] was
/// indistinguishable from one that finished — [`stop_session`] reported
/// `ok: true` and [`resolve_quit`]'s `Stop` arm approved the quit and exited,
/// skipping the detach fallback in `lib.rs`'s `RunEvent::Exit` arm and leaving
/// the game (and its guards) unsupervised.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TeardownWait {
    /// Nothing was live: there was nothing to stop, which is success.
    NothingLive,
    /// The live-session slot cleared because teardown finished.
    Cleared,
    /// The slot cleared, but the record says the run *detached* rather than
    /// tore down — the session is still running with its guards left in place
    /// on purpose ([`sabrage_core::session`]'s module doc). Not a failure, but
    /// not "stopped" either, so a caller must not claim it stopped anything.
    Detached,
    /// Still live at the deadline (or the waiting task itself failed):
    /// teardown is unfinished and nothing may claim otherwise.
    TimedOut,
}

/// What [`resolve_quit`]'s `Stop` arm must tell the user about a teardown that
/// did not simply finish — `None` when it did.
///
/// Pure, and deliberately says only what happened. It used to claim a detach
/// on [`TeardownWait::TimedOut`] ("Sabrage detached from it and quit") and then
/// call `detach_live_session()` to make it true; that call is a provable
/// no-op in both refusal arms, so the message was a lie:
///
/// * `TimedOut` — the stop already fired the session's `cancel` token, and
///   [`sabrage_core::session::reconcile::detach`] returns `Ok(())` without
///   disarming a guard or marking the record once that token is set;
/// * `Detached` — the slot is already clear, so [`detach_live_session`] finds
///   no live handle at all.
///
/// Not detaching is also the *better* outcome, which is why this only fixes
/// the wording: `detached: true` means "leave every guard in place, and no
/// later reconcile may undo that" (`session::reconcile`'s module doc), whereas
/// a record left `detached: false` with a dead owner is exactly the shape
/// `reconcile::classify` recovers on the next Sabrage start — restoring the
/// audio device and removing the `--wired` forwards.
pub(crate) fn quit_stop_refusal(waited: TeardownWait) -> Option<String> {
    match waited {
        TeardownWait::NothingLive | TeardownWait::Cleared => None,
        TeardownWait::Detached => Some(
            "the session detached instead of stopping — it is still running, \
             unsupervised (./demo.sh stop --bottle <name>)"
                .to_string(),
        ),
        TeardownWait::TimedOut => Some(format!(
            "the session did not finish stopping within {} s — Sabrage stopped waiting and quit; \
             the game may still be running (./demo.sh stop --bottle <name>). The next Sabrage \
             start will reconcile whatever is left.",
            LIVE_SESSION_STOP_TIMEOUT.as_secs()
        )),
    }
}

/// Bounded poll for "the live-session slot no longer holds `run_id`",
/// factored out of [`stop_live_session_and_wait`] so the deadline behaviour is
/// unit-testable without a real session: `is_clear` is the predicate the real
/// caller wires to [`sabrage_core::live_session`].
fn wait_for_slot_clear(is_clear: impl Fn() -> bool, timeout: Duration) -> TeardownWait {
    let deadline = Instant::now() + timeout;
    loop {
        if is_clear() {
            return TeardownWait::Cleared;
        }
        if Instant::now() >= deadline {
            return TeardownWait::TimedOut;
        }
        std::thread::sleep(LIVE_SESSION_STOP_POLL);
    }
}

/// How often [`wait_for_slot_clear`] re-checks the live-session slot.
const LIVE_SESSION_STOP_POLL: Duration = Duration::from_millis(100);

/// Fire the live session's `cancel` token — the INT path: stop wine, then
/// restore every guard (see [`sabrage_core::session`]'s module doc) — and
/// wait, bounded at [`LIVE_SESSION_STOP_TIMEOUT`], for
/// [`sabrage_core::live_session`] naming that same run to go back to `None`.
/// [`TeardownWait::NothingLive`] when nothing is live.
///
/// Shared by [`stop_session`]'s live-session branch and [`resolve_quit`]'s
/// `Stop` arm, so the two can never disagree on what "stop the session"
/// means — including on what *failing* to stop it means. Runs its wait inside
/// [`tauri::async_runtime::spawn_blocking`]: this crate has no `tokio::time`
/// re-export to `.await` a sleep with (only `tokio::sync` items reach it, via
/// [`tauri::async_runtime`] — see that module's own doc comment), and only the
/// Frame agent may add a direct `tokio` dependency to pull one in.
async fn stop_live_session_and_wait() -> TeardownWait {
    let Some(handle) = live_session() else {
        return TeardownWait::NothingLive;
    };
    handle.cancel.cancel();
    let run_id = handle.run_id;
    let waited = tauri::async_runtime::spawn_blocking(move || {
        wait_for_slot_clear(
            || !live_session().is_some_and(|h| h.run_id == run_id),
            LIVE_SESSION_STOP_TIMEOUT,
        )
    })
    .await
    // A join failure means nobody watched the slot at all — the honest
    // answer is the same one a hang gets.
    .unwrap_or(TeardownWait::TimedOut);

    if waited == TeardownWait::Cleared && slot_was_released_by_a_detach(handle.run_id) {
        return TeardownWait::Detached;
    }
    waited
}

/// Did the run whose slot just cleared *detach* instead of tearing down?
///
/// `session-state.json` outlives the handle: a detach marks the record
/// `detached` and leaves it in place, while a completed teardown clears it.
/// Both tokens are independent (`session/mod.rs`), so a Stop and a detach can
/// race; this is what stops [`stop_session`] from reporting `ok: true` for a
/// session that is in fact still running.
///
/// Best-effort: an unresolved repo root or an unreadable record answers
/// "no" — the caller then reports the ordinary stop, which is what it did
/// before this check existed.
fn slot_was_released_by_a_detach(run_id: sabrage_core::events::RunId) -> bool {
    let Ok(repo_root) = resolve_repo_root_via_settings() else {
        return false;
    };
    let path = Paths::new(repo_root).session_state_path();
    matches!(
        sabrage_core::session::state::load(&path),
        Ok(Some(state)) if state.run_id == run_id && state.detached
    )
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
    let repo_root = resolve_repo_root_via_settings().map_err(|e| e.to_string())?;
    // Checked: detaching rewrites `session-state.json`, and an unusable
    // `$HOME` would put that file somewhere that is not the user's store.
    let paths = Paths::new_checked(repo_root).map_err(|e| e.to_string())?;
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
    app: AppHandle,
) -> Result<StageOutcome, String> {
    if let Some(handle) = live_session() {
        if stop_targets_live_session(bottle.as_deref(), &handle.bottle) {
            let waited = stop_live_session_and_wait().await;
            drop(on_event);
            return match waited {
                TeardownWait::NothingLive | TeardownWait::Cleared => Ok(StageOutcome {
                    stage: Stage::Run,
                    ok: true,
                    exit_code_equiv: 130,
                }),
                TeardownWait::Detached => Err(format!(
                    "the session for bottle '{}' detached instead of stopping — it is still \
                     running, unsupervised; stop it with ./demo.sh stop --bottle {}",
                    handle.bottle, handle.bottle
                )),
                TeardownWait::TimedOut => Err(format!(
                    "the session for bottle '{}' did not finish tearing down within {} s — it \
                     may still be running; check the launch log and, if needed, ./demo.sh stop \
                     --bottle {}",
                    handle.bottle,
                    LIVE_SESSION_STOP_TIMEOUT.as_secs(),
                    handle.bottle
                )),
            };
        }
    }
    let opts = StageRunOpts {
        bottle,
        bs_dir: None,
        dry_run: None,
    };
    execute_stage(Stage::Stop, opts, on_event, &registry, &app).await
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
/// calls [`quit_intercept_decision`] (which this is the first-request case of)
/// with `!QuitApproved` already inverted into `quit_approved` and
/// `live_session().is_some()`.
pub(crate) fn should_intercept_quit(quit_approved: bool, session_is_live: bool) -> bool {
    !quit_approved && session_is_live
}

/// How long a quit request may sit unanswered before the *next* one stops
/// asking and takes the documented keep-running answer instead.
///
/// The dialog has exactly one responder — the frontend's
/// `app://quit-requested` listener — and a webview that is reloading, crashed,
/// or died before subscribing never answers. Without a bound, every Cmd-Q and
/// every window close is then prevented forever and the app cannot be quit at
/// all.
pub(crate) const QUIT_DIALOG_TIMEOUT: Duration = Duration::from_secs(20);

/// What `lib.rs`'s `ExitRequested`/`CloseRequested` arms should do with one
/// quit request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QuitIntercept {
    /// Let the OS tear the app down: nothing live, or the user already
    /// answered.
    PassThrough,
    /// Prevent the exit and open the "stop / keep / cancel" dialog.
    Ask,
    /// A dialog has been open, unanswered, for longer than
    /// [`QUIT_DIALOG_TIMEOUT`] and the user asked to quit *again*: stop
    /// asking. Also passes the exit through — `RunEvent::Exit`'s
    /// [`detach_on_terminate`] then applies the keep-running answer (detach,
    /// guards disarmed, record marked `detached`), which is exactly what the
    /// un-askable AppKit `terminate:` path already does.
    GiveUp,
}

/// [`QuitIntercept`] for one request. Pure: `pending_for` is how long the
/// currently-open dialog has gone unanswered (`None` when none is open).
pub(crate) fn quit_intercept_decision(
    quit_approved: bool,
    session_is_live: bool,
    pending_for: Option<Duration>,
) -> QuitIntercept {
    if !should_intercept_quit(quit_approved, session_is_live) {
        return QuitIntercept::PassThrough;
    }
    match pending_for {
        Some(d) if d >= QUIT_DIALOG_TIMEOUT => QuitIntercept::GiveUp,
        _ => QuitIntercept::Ask,
    }
}

/// Managed: when the still-unanswered quit dialog was opened, if one is open.
///
/// Set by `lib.rs`'s intercept arms, cleared by [`resolve_quit`] (whichever
/// answer arrives) — the input [`quit_intercept_decision`] needs to tell "the
/// user is thinking about it" from "nothing is listening".
#[derive(Default)]
pub struct PendingQuit(Mutex<Option<Instant>>);

impl PendingQuit {
    /// How long the open dialog has gone unanswered, `None` when none is open.
    pub(crate) fn pending_for(&self) -> Option<Duration> {
        self.0
            .lock()
            .expect("PendingQuit mutex poisoned")
            .map(|since| since.elapsed())
    }

    /// Record that a dialog was just opened. Keeps the *first* instant: a
    /// repeated Cmd-Q must not refresh the deadline it is meant to trip.
    pub(crate) fn mark(&self) {
        self.0
            .lock()
            .expect("PendingQuit mutex poisoned")
            .get_or_insert_with(Instant::now);
    }

    pub(crate) fn clear(&self) {
        *self.0.lock().expect("PendingQuit mutex poisoned") = None;
    }
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
    pending_quit: State<'_, PendingQuit>,
) -> Result<(), String> {
    // The dialog has been answered — whichever way — so the unanswered-quit
    // fallback ([`quit_intercept_decision`]) must not fire on a later request.
    pending_quit.clear();
    match choice {
        QuitChoice::Cancel => {
            quit_approved.0.store(false, Ordering::SeqCst);
        }
        QuitChoice::Stop => {
            // The quit is approved either way — the user must never be left
            // unable to quit — but only a *clean* teardown gets to say so
            // silently: anything else returns the reason, so the dialog can
            // say what actually happened ([`quit_stop_refusal`]).
            let refusal = quit_stop_refusal(stop_live_session_and_wait().await);
            quit_approved.0.store(true, Ordering::SeqCst);
            app.exit(0);
            if let Some(message) = refusal {
                return Err(message);
            }
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

/// Why the GUI fix door refuses `action`, if it does — the two rejections
/// [`fix`] owes before it builds a [`StageCtx`] or touches the operation lock.
///
/// Pure, so both rules are unit-testable without a Tauri runtime:
///
/// * **withheld** — [`FixAction::is_deferred`]; the mirror in `ipc.ts` is not
///   trusted to have kept up with `DEFERRED_CONTRACT_FIX_IDS`;
/// * **unconfirmed destructive** — the backend half of the in-app confirm
///   dialog contract (see [`fix`]).
fn gui_fix_refusal(action: FixAction, confirmed: bool) -> Option<String> {
    if action.is_deferred() {
        return Some(format!(
            "{action} is not offered by this build of Sabrage — see its consequence note; \
             run `sabrage fix {action}` explicitly if you still want it"
        ));
    }
    if action.def().destructive && !confirmed {
        return Some(format!(
            "{action} is destructive and needs confirmation before it can run"
        ));
    }
    None
}

/// Apply one fix ([`FixAction`]). Destructive fixes ([`FixAction::def`];
/// `DeleteSessionJson` is the only one today, and it is *also* withheld — see
/// below) require `confirmed: true`; the frontend shows its own in-app confirm
/// dialog first (never `window.confirm`, which blocks the webview) and this
/// check is the backend's half of that contract, not a substitute for it.
///
/// A fix whose [`FixAction::as_stage`] is `Some` (`RunSetup`/`RunBuild`/
/// `RunInstall`) still works through this command — it delegates to
/// [`sabrage_core::run_stage`] internally, exactly like a user-initiated
/// stage (see [`fixes::apply`]'s doc comment) — but the intended UI path for
/// those three is calling [`run_stage`] directly so the GateModal it opens is
/// the same one a plain stage run uses.
///
/// An action this build **withholds** ([`FixAction::is_deferred`]) is refused
/// here outright, whatever the frontend believed: the GUI's own fix table is a
/// hand-maintained mirror, and `cfg.session-pins` was still rendering a button
/// for the known-bad `delete-session-json` remedy after Rust stopped offering
/// it. The `sabrage fix <id>` CLI path is deliberately *not* gated the same
/// way — a user who has read [`sabrage_core::fixes::FixDef::consequence`] can
/// still ask for it explicitly there.
#[tauri::command]
pub async fn fix(
    action: FixAction,
    opts: FixRunOpts,
    confirmed: bool,
    on_event: Channel<StageEvent>,
    registry: State<'_, RunRegistry>,
) -> Result<FixReport, String> {
    if let Some(refusal) = gui_fix_refusal(action, confirmed) {
        return Err(refusal);
    }
    // A fix waits on the same operation lock a stage does, and its whole-stage
    // forms (`run-setup`/`run-build`/`run-install`) run for minutes — so it
    // needs the same cancellation handle a stage gets. Without this
    // registration a fix, queued or running, could not be cancelled at all.
    // The handle is reachable because `fixes::apply` emits a row carrying
    // `ctx.run_id` *before* it waits for the lock (see its doc comment), so
    // the frontend can name — and cancel — a fix that is still queued; the
    // `forget` below therefore also covers the cancelled path.
    // (The live-session refusal `FixDef::forbidden_while_session_live`
    // describes is enforced inside `fixes::apply`: once before the lock as an
    // immediate rejection, and again with it held.)
    let repo_root = resolve_repo_root_via_settings().map_err(|e| e.to_string())?;
    let paths = Paths::new_checked(repo_root).map_err(|e| e.to_string())?;
    let mut stage_opts = stage_options_from_env_and_gui(opts.bottle, opts.bs_dir);
    stage_opts.dry_run = false;
    let sink = channel_sink(on_event);
    let ctx = StageCtx::new(paths, stage_opts, sink, Default::default());
    let run_id = ctx.run_id.to_string();
    let cancel_handle = ctx.cancel.clone();
    registry.register(run_id.clone(), Box::new(move || cancel_handle.cancel()));
    let result = fixes::apply(action, &ctx, &ctx.sink).await;
    registry.forget(&run_id);
    result.map_err(|e| e.to_string())
}

// ── session status ────────────────────────────────────────────────────────────

/// Managed: the one [`SessionMonitor`] this process polls from, behind a
/// tokio [`Mutex`][tauri::async_runtime::Mutex] — [`get_session_status`] and
/// [`spawn_session_status_broadcaster`] share it rather than each keeping (and
/// re-deriving the tail cursors of) their own. Built lazily on first use,
/// because building one needs a resolved repo root and `.setup()` must not
/// fail app startup over that.
#[derive(Default)]
pub struct SessionMonitorState {
    monitor: tauri::async_runtime::Mutex<Option<SessionMonitor>>,
    /// The last snapshot [`spawn_session_status_broadcaster`] actually emitted
    /// — its dedup memory (see that function's doc comment).
    last_broadcast: Mutex<Option<SessionStatus>>,
}

impl SessionMonitorState {
    /// Record `status` as the broadcaster's latest, returning whether it is
    /// new — i.e. whether it is worth emitting. The first call always is.
    fn remember_broadcast(&self, status: &SessionStatus) -> bool {
        let mut last = self.last_broadcast.lock().expect("last-broadcast poisoned");
        if last.as_ref() == Some(status) {
            return false;
        }
        *last = Some(status.clone());
        true
    }
}

/// Get-or-build the managed monitor. `resolve_repo_root` failing here is the
/// same "no checkout found" condition every other command already surfaces.
fn ensure_monitor(guard: &mut Option<SessionMonitor>) -> Result<&mut SessionMonitor, String> {
    if guard.is_none() {
        let repo_root = resolve_repo_root_via_settings().map_err(|e| e.to_string())?;
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
    let mut guard = monitor.monitor.lock().await;
    let m = ensure_monitor(&mut guard)?;
    Ok(m.snapshot().await)
}

/// Started once from `lib.rs`'s `.setup()`: snapshot the session every second
/// and broadcast it on `session://status` **when it changed**. Runs for the
/// app's lifetime; there is nothing to stop it with.
///
/// The dedup is here rather than in the frontend store: an idle Sabrage sat
/// serializing and emitting an identical `SessionStatus` to every window once
/// a second forever, and the claim that the store deduped it was simply false
/// (`stores/session.svelte.ts` assigns each payload straight through). The
/// first snapshot after startup is always emitted — a listener that attaches
/// late gets a value within a second either way, since
/// [`get_session_status`] is the poll fallback for exactly that window.
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
            let mut guard = state.monitor.lock().await;
            match ensure_monitor(&mut guard) {
                Ok(m) => m.snapshot().await,
                Err(_) => SessionStatus::default(),
            }
        });
        if !state.remember_broadcast(&status) {
            continue;
        }
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
///
/// Takes [`sabrage_core::stages::acquire_operation_lock`] for the pass. A
/// reconciliation restores audio, reaps the recorded dashboard and clears the
/// record — all of them things a launch that has not yet spawned wine is in
/// the middle of setting up — and the run stage does its own reconcile while
/// holding this lock, so without it a Session-screen remount during
/// `Preflight`/`Launching` could tear down the very launch in flight. (The
/// core's own `Reconciled::Busy` classification is the second half of that
/// guard: an in-flight run phase is reported and nothing is touched.)
#[tauri::command]
pub async fn reconcile_session(bottle: Option<String>) -> Result<ReconcileReport, String> {
    let repo_root = resolve_repo_root_via_settings().map_err(|e| e.to_string())?;
    let stage_opts = stage_options_from_env_and_gui(bottle, None);
    let _op = sabrage_core::stages::acquire_operation_lock().await;
    let rows: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let sink_rows = rows.clone();
    let sink: EventSink = Arc::new(move |ev: StageEvent| {
        if let Some(line) = reconcile_row_text(&ev) {
            sink_rows.lock().expect("rows mutex poisoned").push(line);
        }
    });
    // Checked: a reconcile restores the audio device, reaps a recorded
    // dashboard and clears the record — all mutations.
    let paths = Paths::new_checked(repo_root).map_err(|e| e.to_string())?;
    let ctx = StageCtx::new(paths, stage_opts, sink, Default::default());
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

/// The map [`TailRegistry`] keeps, behind an `Arc` so a clone can be moved
/// into the polling task's [`TailGuard`].
type TailMap = Arc<Mutex<HashMap<u64, Arc<AtomicBool>>>>;

/// Tracks in-flight [`start_log_tail`] pollers by an opaque id. Stopping one
/// flips its [`AtomicBool`] rather than aborting the task outright: the task
/// notices on its own next wake (at most [`LOG_TAIL_POLL_INTERVAL`] later) and
/// exits between polls instead of being cut off mid-read.
///
/// Every registration is paired with a [`TailGuard`] that is moved into the
/// polling task, so *every* way that task can end — the stop flag, a send
/// error, an unreadable file — removes its entry. Without it the map kept ids
/// whose task had already died and [`stop_log_tail`] answered `true` for them,
/// contradicting its own "no longer tracked" contract.
#[derive(Default)]
pub struct TailRegistry {
    next_id: AtomicU64,
    tails: TailMap,
}

/// Removes one tail's registry entry when its polling task ends, however it
/// ends. Holds no stop flag: dropping the guard means the task is already
/// gone, so there is nothing left to signal.
struct TailGuard {
    id: u64,
    tails: TailMap,
}

impl Drop for TailGuard {
    fn drop(&mut self) {
        if let Ok(mut map) = self.tails.lock() {
            map.remove(&self.id);
        }
    }
}

impl TailRegistry {
    fn register(&self, stop: Arc<AtomicBool>) -> (u64, TailGuard) {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        self.tails
            .lock()
            .expect("TailRegistry mutex poisoned")
            .insert(id, stop);
        (
            id,
            TailGuard {
                id,
                tails: self.tails.clone(),
            },
        )
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

    /// Stop every tracked tail. `lib.rs` calls this from the builder's
    /// `on_page_load` hook: a webview reload (vite HMR in dev, any
    /// navigation) runs no Svelte `onDestroy`, so the tails that page started
    /// would otherwise poll their files every 250 ms for the rest of the
    /// app's life — a `Channel::send` on macOS is a `webview.eval`, which
    /// keeps succeeding after a reload, so the task cannot notice on its own.
    pub(crate) fn stop_all(&self) {
        let entries: Vec<Arc<AtomicBool>> = self
            .tails
            .lock()
            .expect("TailRegistry mutex poisoned")
            .drain()
            .map(|(_, stop)| stop)
            .collect();
        for stop in entries {
            stop.store(true, Ordering::SeqCst);
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
    let repo_root = resolve_repo_root_via_settings().map_err(|e| e.to_string())?;
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
    let (id, guard) = registry.register(stop.clone());
    tauri::async_runtime::spawn_blocking(move || {
        // Moved in so every exit below — including the two `break`s — drops
        // it and unregisters this id exactly once.
        let _guard = guard;
        while !stop.load(Ordering::SeqCst) {
            match tailer.poll() {
                Ok(batch) if batch.rotated || batch.truncated || !batch.lines.is_empty() => {
                    // A send error means the channel is gone for good. It is
                    // NOT a reliable webview-reload signal: on macOS
                    // `Channel::send` is a `webview.eval`, which succeeds as
                    // long as the webview object exists — `TailRegistry::stop_all`
                    // from `lib.rs`'s `on_page_load` hook is what covers a
                    // reload.
                    if on_batch.send(batch).is_err() {
                        break;
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
    match resolve_repo_root_via_settings() {
        Ok(root) => sabrage_core::logs::list_past_runs(&Paths::new(root).logs_dir()),
        Err(_) => Vec::new(),
    }
}

/// Resolve `source` to a path on this machine, or `None` when nothing matches
/// yet (an empty `logs/` on a fresh checkout, no session ever run).
#[tauri::command]
pub fn get_log_source_path(source: LogSource) -> Option<String> {
    let repo_root = resolve_repo_root_via_settings().ok()?;
    let paths = Paths::new(repo_root);
    sabrage_core::logs::resolve_source(&paths, &source).map(|p| p.display().to_string())
}

// ── Phase 4: settings, library, runtime config ──────────────────────────────
//
// The eleven commands below back the Settings/Library/Edit-game screens.
// None streams over an IPC `Channel` — the brief's IPC contract table gives
// none of them an `on_event` parameter — so every mutation goes through a
// bare `RealExecutor` ([`real_executor`]) rather than a `StageCtx`: there is
// no multi-step stage here, nothing to cancel, and no live listener for a
// single small JSON/TOML write to stream to. Every mutation still goes
// through the `Executor` trait (the crate-wide rule), it just never needs the
// dry-run/stage machinery layered on top of it.
//
// `settings.json`/`library.json`/`oxrsys-runtime.toml` all live under paths
// derived from `$HOME` alone — [`sabrage_core::paths::sabrage_support_dir`]
// and `Paths::oxr_appsup`/`toml_path`, never from the repo root — so
// [`SettingsPathsCache`] tolerates an unresolved repo root (falling back to
// an empty one; `Paths::new` never fails and does no validation of it) rather
// than erroring out: the Settings/Library/Config screens must keep working
// before a wine-vr checkout is even configured. Only [`get_repo_info`] (and
// doctor/stage execution, unchanged above) reports whether the repo root
// itself actually resolved.

/// Load `settings.json`, tolerantly.
///
/// [`settings::load`] already treats a *missing* file as
/// [`settings::Settings::default`]; a file that fails to **parse** is `Err`
/// there on purpose (design-core §4.2: never a silent reset — the one screen
/// that exists to fix a corrupt settings file, [`get_settings`], must see the
/// error). Every OTHER command only wants `repo_root`/`default_bottle`/
/// `allow_adb_probes` etc. as *input*, and must not itself fail (or block a
/// launch) over a settings file some other bug corrupted — brief: "corrupt/
/// missing settings → None + tracing/eprintln, never a failure". This is that
/// degrade-and-log helper; [`get_settings`] deliberately does NOT use it.
fn load_settings() -> settings::Settings {
    let path = settings::settings_path(&sabrage_core::paths::sabrage_support_dir());
    match settings::load(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("sabrage: settings.json unreadable, using defaults ({e})");
            settings::Settings::default()
        }
    }
}

/// [`resolve_repo_root`], honoring `settings.json`'s persisted override — the
/// brief's "settings.repo_root plumbing into EVERY resolve_repo_root call in
/// commands.rs", and every prior bare `resolve_repo_root(None)` call site
/// above is now this. Built on [`load_settings`], so a corrupt settings file
/// degrades to `None` (the env/executable-walk precedence tiers still apply
/// underneath) rather than turning every command that resolves a repo root
/// into a hard failure.
fn resolve_repo_root_via_settings() -> std::result::Result<PathBuf, SabrageError> {
    resolve_repo_root(load_settings().repo_root.as_deref())
}

/// Cached `settings.json` + the [`Paths`] derived from it, for the handful of
/// Phase 4 commands (`read_runtime_config`, `write_runtime_config`,
/// `save_settings`, `get_repo_info`, `get_library`, `new_game_template`,
/// `save_game`, `remove_game`, `validate_game`, `revert_original_steam_dll`,
/// `launch`'s last-session recording) that only ever read `settings.json`
/// and the `$HOME`-derived halves of [`Paths`] (`sabrage_appsup`,
/// `oxr_appsup`/`toml_path`) — never the machine-probed halves (`cx_app`,
/// `wine`, `adb`, …), which stay live-probed by [`run_doctor`]/`get_app_state`
/// (unchanged, uncached — a doctor row or the sidebar footer must reflect a
/// CrossOver reinstall or a freshly plugged `adb` without a restart).
///
/// [`Paths::new`] re-probes the machine (three `stat`s and a `$PATH` walk)
/// and `settings::load` re-reads and re-parses `settings.json` on every call;
/// finding E-C3-settings-paths-cache measured a single Settings-screen mount
/// paying that cost four times over. `SessionMonitorState`'s
/// `Mutex<Option<...>>` is the precedent this follows: empty until first use,
/// filled on demand, dropped — not eagerly recomputed — by whatever
/// invalidates it. The only writer of `settings.json` through this app is
/// [`save_settings`], which calls [`SettingsPathsCache::invalidate`] after
/// every successful save; an edit from outside this app (or a bottle
/// added/removed on disk) can still leave this stale until the next save,
/// same caveat `SessionMonitorState` already accepts for its own
/// externally-changed case.
#[derive(Default)]
pub struct SettingsPathsCache(std::sync::Mutex<Option<(settings::Settings, Paths)>>);

impl SettingsPathsCache {
    /// The settings/paths pair, loading and probing once and caching the
    /// result until [`SettingsPathsCache::invalidate`] runs.
    /// [`settings::load`]'s parse errors are swallowed here exactly as
    /// [`load_settings`] (this cache's uncached equivalent) already does — a
    /// corrupt `settings.json` degrades to defaults, never a failure, for
    /// every command that reads through this cache. [`get_settings`]
    /// deliberately never uses this cache, so it keeps seeing the raw parse
    /// error.
    fn snapshot(&self) -> (settings::Settings, Paths) {
        let mut guard = self.0.lock().expect("settings/paths cache mutex poisoned");
        if let Some(cached) = guard.as_ref() {
            return cached.clone();
        }
        let settings = load_settings();
        let paths =
            Paths::new(resolve_repo_root(settings.repo_root.as_deref()).unwrap_or_default());
        let out = (settings, paths);
        *guard = Some(out.clone());
        out
    }

    /// [`load_settings`], through the cache.
    fn settings(&self) -> settings::Settings {
        self.snapshot().0
    }

    /// [`Paths::new`] (given the same `settings.repo_root`-or-empty
    /// resolution [`SettingsPathsCache::snapshot`] uses), through the cache.
    fn paths(&self) -> Paths {
        self.snapshot().1
    }

    /// [`Paths::new_checked`], through the cache: its
    /// only failure mode is a missing/empty/non-absolute `HOME`
    /// ([`sabrage_core::paths::home_dir_checked`]), a plain environment-variable
    /// read with no filesystem probing of its own — cheap enough to re-check
    /// on every call rather than caching its (extremely unlikely to change
    /// mid-process) outcome, so this still refuses exactly like
    /// [`Paths::new_checked`] would on a broken `HOME`.
    fn paths_checked(&self) -> std::result::Result<Paths, String> {
        sabrage_core::paths::home_dir_checked().map_err(|e| e.to_string())?;
        Ok(self.snapshot().1)
    }

    /// Drop the cached pair so the next read re-loads `settings.json` and
    /// re-probes the machine. Called by [`save_settings`] after every
    /// successful write — the only place this app changes `settings.json`.
    fn invalidate(&self) {
        *self.0.lock().expect("settings/paths cache mutex poisoned") = None;
    }
}

/// Serializes `settings.json` writes against each other.
///
/// The Settings screen autosaves one whole `Settings` object per control
/// change, so two rapid toggles can be in flight at once; without this the two
/// `write_atomic` renames land in an arbitrary order. This does **not** make
/// the UI's optimistic merge safe (that queue belongs in
/// `ui/src/stores/settings.svelte.ts`), it only makes the file's last writer
/// deterministic. Deliberately not [`sabrage_core::stages::OPERATION_LOCK`]: a
/// checkbox must not block behind a ten-minute build.
static SETTINGS_LOCK: LazyLock<tauri::async_runtime::Mutex<()>> =
    LazyLock::new(tauri::async_runtime::Mutex::default);

/// A [`RealExecutor`] for the small, non-stage mutations this section adds —
/// see the module note above for why no [`StageCtx`] applies. `run_id`/the
/// cancellation token are the same `Default::default()` trick this file's
/// "Why no `tokio_util` / `uuid` appear here" note (above,
/// [`RunRegistry`]'s section) already relies on to reach `Uuid`/
/// `CancellationToken` without either being a direct dependency of
/// `sabrage-app`; the sink is [`null_sink`], since none of these eleven
/// commands stream events.
fn real_executor() -> RealExecutor {
    RealExecutor::new(Default::default(), null_sink(), Default::default())
}

// ── runtime config (oxrsys-runtime.toml) ────────────────────────────────────

/// The Settings screen's one read: everything [`sabrage_core::config::read`]
/// already assembles. Never fails — an absent or unparseable file are both
/// *states of the view*, not errors (see that function's own doc comment).
#[tauri::command]
pub fn read_runtime_config(
    cache: State<'_, SettingsPathsCache>,
) -> runtime_config::RuntimeConfigView {
    runtime_config::read(&cache.paths().toml_path)
}

/// Patch `oxrsys-runtime.toml`'s six editable keys, creating it from the
/// shared template first if it does not exist yet
/// ([`runtime_config::write`]'s write-once-on-create rule).
///
/// Refuses **before** calling [`runtime_config::write`] when the file is one
/// Sabrage must not rewrite ([`runtime_config::RuntimeConfigView::parse_error`]):
/// either `toml_edit` cannot parse it at all, or its physical lines and the
/// parsed document disagree about where an editable key is assigned (a
/// `"""…"""` block containing `protocol = …`, a BOM in front of a key) — an
/// edit would then land somewhere the runtime's own line reader does not look.
/// `apply_patch` refuses on both counts too, so `write` is safe on its own;
/// checking here first only buys the message the *view* already computed,
/// without a redundant disk read. The two refusals must therefore stay
/// equivalent — `parse_error` and `apply_patch` share `line_document_mismatch`
/// precisely so they cannot drift.
///
/// Two guards this command did not use to have:
///
/// * **a live session refuses the write** ([`sabrage_core::session::ensure_idle_in`],
///   the same policy `runtime_config::write` enforces on its own). The old
///   note here — "the runtime only reads this file at the next game start" —
///   was wrong: `Config.cpp` re-reads it every 250 ms and the ALVR frame path
///   rebuilds the encoder when `encoder_process`/`video_codec` move, so a
///   Settings save mid-stream is a live reconfiguration. Checked here as well
///   as in the core so the error text and the remedy come from one place, and
///   so nothing is written (not even the backup) on the way to the refusal.
/// * **the operation lock** ([`sabrage_core::stages::acquire_operation_lock`]),
///   held across the whole read-modify-write. `stages::setup` independently
///   checks `oxrsys-runtime.toml`'s existence and writes the template while
///   holding that same lock, so without this a `setup` running concurrently
///   could overwrite a patch that had just reported success — the write-once
///   contract in reverse. It also serializes this against the `edit-protocol`
///   fix, which re-enters the same writer.
#[tauri::command]
pub async fn write_runtime_config(
    patch: runtime_config::RuntimeConfigPatch,
    cache: State<'_, SettingsPathsCache>,
) -> Result<runtime_config::WriteReport, String> {
    let paths = cache.paths_checked()?;
    let view = runtime_config::read(&paths.toml_path);
    if let Some(err) = view.parse_error {
        // Not "is not valid TOML": the multiline-string case parses fine, it
        // just cannot be edited safely. `err` is the specific diagnosis.
        return Err(format!(
            "{}: {err} — refusing to rewrite it; edit it by hand",
            paths.toml_path.display()
        ));
    }
    sabrage_core::session::ensure_idle_in(&paths, "edit oxrsys-runtime.toml")
        .map_err(|e| e.to_string())?;
    let _op = sabrage_core::stages::acquire_operation_lock().await;
    runtime_config::write(
        &real_executor(),
        &paths.toml_path,
        &paths.sabrage_appsup.join("backups"),
        &patch,
    )
    .await
    .map_err(|e| e.to_string())
}

// ── settings.json ────────────────────────────────────────────────────────────

/// Read `settings.json` — unlike [`load_settings`], a corrupt file is
/// surfaced as `Err` rather than degraded to defaults: this is the one
/// screen that exists to let the user see and fix it.
#[tauri::command]
pub fn get_settings() -> Result<settings::Settings, String> {
    let path = settings::settings_path(&sabrage_core::paths::sabrage_support_dir());
    settings::load(&path).map_err(|e| e.to_string())
}

/// Persist `settings.json`, returning it back as-saved (the brief's `Settings`
/// return — there is nothing this side derives beyond what was sent).
#[tauri::command]
pub async fn save_settings(
    settings: settings::Settings,
    cache: State<'_, SettingsPathsCache>,
) -> Result<settings::Settings, String> {
    let paths = cache.paths_checked()?;
    let path = settings::settings_path(&paths.sabrage_appsup);
    let _lock = SETTINGS_LOCK.lock().await;
    settings::save(&real_executor(), &path, &settings)
        .await
        .map_err(|e| e.to_string())?;
    // The only writer of `settings.json` through this app — every other
    // command's cached (settings, Paths) pair is now stale.
    cache.invalidate();
    Ok(settings)
}

// ── repo info ─────────────────────────────────────────────────────────────────

/// Which precedence tier of [`resolve_repo_root`] actually supplied a
/// resolved root. `explicit_present`/`env_present` are "was that tier's input
/// non-empty" (the explicit-override and env tiers of `resolve_repo_root`
/// never themselves fail — only the executable-walk tier can, hence
/// `walk_succeeded` rather than a third `bool`); factored out as a pure
/// function of those three facts so the precedence rule is unit-testable
/// without a real settings file, a real `SABRAGE_REPO_ROOT`, or a real
/// executable path.
fn classify_repo_root_source(
    explicit_present: bool,
    env_present: bool,
    walk_succeeded: bool,
) -> RepoRootSource {
    if explicit_present {
        RepoRootSource::Settings
    } else if env_present {
        RepoRootSource::Env
    } else if walk_succeeded {
        RepoRootSource::Executable
    } else {
        RepoRootSource::Unresolved
    }
}

/// Which precedence tier of [`resolve_repo_root`] supplied
/// [`RepoInfo::repo_root`] — see [`classify_repo_root_source`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RepoRootSource {
    Settings,
    Env,
    Executable,
    Unresolved,
}

/// The Settings screen's "Repository" card: where the repo root came from,
/// whether it looks like a real checkout, whether `contract/` is in sync
/// with the generated shell mirror, and where the root-owned host OpenXR
/// manifest currently points.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoInfo {
    pub repo_root: Option<String>,
    pub source: RepoRootSource,
    pub markers_present: bool,
    pub contract_synced: Option<bool>,
    pub host_manifest_library_path: Option<String>,
    pub host_manifest_points_here: Option<bool>,
    /// The `contract/` digest **this binary was compiled with**
    /// ([`sabrage_core::contract::COMPILED_CONTRACT_SHA256`]) — the pins,
    /// ports, artifact lists and check registry it will actually execute.
    pub compiled_contract_sha256: String,
    /// The same digest recomputed from the resolved checkout's `contract/`
    /// files. `None` when there is no root to hash.
    pub checkout_contract_sha256: Option<String>,
    /// Do the two agree? `false` means this Sabrage build is executing a
    /// different contract than the checkout it is pointed at — an installed
    /// binary aimed at a second checkout, or a stale build — and `contract/`
    /// being internally in sync (`contract_synced`) says nothing about it.
    /// `None` when there is no root to compare against.
    pub binary_contract_matches: Option<bool>,
}

/// Where a Browse… folder picker should start for a Beat Saber dir field, and
/// the path demo.sh would derive for the bottle when the field is left empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BsDirSuggestion {
    /// lib.sh's `${WINEVR_BS_DIR:-$PREFIX/drive_c/Program Files (x86)/Steam/steamapps/common/<leaf>}`
    /// for `bottle`; empty when no bottle is named (the shell's bottle-less
    /// fallback is a quirk, not a suggestion).
    pub derived: String,
    /// The nearest **existing** directory at or above `current` (when given),
    /// else at or above `derived`, else `$HOME` — an NSOpenPanel handed a
    /// path that does not exist just opens wherever it last was, which is
    /// how "Browse… doesn't start in the bottle" looked to the user.
    pub browse_start: String,
}

/// First ancestor-or-self of `p` that is a directory, if any.
fn nearest_existing_dir(p: &Path) -> Option<PathBuf> {
    p.ancestors().find(|a| a.is_dir()).map(Path::to_path_buf)
}

/// Pure body of [`suggest_bs_dir`]; `exists` is injected so the precedence is
/// testable without real directories.
fn suggest_bs_dir_with(
    bottle: Option<&str>,
    current: Option<&str>,
    home: &Path,
    exists: impl Fn(&Path) -> Option<PathBuf>,
) -> BsDirSuggestion {
    let derived = bottle
        .filter(|b| !b.is_empty())
        .map(|b| {
            sabrage_core::paths::resolve_bs_dir(Some(&sabrage_core::Bottle::unvalidated(b)), None)
                .display()
                .to_string()
        })
        .unwrap_or_default();
    let candidates = [
        current.filter(|c| !c.trim().is_empty()),
        Some(derived.as_str()).filter(|d| !d.is_empty()),
    ];
    let browse_start = candidates
        .into_iter()
        .flatten()
        .find_map(|c| exists(Path::new(c)))
        .unwrap_or_else(|| home.to_path_buf());
    BsDirSuggestion {
        derived,
        browse_start: browse_start.display().to_string(),
    }
}

/// Settings' and Edit-game's Browse… start directory + the empty-field
/// placeholder for a Beat Saber dir field. Read-only.
#[tauri::command]
pub fn suggest_bs_dir(bottle: Option<String>, current: Option<String>) -> BsDirSuggestion {
    suggest_bs_dir_with(
        bottle.as_deref(),
        current.as_deref(),
        &sabrage_core::paths::home_dir(),
        nearest_existing_dir,
    )
}

#[tauri::command]
pub fn get_repo_info(cache: State<'_, SettingsPathsCache>) -> RepoInfo {
    let settings = cache.settings();
    let explicit = settings.repo_root.as_deref().filter(|s| !s.is_empty());
    let env_present = std::env::var(sabrage_core::paths::REPO_ROOT_ENV)
        .ok()
        .filter(|s| !s.is_empty())
        .is_some();
    let resolved = resolve_repo_root(explicit);
    let source = classify_repo_root_source(explicit.is_some(), env_present, resolved.is_ok());
    let repo_root = resolved.ok();

    let markers_present = repo_root
        .as_ref()
        .map(|r| {
            sabrage_core::paths::REPO_ROOT_MARKERS
                .iter()
                .all(|m| r.join(m).is_file())
        })
        .unwrap_or(false);

    // meta.contract-sync's own recipe (`checks::meta`), reapplied here for
    // explainability rather than imported (that evaluator returns a
    // `CheckOutcome`, not a bool) — `None` when there is no root to hash.
    let contract_synced = repo_root.as_ref().map(|root| {
        let have = util::contract_gen_recorded_hash(root);
        let want = util::contract_hash(root).ok();
        matches!((&have, &want), (Some(h), Some(w)) if !h.is_empty() && h == w)
    });

    let host_json = PathBuf::from(&contract().paths.host_xr_json);
    let host_manifest_library_path = host::host_manifest_library_path(&host_json);

    let host_manifest_points_here = match (&repo_root, &host_manifest_library_path) {
        (Some(root), Some(lp)) => {
            let prefix = format!("{}/", root.display());
            Some(lp.starts_with(&prefix))
        }
        _ => None,
    };

    // The binary's own compiled contract vs. the checkout's. `contract_synced`
    // above answers a different question (is `contract.gen.sh` a fresh
    // rendering of `contract/`), and a checkout can pass that while this
    // binary still executes an entirely different contract.
    let compiled_contract_sha256 = (*sabrage_core::contract::COMPILED_CONTRACT_SHA256).clone();
    let checkout_contract_sha256 = repo_root
        .as_ref()
        .and_then(|root| util::contract_hash(root).ok());
    let binary_contract_matches = checkout_contract_sha256
        .as_ref()
        .map(|h| *h == compiled_contract_sha256);

    RepoInfo {
        repo_root: repo_root.map(|p| p.display().to_string()),
        source,
        markers_present,
        contract_synced,
        host_manifest_library_path,
        host_manifest_points_here,
        compiled_contract_sha256,
        checkout_contract_sha256,
        binary_contract_matches,
    }
}

// ── library.json ─────────────────────────────────────────────────────────────

/// One row of the Library screen: a stored [`library::GameEntry`] plus a
/// fresh [`library::GameValidity`] snapshot — recomputed on every
/// [`get_library`]/[`save_game`] call, never itself persisted, because the
/// machine can change under a stored entry at any time (design-app §4).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameRow {
    pub entry: library::GameEntry,
    pub validity: library::GameValidity,
}

fn game_row(paths: &Paths, entry: library::GameEntry) -> GameRow {
    let validity = library::validate(paths, Path::new(&entry.bs_dir), &entry.bottle);
    GameRow { entry, validity }
}

/// Every library entry, each with a freshly computed [`library::GameValidity`].
#[tauri::command]
pub fn get_library(cache: State<'_, SettingsPathsCache>) -> Result<Vec<GameRow>, String> {
    let paths = cache.paths();
    let lib_path = library::library_path(&paths.sabrage_appsup);
    let lib = library::load(&lib_path).map_err(|e| e.to_string())?;
    Ok(lib
        .games
        .into_iter()
        .map(|entry| game_row(&paths, entry))
        .collect())
}

/// A fresh, unsaved [`library::GameEntry`] for the Add-game wizard, seeded
/// from `settings.json` and the bottles on this machine.
#[tauri::command]
pub fn new_game_template(cache: State<'_, SettingsPathsCache>) -> library::GameEntry {
    let settings = cache.settings();
    let bottles = sabrage_core::paths::list_bottles();
    let env_bs_dir = std::env::var("WINEVR_BS_DIR").ok();
    library::new_entry_template(&settings, &bottles, env_bs_dir.as_deref())
}

/// Upsert `entry` into `library.json` (by [`library::GameEntry::id`]) and
/// return it back as the row the Library screen would now render.
///
/// Two things this does *not* do, both of them lost-update shapes:
///
/// * it does not load and save around its own mutation — the whole
///   read-modify-write runs inside [`library::transact`], so a concurrent
///   [`remove_game`] or post-launch [`record_last_session`] cannot be
///   overwritten by whichever whole-file rename happens to land last;
/// * it does not write the *whole* incoming entry —
///   [`library::Library::upsert_editable`] keeps the server-owned fields
///   (`last_session`, `added_at_unix_ms`, `appid`) from the stored row, so an
///   Edit-game form that was opened before a session ended cannot delete that
///   session when it is finally saved.
#[tauri::command]
pub async fn save_game(
    entry: library::GameEntry,
    cache: State<'_, SettingsPathsCache>,
) -> Result<GameRow, String> {
    let paths = cache.paths_checked()?;
    let lib_path = library::library_path(&paths.sabrage_appsup);
    let saved = library::transact(&real_executor(), &lib_path, |lib| {
        lib.upsert_editable(entry).clone()
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(game_row(&paths, saved))
}

/// Remove the entry named `id` from `library.json`. `false` (never an error)
/// when no entry with that id exists — matches [`library::Library::remove`]'s
/// own "already gone is not a failure" contract.
#[tauri::command]
pub async fn remove_game(id: String, cache: State<'_, SettingsPathsCache>) -> Result<bool, String> {
    let target = id
        .parse()
        .map_err(|e| format!("{id:?} is not a valid game id: {e}"))?;
    let paths = cache.paths_checked()?;
    let lib_path = library::library_path(&paths.sabrage_appsup);
    // `transact` writes only when the closure actually changed the library,
    // so a removal that found nothing still mints no `library.json`.
    library::transact(&real_executor(), &lib_path, |lib| lib.remove(target))
        .await
        .map_err(|e| e.to_string())
}

/// Read-only install-health probes over one `(bs_dir, bottle)` pair — the
/// Edit-game screen's inline validation, and what [`get_library`]/
/// [`save_game`] also compute per row.
#[tauri::command]
pub fn validate_game(
    bs_dir: String,
    bottle: String,
    cache: State<'_, SettingsPathsCache>,
) -> library::GameValidity {
    library::validate(&cache.paths(), Path::new(&bs_dir), &bottle)
}

/// Restore the real Steam `steam_api64.dll` over the entry named `game_id`'s
/// installed Goldberg one — [`goldberg::revert_original_steam_dll`], looked
/// up by library entry rather than a bare `bs_dir` so the Edit-game screen's
/// button needs only the game id it already has.
/// `expected_bs_dir`, when supplied, must equal the entry's **persisted**
/// `bs_dir`. The Edit-game form validates the path currently typed into it but
/// this command mutates the saved one, so an unsaved path edit followed by
/// Revert would overwrite `steam_api64.dll` in a different installation than
/// the screen displayed and validated. Callers that render a path pass it and
/// fail closed; the argument is optional only so an older frontend keeps
/// working.
///
/// Deliberately does **not** take [`sabrage_core::stages::OPERATION_LOCK`]:
/// [`goldberg::revert_original_steam_dll`] takes it itself (and holds it
/// across its own liveness re-check and the copy — the point of taking it),
/// and `OPERATION_LOCK` is not reentrant.
#[tauri::command]
pub async fn revert_original_steam_dll(
    game_id: String,
    expected_bs_dir: Option<String>,
    cache: State<'_, SettingsPathsCache>,
) -> Result<goldberg::RevertReport, String> {
    let target = game_id
        .parse()
        .map_err(|e| format!("{game_id:?} is not a valid game id: {e}"))?;
    let paths = cache.paths_checked()?;
    let lib_path = library::library_path(&paths.sabrage_appsup);
    let lib = library::load(&lib_path).map_err(|e| e.to_string())?;
    let entry = lib
        .get(target)
        .ok_or_else(|| format!("no game with id {game_id} in the library"))?;
    if let Some(expected) = expected_bs_dir.as_deref() {
        if expected != entry.bs_dir {
            return Err(format!(
                "this game's saved Beat Saber directory is {}, not {} — save your path change \
                 first, then revert",
                entry.bs_dir, expected
            ));
        }
    }
    goldberg::revert_original_steam_dll(&real_executor(), Path::new(&entry.bs_dir))
        .await
        .map_err(|e| e.to_string())
}

// ── last-session recording (launch) ──────────────────────────────────────────

/// What [`launch`]'s tapped sink ([`channel_sink_tee`]) captured from a
/// `StageEvent::Launched`, if the run got that far.
#[derive(Debug, Clone)]
struct LaunchedInfo {
    started_at_unix_ms: u64,
    log_path: String,
}

/// [`launch`]'s pure decision of whether (and what) to record into
/// `library.json` once its stage settles — factored out of the command body
/// so the rule is unit-testable without an async runtime, a `Channel`, or a
/// real stage run. Recording happens only when all three hold: the GUI
/// passed a `gameId`, a `StageEvent::Launched` was actually observed for this
/// run, and the stage settled with `Ok(outcome)` (an `Err` — repo-root
/// resolution failing before a `StageCtx` could even exist — never reaches
/// `Launched` in the first place, but there is then no `exit_code_equiv` to
/// record either way).
fn last_session_to_record(
    game_id: Option<&str>,
    launched: Option<&LaunchedInfo>,
    outcome: Option<&StageOutcome>,
) -> Option<(String, library::LastSession)> {
    let game_id = game_id?;
    let launched = launched?;
    let outcome = outcome?;
    Some((
        game_id.to_string(),
        library::LastSession {
            started_at_unix_ms: launched.started_at_unix_ms,
            ended_at_unix_ms: sabrage_core::session::now_unix_ms(),
            exit_code: Some(outcome.exit_code_equiv),
            log_path: Some(launched.log_path.clone()),
        },
    ))
}

/// Persist [`last_session_to_record`]'s decision into `library.json`.
/// Best-effort: a game removed between launch and exit, a corrupt library
/// file, or a failed write must not turn an otherwise-settled `launch`
/// promise into a rejected one — logged instead, the same tolerance
/// [`load_settings`] already applies to a corrupt settings file.
async fn record_last_session(
    game_id: &str,
    session: library::LastSession,
    cache: &SettingsPathsCache,
) {
    let Ok(target) = game_id.parse() else {
        eprintln!("sabrage: launch carried an unparseable gameId {game_id:?}; not recording");
        return;
    };
    let paths = match cache.paths_checked() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("sabrage: not recording last session ({e})");
            return;
        }
    };
    let lib_path = library::library_path(&paths.sabrage_appsup);
    match library::transact(&real_executor(), &lib_path, |lib| {
        lib.record_last_session(target, session)
    })
    .await
    {
        Ok(true) => {}
        Ok(false) => {
            eprintln!(
                "sabrage: game {game_id} no longer in the library; not recording last session"
            );
        }
        Err(e) => eprintln!("sabrage: failed to save library.json after launch ({e})"),
    }
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
    fn browse_start_prefers_the_field_then_the_bottle_derived_dir_then_home() {
        let home = Path::new("/Users/me");
        let existing = |p: &Path| -> Option<PathBuf> {
            // Only /Volumes/Games and the bottle's drive_c "exist".
            p.ancestors()
                .find(|a| {
                    *a == Path::new("/Volumes/Games")
                        || a.to_string_lossy().ends_with("/Bottles/Steam/drive_c")
                })
                .map(Path::to_path_buf)
        };
        // Field set → its nearest existing ancestor wins even with a bottle.
        let s = suggest_bs_dir_with(
            Some("Steam"),
            Some("/Volumes/Games/Beat Saber 1294"),
            home,
            existing,
        );
        assert_eq!(s.browse_start, "/Volumes/Games");
        assert!(s.derived.ends_with(
            "/Bottles/Steam/drive_c/Program Files (x86)/Steam/steamapps/common/Beat Saber 1294"
        ));
        // Empty field → the bottle-derived dir's nearest existing ancestor.
        let s = suggest_bs_dir_with(Some("Steam"), Some("   "), home, existing);
        assert!(
            s.browse_start.ends_with("/Bottles/Steam/drive_c"),
            "{}",
            s.browse_start
        );
        // No bottle, nothing existing → $HOME, and no derived path at all.
        let s = suggest_bs_dir_with(None, None, home, |_| None);
        assert_eq!(s.derived, "");
        assert_eq!(s.browse_start, "/Users/me");
        let s = suggest_bs_dir_with(Some(""), Some(""), home, |_| None);
        assert_eq!(s.derived, "");
    }

    #[test]
    fn settings_defaults_fill_only_what_env_and_gui_left_unset() {
        let settings = settings::Settings {
            default_bottle: Some("Steam".to_string()),
            default_bs_dir: Some("/Volumes/Games/Beat Saber 1294".to_string()),
            ..settings::Settings::default()
        };
        // Nothing set anywhere else → both defaults apply.
        let filled = fill_stage_options_from_settings(StageOptions::default(), &settings);
        assert_eq!(filled.bottle_name.as_deref(), Some("Steam"));
        assert_eq!(
            filled.bs_dir_override.as_deref(),
            Some(Path::new("/Volumes/Games/Beat Saber 1294"))
        );
        // An env/GUI-supplied value is never overridden by a default.
        let preset = StageOptions {
            bottle_name: Some("VR".to_string()),
            bs_dir_override: Some(PathBuf::from("/elsewhere")),
            ..StageOptions::default()
        };
        let kept = fill_stage_options_from_settings(preset.clone(), &settings);
        assert_eq!(kept, preset);
        // Empty strings in settings.json count as unset, not as "" paths.
        let blank = settings::Settings {
            default_bottle: Some(String::new()),
            default_bs_dir: Some(String::new()),
            ..settings::Settings::default()
        };
        let untouched = fill_stage_options_from_settings(StageOptions::default(), &blank);
        assert_eq!(untouched, StageOptions::default());
    }

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
    fn wait_for_slot_clear_reports_the_deadline_it_hit() {
        // A11-1: the wait used to return `()`, so "teardown finished" and
        // "teardown is still running 30 s later" were the same answer — and
        // `stop_session` reported `ok: true` for both.
        let flips = Arc::new(AtomicU64::new(0));
        let f = flips.clone();
        assert_eq!(
            wait_for_slot_clear(
                move || f.fetch_add(1, Ordering::SeqCst) >= 2,
                Duration::from_secs(5),
            ),
            TeardownWait::Cleared,
            "a predicate that flips mid-poll is a completed teardown"
        );
        assert!(flips.load(Ordering::SeqCst) >= 3);

        assert_eq!(
            wait_for_slot_clear(|| false, Duration::from_millis(0)),
            TeardownWait::TimedOut,
            "a slot that never clears must report the timeout, not success"
        );
    }

    #[test]
    fn the_stop_and_quit_refusal_claims_only_what_happened() {
        // A11-1 (round 2): the `TimedOut` arm used to say "Sabrage detached
        // from it and quit" and call `detach_live_session()` to back that up
        // — but the stop has already fired the session's cancel token, and
        // `reconcile::detach` returns `Ok(())` without detaching once it is
        // set. The claim was unbackable; the message must not make it.
        assert_eq!(quit_stop_refusal(TeardownWait::NothingLive), None);
        assert_eq!(
            quit_stop_refusal(TeardownWait::Cleared),
            None,
            "a clean teardown is a silent, successful quit"
        );
        let timed_out = quit_stop_refusal(TeardownWait::TimedOut).expect("an unfinished teardown");
        assert!(
            !timed_out.contains("detach"),
            "must not claim a detach that provably did not happen: {timed_out}"
        );
        assert!(
            timed_out.contains("may still be running") && timed_out.contains("./demo.sh stop"),
            "must still say what is left and how to finish it: {timed_out}"
        );
        let detached = quit_stop_refusal(TeardownWait::Detached).expect("a detach is not a stop");
        assert!(
            detached.contains("detached instead of stopping"),
            "the one arm where a detach really did happen still names it: {detached}"
        );
    }

    #[test]
    fn a_withheld_fix_reaches_no_doctor_row_and_no_fix_call() {
        // A4-2 / A12-1: `DEFERRED_CONTRACT_FIX_IDS` was enforced only in
        // `FixAction::from_contract_id`, which the GUI mirrored by hand in
        // TypeScript — so `cfg.session-pins` still rendered a Fix button for
        // the known-bad `delete-session-json`. Both IPC doors now refuse it.
        for id in sabrage_core::fixes::DEFERRED_CONTRACT_FIX_IDS {
            assert_eq!(
                offered_fix_id(Some(id)),
                None,
                "{id} is withheld and must never reach the client"
            );
        }
        assert_eq!(offered_fix_id(None), None);
        assert_eq!(
            offered_fix_id(Some("fix.set-graphics-backend")),
            Some("fix.set-graphics-backend".to_string()),
            "an offered id passes through unchanged"
        );
        assert_eq!(
            offered_fix_id(Some("fix.not-a-real-fix")),
            None,
            "an id no FixAction models is not offerable either"
        );

        // The contract's own `cfg.session-pins` row is the reachable case.
        let session_pins = contract().check("cfg.session-pins").expect("slug present");
        assert_eq!(
            session_pins.fix.as_deref(),
            Some("fix.delete-session-json"),
            "the contract still names the remedy; withholding it is Sabrage's decision"
        );
        assert_eq!(offered_fix_id(session_pins.fix.as_deref()), None);

        // …and the `fix` command refuses it even if a frontend asks anyway.
        let refusal = gui_fix_refusal(FixAction::DeleteSessionJson, true)
            .expect("a withheld fix is refused however it was reached");
        assert!(
            refusal.contains("not offered by this build"),
            "unexpected refusal: {refusal}"
        );
        assert!(
            gui_fix_refusal(FixAction::RestageHelper, false).is_none(),
            "an ordinary fix needs neither confirmation nor an exemption"
        );
    }

    #[test]
    fn quit_is_intercepted_once_and_given_up_on_when_nobody_answers() {
        // A11-4: the dialog has exactly one responder (the webview's
        // `app://quit-requested` listener). A webview that is reloading,
        // crashed, or died before subscribing never answers, and without a
        // bound every Cmd-Q and window close stays prevented forever.
        assert_eq!(
            quit_intercept_decision(false, true, None),
            QuitIntercept::Ask,
            "the first request over a live session opens the dialog"
        );
        assert_eq!(
            quit_intercept_decision(false, true, Some(Duration::from_secs(1))),
            QuitIntercept::Ask,
            "a dialog the user is still reading keeps being asked"
        );
        assert_eq!(
            quit_intercept_decision(false, true, Some(QUIT_DIALOG_TIMEOUT)),
            QuitIntercept::GiveUp,
            "asking again after the deadline stops asking — the app must stay quittable"
        );
        assert_eq!(
            quit_intercept_decision(true, true, Some(Duration::from_secs(600))),
            QuitIntercept::PassThrough,
            "an approved quit is never intercepted, pending clock or not"
        );
        assert_eq!(
            quit_intercept_decision(false, false, Some(Duration::from_secs(600))),
            QuitIntercept::PassThrough,
            "nothing live to protect"
        );
        assert_eq!(
            quit_intercept_decision(true, false, None),
            QuitIntercept::PassThrough,
            "an approved quit with nothing live is never intercepted"
        );
    }

    #[test]
    fn pending_quit_keeps_the_first_instant_and_clears() {
        let pending = PendingQuit::default();
        assert_eq!(pending.pending_for(), None);
        pending.mark();
        let first = pending.pending_for().expect("marked");
        pending.mark();
        assert!(
            pending.pending_for().expect("still marked") >= first,
            "a repeated request must not refresh the deadline it is meant to trip"
        );
        pending.clear();
        assert_eq!(pending.pending_for(), None);
    }

    #[test]
    fn a_tail_unregisters_itself_when_its_task_ends() {
        // A11-5: only `stop_log_tail` used to remove entries, so a tail that
        // died on a send/poll error stayed in the map and `stop_log_tail`
        // answered `true` for a task that no longer existed.
        let registry = TailRegistry::default();
        let stop = Arc::new(AtomicBool::new(false));
        let (id, guard) = registry.register(stop.clone());
        assert!(registry.stop(id), "a live tail is tracked");
        assert!(
            stop.load(Ordering::SeqCst),
            "stopping flips the task's flag"
        );
        drop(guard);

        // A second tail whose task ends on its own (the guard drops) must
        // leave nothing behind.
        let stop2 = Arc::new(AtomicBool::new(false));
        let (id2, guard2) = registry.register(stop2.clone());
        drop(guard2);
        assert!(
            !registry.stop(id2),
            "a tail whose task has ended is no longer tracked"
        );
        assert!(
            !stop2.load(Ordering::SeqCst),
            "and nothing is signalled — the task is already gone"
        );
    }

    #[test]
    fn stop_all_stops_every_tracked_tail() {
        let registry = TailRegistry::default();
        let flags: Vec<Arc<AtomicBool>> =
            (0..3).map(|_| Arc::new(AtomicBool::new(false))).collect();
        let guards: Vec<TailGuard> = flags
            .iter()
            .map(|f| registry.register(f.clone()).1)
            .collect();
        registry.stop_all();
        assert!(flags.iter().all(|f| f.load(Ordering::SeqCst)));
        // Every id is gone, so a later `stop_log_tail` answers honestly.
        for g in &guards {
            assert!(!registry.stop(g.id));
        }
        drop(guards);
    }

    #[test]
    fn the_status_broadcast_skips_repeats_but_never_the_first_one() {
        // /simplify E-A11: the 1 Hz broadcast emitted unconditionally, and the
        // "the frontend store dedups" claim in its doc comment was false.
        let state = SessionMonitorState::default();
        let idle = SessionStatus::default();
        assert!(
            state.remember_broadcast(&idle),
            "the first one always emits"
        );
        assert!(
            !state.remember_broadcast(&idle),
            "an identical snapshot does not"
        );
        let changed = SessionStatus {
            phase: sabrage_core::session::SessionPhase::Running,
            ..SessionStatus::default()
        };
        assert!(state.remember_broadcast(&changed));
        assert!(!state.remember_broadcast(&changed));
        assert!(state.remember_broadcast(&idle), "and back again");
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

    // ── Phase 4 ───────────────────────────────────────────────────────────
    //
    // `sabrage-app` carries no JSON-format crate as a direct dependency
    // (`Cargo.toml` is off-limits to this agent, and only `sabrage-core`
    // depends on `serde_json` — see that crate's own store/config tests for
    // the wire-format round trips), so these exercise the pure decision
    // functions and helpers at the Rust level rather than a serialized-JSON
    // comparison.
    // Every new `#[serde(rename_all = "camelCase")]`/`"lowercase"` attribute
    // below follows the exact pattern already established on every sibling
    // struct/enum in this file (`AppState`, `DoctorEvent`, `FixAction`, …).

    #[test]
    fn classify_repo_root_source_follows_resolve_repo_roots_own_precedence() {
        // Finding: `resolve_repo_root`'s explicit-override and env tiers
        // never themselves fail (`paths.rs`'s own doc comment) — an explicit
        // setting wins regardless of whether the walk would also have
        // succeeded, and likewise for env over the walk.
        assert_eq!(
            classify_repo_root_source(true, true, true),
            RepoRootSource::Settings
        );
        assert_eq!(
            classify_repo_root_source(true, false, false),
            RepoRootSource::Settings
        );
        assert_eq!(
            classify_repo_root_source(false, true, true),
            RepoRootSource::Env
        );
        assert_eq!(
            classify_repo_root_source(false, true, false),
            RepoRootSource::Env
        );
        assert_eq!(
            classify_repo_root_source(false, false, true),
            RepoRootSource::Executable
        );
        assert_eq!(
            classify_repo_root_source(false, false, false),
            RepoRootSource::Unresolved
        );
    }

    #[test]
    fn game_row_reflects_a_freshly_computed_validity_not_a_stored_one() {
        // Exercises the private `game_row` helper directly: a `GameEntry`
        // pointing at a bs_dir/bottle that do not exist on disk must come
        // back `NotFound`, regardless of whatever the entry itself claims —
        // `get_library`/`save_game`'s whole point is that validity is never
        // trusted from the stored JSON.
        let paths = Paths::new("/nonexistent/sabrage-commands-game-row-test");
        let entry = library::GameEntry {
            name: "Beat Saber 1.29.4".to_string(),
            bs_dir: "/nonexistent/sabrage-commands-game-row-test/bs".to_string(),
            bottle: "NoSuchBottle".to_string(),
            ..library::GameEntry::default()
        };
        let row = game_row(&paths, entry.clone());
        assert_eq!(row.entry, entry);
        assert_eq!(row.validity.status, library::GameStatus::NotFound);
        assert!(!row.validity.exe_present);
    }

    // ── E-C3-settings-paths-cache ────────────────────────────────────────────

    #[test]
    fn cache_hit_returns_the_stored_pair_without_reloading() {
        // Seeds the cache directly (rather than through `snapshot`'s
        // load-on-miss path, which would touch the real `settings.json`) so
        // this stays hermetic: a populated cache must serve exactly what was
        // stored, proving `snapshot`'s early return is reached before any
        // reload.
        let settings = settings::Settings {
            default_bottle: Some("Steam".to_string()),
            ..Default::default()
        };
        let paths = Paths::new("/nonexistent/sabrage-settings-cache-test");
        let cache = SettingsPathsCache(std::sync::Mutex::new(Some((
            settings.clone(),
            paths.clone(),
        ))));
        assert_eq!(cache.settings(), settings);
        assert_eq!(cache.paths(), paths);
    }

    #[test]
    fn invalidate_drops_the_cached_pair() {
        let paths = Paths::new("/nonexistent/sabrage-settings-cache-test-2");
        let cache = SettingsPathsCache(std::sync::Mutex::new(Some((
            settings::Settings::default(),
            paths,
        ))));
        cache.invalidate();
        assert!(
            cache.0.lock().unwrap().is_none(),
            "save_settings's invalidate() must leave the next read to reload, \
             not keep serving what a just-completed save made stale"
        );
    }

    #[test]
    fn last_session_to_record_needs_a_game_id_a_launched_event_and_a_settled_outcome() {
        let launched = LaunchedInfo {
            started_at_unix_ms: 1_000,
            log_path: "logs/beatsaber-x.log".to_string(),
        };
        let outcome = StageOutcome {
            stage: Stage::Run,
            ok: true,
            exit_code_equiv: 0,
        };

        assert!(
            last_session_to_record(None, Some(&launched), Some(&outcome)).is_none(),
            "no gameId -> nothing to record (an ad hoc Session-screen launch)"
        );
        assert!(
            last_session_to_record(Some("abc"), None, Some(&outcome)).is_none(),
            "no Launched event observed -> nothing to record (died in preflight)"
        );
        assert!(
            last_session_to_record(Some("abc"), Some(&launched), None).is_none(),
            "no settled outcome -> nothing to record (no exit_code_equiv to attach)"
        );

        let (game_id, session) =
            last_session_to_record(Some("abc"), Some(&launched), Some(&outcome))
                .expect("all three present");
        assert_eq!(game_id, "abc");
        assert_eq!(session.started_at_unix_ms, 1_000);
        assert_eq!(session.exit_code, Some(0));
        assert_eq!(session.log_path.as_deref(), Some("logs/beatsaber-x.log"));
        assert!(
            session.ended_at_unix_ms >= session.started_at_unix_ms,
            "ended_at is \"now\", which is after the fixed started_at"
        );
    }
}
