//! Stage orchestration: the context every stage runs in, the operation lock, and
//! the dispatcher. One stage is one `./demo.sh <verb>`, a plain `async fn`
//! readable next to the script it mirrors.
//!
//! [`OPERATION_LOCK`] plus an advisory file lock ([`OPERATION_LOCK_FILE_NAME`])
//! admit one mutating operation at a time across every Sabrage process; doctor is
//! read-only and never takes it, only annotates with [`operation_in_progress`].
//! demo.sh does not participate (PARITY.md § Declared by the 2026-08-30
//! adversarial review (round 1 fixes), "Cross-process operation lock.").
//!
//! `setup`/`build`/`install` refuse while [`live_session_block`] sees a session,
//! and every stage but `stop` refuses on contract skew
//! (`deny_on_contract_skew`); both refusals run before the lock wait and again
//! with the lock held. `all` is a caller-level loop over [`Stage::ALL_CHAIN`]
//! with a fresh [`StageCtx`] per stage, not a sixth stage.
//! See tests::{run_stage_refuses_setup_build_and_install_while_a_session_is_live,
//! a_queued_stage_is_refused_when_a_session_goes_live_during_the_wait,
//! every_mutating_stage_refuses_a_checkout_the_binary_was_not_built_from,
//! a_queued_stage_announces_itself_and_cancels_out_of_the_wait}.
//!
//! # Lock policy for `run`
//!
//! `run` releases the lock once the wine child is up, so `stop` and every fix
//! stay reachable during a session: [`run::run`] takes the guard by value and
//! drops it at the launch boundary; [`run_stage_holding_lock`] passes `None`.
//!
//! `tokio::sync::Mutex` is not reentrant: a caller already holding the lock must
//! use [`run_stage_holding_lock`] / [`crate::fixes::apply_holding_lock`], or it
//! deadlocks in silence.

pub mod build;
pub mod install;
pub mod run;
pub mod setup;
pub mod stop;

use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::checks::{CheckCtx, CheckOptions};
use crate::error::{Result, SabrageError};
use crate::events::{RunId, Severity, Stage, StageEvent, StepId};
use crate::executor::{DryRunExecutor, Executor, RealExecutor};
use crate::paths::{resolve_bs_dir, Bottle, Paths};
use crate::process::ChildSpec;

/// Where a stage's events go.
///
/// A plain callback rather than an `mpsc::Sender`: every producer is synchronous
/// at the point of emission (a check resolving, a line being printed, a pump
/// forwarding a chunk), so a channel would force either an `.await` in those
/// places or a `try_send` that can silently drop a row.
///
/// Sinks are called from arbitrary tasks (both output pumps of every child), so
/// they must be `Send + Sync` and cheap.
pub type EventSink = Arc<dyn Fn(StageEvent) + Send + Sync>;

/// A sink that drops everything — for tests and for probe-only runs.
pub fn null_sink() -> EventSink {
    Arc::new(|_| {})
}

/// The stage-relevant slice of the `WINEVR_*` mirror — all six flags demo.sh
/// accepts, plus Sabrage's own `dry_run`.
///
/// `no_audio` / `no_dashboard` / `wired` are read only by [`Stage::Run`] and by
/// the `run.wired-adb` preflight, which is why [`StageCtx::check_ctx`] forwards
/// them. See tests::check_ctx_forwards_every_launch_flag.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StageOptions {
    /// `WINEVR_BOTTLE` / `--bottle`.
    pub bottle_name: Option<String>,
    /// `WINEVR_BS_DIR` / `--bs-dir`.
    pub bs_dir_override: Option<PathBuf>,
    /// Sabrage-only: plan, do not mutate. Selects [`DryRunExecutor`].
    pub dry_run: bool,
    /// `WINEVR_VERBOSE` / `--verbose`.
    pub verbose: bool,
    /// `WINEVR_NO_AUDIO` / `--no-audio`: leave the Mac's audio output alone.
    pub no_audio: bool,
    /// `WINEVR_NO_DASHBOARD` / `--no-dashboard`: do not launch `alvr_dashboard`.
    pub no_dashboard: bool,
    /// `WINEVR_WIRED` / `--wired`: USB streaming — create the `tcp:9943`/
    /// `tcp:9944` adb forwards instead of clearing them.
    pub wired: bool,
}

impl StageOptions {
    /// Read the `WINEVR_*` environment exactly as demo.sh does (any non-empty
    /// value is true). `dry_run` has no shell counterpart and stays false.
    pub fn from_env() -> StageOptions {
        fn flag(name: &str) -> bool {
            std::env::var_os(name).is_some_and(|v| !v.is_empty())
        }
        StageOptions {
            bottle_name: std::env::var("WINEVR_BOTTLE")
                .ok()
                .filter(|v| !v.is_empty()),
            bs_dir_override: std::env::var("WINEVR_BS_DIR")
                .ok()
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
            dry_run: false,
            verbose: flag("WINEVR_VERBOSE"),
            no_audio: flag("WINEVR_NO_AUDIO"),
            no_dashboard: flag("WINEVR_NO_DASHBOARD"),
            wired: flag("WINEVR_WIRED"),
        }
    }
}

/// Everything a stage is allowed to touch.
#[derive(Clone)]
pub struct StageCtx {
    /// The typed lib.sh path set.
    pub paths: Paths,
    /// `Some` only when a bottle was named **and** its `cxbottle.conf` exists.
    /// Use [`require_bottle`] rather than unwrapping: it produces lib.sh's die
    /// text for both failure shapes.
    pub bottle: Option<Bottle>,
    /// The resolved Beat Saber directory (`BS_DIR`).
    pub bs_dir: PathBuf,
    pub opts: StageOptions,
    /// Every mutation goes through this. Never write to disk directly.
    pub executor: Arc<dyn Executor>,
    pub run_id: RunId,
    pub cancel: CancellationToken,
    pub sink: EventSink,
}

impl std::fmt::Debug for StageCtx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StageCtx")
            .field("root", &self.paths.root)
            .field("bottle", &self.bottle.as_ref().map(|b| &b.name))
            .field("bs_dir", &self.bs_dir)
            .field("dry_run", &self.opts.dry_run)
            .field("run_id", &self.run_id)
            .finish()
    }
}

impl StageCtx {
    /// Build a context, choosing [`RealExecutor`] or [`DryRunExecutor`] from
    /// `opts.dry_run` and minting a fresh `run_id`.
    pub fn new(
        paths: Paths,
        opts: StageOptions,
        sink: EventSink,
        cancel: CancellationToken,
    ) -> StageCtx {
        let run_id = Uuid::new_v4();
        let executor: Arc<dyn Executor> = if opts.dry_run {
            Arc::new(DryRunExecutor::new(run_id, sink.clone(), cancel.clone()))
        } else {
            Arc::new(RealExecutor::new(run_id, sink.clone(), cancel.clone()))
        };
        StageCtx::with_executor(paths, opts, sink, cancel, executor, run_id)
    }

    /// [`StageCtx::new`] with a caller-supplied executor and `run_id` — for
    /// tests, and for a fix that must reuse a run's identity.
    pub fn with_executor(
        paths: Paths,
        opts: StageOptions,
        sink: EventSink,
        cancel: CancellationToken,
        executor: Arc<dyn Executor>,
        run_id: RunId,
    ) -> StageCtx {
        // Same resolution order as CheckCtx: $PREFIX is set as soon as a NAME is
        // given, before the cxbottle.conf existence test, so BS_DIR derives from
        // the named (possibly non-existent) bottle.
        let named = opts.bottle_name.as_deref().map(Bottle::unvalidated);
        let bs_dir = resolve_bs_dir(named.as_ref(), opts.bs_dir_override.as_deref());
        let bottle = named.filter(Bottle::exists);
        StageCtx {
            paths,
            bottle,
            bs_dir,
            opts,
            executor,
            run_id,
            cancel,
            sink,
        }
    }

    /// A self-contained fixture context: [`null_sink`], a fresh
    /// [`CancellationToken`], and always a [`DryRunExecutor`] — `opts.dry_run` is
    /// forced true regardless of the caller, so a fixture can never mutate the
    /// machine.
    ///
    /// Exists for a downstream crate that needs only *a* `StageCtx` to drive a
    /// text-rendering function (`sabrage-parity`'s A1-3 pins over
    /// `stages::run::actions::banner_events`, `bs_win_path`,
    /// `preflight::block_die`, `preflight::post_fix_die`, …) and should not have to
    /// depend on `tokio_util` to build one.
    pub fn for_fixture(paths: Paths, mut opts: StageOptions) -> StageCtx {
        opts.dry_run = true;
        StageCtx::new(paths, opts, null_sink(), CancellationToken::new())
    }

    /// A [`CheckCtx`] over the same machine state, for a preflight that reuses
    /// the doctor registry.
    pub fn check_ctx(&self) -> CheckCtx {
        CheckCtx::new(
            self.paths.clone(),
            CheckOptions {
                bottle_name: self.opts.bottle_name.clone(),
                bs_dir_override: self.opts.bs_dir_override.clone(),
                verbose: self.opts.verbose,
                wired: self.opts.wired,
                no_audio: self.opts.no_audio,
                no_dashboard: self.opts.no_dashboard,
                ..CheckOptions::new()
            },
        )
    }

    /// An executor whose children are attributed to `step`.
    pub fn executor_for(&self, step: StepId) -> Arc<dyn Executor> {
        self.executor.with_step(step)
    }

    /// A [`ChildSpec`] carrying this run's id and the given step.
    pub fn child(&self, program: impl Into<std::ffi::OsString>, step: StepId) -> ChildSpec {
        ChildSpec::new(program, step, self.run_id)
    }

    /// Emit one event.
    pub fn emit(&self, ev: StageEvent) {
        (self.sink)(ev);
    }

    /// A section banner — install.sh's `print -r -- "-- <title>"`. Pass the
    /// title **without** the `-- ` prefix.
    pub fn section(&self, title: impl Into<String>) {
        self.emit(StageEvent::Section {
            run_id: self.run_id,
            title: title.into(),
        });
    }

    /// `info "<text>"`.
    pub fn info(&self, text: impl Into<String>) {
        self.emit(StageEvent::info(self.run_id, None, text));
    }

    /// `ok "<text>"`.
    pub fn ok(&self, text: impl Into<String>) {
        self.emit(StageEvent::ok(self.run_id, None, text));
    }

    /// `die "<message>"`: emits [`StageEvent::Fatal`] and **returns** the error
    /// for the caller to `return Err(…)`. Emitting and returning are one call so
    /// a stage can never abort without the UI hearing why.
    pub fn fatal(&self, message: impl Into<String>, remedy: Option<String>) -> SabrageError {
        let message = message.into();
        self.emit(StageEvent::Fatal {
            run_id: self.run_id,
            message: message.clone(),
            remedy: remedy.clone(),
            fix: None,
        });
        SabrageError::Fatal { message, remedy }
    }

    /// Rows attributed to `step`.
    pub fn step(&self, step: StepId) -> StepEmitter<'_> {
        StepEmitter { ctx: self, step }
    }
}

/// [`StageCtx`]'s row helpers, bound to one step.
#[derive(Debug, Clone, Copy)]
pub struct StepEmitter<'a> {
    ctx: &'a StageCtx,
    step: StepId,
}

impl StepEmitter<'_> {
    pub fn info(&self, text: impl Into<String>) {
        self.row(Severity::Info, text, None);
    }

    pub fn ok(&self, text: impl Into<String>) {
        self.row(Severity::Ok, text, None);
    }

    pub fn warn(&self, text: impl Into<String>) {
        self.row(Severity::Warn, text, None);
    }

    /// `die`, attributed to this step. See [`StageCtx::fatal`].
    pub fn fatal(&self, message: impl Into<String>, remedy: Option<String>) -> SabrageError {
        self.ctx.fatal(message, remedy)
    }

    fn row(&self, severity: Severity, text: impl Into<String>, remedy: Option<String>) {
        self.ctx.emit(StageEvent::Line {
            run_id: self.ctx.run_id,
            step: Some(self.step.to_string()),
            severity,
            text: text.into(),
            remedy,
        });
    }
}

/// `lib.sh`'s `require_bottle`, message text verbatim.
///
/// # Errors
///
/// Two `die` strings. The missing-name one is two lines and ends with the bottle
/// list, which keeps the shell's trailing space (`tr '\n' ' '` appends one per
/// name), so an empty list renders as `Existing bottles: ` with nothing after it;
/// the not-found one is a single line. Deliberately **not** the text of doctor's
/// `bottle.exists` row ([`Bottle::resolve`]), which splits message and remedy — a
/// stage aborting must read exactly like the shell aborting.
/// See tests::require_bottle_reproduces_lib_sh_die_text.
pub fn require_bottle(ctx: &StageCtx) -> Result<&Bottle> {
    let Some(name) = ctx.opts.bottle_name.as_deref() else {
        let listed = crate::paths::list_bottles()
            .into_iter()
            .map(|b| format!("{b} "))
            .collect::<String>();
        return Err(ctx.fatal(
            format!(
                "CrossOver bottle name required: pass --bottle <name> or set WINEVR_BOTTLE.\n       Existing bottles: {listed}"
            ),
            None,
        ));
    };
    match &ctx.bottle {
        Some(b) => Ok(b),
        None => {
            let prefix = Bottle::unvalidated(name).prefix;
            Err(ctx.fatal(
                format!(
                    "bottle '{name}' not found at {} — create it in CrossOver (win11_64) first",
                    prefix.display()
                ),
                None,
            ))
        }
    }
}

/// `run`'s wineserver-reset budget: **5 s, fatal on timeout**.
///
/// Reference: scripts/demo/run.sh, the poll loop over the backgrounded
/// `wineserver -w` that warns "wineserver still alive after 5s" and then dies.
///
/// Deliberately distinct from [`STOP_WINESERVER_WAIT`] — 5 s fatal here, 4 s
/// soft there. Never unify them; collapsing the two constants silently changes
/// one of the behaviours. See tests::the_two_wineserver_budgets_stay_distinct.
pub const RUN_WINESERVER_WAIT: Duration = Duration::from_secs(5);

/// `stop`'s wineserver-wait budget: **4 s, never fatal**.
///
/// Reference: scripts/demo/lib.sh, `stop_wine` — it polls, then gives up
/// (`kill $_wp 2>/dev/null || true`). See [`RUN_WINESERVER_WAIT`] for why the two
/// budgets stay apart. Re-exported as `stages::stop::STOP_WINESERVER_WAIT`.
pub const STOP_WINESERVER_WAIT: Duration = Duration::from_secs(4);

/// One mutating operation at a time — stage or fix. The **in-process** half.
///
/// Doctor never takes it; it is read-only by construction.
pub static OPERATION_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

/// The cross-process half's file name, under Sabrage's own support directory.
///
/// Sabrage-only: `demo.sh` does not take it, so this serializes the GUI against
/// the `sabrage` CLI and against a second GUI instance, not against the shell
/// pipeline (PARITY.md § Declared by the 2026-08-30 adversarial review
/// (round 1 fixes), "Cross-process operation lock.").
pub const OPERATION_LOCK_FILE_NAME: &str = "operation.lock";

/// How often [`acquire_operation_lock`] retries the advisory file lock while
/// another Sabrage process holds it.
const OPERATION_LOCK_POLL: Duration = Duration::from_millis(200);

/// The guard [`acquire_operation_lock`] hands back: the in-process mutex guard
/// **and** the cross-process advisory lock, released together on drop.
///
/// Field order is the release order — the file lock first, then the mutex — so
/// a waiter that has just been handed the mutex can never find the file still
/// locked by the process that handed it over.
pub struct OperationGuard {
    /// `None` when the advisory lock could not be established at all (no
    /// writable support directory, a filesystem without `flock`). The
    /// in-process guarantee is unaffected; only cross-process exclusion
    /// degrades, and degrading is better than refusing to run a stage.
    _file: Option<File>,
    _mutex: tokio::sync::MutexGuard<'static, ()>,
}

impl std::fmt::Debug for OperationGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OperationGuard")
            .field("interprocess", &self._file.is_some())
            .finish()
    }
}

/// Take the operation lock, waiting if another operation — in this process or
/// in another Sabrage process — holds it.
///
/// Uncancellable: for the callers that have no cancellation token of their own
/// (a Doctor fix pass, the store's Goldberg swap). A stage goes through
/// [`acquire_operation_lock_cancellable`] instead, so the user's Stop can break
/// a wait behind another Sabrage process rather than sitting through it.
pub async fn acquire_operation_lock() -> OperationGuard {
    // A token nobody holds a handle to can never fire, so the `select!` arms
    // below degrade to a plain await.
    acquire_operation_lock_cancellable(&CancellationToken::new())
        .await
        .expect("a token with no other handle cannot be cancelled")
}

/// [`acquire_operation_lock`], abandoning the wait when `cancel` fires.
///
/// `None` means "cancelled while waiting" — the caller must not proceed. Both
/// halves are cancellable: the in-process mutex (another stage in this process)
/// and the advisory file lock (a `sabrage` CLI build in another process, which
/// can hold it for minutes), so the user's Stop can reach a queued stage.
/// See tests::the_file_lock_wait_gives_up_when_the_token_fires.
pub async fn acquire_operation_lock_cancellable(
    cancel: &CancellationToken,
) -> Option<OperationGuard> {
    let mutex = tokio::select! {
        biased;
        () = cancel.cancelled() => return None,
        guard = OPERATION_LOCK.lock() => guard,
    };
    let file = match acquire_lock_file(&operation_lock_path(), cancel).await {
        FileLock::Held(f) => Some(f),
        FileLock::Unavailable => None,
        FileLock::Cancelled => return None,
    };
    Some(OperationGuard {
        _file: file,
        _mutex: mutex,
    })
}

/// Where the advisory lock file lives.
///
/// Under `cfg(test)` it is a per-test-process file in the temp directory
/// instead: a test run must never create anything in the user's real Sabrage
/// store, and two test binaries running concurrently must not serialize against
/// each other.
fn operation_lock_path() -> PathBuf {
    #[cfg(test)]
    {
        std::env::temp_dir().join(format!(
            "sabrage-{OPERATION_LOCK_FILE_NAME}-{}",
            std::process::id()
        ))
    }
    #[cfg(not(test))]
    {
        crate::paths::sabrage_support_dir().join(OPERATION_LOCK_FILE_NAME)
    }
}

/// What [`acquire_lock_file`] came back with.
///
/// Three outcomes, not two: "could not be established" and "the caller gave up
/// waiting" mean opposite things to the caller — the first proceeds without the
/// cross-process half, the second must not proceed at all.
enum FileLock {
    /// The advisory lock is held for as long as the `File` lives.
    Held(File),
    /// It could not be established at all — degrade to [`OPERATION_LOCK`].
    Unavailable,
    /// The caller's token fired while another process held it.
    Cancelled,
}

/// `flock(LOCK_EX)`, spelled as a poll over `try_lock` so the async worker is
/// never blocked while another process holds it, and so `cancel` can break the
/// wait (`flock` itself has no cancellable async form).
///
/// Every failure degrades to [`FileLock::Unavailable`] rather than propagating:
/// the lock is an extra safety net over [`OPERATION_LOCK`], and a machine whose
/// support directory cannot be created must still be able to run `build`.
async fn acquire_lock_file(path: &Path, cancel: &CancellationToken) -> FileLock {
    let Some(file) = open_lock_file(path) else {
        return FileLock::Unavailable;
    };
    loop {
        // Checked before the first `try_lock` too: a stage cancelled before it
        // ever reached the lock must not take it.
        if cancel.is_cancelled() {
            return FileLock::Cancelled;
        }
        match file.try_lock() {
            Ok(()) => {
                // Diagnostic only — who is holding it. Best effort: a failed
                // write is not a reason to give up a lock we hold.
                let _ = file.set_len(0);
                let _ = (&file).write_all(format!("{}\n", std::process::id()).as_bytes());
                return FileLock::Held(file);
            }
            Err(std::fs::TryLockError::WouldBlock) => {
                tokio::select! {
                    biased;
                    () = cancel.cancelled() => return FileLock::Cancelled,
                    () = tokio::time::sleep(OPERATION_LOCK_POLL) => {}
                }
            }
            Err(std::fs::TryLockError::Error(_)) => return FileLock::Unavailable,
        }
    }
}

fn open_lock_file(path: &Path) -> Option<File> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok()?;
    }
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .ok()
}

/// Is a mutating operation running right now?
///
/// A `try_lock` probe on the in-process half only, so it is racy by nature —
/// true means "was held at this instant", and an operation in *another* Sabrage
/// process is invisible to it. Doctor uses it to annotate, never to gate.
pub fn operation_in_progress() -> bool {
    OPERATION_LOCK.try_lock().is_err()
}

/// Is a mutating operation running **anywhere on this machine** — this process
/// or another Sabrage process?
///
/// [`operation_in_progress`] plus a probe of the advisory file lock, so a GUI
/// stage that is about to queue behind a `sabrage` CLI build can say so. Racy in
/// exactly the same way (both halves can change hands the instant after they are
/// read), and diagnostic-only for the same reason: never gate on it.
///
/// The probe never *creates* the lock file — a machine that has never taken the
/// operation lock has nothing to report — and releases immediately on drop, so
/// the worst it can cost a real waiter is one extra [`OPERATION_LOCK_POLL`].
pub fn operation_in_progress_anywhere() -> bool {
    operation_in_progress() || operation_lock_file_busy()
}

/// The cross-process half of [`operation_in_progress_anywhere`].
fn operation_lock_file_busy() -> bool {
    let Ok(file) = OpenOptions::new().read(true).open(operation_lock_path()) else {
        return false;
    };
    matches!(file.try_lock(), Err(std::fs::TryLockError::WouldBlock))
}

/// Why a mutating operation must not start right now, or `None` when nothing on
/// this machine looks like a live session.
///
/// A thin alias for [`crate::session::live_session_reason`]: the session layer
/// owns the policy, and this is the name every stage and fix refusal reads
/// (`deny_stage_while_session_live`, [`crate::fixes`]'s `deny_if_session_live`,
/// [`crate::fixes::adb::remove_adb_forwards`]). Do not reintroduce a weaker local
/// copy: the doors that mutate must be guarded by no less than the doors that
/// only refuse. See [`crate::session::session_block_at`] for A4-1's seven signals.
pub fn live_session_block(paths: &Paths) -> Option<String> {
    crate::session::live_session_reason(paths)
}

/// The three stages that replace artifacts a running session has open.
///
/// `run` is exempt (it reconciles the previous session itself) and so is `stop`
/// (it is the way out of a live session).
fn stage_is_forbidden_while_session_live(stage: Stage) -> bool {
    matches!(stage, Stage::Setup | Stage::Build | Stage::Install)
}

/// The `die` a stage produces when [`live_session_block`] sees a session.
///
/// Same shape as [`crate::fixes::backend`]'s and [`crate::fixes::session_json`]'s
/// refusals — message plus the `./demo.sh stop` remedy — so the GUI renders one
/// familiar row for every "stop the session first" refusal.
fn deny_stage_while_session_live(stage: Stage, ctx: &StageCtx) -> Result<()> {
    if !stage_is_forbidden_while_session_live(stage) {
        return Ok(());
    }
    match live_session_block(&ctx.paths) {
        None => Ok(()),
        Some(reason) => Err(ctx.fatal(
            format!(
                "refusing to run {stage} while a session is live — {reason}; stop the session first"
            ),
            Some(format!(
                "./demo.sh stop --bottle {}",
                ctx.opts.bottle_name.as_deref().unwrap_or("<name>")
            )),
        )),
    }
}

/// Write the contract **this test binary was compiled from** into `root`, so a
/// scratch checkout is contract-identical to the binary under test and
/// [`contract_identity_mismatch`] answers `None` for it.
///
/// Every fixture root a mutating stage or fix is pointed at needs this: without
/// it the identity guard (correctly) refuses before the behaviour the test is
/// about is ever reached.
#[cfg(test)]
pub(crate) fn materialize_compiled_contract(root: &Path) {
    use crate::contract::{
        CONTRACT_FILES, HOST_MANIFEST_TEMPLATE, PIPELINE_TOML, RUNTIME_TOML_TEMPLATE,
    };
    let bodies = [PIPELINE_TOML, RUNTIME_TOML_TEMPLATE, HOST_MANIFEST_TEMPLATE];
    for (rel, body) in CONTRACT_FILES.iter().zip(bodies) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().expect("contract paths have a parent")).unwrap();
        std::fs::write(path, body).unwrap();
    }
}

/// The stages that write contract-derived bytes onto the machine.
///
/// `stop` is exempt on purpose: it is the way out of every bad state, including
/// this one, and it writes nothing the contract describes.
fn stage_is_gated_by_contract_identity(stage: Stage) -> bool {
    matches!(
        stage,
        Stage::Setup | Stage::Build | Stage::Install | Stage::Run
    )
}

/// The `die` any mutating door produces when the contract compiled into this
/// binary is not the one the checkout at `ctx.paths.root` describes.
///
/// The predicate and both strings come from
/// [`crate::checks::meta::assert_binary_matches_checkout`], so the abort and the
/// `meta.contract-sync` row say exactly one thing — and a `contract/` that
/// cannot be read at all fails closed there, not here.
///
/// [`crate::fixes::apply`]'s door as much as a stage's: a Doctor "Fix" button
/// run by an X-built binary writes X's ports, pins and templates into checkout
/// Y just as an `install` would.
pub(crate) fn deny_on_contract_skew(ctx: &StageCtx) -> Result<()> {
    match crate::checks::meta::assert_binary_matches_checkout(&ctx.paths.root) {
        Ok(()) => Ok(()),
        Err((message, remedy)) => Err(ctx.fatal(message, Some(remedy))),
    }
}

/// [`deny_on_contract_skew`] for the stages it applies to.
fn deny_stage_on_contract_skew(stage: Stage, ctx: &StageCtx) -> Result<()> {
    if !stage_is_gated_by_contract_identity(stage) {
        return Ok(());
    }
    deny_on_contract_skew(ctx)
}

/// Every refusal a stage owes before it may dispatch: a live session, then
/// contract skew.
///
/// One function because both doors ([`run_stage`], [`run_stage_holding_lock`])
/// and both moments (before the operation lock, and again after acquiring it —
/// see [`run_stage`]) must apply exactly the same policy; a third door cannot
/// forget half of it.
fn deny_before_dispatch(stage: Stage, ctx: &StageCtx) -> Result<()> {
    deny_stage_while_session_live(stage, ctx)?;
    deny_stage_on_contract_skew(stage, ctx)
}

/// What a stage invocation amounted to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageOutcome {
    pub stage: Stage,
    pub ok: bool,
    /// What `./demo.sh <stage>` would have exited with.
    pub exit_code_equiv: i32,
}

impl StageOutcome {
    /// A stage that finished cleanly.
    pub fn success(stage: Stage) -> StageOutcome {
        StageOutcome {
            stage,
            ok: true,
            exit_code_equiv: 0,
        }
    }

    /// The outcome shape carried by the [`StageEvent::StageFinished`] of a failed
    /// stage. `run_stage` returns `Err` in that case; this is what the event
    /// says.
    pub fn failed(stage: Stage, exit_code_equiv: i32) -> StageOutcome {
        StageOutcome {
            stage,
            ok: false,
            exit_code_equiv,
        }
    }

    /// The outcome of a stage that ran to completion and produced an exit code.
    ///
    /// `ok == (code == 0)`. Every stage but `run` can only produce 0 here (a
    /// failure is an `Err`); `run` returns **wine's own exit status**, which
    /// demo.sh propagates verbatim (`exit $rc`) — so a game that crashed with
    /// status 3 is a stage that finished, not-ok, with `exit_code_equiv: 3`.
    pub fn from_code(stage: Stage, exit_code_equiv: i32) -> StageOutcome {
        StageOutcome {
            stage,
            ok: exit_code_equiv == 0,
            exit_code_equiv,
        }
    }
}

/// Run one stage, taking [`OPERATION_LOCK`] for its duration.
///
/// Emits [`StageEvent::StageStarted`] first and [`StageEvent::StageFinished`]
/// last — on the failure path too — so a UI never sees an unfinished stage.
///
/// `StageStarted` is emitted **before** the lock: that event is the front-end's
/// only source of the run id, making the wait behind another process's build
/// visible and cancellable. A cancelled wait ends as [`SabrageError::Cancelled`]
/// (exit 130) with nothing touched. The live-session refusal stays *before* the
/// event, matching demo.sh which dies before printing a stage banner.
///
/// `deny_before_dispatch` runs **again** once the lock is in hand: the wait is
/// unbounded, and a `run` admitted meanwhile publishes its live session and then
/// releases the lock at the launch boundary, so an install could otherwise
/// replace the artifacts of a streaming game. The second refusal goes through
/// `finish_stage`, `StageStarted` having already been emitted.
/// See tests::{run_stage_brackets_the_stage_with_events_even_when_it_fails,
/// a_queued_stage_announces_itself_and_cancels_out_of_the_wait,
/// a_queued_stage_is_refused_when_a_session_goes_live_during_the_wait}.
pub async fn run_stage(stage: Stage, ctx: &StageCtx) -> Result<StageOutcome> {
    // Before the lock, not after: waiting minutes for another process's build
    // only to refuse is worse than refusing straight away.
    deny_before_dispatch(stage, ctx)?;
    ctx.emit(StageEvent::StageStarted {
        run_id: ctx.run_id,
        stage,
    });
    if operation_in_progress_anywhere() {
        ctx.info("waiting for another Sabrage operation to finish");
    }
    let Some(guard) = acquire_operation_lock_cancellable(&ctx.cancel).await else {
        return finish_stage(stage, ctx, Err(SabrageError::Cancelled));
    };
    // The world may have changed during the wait — recheck before the first
    // mutation, with the lock held so the answer cannot go stale again.
    if let Err(e) = deny_before_dispatch(stage, ctx) {
        return finish_stage(stage, ctx, Err(e));
    }
    if stage == Stage::Run {
        // The one stage that gives the lock back early: the guard is *moved
        // into* the stage, which drops it once the wine child is up. `run_stage`
        // must not keep one of its own, or the release would be a no-op.
        let result = run::run(ctx, Some(guard)).await;
        return finish_stage(stage, ctx, result);
    }
    let _guard = guard;
    let result = dispatch(stage, ctx).await;
    finish_stage(stage, ctx, result)
}

/// [`run_stage`] for a caller that already holds [`OPERATION_LOCK`] (a preflight
/// applying a whole-stage auto-fix). Taking the lock twice on one task
/// deadlocks, hence the split.
///
/// [`crate::fixes::apply_holding_lock`] is the fix-shaped door to this function;
/// a preflight that has taken the lock for the whole launch calls that, never
/// [`crate::fixes::apply`].
///
/// The live-session refusal is deliberately absent here (this door is reached
/// from inside `run`, before the live handle is published — see
/// [`crate::fixes::apply_holding_lock`]), but the contract-identity refusal is
/// not: a binary that cannot identify the checkout it is writing into must not
/// mutate through *any* door.
pub async fn run_stage_holding_lock(stage: Stage, ctx: &StageCtx) -> Result<StageOutcome> {
    deny_stage_on_contract_skew(stage, ctx)?;
    ctx.emit(StageEvent::StageStarted {
        run_id: ctx.run_id,
        stage,
    });
    let result = dispatch(stage, ctx).await;
    finish_stage(stage, ctx, result)
}

/// Emit [`StageEvent::StageFinished`] for `result` and project it onto a
/// [`StageOutcome`]. The single place the bracket closes, so both entry points
/// (and the `run` special case) report identically.
fn finish_stage(stage: Stage, ctx: &StageCtx, result: Result<i32>) -> Result<StageOutcome> {
    let outcome = match &result {
        Ok(code) => StageOutcome::from_code(stage, *code),
        Err(e) => StageOutcome::failed(stage, e.exit_code()),
    };
    ctx.emit(StageEvent::StageFinished {
        run_id: ctx.run_id,
        stage,
        ok: outcome.ok,
        exit_code_equiv: outcome.exit_code_equiv,
    });
    result.map(|_| outcome)
}

/// The exit-code-equivalent of one stage.
///
/// setup/build/install/stop map their `()` success onto `0`: demo.sh's
/// dispatcher exits 0 for each of them or dies. Only `run` has a code of its
/// own — wine's — and it is reached through [`run_stage`], never here, because
/// this function cannot hand it the operation-lock guard to release.
async fn dispatch(stage: Stage, ctx: &StageCtx) -> Result<i32> {
    match stage {
        Stage::Setup => setup::run(ctx).await.map(|()| 0),
        Stage::Build => build::run(ctx).await.map(|()| 0),
        Stage::Install => install::run(ctx).await.map(|()| 0),
        Stage::Stop => stop::run(ctx).await.map(|()| 0),
        // Reached only via `run_stage_holding_lock` (tests, whole-stage
        // auto-fixes): the caller already owns the lock, so the stage gets no
        // guard to release and supervises with the lock still held.
        Stage::Run => run::run(ctx, None).await,
    }
}

#[cfg(test)]
mod tests;
