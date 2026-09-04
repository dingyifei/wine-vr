//! Every mutating primitive, behind one trait — so `--dry-run` is a real
//! preview rather than a second, drifting code path (design-core §6.3).
//!
//! * [`RealExecutor`] does the thing.
//! * [`DryRunExecutor`] records a [`PlannedAction`] and does not. **Read-only
//!   probes still execute** — the byte compare behind `copy_if_changed`, the
//!   sha256 behind `download` — so the plan says *unchanged* vs *installed*
//!   truthfully instead of guessing.
//!
//! `setup` execs `curl -fL --retry 3` and `tar -xzf` rather than linking
//! `reqwest`/`flate2`/`tar`: same tool, same flags, same failure modes as
//! `scripts/demo/setup.sh`. [`Executor::dir_copy`] shells out to `/bin/cp -R`
//! because CrossOver's `lib/dxmt` tree contains symlinks a naive recursive
//! walk would dereference.
//!
//! Methods return a [`BoxFuture`] because [`crate::stages::StageCtx`] holds an
//! `Arc<dyn Executor>` and `async fn` in a trait is not object-safe.

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

/// `install_if_changed`'s two branches (scripts/demo/lib.sh).
///
/// The **caller** prints the row: `info "unchanged: <dst>"` for
/// [`Copied::Unchanged`], `ok "installed: <dst>"` for [`Copied::Copied`], with
/// `<dst>` the full destination path. Keeping the strings at the call site is
/// what lets the dry run render "would install: …" from the same outcome.
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
    /// A second name would be created for an existing file (`link(2)`).
    Link,
    /// A pinned artifact would be downloaded.
    Download,
    /// An archive would be extracted.
    Extract,
    /// A file would be created if absent.
    Touch,
    /// A child process would be spawned.
    Spawn,
    /// A child process would be spawned **detached** — surviving this process,
    /// with its output going to a log file or to `/dev/null`.
    SpawnDetached,
}

impl PlannedAction {
    /// One human line: what would happen, and why.
    ///
    /// Paths are rendered exactly as the executor received them (absolute,
    /// unabbreviated). A stage's narrative rows say `"install: <path>"` either
    /// way; only this line distinguishes *would copy* from *would skip because
    /// the bytes already match*, which is why the plan is recorded at all.
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
            PlannedKind::Link => format!("would hard-link {src} → {dst}{why}"),
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

    /// `install_if_changed`: copy `src` over `dst` when the bytes differ; when
    /// only the mode differs, `dst`'s mode is repaired and the result is still
    /// [`Copied::Copied`]. Parent directories are **not** created (`cp` does
    /// not, and the shell `mkdir -p`s explicitly where it needs to).
    fn copy_if_changed<'a>(&'a self, src: &'a Path, dst: &'a Path)
        -> BoxFuture<'a, Result<Copied>>;

    /// Write `bytes` to `path` via a temp file in the same directory plus a
    /// rename, so a reader never sees a half-written file.
    fn write_atomic<'a>(&'a self, path: &'a Path, bytes: &'a [u8]) -> BoxFuture<'a, Result<()>>;

    /// Create `path` with `bytes` **only if it does not exist**: `Ok(true)`
    /// when this call created it, `Ok(false)` when something else got there
    /// first (and the file was left untouched).
    ///
    /// Write-once documents (`oxrsys-runtime.toml` above all) go through this
    /// rather than [`Executor::write_atomic`] because `exists()`-then-write is
    /// a race whose loser silently replaces a hand-edited config; an exclusive
    /// publish makes "did I create it?" the kernel's answer
    /// (tests::create_new_never_clobbers_an_existing_file).
    ///
    /// The default implementation is the racy check-then-write, kept only so a
    /// decorating [`Executor`] (the test doubles) inherits sane behaviour; both
    /// executors in this module override it.
    fn create_new<'a>(&'a self, path: &'a Path, bytes: &'a [u8]) -> BoxFuture<'a, Result<bool>> {
        Box::pin(async move {
            if path.exists() {
                return Ok(false);
            }
            self.write_atomic(path, bytes).await?;
            Ok(true)
        })
    }

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

    /// [`Executor::remove_dir_all`], performed **even when the run has been
    /// cancelled**.
    ///
    /// For one shape only: undoing a partial mutation this stage just made and
    /// must not leave behind — install's `cp -R` of stock DXMT, interrupted
    /// part-way, leaves a truncated tree that every later run would accept as a
    /// finished backup purely because it exists, and Cancel is the likeliest
    /// reason that copy stopped. Every ordinary removal keeps using
    /// [`Executor::remove_dir_all`], which stops at cancellation like every
    /// other mutation.
    fn remove_dir_all_rollback<'a>(&'a self, path: &'a Path) -> BoxFuture<'a, Result<()>> {
        self.remove_dir_all(path)
    }

    /// `mkdir -p path`.
    fn create_dir_all<'a>(&'a self, path: &'a Path) -> BoxFuture<'a, Result<()>>;

    /// `link(2)`: a second name, `dst`, for the bytes `src` names **at this
    /// instant**. Never replaces anything — a taken `dst` fails with
    /// [`std::io::ErrorKind::AlreadyExists`].
    ///
    /// Sabrage uses it to capture the inode a write is about to displace: an
    /// outside editor that does not participate in the config lock can save
    /// between the last comparison and the rename, and a link taken just before
    /// the rename keeps those bytes recoverable instead of losing them.
    ///
    /// The default implementation is a copy (a decorating [`Executor`]'s sane
    /// fallback; it inherits dry-run honesty from
    /// [`Executor::copy_if_changed`]). Both executors here override it.
    fn hard_link<'a>(&'a self, src: &'a Path, dst: &'a Path) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move { self.copy_if_changed(src, dst).await.map(|_| ()) })
    }

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
    /// Divergence: the `.tmp` file is removed when the download or the hash
    /// check fails (PARITY.md § Setup, "A pinned download's `.tmp` file is
    /// removed when curl or the sha256 check fails").
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
    /// Exactly two callers, both in the run stage: wine launch ([`DetachedStdio::LogFile`])
    /// and ALVR dashboard ([`DetachedStdio::Null`]). Everything else uses [`Executor::run_child`].
    ///
    /// Never [`crate::process::spawn_streamed`] for those two: its
    /// `kill_on_drop(true)` SIGKILLs the CrossOver wine wrapper the moment
    /// Sabrage quits, orphaning wineserver and the game (design-core §3.3;
    /// PARITY.md § Run (launch), "The wine child is spawned in its **own
    /// process group**").
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
    /// (`process::spawn_streamed`'s `select!`): install's layers 1-3 are dozens
    /// of copies with no child in between.
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
                // Bytes match — but a staged file that lost its execute bit is
                // *not* installed, and rebuilding cannot repair it (bytes never
                // change; checks/build.rs requires the bit, fixes/helper.rs
                // restages through this primitive). Repair the mode and report
                // as work done, where lib.sh's `install_if_changed` would print
                // `unchanged: <dst>` (PARITY.md § Install (the one privileged
                // write), "`copy_if_changed` repairs the destination's mode
                // when the bytes already match").
                return match (mode_of(src), mode_of(dst)) {
                    (Some(want), Some(have)) if want != have => {
                        tokio::fs::set_permissions(dst, permissions(want))
                            .await
                            .map_err(|e| SabrageError::io(dst, e))?;
                        Ok(Copied::Copied)
                    }
                    _ => Ok(Copied::Unchanged),
                };
            }
            copy_atomic(src, dst).await?;
            Ok(Copied::Copied)
        })
    }

    fn write_atomic<'a>(&'a self, path: &'a Path, bytes: &'a [u8]) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            self.guard()?;
            write_atomic_real(path, bytes).await
        })
    }

    fn create_new<'a>(&'a self, path: &'a Path, bytes: &'a [u8]) -> BoxFuture<'a, Result<bool>> {
        Box::pin(async move {
            self.guard()?;
            create_new_real(path, bytes).await
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

    fn remove_dir_all_rollback<'a>(&'a self, path: &'a Path) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            // Deliberately no `guard()`: this undoes a mutation that already
            // happened, and a cancelled run is the likeliest reason it has to
            // run at all. Refusing here is what leaves the half-copy behind.
            match tokio::fs::remove_dir_all(path).await {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(SabrageError::io(path, e)),
            }
        })
    }

    fn hard_link<'a>(&'a self, src: &'a Path, dst: &'a Path) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            self.guard()?;
            tokio::fs::hard_link(src, dst)
                .await
                .map_err(|e| SabrageError::io(dst, e))
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
    // detached child must survive this process exiting (design-core §3.3;
    // PARITY.md § Run (launch), "The wine child is spawned in its **own
    // process group**").
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

/// The permission bits of `p`, when it exists.
fn mode_of(p: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p)
        .ok()
        .map(|m| m.permissions().mode() & 0o7777)
}

fn permissions(mode: u32) -> std::fs::Permissions {
    use std::os::unix::fs::PermissionsExt;
    std::fs::Permissions::from_mode(mode)
}

/// A unique temp name beside `path`, for the write-then-rename dance.
fn sibling_tmp(path: &Path) -> PathBuf {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    dir.join(format!(".sabrage-{}.tmp", uuid::Uuid::new_v4().as_simple()))
}

/// `cp src dst`, via a sibling temp file and a rename.
///
/// The destination is never truncated: `std::fs::copy` opens the destination
/// `create|truncate`, so a failure *after* that (ENOSPC, an unreadable source,
/// this process being killed mid-copy) leaves a damaged file behind — and the
/// destinations here are CrossOver's global DXMT overlay and `wineopenxr`,
/// where one damaged file breaks every bottle (install.rs names ENOSPC among
/// the expected failures). Either the old bytes stay or the new ones land.
///
/// Every failure is reported against `dst`, not against the temp: that is the
/// path the caller prints, and the path `privilege::classify_write_error`
/// reasons about when it decides whether a `PermissionDenied` is App
/// Management (TCC).
///
/// The one case this loses to plain `cp`: a destination *directory* that
/// forbids creating files while the existing destination file is writable. No
/// install target is shaped that way (the DXMT overlay, `lib/wine`, and the
/// bottle's `system32` are all user-owned directories), and a half-written
/// global dylib is the worse failure by a distance.
async fn copy_atomic(src: &Path, dst: &Path) -> Result<()> {
    let tmp = sibling_tmp(dst);
    let result = async {
        tokio::fs::copy(src, &tmp)
            .await
            .map_err(|e| SabrageError::io(dst, e))?;
        // `fs::copy` already carries the source's mode over; make it explicit
        // so the execute bit is part of the contract rather than a side effect.
        if let Some(mode) = mode_of(src) {
            tokio::fs::set_permissions(&tmp, permissions(mode))
                .await
                .map_err(|e| SabrageError::io(dst, e))?;
        }
        tokio::fs::rename(&tmp, dst)
            .await
            .map_err(|e| SabrageError::io(dst, e))
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&tmp).await;
    }
    result
}

/// Write `bytes` to `path` through a sibling temp file and a rename.
///
/// Durable, not merely atomic: the temp is `fsync`ed before the rename and the
/// containing directory after it, because the caller ordering that matters
/// most here is persist-before-mutate — `run`'s audio guard saves the recovery
/// record and *then* switches the Mac's output device (paths.rs,
/// `session_state_path`). Without the syncs a power loss can lose the record
/// while the device switch survives, which is the one state that file exists to
/// prevent.
///
/// Mode: an existing destination keeps its bits; a fresh file lands at 0644
/// regardless of this process's umask (the temp is created 0600 so no reader
/// can ever see it wider than the final file).
async fn write_atomic_real(path: &Path, bytes: &[u8]) -> Result<()> {
    use tokio::io::AsyncWriteExt;

    let tmp = sibling_tmp(path);
    let mode = mode_of(path).unwrap_or(0o644);
    let result = async {
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)
            .await
            .map_err(|e| SabrageError::io(&tmp, e))?;
        file.write_all(bytes)
            .await
            .map_err(|e| SabrageError::io(&tmp, e))?;
        // Mode first, *then* the sync: `fsync` flushes the inode's metadata
        // along with its contents, so a chmod after the sync would be the one
        // part of the published file that was never made durable.
        tokio::fs::set_permissions(&tmp, permissions(mode))
            .await
            .map_err(|e| SabrageError::io(&tmp, e))?;
        file.sync_all()
            .await
            .map_err(|e| SabrageError::io(&tmp, e))?;
        drop(file);
        tokio::fs::rename(&tmp, path)
            .await
            .map_err(|e| SabrageError::io(path, e))
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&tmp).await;
        return result;
    }
    // The rename is only as durable as the directory entry it created, so the
    // parent is synced too — and a failure there is *reported*: the audio guard
    // acts on "persisted" for `session-state.json` by switching the Mac's
    // output device (tests::a_parent_that_cannot_be_synced_is_reported_not_swallowed).
    sync_parent_dir(path).await
}

/// `fsync` the directory that contains `path`, so the rename that published it
/// survives a power loss and not merely a crash.
///
/// Failures propagate, with one exception: a filesystem that refuses `fsync` on
/// a directory fd at all cannot offer this guarantee to anybody, and failing
/// every write there would cost more than the durability it cannot give.
/// `EIO` — the errno that actually means the entry may be lost — is an error.
async fn sync_parent_dir(path: &Path) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let dir = tokio::fs::File::open(parent)
        .await
        .map_err(|e| SabrageError::io(parent, e))?;
    match dir.sync_all().await {
        Ok(()) => Ok(()),
        Err(e) if dir_fsync_unsupported(&e) => Ok(()),
        Err(e) => Err(SabrageError::io(parent, e)),
    }
}

/// Does this `fsync` failure mean "this filesystem does not do that", rather
/// than "your data may be gone"?
fn dir_fsync_unsupported(e: &std::io::Error) -> bool {
    use nix::libc;
    // Written as comparisons rather than a `matches!`: `ENOTSUP` and
    // `EOPNOTSUPP` are the same number on some platforms, which would make one
    // of two constant patterns unreachable.
    let Some(errno) = e.raw_os_error() else {
        return false;
    };
    errno == libc::ENOTSUP
        || errno == libc::EOPNOTSUPP
        || errno == libc::EINVAL
        || errno == libc::EBADF
}

/// Create `path` with `bytes` only if it does not exist: `Ok(true)` when this
/// call created it, `Ok(false)` when something else got there first.
///
/// The exclusive create happens on a **sibling temp** that is written, chmodded
/// and `fsync`ed first, and the final name is then claimed with `link(2)` —
/// which, like `O_EXCL`, refuses to replace an existing name, so "did I create
/// it?" is the kernel's answer rather than a stale `exists()`. Claiming the
/// final name before the bytes are written would let a SIGKILL or a power loss
/// strand an empty file that every later call answers `Ok(false)` for — and an
/// empty `oxrsys-runtime.toml` is *valid TOML*, so setup would treat a
/// zero-byte config as hand-edited content it must not overwrite
/// (tests::create_new_publishes_finished_bytes_and_leaves_no_temp).
async fn create_new_real(path: &Path, bytes: &[u8]) -> Result<bool> {
    use tokio::io::AsyncWriteExt;

    let tmp = sibling_tmp(path);
    let staged = async {
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o644)
            .open(&tmp)
            .await
            .map_err(|e| SabrageError::io(path, e))?;
        file.write_all(bytes)
            .await
            .map_err(|e| SabrageError::io(path, e))?;
        // Explicit, so the mode does not depend on this process's umask (the
        // GUI inherits Finder's, the CLI a login shell's), and set before the
        // sync so the published inode's metadata is durable too.
        tokio::fs::set_permissions(&tmp, permissions(0o644))
            .await
            .map_err(|e| SabrageError::io(path, e))?;
        file.sync_all().await.map_err(|e| SabrageError::io(path, e))
    }
    .await;
    if let Err(e) = staged {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(e);
    }
    // `link(2)` publishes the finished bytes under the final name or fails with
    // `EEXIST`; either way the temp goes, so no `.sabrage-*.tmp` survives.
    let published = tokio::fs::hard_link(&tmp, path).await;
    let _ = tokio::fs::remove_file(&tmp).await;
    match published {
        Ok(()) => {
            sync_parent_dir(path).await?;
            Ok(true)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(e) => Err(SabrageError::io(path, e)),
    }
}

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
                // Same mode repair the real executor performs — the plan has to
                // show it, or a dry run reports a restage it would not do.
                if let (Some(want), Some(have)) = (mode_of(src), mode_of(dst)) {
                    if want != have {
                        self.record(
                            PlannedKind::Copy,
                            Some(src),
                            Some(dst),
                            &format!("bytes match, mode {have:04o} differs from source {want:04o}"),
                        );
                        return Ok(Copied::Copied);
                    }
                }
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

    fn create_new<'a>(&'a self, path: &'a Path, bytes: &'a [u8]) -> BoxFuture<'a, Result<bool>> {
        Box::pin(async move {
            // Real existence probe, like every other dry-run predicate: the
            // plan must say which branch the caller would take.
            if path.exists() {
                self.record(PlannedKind::Skip, None, Some(path), "already exists");
                return Ok(false);
            }
            self.record(
                PlannedKind::Write,
                None,
                Some(path),
                &format!("{} bytes", bytes.len()),
            );
            Ok(true)
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

    fn hard_link<'a>(&'a self, src: &'a Path, dst: &'a Path) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            self.record(PlannedKind::Link, Some(src), Some(dst), "link(2)");
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

    fn mode_bits(p: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).unwrap().permissions().mode() & 0o7777
    }

    /// The destination is never truncated by a copy that then fails: install's
    /// destinations are CrossOver's *global* DXMT and wineopenxr files, where a
    /// half-written file breaks every bottle.
    #[tokio::test]
    async fn a_failed_copy_leaves_the_previous_destination_intact() {
        let dir = scratch("copy-fail");
        let (run_id, sink, cancel) = sinks();
        let ex = RealExecutor::new(run_id, sink, cancel);
        let src = dir.join("src.dylib");
        let dst = dir.join("dst.dylib");
        std::fs::write(&src, b"new bytes").unwrap();
        std::fs::write(&dst, b"the last good overlay").unwrap();
        // An unreadable source: the copy fails after the compare said "differs".
        std::fs::set_permissions(&src, permissions(0o000)).unwrap();

        let err = ex.copy_if_changed(&src, &dst).await.unwrap_err();
        assert_eq!(err.kind(), "io");
        match &err {
            SabrageError::Io { path, .. } => assert_eq!(path, &dst, "error names the destination"),
            other => panic!("expected Io, got {other:?}"),
        }
        assert_eq!(std::fs::read(&dst).unwrap(), b"the last good overlay");
        let strays: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(".sabrage-"))
            .collect();
        assert!(strays.is_empty(), "temp files left: {strays:?}");

        std::fs::set_permissions(&src, permissions(0o644)).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A staged helper whose bytes match but whose execute bit is gone is not
    /// installed — and rebuilding cannot repair it, because the bytes never
    /// change. The copy primitive repairs the mode and reports work done.
    #[tokio::test]
    async fn a_byte_identical_destination_with_a_lost_execute_bit_is_repaired() {
        let dir = scratch("copy-mode");
        let (run_id, sink, cancel) = sinks();
        let ex = RealExecutor::new(run_id, sink, cancel);
        let src = dir.join("oxrsys-encoder-helper");
        let dst = dir.join("staged-helper");
        std::fs::write(&src, b"helper").unwrap();
        std::fs::set_permissions(&src, permissions(0o755)).unwrap();

        // Fresh copy carries the execute bit over.
        ex.copy_if_changed(&src, &dst).await.unwrap();
        assert_eq!(mode_bits(&dst), 0o755);
        // The identical repeat must not error (its `Unchanged` result is pinned
        // by `copy_if_changed_matches_install_if_changed`).
        ex.copy_if_changed(&src, &dst).await.unwrap();

        // Bytes still equal, mode drifted: repaired, and reported as Copied.
        std::fs::set_permissions(&dst, permissions(0o644)).unwrap();
        assert_eq!(
            ex.copy_if_changed(&src, &dst).await.unwrap(),
            Copied::Copied
        );
        assert_eq!(mode_bits(&dst), 0o755);
        assert_eq!(std::fs::read(&dst).unwrap(), b"helper");

        // The dry run plans that repair instead of calling it a skip.
        std::fs::set_permissions(&dst, permissions(0o644)).unwrap();
        let (run_id, sink, cancel) = sinks();
        let dry = DryRunExecutor::new(run_id, sink, cancel);
        assert_eq!(
            dry.copy_if_changed(&src, &dst).await.unwrap(),
            Copied::Copied
        );
        assert_eq!(mode_bits(&dst), 0o644, "dry run changed the mode");
        assert_eq!(dry.planned()[0].kind, PlannedKind::Copy);
        assert!(dry.planned()[0].reason.contains("mode"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// `O_EXCL`, so "the file was absent" is the kernel's answer rather than a
    /// stale `exists()`: the loser of a race must not replace a hand-edited
    /// `oxrsys-runtime.toml` that never got backed up.
    #[tokio::test]
    async fn create_new_never_clobbers_an_existing_file() {
        let dir = scratch("create-new");
        let (run_id, sink, cancel) = sinks();
        let ex = RealExecutor::new(run_id, sink, cancel);
        let f = dir.join("oxrsys-runtime.toml");

        assert!(ex.create_new(&f, b"template").await.unwrap());
        assert_eq!(std::fs::read(&f).unwrap(), b"template");
        assert_eq!(mode_bits(&f), 0o644);

        std::fs::write(&f, b"hand edited").unwrap();
        assert!(!ex.create_new(&f, b"template").await.unwrap());
        assert_eq!(std::fs::read(&f).unwrap(), b"hand edited");

        // The dry run probes for real and writes nothing, either branch.
        let (run_id, sink, cancel) = sinks();
        let dry = DryRunExecutor::new(run_id, sink, cancel);
        let absent = dir.join("absent.toml");
        assert!(dry.create_new(&absent, b"template").await.unwrap());
        assert!(!absent.exists(), "dry run created the file");
        assert!(!dry.create_new(&f, b"template").await.unwrap());
        assert_eq!(std::fs::read(&f).unwrap(), b"hand edited");
        let plan = dry.planned();
        assert_eq!(plan[0].kind, PlannedKind::Write);
        assert_eq!(plan[0].reason, "8 bytes");
        assert_eq!(plan[1].kind, PlannedKind::Skip);
        assert_eq!(plan[1].reason, "already exists");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// An atomic write replaces a file; it must not silently widen it.
    #[tokio::test]
    async fn write_atomic_keeps_an_existing_files_mode() {
        let dir = scratch("atomic-mode");
        let (run_id, sink, cancel) = sinks();
        let ex = RealExecutor::new(run_id, sink, cancel);

        let fresh = dir.join("new.json");
        ex.write_atomic(&fresh, b"{}").await.unwrap();
        assert_eq!(mode_bits(&fresh), 0o644);

        let tight = dir.join("session-state.json");
        std::fs::write(&tight, b"old").unwrap();
        std::fs::set_permissions(&tight, permissions(0o600)).unwrap();
        ex.write_atomic(&tight, b"new").await.unwrap();
        assert_eq!(std::fs::read(&tight).unwrap(), b"new");
        assert_eq!(mode_bits(&tight), 0o600, "replacement widened the file");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A published rename is only as durable as its directory entry, so the
    /// parent fsync is part of the contract rather than a best-effort extra: a
    /// failure to even open the parent must not be reported as "persisted",
    /// because `session-state.json`'s caller switches the Mac's audio device on
    /// the strength of that answer.
    #[tokio::test]
    async fn a_parent_that_cannot_be_synced_is_reported_not_swallowed() {
        let dir = scratch("atomic-parent");
        let f = dir.join("session-state.json");
        // The happy path first: a real directory syncs.
        std::fs::write(&f, b"{}").unwrap();
        sync_parent_dir(&f).await.unwrap();

        // And a parent that cannot be opened at all is an error, not silence.
        let gone = dir.join("vanished/session-state.json");
        let err = sync_parent_dir(&gone).await.unwrap_err();
        assert_eq!(err.kind(), "io");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// r2:A2-4 regression: the write-once config is published whole or not at
    /// all. A crash strands a temp, never a zero-length `oxrsys-runtime.toml`
    /// that later runs read as hand-edited content they must not replace.
    #[tokio::test]
    async fn create_new_publishes_finished_bytes_and_leaves_no_temp() {
        let dir = scratch("create-new-publish");
        let (run_id, sink, cancel) = sinks();
        let ex = RealExecutor::new(run_id, sink, cancel);
        let f = dir.join("oxrsys-runtime.toml");

        assert!(ex.create_new(&f, b"protocol = \"alvr\"\n").await.unwrap());
        assert_eq!(std::fs::read(&f).unwrap(), b"protocol = \"alvr\"\n");
        // One link only: the temp is unlinked whichever way the publish went.
        let names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["oxrsys-runtime.toml".to_string()]);

        // A file already at the final name is never replaced, including an empty one.
        let empty = dir.join("empty.toml");
        std::fs::write(&empty, b"").unwrap();
        assert!(!ex.create_new(&empty, b"template").await.unwrap());
        assert_eq!(std::fs::read(&empty).unwrap(), b"");
        let leftovers: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(".sabrage-"))
            .collect();
        assert!(leftovers.is_empty(), "temp files left: {leftovers:?}");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// `hard_link` captures the bytes a name points at right now, and refuses
    /// to replace an existing name.
    #[tokio::test]
    async fn hard_link_captures_the_live_bytes_without_replacing() {
        let dir = scratch("publish");
        let (run_id, sink, cancel) = sinks();
        let ex = RealExecutor::new(run_id, sink, cancel);

        let live = dir.join("oxrsys-runtime.toml");
        std::fs::write(&live, b"displaced by an outside editor").unwrap();
        let captured = dir.join("oxrsys-runtime.toml.displaced");
        ex.hard_link(&live, &captured).await.unwrap();
        // The link holds the bytes that were live at that instant, even after
        // the linked-from name is replaced by an atomic rename.
        ex.write_atomic(&live, b"sabrage wrote this").await.unwrap();
        assert_eq!(
            std::fs::read(&captured).unwrap(),
            b"displaced by an outside editor"
        );
        // A taken name is never replaced.
        let err = ex.hard_link(&live, &captured).await.unwrap_err();
        match err {
            SabrageError::Io { source, .. } => {
                assert_eq!(source.kind(), std::io::ErrorKind::AlreadyExists)
            }
            other => panic!("expected Io/AlreadyExists, got {other:?}"),
        }

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn write_atomic_leaves_no_temp_files() {
        let dir = scratch("atomic");
        let (run_id, sink, cancel) = sinks();
        let ex = RealExecutor::new(run_id, sink, cancel);
        let f = dir.join("out.json");
        ex.write_atomic(&f, b"one").await.unwrap();
        ex.write_atomic(&f, b"two").await.unwrap();
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
            ex.create_new(&out, b"bytes").await.err(),
            ex.create_dir_all(&sub).await.err(),
            ex.remove_dir_all(&dir).await.err(),
            ex.remove_file(&victim).await.err(),
            ex.hard_link(&victim, &dst).await.err(),
            ex.touch(&out).await.err(),
        ] {
            assert!(
                matches!(e, Some(SabrageError::Cancelled)),
                "expected Cancelled, got {e:?}"
            );
        }
        assert!(!dst.exists() && !out.exists() && !sub.exists());
        assert!(victim.is_file() && dir.is_dir());

        // The one exception, and why it exists: rolling back a mutation this
        // stage already made must happen *because* the run was cancelled, not
        // in spite of it (install's interrupted stock-DXMT capture).
        let half_copy = dir.join("dxmt.stock-backup.partial");
        std::fs::create_dir_all(half_copy.join("inner")).unwrap();
        std::fs::write(half_copy.join("inner/one.dll"), b"first entry").unwrap();
        ex.remove_dir_all_rollback(&half_copy).await.unwrap();
        assert!(!half_copy.exists(), "a cancelled run kept the partial copy");
        // Idempotent, like `remove_dir_all`.
        ex.remove_dir_all_rollback(&half_copy).await.unwrap();

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The invariant the whole trait exists for: under `--dry-run` **nothing**
    /// on disk changes, whichever primitive a stage reaches for. Probes still
    /// run (that is what makes the plan truthful), but they are reads.
    #[tokio::test]
    async fn a_dry_run_mutates_nothing_at_all() {
        let dir = scratch("dry-nothing");
        let (run_id, sink, cancel) = sinks();
        let ex = DryRunExecutor::new(run_id, sink, cancel);

        let src = dir.join("src");
        let existing = dir.join("existing");
        std::fs::write(&src, b"payload").unwrap();
        std::fs::write(&existing, b"keep me").unwrap();
        let sub = dir.join("sub");
        std::fs::create_dir(&sub).unwrap();
        let before = snapshot(&dir);

        let absent = dir.join("absent");
        ex.copy_if_changed(&src, &absent).await.unwrap();
        ex.write_atomic(&existing, b"clobbered").await.unwrap();
        ex.create_new(&absent, b"new").await.unwrap();
        ex.create_dir_all(&dir.join("deep/deeper")).await.unwrap();
        ex.remove_file(&existing).await.unwrap();
        ex.remove_dir_all(&sub).await.unwrap();
        ex.dir_copy(&sub, &dir.join("sub-copy")).await.unwrap();
        let dir_copy = ex.planned().last().expect("dir_copy recorded").clone();
        assert_eq!(dir_copy.kind, PlannedKind::DirCopy, "{dir_copy:#?}");
        assert_eq!(dir_copy.src.as_deref(), Some(sub.as_path()));
        assert_eq!(
            dir_copy.dst.as_deref(),
            Some(dir.join("sub-copy").as_path())
        );
        ex.hard_link(&existing, &absent).await.unwrap();
        ex.remove_dir_all_rollback(&sub).await.unwrap();
        ex.touch(&absent).await.unwrap();
        ex.tar_xzf(&src, &dir).await.unwrap();
        ex.download("https://h/x.tgz", &absent, "deadbeef", "X")
            .await
            .unwrap();
        ex.run_child(&ChildSpec::new("/bin/rm", step::BUILD_TOOLS, run_id).arg(&existing))
            .await
            .unwrap();
        ex.spawn_detached(
            &ChildSpec::new("/bin/echo", step::BUILD_TOOLS, run_id),
            DetachedStdio::LogFile(dir.join("run.log")),
        )
        .await
        .unwrap();

        assert_eq!(snapshot(&dir), before, "a dry run touched the filesystem");
        assert_eq!(ex.planned().len(), 14, "every call recorded one action");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Every path under `dir`, with each file's bytes — the "nothing changed"
    /// witness.
    fn snapshot(dir: &Path) -> Vec<(PathBuf, Option<Vec<u8>>)> {
        fn walk(dir: &Path, out: &mut Vec<(PathBuf, Option<Vec<u8>>)>) {
            let mut entries: Vec<_> = std::fs::read_dir(dir)
                .unwrap()
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .collect();
            entries.sort();
            for p in entries {
                if p.is_dir() {
                    out.push((p.clone(), None));
                    walk(&p, out);
                } else {
                    out.push((p.clone(), Some(std::fs::read(&p).unwrap())));
                }
            }
        }
        let mut out = Vec::new();
        walk(dir, &mut out);
        out
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
                    PlannedKind::Link,
                    Some("/b/oxrsys-runtime.toml"),
                    Some("/b/backups/oxrsys-runtime.toml.displaced"),
                    "link(2)",
                ),
                "would hard-link /b/oxrsys-runtime.toml → \
                 /b/backups/oxrsys-runtime.toml.displaced (link(2))",
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
