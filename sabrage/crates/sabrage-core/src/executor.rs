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
mod tests;

#[cfg(test)]
mod detached_tests;
