//! Every mutating primitive, behind one trait — so `--dry-run` is a real
//! preview rather than a second, drifting code path (design-core §6.3).
//!
//! Two implementations:
//!
//! * [`RealExecutor`] does the thing.
//! * [`DryRunExecutor`] records a [`PlannedAction`] and does not. **Read-only
//!   probes still execute** — the byte compare behind `copy_if_changed`, the
//!   sha256 behind `download` — so the plan says *unchanged* vs *installed*
//!   truthfully instead of guessing.
//!
//! # Why `curl` and `tar` instead of crates
//!
//! `setup` execs `curl -fL --retry 3` and `tar -xzf` exactly like
//! `scripts/demo/setup.sh`. Same tool, same flags, same failure modes, same
//! progress output — and three fewer dependency trees (`reqwest`, `flate2`,
//! `tar`) in a binary whose whole point is to agree with a shell script.
//! `cp -R` for [`Executor::dir_copy`] is the same argument with teeth: CrossOver's
//! `lib/dxmt` tree contains symlinks, and `/bin/cp -R` preserves them where a
//! naive recursive walk would dereference them.
//!
//! # Async shape
//!
//! Methods return a boxed future rather than being `async fn`, because
//! [`crate::stages::StageCtx`] holds an `Arc<dyn Executor>` and `async fn` in a
//! trait is not object-safe. One convention, used consistently — see
//! [`BoxFuture`].

use std::fmt;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::{ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::error::{Result, SabrageError};
use crate::events::{RunId, StageEvent, StepId};
use crate::process::{self, ChildSpec};
use crate::stages::EventSink;

/// The one future shape this crate's object-safe traits return.
pub type BoxFuture<'a, T> = Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// Step id attributed to children the executor spawns on its own behalf when the
/// caller did not narrow it with [`Executor::with_step`].
pub const EXECUTOR_STEP: StepId = "executor";

// ── outcomes ──────────────────────────────────────────────────────────────────

/// `install_if_changed`'s two branches.
///
/// The **caller** prints the row, exactly as `lib.sh` does:
///
/// ```zsh
/// install_if_changed() { # src dst
///   if cmp -s "$1" "$2" 2>/dev/null; then info "unchanged: $2"
///   else cp "$1" "$2" || die "copy failed: $1 -> $2"; ok "installed: $2"; fi
/// }
/// ```
///
/// i.e. `info "unchanged: <dst>"` for [`Copied::Unchanged`] and
/// `ok "installed: <dst>"` for [`Copied::Copied`], with `<dst>` the full
/// destination path. Keeping the strings at the call site is what lets the dry
/// run render "would install: …" from the same outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Copied {
    /// Contents differed (or the destination was absent): the file was copied.
    Copied,
    /// `cmp -s` said the files are byte-identical: nothing was written.
    Unchanged,
}

/// `fetch_pinned`'s two branches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Downloaded {
    /// The destination already hashed to the pin: nothing was fetched.
    AlreadyPresent,
    /// Fetched to `<dest>.tmp`, verified, renamed into place.
    Fetched,
}

/// One thing a dry run *would* have done.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedAction {
    pub kind: PlannedKind,
    pub src: Option<PathBuf>,
    pub dst: Option<PathBuf>,
    /// Why — "differs from source", "already current", "sha256 pin missing".
    pub reason: String,
}

/// The vocabulary of [`PlannedAction::kind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PlannedKind {
    /// A file would be copied.
    Copy,
    /// A copy that would be skipped because the bytes already match.
    Skip,
    /// A file would be written atomically.
    Write,
    /// A directory tree would be created.
    CreateDir,
    /// A directory tree would be removed.
    RemoveDir,
    /// A single file would be removed.
    RemoveFile,
    /// A directory tree would be copied (`cp -R`).
    DirCopy,
    /// A pinned artifact would be downloaded.
    Download,
    /// An archive would be extracted.
    Extract,
    /// A file would be created if absent.
    Touch,
    /// A child process would be spawned.
    Spawn,
    /// A child process would be spawned **detached** — surviving this process,
    /// writing straight into a log file. Only the wine launch does this.
    SpawnDetached,
}

impl PlannedAction {
    /// One human line: what would happen, and why.
    ///
    /// This is what `--dry-run` prints after the stage's narrative rows, and it
    /// is the reason the plan is recorded at all — the narrative says
    /// `"install: <path>"` either way, only the plan distinguishes *would copy*
    /// from *would skip because the bytes already match*. Paths are rendered
    /// exactly as the executor received them (absolute, unabbreviated).
    pub fn describe(&self) -> String {
        let src = self
            .src
            .as_deref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "?".to_string());
        let dst = self
            .dst
            .as_deref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "?".to_string());
        let why = if self.reason.is_empty() {
            String::new()
        } else {
            format!(" ({})", self.reason)
        };
        match self.kind {
            PlannedKind::Copy => format!("would copy {src} → {dst}{why}"),
            PlannedKind::Skip => format!("would skip {dst}{why}"),
            PlannedKind::Write => format!("would write {dst}{why}"),
            PlannedKind::CreateDir => format!("would create directory {dst}{why}"),
            PlannedKind::RemoveDir => format!("would remove directory {dst}{why}"),
            PlannedKind::RemoveFile => format!("would remove {dst}{why}"),
            PlannedKind::DirCopy => format!("would copy directory {src} → {dst}{why}"),
            PlannedKind::Download => format!("would download {src} → {dst}{why}"),
            PlannedKind::Extract => format!("would extract {src} → {dst}{why}"),
            PlannedKind::Touch => format!("would create {dst} if absent{why}"),
            // Spawn carries the argv in `reason` and the child's cwd in `dst`,
            // so it reads as a command rather than as a file operation.
            PlannedKind::Spawn => match self.dst.as_deref() {
                Some(cwd) => format!("would spawn: {} (in {})", self.reason, cwd.display()),
                None => format!("would spawn: {}", self.reason),
            },
            // Like Spawn, `reason` is the argv; `dst` is the log file the
            // child's stdout+stderr would be redirected into (absent = the
            // shell's `>/dev/null 2>&1`, which is how the dashboard runs).
            PlannedKind::SpawnDetached => {
                let target = self
                    .dst
                    .as_deref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "/dev/null".to_string());
                format!("would launch (detached): {} > {}", self.reason, target)
            }
        }
    }
}

impl fmt::Display for PlannedAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.describe())
    }
}

/// The section title both front-ends print the dry-run plan under — the CLI as
/// `-- plan (dry run)` (its [`crate::events::StageEvent::Section`] shape), the
/// GUI as the same section in the run log.
pub const DRY_RUN_PLAN_TITLE: &str = "plan (dry run)";

/// What the plan section says when a stage recorded no action at all (an early
/// die, or a preflight that failed before the first mutating step). Printed
/// rather than omitted, so "the plan is empty" and "the plan was never
/// rendered" are distinguishable.
pub const DRY_RUN_PLAN_EMPTY: &str = "(nothing planned)";

/// The plan section's body: one [`PlannedAction::describe`] line per recorded
/// action in order, or a single [`DRY_RUN_PLAN_EMPTY`] line.
///
/// Lives here rather than in either front-end because both render it — the CLI
/// after a stage's narrative rows, the GUI's GateModal as trailing rows in the
/// same run log — and the two must say the same thing word for word.
pub fn dry_run_plan_body(plan: &[PlannedAction]) -> Vec<String> {
    if plan.is_empty() {
        return vec![DRY_RUN_PLAN_EMPTY.to_string()];
    }
    plan.iter().map(PlannedAction::describe).collect()
}

// ── the trait ─────────────────────────────────────────────────────────────────

/// Every filesystem or process mutation the pipeline performs.
///
/// Nothing outside this trait may write to disk or spawn a process on a stage's
/// behalf; that invariant is what makes `--dry-run` trustworthy.
pub trait Executor: Send + Sync + fmt::Debug {
    /// A view of this executor whose spawned children are attributed to `step`.
    /// Shares all state with the original (a dry run's plan list included).
    fn with_step(&self, step: StepId) -> Arc<dyn Executor>;

    /// True for [`DryRunExecutor`].
    fn is_dry_run(&self) -> bool {
        false
    }

    /// The recorded plan, newest last. Always empty for [`RealExecutor`].
    fn planned(&self) -> Vec<PlannedAction> {
        Vec::new()
    }

    /// `install_if_changed`: copy `src` over `dst` only when the bytes differ.
    /// Parent directories are **not** created (`cp` does not, and the shell
    /// `mkdir -p`s explicitly where it needs to).
    fn copy_if_changed<'a>(&'a self, src: &'a Path, dst: &'a Path)
        -> BoxFuture<'a, Result<Copied>>;

    /// Write `bytes` to `path` via a temp file in the same directory plus a
    /// rename, so a reader never sees a half-written file.
    fn write_atomic<'a>(&'a self, path: &'a Path, bytes: &'a [u8]) -> BoxFuture<'a, Result<()>>;

    /// `rm -rf path`. A missing path is success.
    fn remove_dir_all<'a>(&'a self, path: &'a Path) -> BoxFuture<'a, Result<()>>;

    /// `rm -f path`. A missing path is success.
    ///
    /// Deliberately per-file. The only file the pipeline removes on its own is
    /// ALVR's `session.json`, and [`Executor::remove_dir_all`] on its parent
    /// would take the whole `alvr/` directory — trusted-client state included —
    /// with it. Without this primitive that one deletion would have to bypass
    /// the executor, which is exactly how a "dry run" ends up deleting a file.
    fn remove_file<'a>(&'a self, path: &'a Path) -> BoxFuture<'a, Result<()>>;

    /// `mkdir -p path`.
    fn create_dir_all<'a>(&'a self, path: &'a Path) -> BoxFuture<'a, Result<()>>;

    /// `cp -R src dst` — symlink- and permission-preserving, via `/bin/cp`.
    fn dir_copy<'a>(&'a self, src: &'a Path, dst: &'a Path) -> BoxFuture<'a, Result<()>>;

    /// `lib.sh`'s `fetch_pinned`: skip when `dest` already hashes to `sha256`,
    /// else `curl -fL --retry 3` to `<dest>.tmp`, verify, rename.
    ///
    /// Unlike the other primitives this one **emits its own rows**, because they
    /// interleave with the transfer:
    ///
    /// * `info "already present: <label>"`,
    /// * `info "downloading <label> ..."` then curl's progress on stderr,
    /// * `ok "fetched <label> (sha256 verified)"`.
    ///
    /// Divergence (PARITY.md, "Setup"): the `.tmp` file is
    /// removed when the download or the hash check fails. `fetch_pinned` leaves
    /// it behind, where it confuses the next run.
    fn download<'a>(
        &'a self,
        url: &'a str,
        dest: &'a Path,
        sha256: &'a str,
        label: &'a str,
    ) -> BoxFuture<'a, Result<Downloaded>>;

    /// `tar -xzf archive -C into_dir`.
    fn tar_xzf<'a>(&'a self, archive: &'a Path, into_dir: &'a Path) -> BoxFuture<'a, Result<()>>;

    /// Create `path` empty if it does not exist. An existing file is left
    /// untouched — contents and mtime both (nothing in the pipeline needs an
    /// mtime bump; run's Goldberg flag files want truncate-create, which is
    /// [`Executor::write_atomic`] with empty bytes).
    fn touch<'a>(&'a self, path: &'a Path) -> BoxFuture<'a, Result<()>>;

    /// Spawn a child, streaming its output. Non-zero exit is returned as an
    /// `Ok(ExitStatus)`, not an error — see [`crate::process::run_ok`].
    fn run_child<'a>(&'a self, spec: &'a ChildSpec) -> BoxFuture<'a, Result<ExitStatus>>;

    /// Spawn a **detached** child: its own process group, no pipes this
    /// process pumps, and — the whole point — `kill_on_drop(false)`, so it
    /// outlives Sabrage.
    ///
    /// Exactly two callers, both in the run stage: the wine launch (with
    /// [`DetachedStdio::LogFile`]) and the ALVR dashboard (with
    /// [`DetachedStdio::Null`], mirroring run.sh's `>/dev/null 2>&1 &`).
    /// Everything else uses [`Executor::run_child`].
    ///
    /// This is the primitive [`crate::process::spawn_streamed`]'s header
    /// forbids itself from being: that one sets `kill_on_drop(true)`, which
    /// applied to the wine child would SIGKILL the CrossOver wine wrapper
    /// mid-session the moment Sabrage quits — leaving wineserver and the game
    /// running, orphaned, with the headset still streaming. Neither teardown
    /// nor detach (design-core §3.3; critique.md, "app-quit semantics for a
    /// live session"). App-quit runs the INT path *deliberately* or detaches
    /// *deliberately*; it never happens as a side effect of a dropped future.
    ///
    /// Returns `Ok(None)` under [`DryRunExecutor`] — a dry run never spawns —
    /// so callers must treat "no child" as the planned case, not as failure.
    fn spawn_detached<'a>(
        &'a self,
        spec: &'a ChildSpec,
        stdio: DetachedStdio,
    ) -> BoxFuture<'a, Result<Option<DetachedChild>>>;
}

/// Where a detached child's stdout and stderr go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetachedStdio {
    /// `>/dev/null 2>&1` — the ALVR dashboard.
    Null,
    /// Both pipes point at this one file, opened `create_new` (never
    /// truncating an existing log) and `dup`ed so the child's own writes
    /// interleave in order. The wine launch.
    LogFile(PathBuf),
}

/// A live detached child plus the identity that outlives it.
///
/// `identity` is what gets persisted: after Sabrage restarts, `child` is gone
/// but [`crate::process::ProcInfo::is_same_process`] can still tell the
/// original process from a recycled pid.
#[derive(Debug)]
pub struct DetachedChild {
    pub identity: process::ProcInfo,
    pub child: tokio::process::Child,
}

// ── real ──────────────────────────────────────────────────────────────────────

/// The executor that actually mutates the machine.
#[derive(Clone)]
pub struct RealExecutor {
    run_id: RunId,
    sink: EventSink,
    cancel: CancellationToken,
    step: StepId,
}

impl RealExecutor {
    pub fn new(run_id: RunId, sink: EventSink, cancel: CancellationToken) -> RealExecutor {
        RealExecutor {
            run_id,
            sink,
            cancel,
            step: EXECUTOR_STEP,
        }
    }

    /// Build a `ChildSpec` for one of this executor's internal children.
    fn spec(&self, program: &str) -> ChildSpec {
        ChildSpec::new(program, self.step, self.run_id)
    }

    /// Refuse to mutate anything once the run is cancelled.
    ///
    /// Every filesystem primitive calls this first, so cancellation lands
    /// between two file copies and not only at the next child-spawn boundary
    /// (`process::spawn_streamed`'s `select!`). Install's layers 1–3 are dozens
    /// of copies with no child in between; without this, Cancel kept writing
    /// DXMT dlls until the `reg add` child noticed.
    fn guard(&self) -> Result<()> {
        if self.cancel.is_cancelled() {
            return Err(SabrageError::Cancelled);
        }
        Ok(())
    }
}

impl fmt::Debug for RealExecutor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RealExecutor")
            .field("step", &self.step)
            .finish()
    }
}

impl Executor for RealExecutor {
    fn with_step(&self, step: StepId) -> Arc<dyn Executor> {
        Arc::new(RealExecutor {
            step,
            ..self.clone()
        })
    }

    fn copy_if_changed<'a>(
        &'a self,
        src: &'a Path,
        dst: &'a Path,
    ) -> BoxFuture<'a, Result<Copied>> {
        Box::pin(async move {
            self.guard()?;
            if crate::util::cmp_files(src, dst) {
                return Ok(Copied::Unchanged);
            }
            tokio::fs::copy(src, dst)
                .await
                .map_err(|e| SabrageError::io(dst, e))?;
            Ok(Copied::Copied)
        })
    }

    fn write_atomic<'a>(&'a self, path: &'a Path, bytes: &'a [u8]) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            self.guard()?;
            write_atomic_real(path, bytes).await
        })
    }

    fn remove_dir_all<'a>(&'a self, path: &'a Path) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            self.guard()?;
            match tokio::fs::remove_dir_all(path).await {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(SabrageError::io(path, e)),
            }
        })
    }

    fn remove_file<'a>(&'a self, path: &'a Path) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            self.guard()?;
            match tokio::fs::remove_file(path).await {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(SabrageError::io(path, e)),
            }
        })
    }

    fn create_dir_all<'a>(&'a self, path: &'a Path) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            self.guard()?;
            tokio::fs::create_dir_all(path)
                .await
                .map_err(|e| SabrageError::io(path, e))
        })
    }

    fn dir_copy<'a>(&'a self, src: &'a Path, dst: &'a Path) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let spec = self.spec("/bin/cp").arg("-R").arg(src).arg(dst);
            process::run_ok(&spec, &self.sink, &self.cancel).await?;
            Ok(())
        })
    }

    fn download<'a>(
        &'a self,
        url: &'a str,
        dest: &'a Path,
        sha256: &'a str,
        label: &'a str,
    ) -> BoxFuture<'a, Result<Downloaded>> {
        Box::pin(async move {
            let run_id = self.run_id;
            if crate::util::file_sha256_matches(dest, sha256) {
                (self.sink)(StageEvent::info(
                    run_id,
                    Some(self.step),
                    format!("already present: {label}"),
                ));
                return Ok(Downloaded::AlreadyPresent);
            }
            if let Some(parent) = dest.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|e| SabrageError::io(parent, e))?;
            }
            (self.sink)(StageEvent::info(
                run_id,
                Some(self.step),
                format!("downloading {label} ..."),
            ));
            let tmp = tmp_path(dest);
            let spec = self
                .spec("curl")
                .args(["-fL", "--retry", "3", "--progress-bar", "-o"])
                .arg(&tmp)
                .arg(url);
            let status = process::spawn_streamed(&spec, &self.sink, &self.cancel).await?;
            if !status.success() {
                let _ = tokio::fs::remove_file(&tmp).await; // DIVERGENCE: shell leaves it
                return Err(SabrageError::Download {
                    url: url.to_string(),
                    detail: Some(format!("curl exited {}", process::exit_code_of(status))),
                });
            }
            let got = crate::util::sha256_file(&tmp).map_err(|e| SabrageError::io(&tmp, e))?;
            if got != sha256 {
                let _ = tokio::fs::remove_file(&tmp).await; // DIVERGENCE: shell leaves it
                return Err(SabrageError::HashMismatch {
                    label: label.to_string(),
                    got,
                });
            }
            tokio::fs::rename(&tmp, dest)
                .await
                .map_err(|e| SabrageError::io(dest, e))?;
            (self.sink)(StageEvent::ok(
                run_id,
                Some(self.step),
                format!("fetched {label} (sha256 verified)"),
            ));
            Ok(Downloaded::Fetched)
        })
    }

    fn tar_xzf<'a>(&'a self, archive: &'a Path, into_dir: &'a Path) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let spec = self
                .spec("tar")
                .arg("-xzf")
                .arg(archive)
                .arg("-C")
                .arg(into_dir);
            process::run_ok(&spec, &self.sink, &self.cancel).await?;
            Ok(())
        })
    }

    fn touch<'a>(&'a self, path: &'a Path) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            self.guard()?;
            if path.exists() {
                return Ok(());
            }
            tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .await
                .map_err(|e| SabrageError::io(path, e))?;
            Ok(())
        })
    }

    fn run_child<'a>(&'a self, spec: &'a ChildSpec) -> BoxFuture<'a, Result<ExitStatus>> {
        Box::pin(async move { process::spawn_streamed(spec, &self.sink, &self.cancel).await })
    }

    fn spawn_detached<'a>(
        &'a self,
        spec: &'a ChildSpec,
        stdio: DetachedStdio,
    ) -> BoxFuture<'a, Result<Option<DetachedChild>>> {
        Box::pin(async move {
            self.guard()?;
            spawn_detached_real(spec, stdio).await.map(Some)
        })
    }
}

/// How long to keep asking `sysinfo` about a just-spawned pid before giving up
/// on observing its start time, and how often. macOS usually answers on the
/// first try; a loaded machine occasionally needs a second one.
const OBSERVE_RETRY: Duration = Duration::from_millis(40);
const OBSERVE_ATTEMPTS: u32 = 5;

async fn spawn_detached_real(spec: &ChildSpec, stdio: DetachedStdio) -> Result<DetachedChild> {
    let mut cmd = tokio::process::Command::new(&spec.program);
    cmd.args(&spec.args).stdin(Stdio::null());

    let log_path = match &stdio {
        DetachedStdio::Null => {
            cmd.stdout(Stdio::null()).stderr(Stdio::null());
            None
        }
        DetachedStdio::LogFile(path) => {
            // create_new: an existing log is NEVER truncated. On EEXIST the
            // caller picks another name (`logs::wine_log_candidate`'s `-2`
            // suffix on a same-second collision) — the one place where losing
            // a previous run's log would be silent and unrecoverable.
            let file = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(path)
                .map_err(|e| SabrageError::io(path, e))?;
            // One fd, dup'ed: stdout and stderr share a file offset, so the
            // child's interleaving is preserved exactly as `> >(tee $LOG) 2>&1`
            // preserves it.
            let err = file.try_clone().map_err(|e| SabrageError::io(path, e))?;
            cmd.stdout(Stdio::from(file)).stderr(Stdio::from(err));
            Some(path.clone())
        }
    };

    if let Some(dir) = &spec.cwd {
        cmd.current_dir(dir);
    }
    for (k, v) in &spec.env {
        cmd.env(k, v);
    }
    if let Some(path) = &spec.env_path {
        cmd.env("PATH", path);
    }
    // Own process group, like every other child — but NOT kill_on_drop: a
    // detached child must survive this process exiting. That is the whole
    // point of this function (design-core §3.3; critique.md's app-quit issue).
    cmd.process_group(0).kill_on_drop(false);

    let child = cmd.spawn().map_err(|e| {
        // A failed spawn leaves the freshly created log file behind with
        // nothing to write into it; the caller's next attempt would then skip
        // that name for no reason.
        if let Some(p) = &log_path {
            let _ = std::fs::remove_file(p);
        }
        SabrageError::io(PathBuf::from(&spec.program), e)
    })?;
    let pid = child.id().ok_or_else(|| {
        SabrageError::fatal_bare(format!(
            "{} exited before it could be supervised",
            spec.argv0()
        ))
    })?;

    Ok(DetachedChild {
        identity: observe_with_retry(pid, &spec.program).await,
        child,
    })
}

/// [`process::ProcInfo::observe`], retried for ~200 ms.
///
/// Falls back to `start_time: 0` when the process table still cannot see the
/// pid — a value no live process reports, so
/// [`process::ProcInfo::is_same_process`] answers `false` for it and the
/// reconcile path treats such a session as an identity mismatch (restore the
/// guards, never signal the pid) rather than trusting a bare pid.
async fn observe_with_retry(pid: u32, program: &std::ffi::OsStr) -> process::ProcInfo {
    for attempt in 0..OBSERVE_ATTEMPTS {
        if let Some(info) = process::ProcInfo::observe(pid) {
            return info;
        }
        if attempt + 1 < OBSERVE_ATTEMPTS {
            tokio::time::sleep(OBSERVE_RETRY).await;
        }
    }
    process::ProcInfo {
        pid,
        start_time: 0,
        exe: PathBuf::from(program),
    }
}

/// `<path>.tmp`, the way `fetch_pinned` spells it (`"$dest.tmp"` — a suffix on
/// the whole name, not an extension swap).
fn tmp_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".tmp");
    PathBuf::from(s)
}

async fn write_atomic_real(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let tmp = dir.join(format!(".sabrage-{}.tmp", uuid::Uuid::new_v4().as_simple()));
    tokio::fs::write(&tmp, bytes)
        .await
        .map_err(|e| SabrageError::io(&tmp, e))?;
    match tokio::fs::rename(&tmp, path).await {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            Err(SabrageError::io(path, e))
        }
    }
}

// ── dry run ───────────────────────────────────────────────────────────────────

/// Records what would happen. Read-only probes still run, so the plan is
/// accurate rather than optimistic.
#[derive(Clone)]
pub struct DryRunExecutor {
    #[allow(dead_code)]
    run_id: RunId,
    #[allow(dead_code)]
    sink: EventSink,
    #[allow(dead_code)]
    cancel: CancellationToken,
    step: StepId,
    plan: Arc<Mutex<Vec<PlannedAction>>>,
}

impl DryRunExecutor {
    pub fn new(run_id: RunId, sink: EventSink, cancel: CancellationToken) -> DryRunExecutor {
        DryRunExecutor {
            run_id,
            sink,
            cancel,
            step: EXECUTOR_STEP,
            plan: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn record(&self, kind: PlannedKind, src: Option<&Path>, dst: Option<&Path>, reason: &str) {
        if let Ok(mut p) = self.plan.lock() {
            p.push(PlannedAction {
                kind,
                src: src.map(Path::to_path_buf),
                dst: dst.map(Path::to_path_buf),
                reason: reason.to_string(),
            });
        }
    }
}

impl fmt::Debug for DryRunExecutor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DryRunExecutor")
            .field("step", &self.step)
            .field("planned", &self.planned().len())
            .finish()
    }
}

impl Executor for DryRunExecutor {
    fn with_step(&self, step: StepId) -> Arc<dyn Executor> {
        Arc::new(DryRunExecutor {
            step,
            ..self.clone()
        })
    }

    fn is_dry_run(&self) -> bool {
        true
    }

    fn planned(&self) -> Vec<PlannedAction> {
        self.plan.lock().map(|p| p.clone()).unwrap_or_default()
    }

    fn copy_if_changed<'a>(
        &'a self,
        src: &'a Path,
        dst: &'a Path,
    ) -> BoxFuture<'a, Result<Copied>> {
        Box::pin(async move {
            // Real compare: the plan must distinguish "would install" from
            // "would skip (unchanged)".
            if crate::util::cmp_files(src, dst) {
                self.record(
                    PlannedKind::Skip,
                    Some(src),
                    Some(dst),
                    "bytes already match",
                );
                Ok(Copied::Unchanged)
            } else {
                let reason = if dst.exists() {
                    "differs from source"
                } else {
                    "destination absent"
                };
                self.record(PlannedKind::Copy, Some(src), Some(dst), reason);
                Ok(Copied::Copied)
            }
        })
    }

    fn write_atomic<'a>(&'a self, path: &'a Path, bytes: &'a [u8]) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            self.record(
                PlannedKind::Write,
                None,
                Some(path),
                &format!("{} bytes", bytes.len()),
            );
            Ok(())
        })
    }

    fn remove_dir_all<'a>(&'a self, path: &'a Path) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let reason = if path.exists() {
                "exists"
            } else {
                "already absent"
            };
            self.record(PlannedKind::RemoveDir, None, Some(path), reason);
            Ok(())
        })
    }

    fn remove_file<'a>(&'a self, path: &'a Path) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let reason = if path.exists() {
                "exists"
            } else {
                "already absent"
            };
            self.record(PlannedKind::RemoveFile, None, Some(path), reason);
            Ok(())
        })
    }

    fn create_dir_all<'a>(&'a self, path: &'a Path) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let reason = if path.is_dir() {
                "already exists"
            } else {
                "absent"
            };
            self.record(PlannedKind::CreateDir, None, Some(path), reason);
            Ok(())
        })
    }

    fn dir_copy<'a>(&'a self, src: &'a Path, dst: &'a Path) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            self.record(PlannedKind::DirCopy, Some(src), Some(dst), "cp -R");
            Ok(())
        })
    }

    fn download<'a>(
        &'a self,
        url: &'a str,
        dest: &'a Path,
        sha256: &'a str,
        label: &'a str,
    ) -> BoxFuture<'a, Result<Downloaded>> {
        Box::pin(async move {
            // Real hash probe, so "already present" is the truth.
            if crate::util::file_sha256_matches(dest, sha256) {
                self.record(
                    PlannedKind::Skip,
                    None,
                    Some(dest),
                    &format!("{label}: already present (sha256 matches)"),
                );
                return Ok(Downloaded::AlreadyPresent);
            }
            self.record(
                PlannedKind::Download,
                Some(Path::new(url)),
                Some(dest),
                label,
            );
            Ok(Downloaded::Fetched)
        })
    }

    fn tar_xzf<'a>(&'a self, archive: &'a Path, into_dir: &'a Path) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            self.record(
                PlannedKind::Extract,
                Some(archive),
                Some(into_dir),
                "tar -xzf",
            );
            Ok(())
        })
    }

    fn touch<'a>(&'a self, path: &'a Path) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let reason = if path.exists() {
                "already exists"
            } else {
                "absent"
            };
            self.record(PlannedKind::Touch, None, Some(path), reason);
            Ok(())
        })
    }

    fn run_child<'a>(&'a self, spec: &'a ChildSpec) -> BoxFuture<'a, Result<ExitStatus>> {
        Box::pin(async move {
            self.record(
                PlannedKind::Spawn,
                None,
                spec.cwd.as_deref(),
                &spec.display(),
            );
            Ok(process::exit_status_from_code(0))
        })
    }

    fn spawn_detached<'a>(
        &'a self,
        spec: &'a ChildSpec,
        stdio: DetachedStdio,
    ) -> BoxFuture<'a, Result<Option<DetachedChild>>> {
        Box::pin(async move {
            let log = match &stdio {
                DetachedStdio::Null => None,
                DetachedStdio::LogFile(p) => Some(p.clone()),
            };
            self.record(
                PlannedKind::SpawnDetached,
                None,
                log.as_deref(),
                &spec.display(),
            );
            // No child, and no log file created either: a dry run of `run`
            // must not leave a zero-byte beatsaber-<ts>.log behind.
            Ok(None)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::step;

    fn sinks() -> (RunId, EventSink, CancellationToken) {
        let sink: EventSink = Arc::new(|_| {});
        (uuid::Uuid::new_v4(), sink, CancellationToken::new())
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sabrage-exec-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn copy_if_changed_matches_install_if_changed() {
        let dir = scratch("copy");
        let (run_id, sink, cancel) = sinks();
        let ex = RealExecutor::new(run_id, sink, cancel);
        let src = dir.join("src");
        let dst = dir.join("dst");
        std::fs::write(&src, b"payload").unwrap();

        // Absent destination -> copied.
        assert_eq!(
            ex.copy_if_changed(&src, &dst).await.unwrap(),
            Copied::Copied
        );
        assert_eq!(std::fs::read(&dst).unwrap(), b"payload");
        // Identical -> untouched.
        assert_eq!(
            ex.copy_if_changed(&src, &dst).await.unwrap(),
            Copied::Unchanged
        );
        // Differing -> copied again.
        std::fs::write(&dst, b"stale").unwrap();
        assert_eq!(
            ex.copy_if_changed(&src, &dst).await.unwrap(),
            Copied::Copied
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn write_atomic_replaces_and_leaves_no_temp_files() {
        let dir = scratch("atomic");
        let (run_id, sink, cancel) = sinks();
        let ex = RealExecutor::new(run_id, sink, cancel);
        let f = dir.join("out.json");
        ex.write_atomic(&f, b"one").await.unwrap();
        ex.write_atomic(&f, b"two").await.unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), b"two");
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n != "out.json")
            .collect();
        assert!(leftovers.is_empty(), "temp files left: {leftovers:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn dry_run_probes_for_real_but_writes_nothing() {
        let dir = scratch("dry");
        let (run_id, sink, cancel) = sinks();
        let ex = DryRunExecutor::new(run_id, sink, cancel);
        let src = dir.join("src");
        let same = dir.join("same");
        let missing = dir.join("missing");
        std::fs::write(&src, b"x").unwrap();
        std::fs::write(&same, b"x").unwrap();

        assert_eq!(
            ex.copy_if_changed(&src, &same).await.unwrap(),
            Copied::Unchanged
        );
        assert_eq!(
            ex.copy_if_changed(&src, &missing).await.unwrap(),
            Copied::Copied
        );
        assert!(!missing.exists(), "dry run wrote a file");

        let plan = ex.planned();
        assert_eq!(plan.len(), 2);
        assert_eq!(plan[0].kind, PlannedKind::Skip);
        assert_eq!(plan[0].reason, "bytes already match");
        assert_eq!(plan[1].kind, PlannedKind::Copy);
        assert_eq!(plan[1].reason, "destination absent");
        assert!(ex.is_dry_run());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn dry_run_child_reports_success_without_spawning() {
        let (run_id, sink, cancel) = sinks();
        let ex = DryRunExecutor::new(run_id, sink, cancel);
        let spec = ChildSpec::new("/bin/false", step::BUILD_TOOLS, uuid::Uuid::nil());
        let status = ex.run_child(&spec).await.unwrap();
        assert!(status.success());
        assert_eq!(ex.planned()[0].kind, PlannedKind::Spawn);
        assert_eq!(ex.planned()[0].reason, "/bin/false");
    }

    #[test]
    fn with_step_shares_the_plan() {
        let (run_id, sink, cancel) = sinks();
        let ex = DryRunExecutor::new(run_id, sink, cancel);
        let narrowed = ex.with_step(step::SETUP_PINNED);
        ex.record(PlannedKind::Touch, None, None, "from the original");
        assert_eq!(narrowed.planned().len(), 1);
    }

    #[tokio::test]
    async fn remove_file_deletes_and_tolerates_a_missing_path() {
        let dir = scratch("rmfile");
        let (run_id, sink, cancel) = sinks();
        let ex = RealExecutor::new(run_id, sink, cancel);
        let f = dir.join("session.json");
        std::fs::write(&f, b"{}").unwrap();
        ex.remove_file(&f).await.unwrap();
        assert!(!f.exists());
        // Idempotent: a second removal is success, not ENOENT.
        ex.remove_file(&f).await.unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn dry_run_records_a_removal_instead_of_performing_it() {
        let dir = scratch("dry-rmfile");
        let (run_id, sink, cancel) = sinks();
        let ex = DryRunExecutor::new(run_id, sink, cancel);
        let f = dir.join("session.json");
        std::fs::write(&f, b"{}").unwrap();
        ex.remove_file(&f).await.unwrap();
        assert!(f.is_file(), "dry run deleted a file");
        let plan = ex.planned();
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].kind, PlannedKind::RemoveFile);
        assert_eq!(plan[0].dst.as_deref(), Some(f.as_path()));
        assert_eq!(plan[0].reason, "exists");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Cancellation must land inside a run of pure filesystem work, not only at
    /// the next child-spawn boundary — install layers 1–3 have no child at all.
    #[tokio::test]
    async fn a_cancelled_run_refuses_every_filesystem_mutation() {
        let dir = scratch("cancelled");
        let (run_id, sink, _) = sinks();
        let cancel = CancellationToken::new();
        cancel.cancel();
        let ex = RealExecutor::new(run_id, sink, cancel);

        let src = dir.join("src");
        std::fs::write(&src, b"payload").unwrap();
        let dst = dir.join("dst");
        let out = dir.join("out");
        let sub = dir.join("sub");
        let victim = dir.join("victim");
        std::fs::write(&victim, b"x").unwrap();

        for e in [
            ex.copy_if_changed(&src, &dst).await.err(),
            ex.write_atomic(&out, b"bytes").await.err(),
            ex.create_dir_all(&sub).await.err(),
            ex.remove_dir_all(&dir).await.err(),
            ex.remove_file(&victim).await.err(),
            ex.touch(&out).await.err(),
        ] {
            assert!(
                matches!(e, Some(SabrageError::Cancelled)),
                "expected Cancelled, got {e:?}"
            );
        }
        // And nothing happened.
        assert!(!dst.exists() && !out.exists() && !sub.exists());
        assert!(victim.is_file() && dir.is_dir());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn every_planned_kind_renders_one_readable_line() {
        let act = |kind, src: Option<&str>, dst: Option<&str>, reason: &str| PlannedAction {
            kind,
            src: src.map(PathBuf::from),
            dst: dst.map(PathBuf::from),
            reason: reason.to_string(),
        };
        let cases = [
            (
                act(
                    PlannedKind::Copy,
                    Some("/a/x"),
                    Some("/b/x"),
                    "differs from source",
                ),
                "would copy /a/x → /b/x (differs from source)",
            ),
            (
                act(
                    PlannedKind::Skip,
                    Some("/a/x"),
                    Some("/b/x"),
                    "bytes already match",
                ),
                "would skip /b/x (bytes already match)",
            ),
            (
                act(PlannedKind::Write, None, Some("/b/f.toml"), "412 bytes"),
                "would write /b/f.toml (412 bytes)",
            ),
            (
                act(PlannedKind::CreateDir, None, Some("/b/d"), "absent"),
                "would create directory /b/d (absent)",
            ),
            (
                act(PlannedKind::RemoveDir, None, Some("/b/d"), "exists"),
                "would remove directory /b/d (exists)",
            ),
            (
                act(
                    PlannedKind::RemoveFile,
                    None,
                    Some("/b/session.json"),
                    "exists",
                ),
                "would remove /b/session.json (exists)",
            ),
            (
                act(PlannedKind::DirCopy, Some("/a/d"), Some("/b/d"), "cp -R"),
                "would copy directory /a/d → /b/d (cp -R)",
            ),
            (
                act(
                    PlannedKind::Download,
                    Some("https://h/x.tgz"),
                    Some("/b/x.tgz"),
                    "DXMT",
                ),
                "would download https://h/x.tgz → /b/x.tgz (DXMT)",
            ),
            (
                act(
                    PlannedKind::Extract,
                    Some("/b/x.tgz"),
                    Some("/b"),
                    "tar -xzf",
                ),
                "would extract /b/x.tgz → /b (tar -xzf)",
            ),
            (
                act(PlannedKind::Touch, None, Some("/b/flag"), "absent"),
                "would create /b/flag if absent (absent)",
            ),
            (
                act(PlannedKind::Spawn, None, None, "git submodule update"),
                "would spawn: git submodule update",
            ),
            (
                act(PlannedKind::Spawn, None, Some("/repo"), "ninja -C build"),
                "would spawn: ninja -C build (in /repo)",
            ),
            (
                act(
                    PlannedKind::SpawnDetached,
                    None,
                    Some("/repo/logs/beatsaber-20260829-101112.log"),
                    "wine --bottle Steam --no-update --cx-app C:\\Beat Saber.exe",
                ),
                "would launch (detached): wine --bottle Steam --no-update --cx-app \
                 C:\\Beat Saber.exe > /repo/logs/beatsaber-20260829-101112.log",
            ),
            (
                act(PlannedKind::SpawnDetached, None, None, "alvr_dashboard"),
                "would launch (detached): alvr_dashboard > /dev/null",
            ),
        ];
        for (action, want) in cases {
            assert_eq!(action.describe(), want);
            // Display and describe() are the same one line.
            assert_eq!(action.to_string(), want);
        }
    }

    #[test]
    fn tmp_path_appends_the_suffix_to_the_whole_name() {
        assert_eq!(
            tmp_path(Path::new("/a/b/dxmt-artifacts.tar.gz")),
            PathBuf::from("/a/b/dxmt-artifacts.tar.gz.tmp")
        );
    }
}

#[cfg(test)]
mod detached_tests {
    use super::*;
    use crate::events::step;

    fn sinks() -> (RunId, EventSink, CancellationToken) {
        let sink: EventSink = Arc::new(|_| {});
        (uuid::Uuid::new_v4(), sink, CancellationToken::new())
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sabrage-detach-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The whole point of the primitive: both pipes land in the log, the child
    /// outlives the future that spawned it, and its identity is observable.
    #[tokio::test]
    async fn a_detached_child_writes_both_pipes_into_the_log_and_is_identified() {
        let dir = scratch("log");
        let log = dir.join("beatsaber-20260829-101112.log");
        let (run_id, sink, cancel) = sinks();
        let ex = RealExecutor::new(run_id, sink, cancel);
        let spec = ChildSpec::new("/bin/sh", step::BUILD_TOOLS, run_id)
            .arg("-c")
            .arg("printf 'out\\n'; printf 'err\\n' >&2");

        let mut d = ex
            .spawn_detached(&spec, DetachedStdio::LogFile(log.clone()))
            .await
            .unwrap()
            .expect("a real executor spawns");
        assert_eq!(d.identity.pid, d.child.id().unwrap());
        d.child.wait().await.unwrap();

        let text = std::fs::read_to_string(&log).unwrap();
        assert!(text.contains("out") && text.contains("err"), "{text:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// `create_new`: an existing log is never truncated — the caller must pick
    /// another name (`logs::wine_log_candidate`'s `-2` suffix).
    #[tokio::test]
    async fn an_existing_log_is_never_truncated() {
        let dir = scratch("exists");
        let log = dir.join("beatsaber-20260829-101112.log");
        std::fs::write(&log, b"a previous run\n").unwrap();
        let (run_id, sink, cancel) = sinks();
        let ex = RealExecutor::new(run_id, sink, cancel);
        let spec = ChildSpec::new("/bin/echo", step::BUILD_TOOLS, run_id);

        let err = ex
            .spawn_detached(&spec, DetachedStdio::LogFile(log.clone()))
            .await
            .unwrap_err();
        assert_eq!(err.kind(), "io");
        assert_eq!(std::fs::read(&log).unwrap(), b"a previous run\n");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn a_dry_run_neither_spawns_nor_creates_the_log() {
        let dir = scratch("dry");
        let log = dir.join("beatsaber-20260829-101112.log");
        let (run_id, sink, cancel) = sinks();
        let ex = DryRunExecutor::new(run_id, sink, cancel);
        let spec = ChildSpec::new("/bin/echo", step::BUILD_TOOLS, run_id).arg("hi");

        assert!(ex
            .spawn_detached(&spec, DetachedStdio::LogFile(log.clone()))
            .await
            .unwrap()
            .is_none());
        assert!(!log.exists(), "dry run created the log file");

        let plan = ex.planned();
        assert_eq!(plan[0].kind, PlannedKind::SpawnDetached);
        assert_eq!(plan[0].dst.as_deref(), Some(log.as_path()));
        assert!(plan[0]
            .describe()
            .ends_with(&format!("> {}", log.display())));

        // Null stdio renders as /dev/null, the dashboard's shape.
        ex.spawn_detached(&spec, DetachedStdio::Null).await.unwrap();
        assert_eq!(
            ex.planned()[1].describe(),
            "would launch (detached): /bin/echo hi > /dev/null"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn a_cancelled_run_refuses_to_launch() {
        let (run_id, sink, _) = sinks();
        let cancel = CancellationToken::new();
        cancel.cancel();
        let ex = RealExecutor::new(run_id, sink, cancel);
        let spec = ChildSpec::new("/bin/echo", step::BUILD_TOOLS, run_id);
        let err = ex
            .spawn_detached(&spec, DetachedStdio::Null)
            .await
            .unwrap_err();
        assert!(matches!(err, SabrageError::Cancelled));
    }
}
