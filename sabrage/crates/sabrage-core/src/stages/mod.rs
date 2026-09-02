//! Stage orchestration: the context every stage runs in, the operation lock, and
//! the dispatcher.
//!
//! A stage is one `./demo.sh <verb>`. Each is a plain `async fn run(&StageCtx)`
//! rather than a trait impl: there is exactly one implementation of each, they
//! share no behaviour worth abstracting, and a free function keeps the whole
//! stage readable top-to-bottom next to the shell script it mirrors.
//!
//! # Serialization
//!
//! [`OPERATION_LOCK`] admits one mutating operation at a time. Doctor is
//! read-only and runs concurrently; it may *annotate* rows with
//! [`operation_in_progress`] so a row that fails because a build is halfway
//! through says so.
//!
//! The lock is acquired by the outermost entry points — [`run_stage`] and
//! [`crate::fixes::apply`] — and by nothing below them. Callers already inside a
//! held lock (a launch preflight applying an auto-fix that is itself a whole
//! stage) use [`run_stage_holding_lock`] / [`crate::fixes::apply_holding_lock`]:
//! `tokio::sync::Mutex` is not reentrant, and taking it twice on one task
//! deadlocks in silence.
//!
//! A `tokio::sync::Mutex` is a *per-process* primitive, and Sabrage has two
//! native front-ends (the GUI and the `sabrage` CLI) that write the same
//! artifacts. [`acquire_operation_lock`] therefore takes a second, **advisory
//! file lock** ([`OPERATION_LOCK_FILE_NAME`]) in the same call, so a CLI build
//! cannot overwrite `build-x64/` while the GUI's install is copying out of it.
//! Both halves are released together when the [`OperationGuard`] drops —
//! including at `run`'s launch boundary below. demo.sh deliberately does **not**
//! participate (PARITY.md): the shell pipeline stays a zero-dependency script,
//! so a concurrent `./demo.sh build` is still unserialized.
//!
//! # Live sessions
//!
//! Serialization is not the whole policy: `setup`, `build` and `install` replace
//! the very artifacts a *running* session has open, so [`run_stage`] refuses
//! them outright while [`live_session_block`] can see a session. `run` and
//! `stop` are exempt — `run` has its own reconciliation, and `stop` is the way
//! out. [`crate::fixes::apply`] applies the same policy to every fix whose
//! registry entry is `forbidden_while_session_live`.
//!
//! Both doors refuse **twice**: once before waiting on the operation lock (fast,
//! so a queued operation is not admitted only to be refused later) and once with
//! the lock in hand, because a `run` that acquired first publishes its live
//! session and then gives the lock back at the launch boundary.
//!
//! # Contract identity
//!
//! [`deny_on_contract_skew`] is the second half of the same policy: every stage
//! that writes contract-derived bytes (`setup`, `build`, `install`, `run` — not
//! `stop`, the way out) refuses when the contract compiled into this binary is
//! not the one the checkout at `paths.root` describes
//! ([`crate::checks::meta::assert_binary_matches_checkout`]). `meta.contract-sync`
//! reports the same skew as a doctor row; this is the enforcement, so an
//! X-built binary cannot install X's pins into checkout Y just because nobody
//! ran doctor first.
//!
//! # Lock policy for `run` (the one exception)
//!
//! [`run_stage`]`(`[`Stage::Run`]`)` holds [`OPERATION_LOCK`] through
//! **preflight + prepare + guards + spawn**, and **releases it the moment the
//! wine child is up** — before Supervise. A session lasts hours: holding the
//! lock for its duration would mean `stop`, every fix, and every other stage
//! block until the user quits Beat Saber, which is precisely when they are
//! most likely to reach for Stop.
//!
//! Mechanically: [`run_stage`] takes the guard and hands it to
//! [`run::run`]`(ctx, Some(guard))`, which drops it at the launch boundary.
//! [`run_stage_holding_lock`] passes `None` — the guard it inherits is never
//! released, which is correct for a caller that already owns the lock and
//! wrong for a real session, so that door is for tests and for whole-stage
//! auto-fixes only.
//!
//! What this costs: between the release and the wine child exiting, a second
//! operation *can* start. That is deliberate — `stop` during a live session is
//! the whole point — and the run stage's teardown is written to tolerate a
//! `stop` having already killed its child.
//!
//! # `all`
//!
//! `./demo.sh all` re-executes the dispatcher once per stage so each gets a
//! fresh process. The native equivalent is a caller-level loop over
//! [`Stage::ALL_CHAIN`] building a **fresh [`StageCtx`] per stage** (fresh
//! `run_id`, fresh executor), aborting on the first failure with that stage's
//! exit code, with `require_bottle` checked once up front (fail-fast parity).
//! It is not a sixth stage.

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
/// A plain callback rather than an `mpsc::Sender`, because every producer is
/// synchronous at the point of emission (a check resolving, a line being
/// printed, a pump forwarding a chunk) and a channel would force either an
/// `.await` in those places or a `try_send` that can silently drop a row. The
/// Tauri layer wraps `app_handle.emit`; the CLI wraps its renderer; tests wrap a
/// `Vec`. A consumer that *wants* a channel makes the sink `tx.blocking_send`.
///
/// Sinks are called from arbitrary tasks (both output pumps of every child), so
/// they must be `Send + Sync` and cheap.
pub type EventSink = Arc<dyn Fn(StageEvent) + Send + Sync>;

/// A sink that drops everything — for tests and for probe-only runs.
pub fn null_sink() -> EventSink {
    Arc::new(|_| {})
}

// ── options ───────────────────────────────────────────────────────────────────

/// The stage-relevant slice of the `WINEVR_*` mirror — all six flags demo.sh
/// accepts, plus Sabrage's own `dry_run`.
///
/// `no_audio` / `no_dashboard` / `wired` are read only by [`Stage::Run`] (and
/// by the `run.wired-adb` preflight, which is why [`StageCtx::check_ctx`]
/// forwards them), exactly as demo.sh reads `WINEVR_NO_AUDIO` /
/// `WINEVR_NO_DASHBOARD` / `WINEVR_WIRED` only inside `run.sh`.
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

// ── context ───────────────────────────────────────────────────────────────────

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
    /// [`CancellationToken`], and always a [`DryRunExecutor`] regardless of
    /// `opts.dry_run` — so a caller can never accidentally mutate the
    /// machine through it. Forces `opts.dry_run = true` for the same reason
    /// [`StageOptions::from_env`]'s own doc names: it has no shell
    /// counterpart, and a fixture that quietly picked [`RealExecutor`] would
    /// be exactly the kind of test that only fails on someone else's machine.
    ///
    /// For a text-rendering function that never touches `ctx.executor` at all
    /// (`sabrage-parity`'s A1-3 pins over `stages::run::actions::banner_events`,
    /// `bs_win_path`, `preflight::block_die`, `preflight::post_fix_die`, …),
    /// so a downstream crate that needs only *a* `StageCtx` — never a
    /// `tokio_util::sync::CancellationToken` of its own — has one call to
    /// reach for instead of depending on `tokio_util` just to build a fixture.
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
                // The launch preflight's own gating reads these: `wired`
                // decides whether `run.wired-adb` is evaluated at all, and the
                // other two mirror demo.sh's flags so a preflight row can say
                // the same thing run.sh's would.
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

// ── require_bottle ────────────────────────────────────────────────────────────

/// `lib.sh`'s `require_bottle`, message text verbatim:
///
/// ```zsh
/// require_bottle() {
///   [ -n "${WINEVR_BOTTLE:-}" ] || die "CrossOver bottle name required: pass --bottle <name> or set WINEVR_BOTTLE.
///        Existing bottles: $(ls "$HOME/Library/Application Support/CrossOver/Bottles" 2>/dev/null | tr '\n' ' ')"
///   …
///   [ -f "$PREFIX/cxbottle.conf" ] || die "bottle '$WINEVR_BOTTLE' not found at $PREFIX — create it in CrossOver (win11_64) first"
/// }
/// ```
///
/// Both messages are single `die` strings; the first is two lines, the second
/// one. The bottle list keeps the shell's trailing space (`tr '\n' ' '` appends
/// one per name) — an empty list therefore renders as `Existing bottles: ` with
/// nothing after it.
///
/// Note this is **not** the same text as doctor's `bottle.exists` row
/// ([`Bottle::resolve`]), which splits message and remedy; a stage aborting
/// must read exactly like the shell aborting.
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

// ── wineserver budgets ────────────────────────────────────────────────────────

/// `run`'s wineserver-reset budget: **5 s, fatal on timeout**.
///
/// run.sh spells it as a poll loop over the backgrounded `wineserver -w`:
///
/// ```zsh
/// for _i in {1..50}; do kill -0 $_wpid 2>/dev/null || break; sleep 0.1; done
/// if kill -0 $_wpid 2>/dev/null; then
///   kill $_wpid 2>/dev/null
///   warn "wineserver still alive after 5s: $(pgrep -lf wineserver | tr '\n' ' ')"
///   die "kill the listed wineserver(s) manually, then re-run"
/// fi
/// ```
///
/// Deliberately **distinct** from [`STOP_WINESERVER_WAIT`] — 5 s fatal here,
/// 4 s advisory there (design-core §10, parity decision 18; PARITY.md's
/// "wineserver budgets (5 s fatal / 4 s soft)"). Collapsing them into one
/// constant would silently change one of the two behaviours.
pub const RUN_WINESERVER_WAIT: Duration = Duration::from_secs(5);

/// `stop`'s wineserver-wait budget: **4 s, never fatal**.
///
/// `lib.sh`'s `stop_wine` polls `for _i in {1..40}; do … sleep 0.1; done` and
/// then simply gives up (`kill $_wp 2>/dev/null || true`). See
/// [`RUN_WINESERVER_WAIT`] for why the two budgets stay apart.
///
/// Re-exported as `stages::stop::STOP_WINESERVER_WAIT`, where it used to live.
pub const STOP_WINESERVER_WAIT: Duration = Duration::from_secs(4);

// ── operation lock ────────────────────────────────────────────────────────────

/// One mutating operation at a time — stage or fix. The **in-process** half.
///
/// Doctor never takes it; it is read-only by construction.
pub static OPERATION_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

/// The cross-process half's file name, under Sabrage's own support directory.
///
/// Sabrage-only on purpose: `demo.sh` does not take it (PARITY.md), so this
/// serializes the GUI against the `sabrage` CLI and against a second GUI
/// instance, not against the shell pipeline.
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
/// can hold it for minutes). Without this, Stop could not reach a queued stage
/// at all: the poll loop in [`acquire_lock_file`] never yielded to anything but
/// the lock becoming free.
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

// ── live-session policy ───────────────────────────────────────────────────────

/// Why a mutating operation must not start right now, or `None` when nothing on
/// this machine looks like a live session.
///
/// A thin alias for [`crate::session::live_session_reason`] — the session
/// layer owns the policy, and this name is kept because it is the one every
/// stage and fix refusal reads
/// ([`deny_stage_while_session_live`], [`crate::fixes`]'s `deny_if_session_live`,
/// [`crate::fixes::adb::remove_adb_forwards`]).
///
/// It used to be a fourth, weaker copy of that predicate — missing the
/// `Unverifiable` classification and the foreign-owner window — which meant the
/// doors that mutate (`setup`/`build`/`install` and every gated fix) were
/// guarded by *less* than the doors that only refuse. One predicate, one
/// policy: see [`crate::session::session_block_at`] for the seven signals.
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

// ── contract identity policy ──────────────────────────────────────────────────

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

// ── dispatch ──────────────────────────────────────────────────────────────────

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
/// last — on the failure path too, with the error's `exit_code_equiv` — so a UI
/// that only listens to events never sees a stage that started and never ended.
///
/// `StageStarted` is emitted **before** the operation lock is taken, not after.
/// That event is the front-end's only source of the run id, and the run id is
/// what enables Cancel: a stage queued behind another Sabrage process's build
/// can wait minutes, and it used to do so with no row and a disabled Cancel
/// button, then mutate the machine when its turn came. Waiting is now a visible,
/// cancellable part of the run — [`acquire_operation_lock_cancellable`] returns
/// the moment `ctx.cancel` fires, and the stage ends as
/// [`SabrageError::Cancelled`] (exit 130) without having touched anything.
///
/// The live-session refusal stays *before* the event: it is an immediate,
/// argument-shaped rejection, and demo.sh's equivalent dies before printing a
/// stage banner too.
///
/// # Refused twice, on purpose
///
/// [`deny_before_dispatch`] runs **again** after the operation lock is in hand.
/// The wait is unbounded (another Sabrage process's build, minutes long) and the
/// world moves during it: a queued `install` admitted while the machine was idle
/// used to lose the race to a concurrent `run`, which publishes its live handle
/// and then deliberately *releases* the lock at the launch boundary — so the
/// install acquired the lock a moment later and replaced the artifacts of a game
/// that was by then streaming. The second refusal is the same predicate on the
/// same state; only its timing (after the wait, before the first mutation)
/// matters. It goes through [`finish_stage`] because `StageStarted` has already
/// been emitted by then.
pub async fn run_stage(stage: Stage, ctx: &StageCtx) -> Result<StageOutcome> {
    // Before the lock, not after: waiting for a build to finish only to refuse
    // is worse than refusing straight away, and the operation lock is free for
    // the whole of a live session by design (see "Lock policy for `run`").
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
        // The one stage that gives the lock back early — see this module's
        // "Lock policy for `run`". The guard is *moved into* the stage, which
        // drops it once the wine child is up; `run_stage` must not keep one of
        // its own or the release would be a no-op.
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
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    fn ctx_with(opts: StageOptions) -> (StageCtx, Arc<StdMutex<Vec<StageEvent>>>) {
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let s = seen.clone();
        let sink: EventSink = Arc::new(move |ev| s.lock().unwrap().push(ev));
        let ctx = StageCtx::new(
            Paths::new("/nonexistent/sabrage/repo"),
            opts,
            sink,
            CancellationToken::new(),
        );
        (ctx, seen)
    }

    #[test]
    fn dry_run_selects_the_recording_executor() {
        let (real, _) = ctx_with(StageOptions::default());
        assert!(!real.executor.is_dry_run());
        let (dry, _) = ctx_with(StageOptions {
            dry_run: true,
            ..Default::default()
        });
        assert!(dry.executor.is_dry_run());
    }

    #[test]
    fn require_bottle_reproduces_lib_sh_die_text() {
        let (ctx, seen) = ctx_with(StageOptions::default());
        let err = require_bottle(&ctx).unwrap_err();
        let msg = err.to_string();
        let (first, second) = msg.split_once('\n').expect("two-line message");
        assert_eq!(
            first,
            "CrossOver bottle name required: pass --bottle <name> or set WINEVR_BOTTLE."
        );
        assert!(
            second.starts_with("       Existing bottles: "),
            "second line was {second:?}"
        );
        // Each listed bottle is followed by a space (tr '\n' ' '), so the line
        // either ends in a space or lists nothing at all.
        let listed = second.trim_start_matches("       Existing bottles: ");
        assert!(listed.is_empty() || listed.ends_with(' '));
        // A die always announces itself.
        assert!(matches!(
            seen.lock().unwrap().last(),
            Some(StageEvent::Fatal { .. })
        ));

        // Named but absent bottle: the other die string.
        let (ctx, _) = ctx_with(StageOptions {
            bottle_name: Some("NoSuchBottle".into()),
            ..Default::default()
        });
        let msg = require_bottle(&ctx).unwrap_err().to_string();
        assert!(
            msg.starts_with("bottle 'NoSuchBottle' not found at ")
                && msg.ends_with(" — create it in CrossOver (win11_64) first"),
            "{msg}"
        );
    }

    #[tokio::test]
    async fn run_stage_brackets_the_stage_with_events_even_when_it_fails() {
        // `stop` with no bottle: `require_bottle` dies before touching the
        // machine, which is the cheapest real failure any stage can have.
        let (ctx, seen) = ctx_with(StageOptions::default());
        let err = run_stage(Stage::Stop, &ctx).await.unwrap_err();
        assert!(err
            .to_string()
            .starts_with("CrossOver bottle name required"));

        let evs = seen.lock().unwrap().clone();
        assert!(matches!(
            evs.first(),
            Some(StageEvent::StageStarted {
                stage: Stage::Stop,
                ..
            })
        ));
        assert!(matches!(
            evs.last(),
            Some(StageEvent::StageFinished {
                stage: Stage::Stop,
                ok: false,
                exit_code_equiv: 1,
                ..
            })
        ));
        assert!(evs.iter().any(|e| matches!(e, StageEvent::Fatal { .. })));
    }

    #[test]
    fn stage_outcome_from_code_is_ok_only_for_zero() {
        // `run` propagates wine's status: a non-zero code is a stage that
        // finished, not-ok — not an error.
        assert_eq!(
            StageOutcome::from_code(Stage::Run, 0),
            StageOutcome::success(Stage::Run)
        );
        let crashed = StageOutcome::from_code(Stage::Run, 3);
        assert!(!crashed.ok);
        assert_eq!(crashed.exit_code_equiv, 3);
    }

    #[test]
    fn the_two_wineserver_budgets_stay_distinct() {
        // PARITY.md: 5 s fatal (run) vs 4 s soft (stop). Never unify.
        assert_eq!(RUN_WINESERVER_WAIT, Duration::from_secs(5));
        assert_eq!(STOP_WINESERVER_WAIT, Duration::from_secs(4));
        assert_ne!(RUN_WINESERVER_WAIT, STOP_WINESERVER_WAIT);
        // The re-export from `stop` is the same constant.
        assert_eq!(stop::STOP_WINESERVER_WAIT, STOP_WINESERVER_WAIT);
    }

    #[test]
    fn check_ctx_forwards_every_launch_flag() {
        let (ctx, _) = ctx_with(StageOptions {
            bottle_name: Some("Steam".into()),
            verbose: true,
            no_audio: true,
            no_dashboard: true,
            wired: true,
            ..Default::default()
        });
        let cc = ctx.check_ctx();
        assert!(cc.opts.verbose && cc.opts.no_audio && cc.opts.no_dashboard && cc.opts.wired);
        assert_eq!(cc.opts.bottle_name.as_deref(), Some("Steam"));
        // doctor parity: adb probing stays on unless a caller turns it off.
        assert!(cc.opts.allow_adb_probes);
    }

    #[tokio::test]
    async fn the_operation_lock_admits_one_holder() {
        // Only the "held ⇒ reported" direction is deterministic: the test
        // binary runs tests in parallel, and a sibling test taking the lock
        // would make the converse flaky.
        let guard = acquire_operation_lock().await;
        assert!(operation_in_progress());
        assert!(OPERATION_LOCK.try_lock().is_err());
        drop(guard);
    }

    /// The in-process mutex is only half of the lock: two Sabrage processes
    /// (GUI and CLI) have one `OPERATION_LOCK` each, so the exclusion that
    /// actually protects `build-x64/` is the advisory file lock.
    ///
    /// `flock` is per open file description, so a second `File` on the same
    /// path — even in this same process — is exactly what a second process
    /// sees.
    #[tokio::test]
    async fn the_advisory_file_lock_excludes_a_second_holder_and_releases_on_drop() {
        let path = std::env::temp_dir().join(format!(
            "sabrage-oplock-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::remove_file(&path).ok();

        let FileLock::Held(held) = acquire_lock_file(&path, &CancellationToken::new()).await else {
            panic!("lock acquired");
        };
        let other = open_lock_file(&path).expect("second handle opens");
        assert!(
            matches!(other.try_lock(), Err(std::fs::TryLockError::WouldBlock)),
            "a second process must not be able to take the lock"
        );
        // The pid is written for a diagnostic "held by …" message.
        assert_eq!(
            std::fs::read_to_string(&path).unwrap().trim(),
            std::process::id().to_string()
        );

        drop(held);
        assert!(
            other.try_lock().is_ok(),
            "dropping the guard must release the file lock"
        );
        drop(other);
        std::fs::remove_file(&path).ok();
    }

    /// The real guard takes both halves. Only the "held ⇒ locked" direction is
    /// deterministic here (a sibling test may take the lock the instant this
    /// one lets go), the same asymmetry `the_operation_lock_admits_one_holder`
    /// documents.
    #[tokio::test]
    async fn acquire_operation_lock_takes_the_file_lock_too() {
        let guard = acquire_operation_lock().await;
        let probe = open_lock_file(&operation_lock_path()).expect("lock file opens");
        assert!(matches!(
            probe.try_lock(),
            Err(std::fs::TryLockError::WouldBlock)
        ));
        drop(probe);
        drop(guard);
    }

    /// The advisory-lock wait is cancellable: without this, Stop could not
    /// reach a stage queued behind another Sabrage process (the poll loop only
    /// ever woke for the lock becoming free).
    #[tokio::test]
    async fn the_file_lock_wait_gives_up_when_the_token_fires() {
        let path = std::env::temp_dir().join(format!(
            "sabrage-oplock-cancel-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::remove_file(&path).ok();
        // Stand in for the other process: a second open file description on the
        // same path is exactly what `flock` treats as a foreign holder.
        let blocker = open_lock_file(&path).expect("blocker opens");
        blocker.try_lock().expect("blocker takes the lock");

        // Already cancelled before the first probe.
        let precancelled = CancellationToken::new();
        precancelled.cancel();
        assert!(matches!(
            acquire_lock_file(&path, &precancelled).await,
            FileLock::Cancelled
        ));

        // Cancelled from another task while the poll loop is asleep.
        let cancel = CancellationToken::new();
        let c = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            c.cancel();
        });
        let waited =
            tokio::time::timeout(Duration::from_secs(5), acquire_lock_file(&path, &cancel))
                .await
                .expect("the wait must end when the token fires");
        assert!(matches!(waited, FileLock::Cancelled));

        drop(blocker);
        std::fs::remove_file(&path).ok();
    }

    /// A queued stage is visible (`StageStarted` before the wait, so the run id
    /// — and Cancel — exist during it) and cancellable, and it never reaches
    /// its dispatch.
    #[tokio::test]
    async fn a_queued_stage_announces_itself_and_cancels_out_of_the_wait() {
        let held = acquire_operation_lock().await;
        let (ctx, seen) = ctx_with(StageOptions {
            dry_run: true,
            ..Default::default()
        });
        let cancel = ctx.cancel.clone();
        let run_id = ctx.run_id;
        let task = tokio::spawn(async move { run_stage(Stage::Stop, &ctx).await });

        // The started event must arrive while the lock is still held elsewhere.
        let started = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let has = seen.lock().unwrap().iter().any(|ev| {
                    matches!(ev, StageEvent::StageStarted { stage, .. } if *stage == Stage::Stop)
                });
                if has {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await;
        assert!(
            started.is_ok(),
            "StageStarted must be emitted before the operation lock is taken"
        );

        cancel.cancel();
        let err = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("cancelling must end the wait")
            .expect("the stage task must not panic")
            .expect_err("a cancelled stage fails");
        assert!(matches!(err, SabrageError::Cancelled), "{err:?}");
        assert_eq!(err.exit_code(), 130);

        let events = seen.lock().unwrap().clone();
        assert!(events.iter().any(|ev| matches!(
            ev,
            StageEvent::StageFinished { run_id: r, ok: false, exit_code_equiv: 130, .. } if *r == run_id
        )));
        // The queue notice, and nothing from `stop` itself.
        assert!(events.iter().any(|ev| matches!(
            ev,
            StageEvent::Line { text, .. } if text.contains("waiting for another Sabrage operation")
        )));
        assert!(
            !events
                .iter()
                .any(|ev| matches!(ev, StageEvent::Section { .. })),
            "a cancelled stage must not have run: {events:?}"
        );
        drop(held);
    }

    /// The queue probe sees the cross-process half, which is the wait that
    /// actually lasts minutes (a `sabrage` CLI build).
    #[tokio::test]
    async fn operation_in_progress_anywhere_sees_the_advisory_file_lock() {
        let foreign = open_lock_file(&operation_lock_path()).expect("lock file opens");
        // A sibling test may hold the lock; then there is nothing to prove here
        // (and `_anywhere` is already true for the in-process reason).
        if foreign.try_lock().is_err() {
            assert!(operation_in_progress_anywhere());
            return;
        }
        assert!(operation_lock_file_busy());
        assert!(operation_in_progress_anywhere());
        drop(foreign);
    }

    /// A test-run lock file never lands in the user's real Sabrage store.
    #[test]
    fn the_test_lock_file_is_not_in_the_user_support_directory() {
        let path = operation_lock_path();
        assert!(
            !path.starts_with(crate::paths::sabrage_support_dir()),
            "{} must not be under the real support directory during tests",
            path.display()
        );
        assert!(path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains(OPERATION_LOCK_FILE_NAME));
    }

    /// A ctx whose session-state and OXRSys stores are scratch directories, so
    /// `live_session_block` reads fixtures rather than the real machine — and
    /// whose scratch root carries this binary's own contract, so the identity
    /// guard is satisfied and the *other* refusals are what the tests observe.
    fn ctx_at(root: &std::path::Path, bottle: Option<&str>) -> StageCtx {
        materialize_compiled_contract(root);
        let mut paths = Paths::new(root);
        paths.sabrage_appsup = root.join("Sabrage");
        paths.oxr_appsup = root.join("OXRSys");
        let opts = StageOptions {
            bottle_name: bottle.map(str::to_string),
            ..StageOptions::default()
        };
        StageCtx::new(paths, opts, null_sink(), CancellationToken::new())
    }

    /// Write a `session-state.json` whose recorded wine identity is **this**
    /// process: alive, and reporting the start time recorded for it, which is
    /// exactly what `classify` calls `Live`.
    fn write_live_session_state(ctx: &StageCtx) {
        let path = ctx.paths.session_state_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut state = crate::session::state::SessionState::new(
            Uuid::new_v4(),
            "FixtureBottle",
            "/bs",
            "/log",
            0,
        );
        state.wine = crate::process::ProcInfo::observe(std::process::id());
        assert!(state.wine.is_some(), "this process must be observable");
        std::fs::write(&path, serde_json::to_string(&state).unwrap()).unwrap();
    }

    /// setup/build/install replace what a running session has open, so they are
    /// refused outright; `stop` (the way out) and `run` (which reconciles for
    /// itself) are not.
    #[tokio::test]
    async fn run_stage_refuses_setup_build_and_install_while_a_session_is_live() {
        let _g = crate::session::lock_session_globals();
        let root = std::env::temp_dir().join(format!("sabrage-live-stage-{}", std::process::id()));
        let ctx = ctx_at(&root, Some("FixtureBottle"));
        write_live_session_state(&ctx);

        for stage in [Stage::Setup, Stage::Build, Stage::Install] {
            let err = run_stage(stage, &ctx).await.unwrap_err();
            let msg = err.to_string();
            assert!(
                msg.starts_with(&format!("refusing to run {stage} while a session is live")),
                "{msg}"
            );
        }
        assert!(!stage_is_forbidden_while_session_live(Stage::Run));
        assert!(!stage_is_forbidden_while_session_live(Stage::Stop));
        // `stop` still gets as far as its own bottle check, not the refusal.
        let stop_err = run_stage(Stage::Stop, &ctx_at(&root, None))
            .await
            .unwrap_err();
        assert!(stop_err
            .to_string()
            .starts_with("CrossOver bottle name required"));

        std::fs::remove_dir_all(&root).ok();
    }

    /// The refusal is checked before the operation lock **and again after it**.
    ///
    /// The window it closes: a stage admitted while the machine was idle waits
    /// (minutes, behind another Sabrage process's build), a `run` acquires
    /// first, publishes its live session and hands the lock back at its launch
    /// boundary — and the queued stage used to walk straight into `dispatch`
    /// and replace the artifacts of a game that was by then streaming.
    #[tokio::test]
    async fn a_queued_stage_is_refused_when_a_session_goes_live_during_the_wait() {
        let _g = crate::session::lock_session_globals();
        let root = std::env::temp_dir().join(format!("sabrage-queued-live-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        materialize_compiled_contract(&root);

        let mut paths = Paths::new(&root);
        paths.sabrage_appsup = root.join("Sabrage");
        paths.oxr_appsup = root.join("OXRSys");
        let oxr = paths.oxr_appsup.clone();
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let s = seen.clone();
        let sink: EventSink = Arc::new(move |ev| s.lock().unwrap().push(ev));
        let ctx = StageCtx::new(
            paths,
            StageOptions {
                // Nothing here may mutate even if the guard were to fail open.
                dry_run: true,
                bottle_name: Some("FixtureBottle".into()),
                ..StageOptions::default()
            },
            sink,
            CancellationToken::new(),
        );
        assert_eq!(
            live_session_block(&ctx.paths),
            None,
            "the fixture must be idle at admission"
        );

        let held = acquire_operation_lock().await;
        let task = tokio::spawn(async move { run_stage(Stage::Build, &ctx).await });

        // Admitted: the stage got past the pre-lock refusal and is now waiting.
        let started = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let has = seen.lock().unwrap().iter().any(|ev| {
                    matches!(ev, StageEvent::StageStarted { stage, .. } if *stage == Stage::Build)
                });
                if has {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await;
        assert!(started.is_ok(), "the queued stage must announce itself");

        // A session starts while it waits — the `run` that won the lock race.
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
            .expect("the queued stage must settle once the lock is free")
            .expect("the stage task must not panic")
            .expect_err("a stage that acquires the lock under a live session is refused");
        assert!(
            err.to_string()
                .starts_with("refusing to run build while a session is live"),
            "{err}"
        );

        let events = seen.lock().unwrap().clone();
        assert!(
            !events
                .iter()
                .any(|ev| matches!(ev, StageEvent::Section { .. })),
            "the refused stage must never have reached its dispatch: {events:?}"
        );
        // The bracket still closes: StageStarted was emitted before the wait.
        assert!(matches!(
            events.last(),
            Some(StageEvent::StageFinished {
                stage: Stage::Build,
                ok: false,
                ..
            })
        ));

        std::fs::remove_dir_all(&root).ok();
    }

    /// A scratch checkout whose `contract/` is *not* the one this binary was
    /// compiled from — the X-binary/Y-checkout skew `meta.contract-sync`
    /// reports and this guard refuses.
    fn skewed_checkout(tag: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("sabrage-skew-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(root.join("contract")).unwrap();
        for rel in crate::contract::CONTRACT_FILES {
            std::fs::write(
                root.join(rel),
                b"not-the-contract-this-binary-was-built-from\n",
            )
            .unwrap();
        }
        root
    }

    /// Every mutating stage refuses a checkout this binary does not describe,
    /// through both doors, before any event or executor call. `stop` stays open
    /// — it is the way out.
    #[tokio::test]
    async fn every_mutating_stage_refuses_a_checkout_the_binary_was_not_built_from() {
        let _g = crate::session::lock_session_globals();
        let root = skewed_checkout("stages");
        // The abort says exactly what the `meta.contract-sync` row says: both
        // read it from `checks::meta`, which owns the wording.
        let (expected_message, expected_remedy) =
            crate::checks::meta::assert_binary_matches_checkout(&root)
                .expect_err("the fixture must be a skewed checkout");
        for stage in [Stage::Setup, Stage::Build, Stage::Install, Stage::Run] {
            let (ctx, seen) = {
                let mut paths = Paths::new(&root);
                paths.sabrage_appsup = root.join("Sabrage");
                paths.oxr_appsup = root.join("OXRSys");
                let seen = Arc::new(StdMutex::new(Vec::new()));
                let s = seen.clone();
                let sink: EventSink = Arc::new(move |ev| s.lock().unwrap().push(ev));
                (
                    StageCtx::new(
                        paths,
                        StageOptions {
                            dry_run: true,
                            bottle_name: Some("FixtureBottle".into()),
                            ..StageOptions::default()
                        },
                        sink,
                        CancellationToken::new(),
                    ),
                    seen,
                )
            };

            let err = run_stage(stage, &ctx).await.unwrap_err();
            assert_eq!(err.to_string(), expected_message, "{stage}");
            assert!(
                matches!(&err, SabrageError::Fatal { remedy, .. }
                    if remedy.as_deref() == Some(expected_remedy.as_str())),
                "{stage}: the abort must carry the row's remedy: {err:?}"
            );
            let events = seen.lock().unwrap().clone();
            assert!(
                !events
                    .iter()
                    .any(|ev| matches!(ev, StageEvent::StageStarted { .. })),
                "the refusal precedes the stage banner: {events:?}"
            );

            // The holding-lock door is gated too — it is reached from the
            // launch preflight's whole-stage auto-fixes.
            let guard = acquire_operation_lock().await;
            let err = run_stage_holding_lock(stage, &ctx).await.unwrap_err();
            assert_eq!(err.to_string(), expected_message, "{stage}");
            drop(guard);
        }

        // `stop` is never gated: it gets as far as its own bottle check.
        let mut paths = Paths::new(&root);
        paths.sabrage_appsup = root.join("Sabrage");
        paths.oxr_appsup = root.join("OXRSys");
        let ctx = StageCtx::new(
            paths,
            StageOptions::default(),
            null_sink(),
            CancellationToken::new(),
        );
        let stop_err = run_stage(Stage::Stop, &ctx).await.unwrap_err();
        assert!(stop_err
            .to_string()
            .starts_with("CrossOver bottle name required"));

        std::fs::remove_dir_all(&root).ok();
    }

    /// The abort and the `meta.contract-sync` row say the same thing because
    /// both read it from `checks::meta`: a self-consistent-but-foreign checkout
    /// makes the doctor row and the stage refusal agree word for word.
    #[test]
    fn the_contract_skew_die_is_the_meta_row_verbatim() {
        let root = skewed_checkout("meta-text");
        std::fs::create_dir_all(root.join("scripts/demo")).unwrap();
        let checkout = crate::util::contract_hash(&root).expect("just-written files are readable");
        std::fs::write(
            root.join(crate::contract::CONTRACT_GEN_REL_PATH),
            format!("# contract-sha256: {checkout}\n"),
        )
        .unwrap();

        let outcome = crate::checks::registry()
            .get("meta.contract-sync")
            .expect("the slug is bound")
            .evaluate(&CheckCtx::new(Paths::new(&root), CheckOptions::new()));
        let (message, remedy) = crate::checks::meta::assert_binary_matches_checkout(&root)
            .expect_err("a foreign checkout must not pass the identity guard");
        assert_eq!(outcome.message, message);
        assert_eq!(outcome.remedy.as_deref(), Some(remedy.as_str()));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn step_emitter_attributes_rows() {
        let (ctx, seen) = ctx_with(StageOptions::default());
        let st = ctx.step(crate::events::step::INSTALL_BOTTLE);
        st.ok("ActiveRuntime registered");
        ctx.ok("no step");
        let evs = seen.lock().unwrap().clone();
        assert_eq!(evs[0].step(), Some("install.3.bottle"));
        assert_eq!(evs[1].step(), None);
    }
}
