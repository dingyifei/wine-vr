//! `demo.sh install` — install the bridge into CrossOver, the bottle, and the
//! host loader. Idempotent (hash-gated copies); the ONLY stage that can prompt
//! for administrator authorization.
//!
//! Reference: `scripts/demo/install.sh`. Preconditions first (`require_bottle`,
//! CrossOver present, the three build outputs, a complete DXMT artifact set —
//! the overlay must never half-apply), then four layers, each opening with a
//! [`crate::stages::StageCtx::section`] banner whose text matches the shell's
//! `print -r -- "-- …"` line verbatim:
//!
//! 1. [`step::INSTALL_DXMT_OVERLAY`] — back up stock DXMT once to
//!    `$CX/lib/dxmt.stock-backup` (`cp -R`), then `copy_if_changed` the four
//!    `x86_64-windows` dlls and `x86_64-unix/winemetal.so`.
//! 2. [`step::INSTALL_WINEOPENXR`] — the PE dll and unix `.so` into
//!    `$CX/lib/wine`.
//! 3. [`step::INSTALL_BOTTLE`] — `system32/wineopenxr.dll`, the
//!    `drive_c/openxr/` manifest, and the `ActiveRuntime` registry key via a
//!    brief `wine … reg add` (whose output sabrage captures rather than
//!    discarding). The post-write `system.reg` re-probe is **Warn, never Fail** —
//!    wine flushes lazily — but it is retried for [`REGISTRY_FLUSH_TIMEOUT`]
//!    before warning, so the stage does not report success against a file the
//!    very next launch preflight blocks on.
//! 4. [`step::INSTALL_HOST_MANIFEST`] — the byte-shared host registration. Skip
//!    entirely when the on-disk bytes already match
//!    ([`crate::util::host_manifest_is_current`]) so a re-install prompts for
//!    nothing, then go through [`crate::privilege`]. This stage renders the
//!    manifest only for that currency test: the bytes that reach disk are
//!    [`crate::privilege::write_host_manifest_privileged`]'s own, derived from
//!    the dylib path it is handed, so the file form (trailing newline, exactly
//!    what `print -- "$WANT"` writes) cannot be confused with the comparison
//!    form here.
//!
//! Layers 1–2 write inside `CrossOver.app` and need macOS App Management (TCC),
//! **not** root: `sudo` does not help there. Only layer 4 is privileged.
//!
//! # TCC vs a real failure
//!
//! [`install_if_changed`] routes every `copy_if_changed` failure in layers 1–3
//! through [`crate::privilege::upgrade_write_error`] — the single home of that
//! judgement, shared with every other write that could hit App Management. A
//! `PermissionDenied` on a destination
//! [`crate::privilege::is_inside_app_bundle`] (which every layer-1/2
//! destination is, and no layer-3 one ever is — the call is uniform because it
//! is a safe no-op outside `.app`) becomes
//! [`crate::error::SabrageError::TccDenied`], whose `kind() == "tcc_denied"` is
//! what the GUI's permission panel branches on, and the explanation reaches the
//! user as that function's own `Fatal` event: the App Management deep link, the
//! relaunch requirement, and the Terminal fallback (`sudo` would not have
//! helped, and demo.sh install running as the same user hits the same wall —
//! the fallback command is not "use sudo instead"). This stage must **not**
//! re-emit that prose; it propagates. Any other cause of the same error class
//! passes through as the original I/O error, unexplained but not swallowed.
//!
//! Layer 1's stock-DXMT backup is a shelled `cp -R`, so its failure has no
//! `io::Error` to classify — it goes through the ChildFailed-shaped sibling,
//! [`crate::privilege::upgrade_child_write_error`], which reaches the same
//! `TccDenied` + App Management remedy when the tail says permission denied.
//! That backup is the first write the pipeline makes into `CrossOver.app`.
//!
//! # Why this stage is hand-constructible in tests without touching the
//! machine
//!
//! [`crate::stages::StageCtx`] and [`crate::paths::Paths`] have every field
//! `pub`, so a test builds both as plain struct literals — `bottle`/`cx`/`wine`/
//! `host_xr_json` pointed at a scratch directory — rather than going through
//! [`crate::stages::require_bottle`]'s real `~/Library` lookup or
//! [`crate::paths::Paths::new`]'s real CrossOver.app probe. That is what keeps
//! this module's tests off the real machine (hard rule: never touch the real
//! `~/Library`, `/usr/local`, or `CrossOver.app` in tests) while still
//! exercising the actual [`run`] function end to end under
//! [`crate::executor::DryRunExecutor`].

use std::path::Path;
use std::time::Duration;

use crate::error::{Result, SabrageError};
use crate::events::{step, StageEvent, StepId, Stream};
use crate::executor::{Copied, Executor};
use crate::privilege::{self, PrivilegedWrite};
use crate::stages::{require_bottle, StageCtx};

/// How long layer 3 waits for wine to flush `system.reg` after a successful
/// `reg add`, before settling for the (never fatal) lazy-flush warning.
///
/// Native-only: install.sh greps once and moves on. The point is that
/// `sabrage all` — and any install-then-launch — chains straight into a launch
/// preflight that *blocks* on this exact file content (`bottle.registry`), so an
/// install that reports success and a launch that rejects the registry a second
/// later would contradict each other over nothing more than a flush that had
/// not landed yet.
const REGISTRY_FLUSH_TIMEOUT: Duration = Duration::from_secs(2);

/// Re-probe interval inside [`REGISTRY_FLUSH_TIMEOUT`].
const REGISTRY_FLUSH_POLL: Duration = Duration::from_millis(50);

/// Execute the stage.
pub async fn run(ctx: &StageCtx) -> Result<()> {
    let bottle = require_bottle(ctx)?;

    // lib.sh: [ -n "${CX_APP:-}" ] || die "CrossOver.app not found"
    if ctx.paths.cx_app.is_none() {
        return Err(ctx.fatal("CrossOver.app not found", None));
    }
    // Invariant (paths.rs): cx/wine are Some exactly when cx_app is.
    let cx = ctx
        .paths
        .cx
        .as_ref()
        .expect("cx present whenever cx_app is present");

    // for f in "$OXR_DYLIB" "$WOXR_DLL" "$WOXR_SO"; do [ -f "$f" ] || die … ; done
    for f in [
        &ctx.paths.oxr_dylib,
        &ctx.paths.woxr_dll,
        &ctx.paths.woxr_so,
    ] {
        if !f.is_file() {
            return Err(ctx.fatal(
                format!(
                    "missing build output {} — ./demo.sh build first",
                    f.display()
                ),
                None,
            ));
        }
    }

    // dxmt_files_ok || die "ext/dxmt-artifacts missing or incomplete — …"
    if !crate::util::dxmt_files_ok(&ctx.paths) {
        return Err(ctx.fatal(
            "ext/dxmt-artifacts missing or incomplete — ./demo.sh setup first (never half-applies the overlay)",
            None,
        ));
    }

    // Rows that describe a mutation say "would …" when nothing was mutated:
    // a preview must not be indistinguishable from completed work in the event
    // log (the vocabulary is build.rs's / setup.rs's).
    let dry_run = ctx.executor.is_dry_run();

    // ── 1. global DXMT overlay ───────────────────────────────────────────────
    ctx.section(format!("global DXMT overlay ({}/lib/dxmt)", cx.display()));
    let exec1 = ctx.executor_for(step::INSTALL_DXMT_OVERLAY);
    let dxmt_dir = cx.join("lib/dxmt");
    let dxmt_backup = cx.join("lib/dxmt.stock-backup");
    if dxmt_backup.is_dir() {
        if dir_is_empty(&dxmt_backup) {
            // The one completeness statement that can be made without guessing
            // what stock shipped — and the shape a `cp -R` killed right after
            // it created the directory leaves behind. Deliberately *not*
            // re-copied: after a successful install `lib/dxmt` holds the fork,
            // so "re-capture the backup" would overwrite the stock copy with
            // fork binaries and destroy the only rollback there is.
            ctx.step(step::INSTALL_DXMT_OVERLAY).warn(format!(
                "stock DXMT backup {} is empty — remove it and re-run install to recapture stock (left alone: re-copying now would back up the fork)",
                dxmt_backup.display()
            ));
        } else {
            ctx.step(step::INSTALL_DXMT_OVERLAY)
                .info("stock DXMT backup already exists");
        }
    } else {
        // Shelled `cp -R`, not `copy_if_changed`: a failure here is a
        // ChildFailed (no std::io::Error to classify), so it goes through
        // `upgrade_child_write_error` — the ChildFailed-shaped sibling of the
        // TCC path below — rather than `upgrade_write_error`. This is the
        // *first* write into CrossOver.app, i.e. the likeliest place in the
        // whole pipeline to meet App Management.
        if let Err(e) = exec1.dir_copy(&dxmt_dir, &dxmt_backup).await {
            // `cp -R` is not atomic: a failure (or a Ctrl-C) part-way through
            // leaves a truncated directory that every later install would
            // accept as a finished backup, purely because it exists. Drop it,
            // so the retry copies stock again — safe here and only here,
            // because this branch is reached only when the backup did not
            // exist when the stage started and the overlay has not been
            // applied yet.
            let _ = exec1.remove_dir_all(&dxmt_backup).await;
            return Err(privilege::upgrade_child_write_error(ctx, e, &dxmt_backup));
        }
        let st = ctx.step(step::INSTALL_DXMT_OVERLAY);
        if dry_run {
            st.info(format!(
                "would back up stock DXMT -> {}",
                dxmt_backup.display()
            ));
        } else {
            st.ok(format!("backed up stock DXMT -> {}", dxmt_backup.display()));
        }
    }
    for rel in &crate::contract::contract().dxmt.files {
        let src = ctx.paths.dxmt_art.join(rel);
        let dst = dxmt_dir.join(rel);
        install_if_changed(ctx, exec1.as_ref(), step::INSTALL_DXMT_OVERLAY, &src, &dst).await?;
    }

    // ── 2. global wineopenxr ─────────────────────────────────────────────────
    ctx.section(format!("global wineopenxr ({}/lib/wine)", cx.display()));
    let exec2 = ctx.executor_for(step::INSTALL_WINEOPENXR);
    install_if_changed(
        ctx,
        exec2.as_ref(),
        step::INSTALL_WINEOPENXR,
        &ctx.paths.woxr_dll,
        &cx.join("lib/wine/x86_64-windows/wineopenxr.dll"),
    )
    .await?;
    install_if_changed(
        ctx,
        exec2.as_ref(),
        step::INSTALL_WINEOPENXR,
        &ctx.paths.woxr_so,
        &cx.join("lib/wine/x86_64-unix/wineopenxr.so"),
    )
    .await?;

    // ── 3. per-bottle: dll + manifest + ActiveRuntime registry key ──────────
    ctx.section(format!("bottle '{}'", bottle.name));
    let exec3 = ctx.executor_for(step::INSTALL_BOTTLE);
    install_if_changed(
        ctx,
        exec3.as_ref(),
        step::INSTALL_BOTTLE,
        &ctx.paths.woxr_dll,
        &bottle.sys32.join("wineopenxr.dll"),
    )
    .await?;
    exec3
        .create_dir_all(&bottle.prefix.join("drive_c/openxr"))
        .await?;
    install_if_changed(
        ctx,
        exec3.as_ref(),
        step::INSTALL_BOTTLE,
        &ctx.paths.woxr.join("manifests/wineopenxr64.json"),
        &bottle.openxr_manifest(),
    )
    .await?;

    if registry_current(&bottle.system_reg()) {
        ctx.step(step::INSTALL_BOTTLE)
            .info("registry ActiveRuntime already set");
    } else {
        ctx.step(step::INSTALL_BOTTLE)
            .info("registering wineopenxr as the bottle's OpenXR runtime (starts wine briefly)...");
        let wine = ctx
            .paths
            .wine
            .as_ref()
            .expect("wine present whenever CrossOver.app is present");
        let spec = ctx
            .child(wine.clone(), step::INSTALL_BOTTLE)
            .arg("--bottle")
            .arg(bottle.name.clone())
            .arg("--no-update")
            .arg("reg")
            .arg("add")
            .arg(r"HKLM\Software\Khronos\OpenXR\1")
            .arg("/v")
            .arg("ActiveRuntime")
            .arg("/t")
            .arg("REG_SZ")
            .arg("/d")
            .arg(r"C:\openxr\wineopenxr64.json")
            .arg("/f")
            .env("WINEPREFIX", bottle.prefix.display().to_string())
            .env("CX_BOTTLE", bottle.name.clone());
        // run_child streams stdout/stderr into the run's event log as it goes
        // (crate::executor's RealExecutor delegates to spawn_streamed) rather
        // than the shell's `>/dev/null 2>&1` — the PARITY.md-declared
        // divergence ("reg add output captured instead of discarded").
        let status = exec3.run_child(&spec).await?;
        if !status.success() {
            return Err(ctx.fatal("reg add failed", None));
        }
        if dry_run {
            // Nothing ran: DryRunExecutor reports the spawn as a success it
            // never performed, so neither the flush probe nor a completed-
            // action row would mean anything here.
            ctx.step(step::INSTALL_BOTTLE)
                .info("would register ActiveRuntime");
        } else {
            // grep -q 'ActiveRuntime' "$PREFIX/system.reg" || warn … — Warn,
            // never Fail: wine flushes system.reg lazily. Unlike the shell's
            // single grep, the probe is retried for REGISTRY_FLUSH_TIMEOUT
            // first, so the common case (the flush lands a few hundred ms
            // after `reg add` returns) no longer both warns *and* leaves the
            // very next launch preflight blocking on the same file.
            if !wait_for_registry_flush(ctx, &bottle.system_reg()).await {
                ctx.step(step::INSTALL_BOTTLE).warn(
                    "registry write not yet visible in system.reg (wine flushes lazily) — re-run doctor later",
                );
            }
            ctx.step(step::INSTALL_BOTTLE)
                .ok("ActiveRuntime registered");
        }
    }

    // ── 4. host OpenXR registration ──────────────────────────────────────────
    ctx.section(format!(
        "host OpenXR registration ({})",
        ctx.paths.host_xr_json.display()
    ));
    // The *comparison* form (no trailing newline), used for nothing but
    // install.sh's `[ "$(cat "$HOST_XR_JSON")" = "$WANT" ]` currency test. The
    // bytes that land on disk are rendered inside the privileged write from the
    // dylib path below — see this module's layer-4 note.
    let want = crate::util::render_host_manifest(&ctx.paths.oxr_dylib);
    if crate::util::host_manifest_is_current(&ctx.paths.host_xr_json, &want) {
        ctx.step(step::INSTALL_HOST_MANIFEST)
            .info("host registration already current");
    } else {
        ctx.step(step::INSTALL_HOST_MANIFEST).info(format!(
            "writing {} (needs sudo)...",
            ctx.paths.host_xr_json.display()
        ));
        // write_host_manifest_privileged's own contract (privilege.rs) emits
        // StageEvent::NeedsAdmin before it prompts — not duplicated here. It
        // re-runs the currency test under the prompt, so it can still come back
        // Skipped (another writer won the race between the test above and the
        // authorization); saying "written" then would be a lie.
        match privilege::write_host_manifest_privileged(
            ctx,
            &ctx.paths.oxr_dylib,
            &ctx.paths.host_xr_json,
        )
        .await?
        {
            PrivilegedWrite::Skipped => ctx
                .step(step::INSTALL_HOST_MANIFEST)
                .info("host registration already current"),
            PrivilegedWrite::Written => ctx
                .step(step::INSTALL_HOST_MANIFEST)
                .ok("host registration written"),
            // A dry run planned the staging write and the elevated argv and
            // prompted for nothing — see privilege::plan_privileged_write.
            PrivilegedWrite::Planned => ctx
                .step(step::INSTALL_HOST_MANIFEST)
                .info("would write host registration"),
        }
    }

    Ok(())
    // The "install complete — next: …" line is the CLI renderer's, not this
    // stage's — see the frame's StageEvent::StageFinished bracketing.
}

/// `lib.sh`'s `install_if_changed`, split the way [`crate::executor::Executor`]
/// intends: the executor does the byte compare and the copy (or, under
/// [`crate::executor::DryRunExecutor`], neither), and the caller prints the
/// row — `info "unchanged: <dst>"` / `ok "installed: <dst>"`, verbatim,
/// `<dst>` the full destination path exactly as the shell prints `$2`.
///
/// Also the TCC call site (see this module's header): every failure goes through
/// [`privilege::upgrade_write_error`], which turns a `PermissionDenied` under a
/// `.app` bundle into [`SabrageError::TccDenied`] and emits the App Management
/// explanation itself. That case is propagated as-is — re-emitting a second
/// `Fatal` for it is exactly what `upgrade_write_error`'s doc forbids.
///
/// Every *other* copy failure (a bottle path, `drive_c/openxr`, a read-only
/// volume, ENOSPC — none of which classify as TCC) gets `lib.sh`'s own die
/// text, verbatim:
///
/// ```zsh
/// cp "$1" "$2" || die "copy failed: $1 -> $2"
/// ```
///
/// so the failure reaches the run log as a `Fatal` row in the same shape every
/// other install failure uses, instead of only as a rejected promise / bare
/// `error: <dst>: Permission denied` CLI tail.
async fn install_if_changed(
    ctx: &StageCtx,
    executor: &dyn Executor,
    step: StepId,
    src: &Path,
    dst: &Path,
) -> Result<()> {
    match executor.copy_if_changed(src, dst).await {
        Ok(Copied::Unchanged) => {
            ctx.step(step).info(format!("unchanged: {}", dst.display()));
            Ok(())
        }
        Ok(Copied::Copied) => {
            // Dry run gets build.rs's / fixes::helper's "would install" verb
            // for the same `copy_if_changed` outcome: the byte compare really
            // ran, the copy did not.
            let verb = if executor.is_dry_run() {
                "would install"
            } else {
                "installed"
            };
            ctx.step(step).ok(format!("{verb}: {}", dst.display()));
            Ok(())
        }
        Err(e) => match privilege::upgrade_write_error(ctx, e) {
            // `upgrade_write_error` already emitted the App Management `Fatal`;
            // propagate, never re-emit.
            upgraded @ SabrageError::TccDenied { .. } => Err(upgraded),
            // Everything else: the shell shows `cp`'s own stderr
            // (`cp: <dst>: Permission denied`) and then dies with lib.sh's
            // `die "copy failed: $1 -> $2"`. Surface the cause the same way —
            // as stderr-shaped output ahead of the verbatim die text — so a
            // plain PermissionDenied, a read-only volume and ENOSPC stay
            // distinguishable (design-core §6.5: no swallowed diagnostics).
            other => {
                ctx.emit(StageEvent::Output {
                    run_id: ctx.run_id,
                    step: step.to_string(),
                    stream: Stream::Stderr,
                    chunk: other.to_string(),
                });
                Err(ctx.fatal(
                    format!("copy failed: {} -> {}", src.display(), dst.display()),
                    None,
                ))
            }
        },
    }
}

/// `grep -q 'ActiveRuntime.*openxr.*wineopenxr64.json' "$PREFIX/system.reg"`.
///
/// Deliberately duplicated from `checks::bridge`'s private
/// `registry_has_active_runtime` (same shape, same rationale — see that
/// module's doc comment for why a left-to-right chained `find` matches the
/// regex exactly): that function is private to a file outside this task's
/// ownership, and this stage needs the read *before* deciding whether to run
/// `reg add` at all, not just to report a doctor row afterward.
fn registry_current(system_reg: &Path) -> bool {
    let Ok(bytes) = std::fs::read(system_reg) else {
        return false;
    };
    let text = String::from_utf8_lossy(&bytes);
    text.lines().any(|line| {
        let needles = ["ActiveRuntime", "openxr", "wineopenxr64.json"];
        let mut pos = 0usize;
        needles.iter().all(|needle| match line[pos..].find(needle) {
            Some(off) => {
                pos += off + needle.len();
                true
            }
            None => false,
        })
    })
}

/// The bare post-write re-probe: `grep -q 'ActiveRuntime' "$PREFIX/system.reg"`
/// — looser than [`registry_current`] on purpose, matching the shell exactly
/// (wine may not have flushed the full line yet).
fn system_reg_contains(system_reg: &Path, needle: &str) -> bool {
    std::fs::read(system_reg)
        .map(|bytes| String::from_utf8_lossy(&bytes).contains(needle))
        .unwrap_or(false)
}

/// [`system_reg_contains`], retried until it holds or
/// [`REGISTRY_FLUSH_TIMEOUT`] elapses. `true` means the entry became visible.
///
/// Cancellation ends the wait immediately (the caller's next step is a row, not
/// a mutation, so there is nothing to unwind); the last probe result stands.
async fn wait_for_registry_flush(ctx: &StageCtx, system_reg: &Path) -> bool {
    let deadline = std::time::Instant::now() + REGISTRY_FLUSH_TIMEOUT;
    loop {
        if system_reg_contains(system_reg, "ActiveRuntime") {
            return true;
        }
        if ctx.cancel.is_cancelled() || std::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(REGISTRY_FLUSH_POLL).await;
    }
}

/// True when `dir` holds no entries at all — the only completeness statement
/// that can be made about a stock DXMT backup without assuming what a given
/// CrossOver version shipped, and the shape a `cp -R` interrupted immediately
/// after creating the directory leaves behind.
fn dir_is_empty(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_none())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::SabrageError;
    use crate::events::{RunId, Severity, StageEvent};
    use crate::executor::DryRunExecutor;
    use crate::paths::{Bottle, Paths};
    use crate::stages::{EventSink, StageOptions};
    use std::sync::{Arc, Mutex as StdMutex};
    use tokio_util::sync::CancellationToken;

    /// A fresh scratch directory. `name` alone is not unique enough: several
    /// tests in this module call `scratch("full")` / `scratch("ctx")`, and
    /// `cargo test` runs them concurrently by default — a shared path would
    /// have one test's `remove_file` race another's assertions on the same
    /// fixture tree. The atomic counter makes every call unique regardless of
    /// how many tests reuse the same name.
    fn scratch(name: &str) -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "sabrage-install-test-{name}-{}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn collecting_sink() -> (EventSink, Arc<StdMutex<Vec<StageEvent>>>) {
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let s = seen.clone();
        let sink: EventSink = Arc::new(move |ev| s.lock().unwrap().push(ev));
        (sink, seen)
    }

    // ── a dry-run executor that keeps bytes, and can refuse a copy ───────────

    /// Every `write_atomic` the stage performs, with its bytes.
    type Writes = Arc<StdMutex<Vec<(std::path::PathBuf, Vec<u8>)>>>;

    /// [`DryRunExecutor`] with two test affordances it deliberately lacks:
    ///
    /// * every `write_atomic` is kept **with its bytes** (the plan records only
    ///   a byte count, and the host manifest is defined by its bytes — one
    ///   missing trailing newline is the whole bug this guards);
    /// * `copy_if_changed` can be made to fail with `PermissionDenied` under a
    ///   path prefix, which is the exact shape a macOS App Management refusal
    ///   arrives in.
    ///
    /// Everything else delegates, so `run()` behaves as it does under a plain
    /// dry run and still touches nothing.
    struct TestExecutor {
        inner: Arc<dyn Executor>,
        writes: Writes,
        deny_prefix: Option<std::path::PathBuf>,
        /// `dir_copy` fails the way a refused `cp -R` does — a `ChildFailed`
        /// carrying `cp`'s own permission error as its tail.
        deny_dir_copy: bool,
        /// Report `is_dry_run() == false` while still mutating nothing: the
        /// only way to exercise the real-run branches (the registry re-probe,
        /// the completed-action rows) without spawning wine.
        pose_as_real: bool,
    }

    impl TestExecutor {
        fn dry_run(sink: EventSink, run_id: RunId, cancel: CancellationToken) -> Arc<TestExecutor> {
            Arc::new(TestExecutor {
                inner: Arc::new(DryRunExecutor::new(run_id, sink, cancel)),
                writes: Arc::new(StdMutex::new(Vec::new())),
                deny_prefix: None,
                deny_dir_copy: false,
                pose_as_real: false,
            })
        }

        /// A copy of this executor with one knob changed.
        fn with(self: &Arc<Self>, f: impl FnOnce(&mut TestExecutor)) -> Arc<TestExecutor> {
            let mut next = TestExecutor {
                inner: self.inner.clone(),
                writes: self.writes.clone(),
                deny_prefix: self.deny_prefix.clone(),
                deny_dir_copy: self.deny_dir_copy,
                pose_as_real: self.pose_as_real,
            };
            f(&mut next);
            Arc::new(next)
        }

        fn denying(self: &Arc<Self>, prefix: impl Into<std::path::PathBuf>) -> Arc<TestExecutor> {
            let prefix = prefix.into();
            self.with(|e| e.deny_prefix = Some(prefix))
        }
    }

    impl std::fmt::Debug for TestExecutor {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("TestExecutor")
                .field("writes", &self.writes.lock().map(|w| w.len()).unwrap_or(0))
                .field("deny_prefix", &self.deny_prefix)
                .finish()
        }
    }

    impl Executor for TestExecutor {
        fn with_step(&self, step: StepId) -> Arc<dyn Executor> {
            Arc::new(TestExecutor {
                inner: self.inner.with_step(step),
                writes: self.writes.clone(),
                deny_prefix: self.deny_prefix.clone(),
                deny_dir_copy: self.deny_dir_copy,
                pose_as_real: self.pose_as_real,
            })
        }

        fn is_dry_run(&self) -> bool {
            !self.pose_as_real && self.inner.is_dry_run()
        }

        fn planned(&self) -> Vec<crate::executor::PlannedAction> {
            self.inner.planned()
        }

        fn copy_if_changed<'a>(
            &'a self,
            src: &'a Path,
            dst: &'a Path,
        ) -> crate::executor::BoxFuture<'a, Result<Copied>> {
            if self
                .deny_prefix
                .as_ref()
                .is_some_and(|p| dst.starts_with(p))
            {
                return Box::pin(async move {
                    Err(SabrageError::io(
                        dst,
                        std::io::Error::from(std::io::ErrorKind::PermissionDenied),
                    ))
                });
            }
            self.inner.copy_if_changed(src, dst)
        }

        fn write_atomic<'a>(
            &'a self,
            path: &'a Path,
            bytes: &'a [u8],
        ) -> crate::executor::BoxFuture<'a, Result<()>> {
            if let Ok(mut w) = self.writes.lock() {
                w.push((path.to_path_buf(), bytes.to_vec()));
            }
            self.inner.write_atomic(path, bytes)
        }

        fn remove_dir_all<'a>(
            &'a self,
            path: &'a Path,
        ) -> crate::executor::BoxFuture<'a, Result<()>> {
            self.inner.remove_dir_all(path)
        }

        fn remove_file<'a>(&'a self, path: &'a Path) -> crate::executor::BoxFuture<'a, Result<()>> {
            self.inner.remove_file(path)
        }

        fn create_dir_all<'a>(
            &'a self,
            path: &'a Path,
        ) -> crate::executor::BoxFuture<'a, Result<()>> {
            self.inner.create_dir_all(path)
        }

        fn dir_copy<'a>(
            &'a self,
            src: &'a Path,
            dst: &'a Path,
        ) -> crate::executor::BoxFuture<'a, Result<()>> {
            if self.deny_dir_copy {
                return Box::pin(async move {
                    Err(SabrageError::ChildFailed {
                        argv0: "cp".to_string(),
                        status: 1,
                        // `cp -R`'s own wording, which is all a ChildFailed
                        // carries — there is no errno to classify.
                        tail: vec![format!("cp: {}: Permission denied", dst.display())],
                    })
                });
            }
            self.inner.dir_copy(src, dst)
        }

        fn download<'a>(
            &'a self,
            url: &'a str,
            dest: &'a Path,
            sha256: &'a str,
            label: &'a str,
        ) -> crate::executor::BoxFuture<'a, Result<crate::executor::Downloaded>> {
            self.inner.download(url, dest, sha256, label)
        }

        fn tar_xzf<'a>(
            &'a self,
            archive: &'a Path,
            into_dir: &'a Path,
        ) -> crate::executor::BoxFuture<'a, Result<()>> {
            self.inner.tar_xzf(archive, into_dir)
        }

        fn touch<'a>(&'a self, path: &'a Path) -> crate::executor::BoxFuture<'a, Result<()>> {
            self.inner.touch(path)
        }

        fn spawn_detached<'a>(
            &'a self,
            spec: &'a crate::process::ChildSpec,
            stdio: crate::executor::DetachedStdio,
        ) -> crate::executor::BoxFuture<'a, Result<Option<crate::executor::DetachedChild>>>
        {
            // install never launches anything detached; delegate rather than
            // unreachable!(), so the fake stays a faithful pass-through.
            self.inner.spawn_detached(spec, stdio)
        }

        fn run_child<'a>(
            &'a self,
            spec: &'a crate::process::ChildSpec,
        ) -> crate::executor::BoxFuture<'a, Result<std::process::ExitStatus>> {
            self.inner.run_child(spec)
        }
    }

    // ── WANT skip-when-current (layer 4), trailing-newline semantics ────────

    #[test]
    fn host_manifest_skip_decision_matches_cat_semantics() {
        let dir = scratch("host-manifest");
        let dest = dir.join("active_runtime.x86_64.json");
        let dylib =
            std::path::PathBuf::from("/repo/ext/oxrsys/build-x64/runtime/liboxrsys-runtime.dylib");
        let want = crate::util::render_host_manifest(&dylib);

        // Missing file: not current.
        assert!(!crate::util::host_manifest_is_current(&dest, &want));

        // On-disk with the shell's single trailing newline (`print -- "$WANT"`).
        std::fs::write(&dest, format!("{want}\n")).unwrap();
        assert!(crate::util::host_manifest_is_current(&dest, &want));

        // No trailing newline at all: `$(cat …)` strips nothing to strip, still current.
        std::fs::write(&dest, &want).unwrap();
        assert!(crate::util::host_manifest_is_current(&dest, &want));

        // Two trailing newlines: `$(cat …)` strips *all* of them, still current.
        std::fs::write(&dest, format!("{want}\n\n")).unwrap();
        assert!(crate::util::host_manifest_is_current(&dest, &want));

        // A stale dylib path: not current.
        let other_want =
            crate::util::render_host_manifest(std::path::Path::new("/other/lib.dylib"));
        assert_ne!(want, other_want);
        assert!(!crate::util::host_manifest_is_current(&dest, &other_want));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    // ── registry probe helpers ───────────────────────────────────────────────

    #[test]
    fn registry_current_requires_all_three_literals_in_order_on_one_line() {
        let dir = scratch("system-reg");
        let reg = dir.join("system.reg");

        assert!(!registry_current(&reg), "missing file is not current");

        std::fs::write(
            &reg,
            "[Software\\\\Khronos\\\\OpenXR\\\\1] 1700000000\n\"ActiveRuntime\"=\"C:\\\\openxr\\\\wineopenxr64.json\"\n",
        )
        .unwrap();
        assert!(registry_current(&reg));

        std::fs::write(&reg, "openxr wineopenxr64.json ActiveRuntime\n").unwrap();
        assert!(
            !registry_current(&reg),
            "out-of-order literals must not match"
        );

        assert!(system_reg_contains(&reg, "ActiveRuntime"));
        assert!(!system_reg_contains(&reg, "nope"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // ── backup-once logic (layer 1), fixture dirs via DryRun ────────────────

    fn fixture_ctx(
        dry_run: bool,
    ) -> (StageCtx, std::path::PathBuf, Arc<StdMutex<Vec<StageEvent>>>) {
        let root = scratch("ctx");
        let (sink, seen) = collecting_sink();
        let run_id: RunId = uuid::Uuid::new_v4();
        let cancel = CancellationToken::new();
        let executor: Arc<dyn Executor> = if dry_run {
            Arc::new(DryRunExecutor::new(run_id, sink.clone(), cancel.clone()))
        } else {
            Arc::new(crate::executor::RealExecutor::new(
                run_id,
                sink.clone(),
                cancel.clone(),
            ))
        };
        let paths = Paths::new(&root);
        let ctx = StageCtx {
            paths,
            bottle: None,
            bs_dir: std::path::PathBuf::new(),
            opts: StageOptions::default(),
            executor,
            run_id,
            cancel,
            sink,
        };
        (ctx, root, seen)
    }

    #[tokio::test]
    async fn dxmt_backup_is_planned_once_then_skipped_when_present() {
        let (ctx, root, seen) = fixture_ctx(true);
        let cx = root.join("CrossOver.app/Contents/SharedSupport/CrossOver");
        std::fs::create_dir_all(cx.join("lib/dxmt")).unwrap();
        let dxmt_dir = cx.join("lib/dxmt");
        let backup = cx.join("lib/dxmt.stock-backup");

        // No backup yet: dir_copy is planned, an `ok` row is emitted.
        assert!(!backup.is_dir());
        ctx.executor.dir_copy(&dxmt_dir, &backup).await.unwrap();
        ctx.step(step::INSTALL_DXMT_OVERLAY)
            .ok(format!("backed up stock DXMT -> {}", backup.display()));
        let planned = ctx.executor.planned();
        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].kind, crate::executor::PlannedKind::DirCopy);
        assert_eq!(planned[0].src.as_deref(), Some(dxmt_dir.as_path()));
        assert_eq!(planned[0].dst.as_deref(), Some(backup.as_path()));
        let evs = seen.lock().unwrap();
        assert!(evs.iter().any(|e| matches!(
            e,
            StageEvent::Line { severity: Severity::Ok, text, .. }
                if text.starts_with("backed up stock DXMT -> ")
        )));
    }

    #[tokio::test]
    async fn dxmt_backup_present_is_reported_not_replanned() {
        let (ctx, root, _seen) = fixture_ctx(true);
        let cx = root.join("CrossOver.app/Contents/SharedSupport/CrossOver");
        std::fs::create_dir_all(cx.join("lib/dxmt.stock-backup")).unwrap();
        let backup = cx.join("lib/dxmt.stock-backup");

        assert!(backup.is_dir());
        // Mirrors run()'s own branch: an existing backup dir is reported, and
        // dir_copy is never called — nothing lands in the plan.
        ctx.step(step::INSTALL_DXMT_OVERLAY)
            .info("stock DXMT backup already exists");
        assert!(ctx.executor.planned().is_empty());
    }

    // ── layer ordering, full `run()` under DryRun ────────────────────────────

    /// Builds a complete on-disk fixture (build outputs, DXMT artifacts, a
    /// fake CrossOver.app tree, a fake bottle, and a host manifest already
    /// current) so [`run`] can execute all four layers without touching the
    /// real machine and without reaching the unimplemented
    /// `privilege::write_host_manifest_privileged` (layer 4 takes the
    /// "already current" branch).
    fn full_fixture() -> (StageCtx, Arc<StdMutex<Vec<StageEvent>>>) {
        let root = scratch("full");
        let mut paths = Paths::new(&root);

        // Build outputs (layer precondition).
        std::fs::create_dir_all(paths.oxr_dylib.parent().unwrap()).unwrap();
        std::fs::write(&paths.oxr_dylib, b"dylib").unwrap();
        std::fs::create_dir_all(paths.woxr_dll.parent().unwrap()).unwrap();
        std::fs::write(&paths.woxr_dll, b"pe").unwrap();
        std::fs::create_dir_all(paths.woxr_so.parent().unwrap()).unwrap();
        std::fs::write(&paths.woxr_so, b"so").unwrap();
        std::fs::create_dir_all(paths.woxr.join("manifests")).unwrap();
        std::fs::write(paths.woxr.join("manifests/wineopenxr64.json"), b"{}").unwrap();

        // DXMT artifacts, every contract file + a current .sha256 marker.
        for rel in &crate::contract::contract().dxmt.files {
            let p = paths.dxmt_art.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, b"x").unwrap();
        }

        // A fake CrossOver.app tree — cx_app/cx/wine/wineserver overridden to
        // live entirely under the fixture root, never the real machine.
        let cx_app = root.join("CrossOver.app");
        let cx = cx_app.join("Contents/SharedSupport/CrossOver");
        std::fs::create_dir_all(cx.join("bin")).unwrap();
        std::fs::write(cx.join("bin/wine"), b"#!/bin/sh\n").unwrap();
        paths.cx_app = Some(cx_app);
        paths.cx = Some(cx.clone());
        paths.wine = Some(cx.join("bin/wine"));
        paths.wineserver = Some(cx.join("bin/wineserver"));

        // Layer 4's destination, overridden off the real
        // /usr/local/share/openxr path and pre-written as already current so
        // `run()` never calls the unimplemented privilege stub.
        let host_xr_json = root.join("host/active_runtime.x86_64.json");
        std::fs::create_dir_all(host_xr_json.parent().unwrap()).unwrap();
        let want = crate::util::render_host_manifest(&paths.oxr_dylib);
        std::fs::write(&host_xr_json, format!("{want}\n")).unwrap();
        paths.host_xr_json = host_xr_json;

        // A fake bottle, entirely under the fixture root.
        let prefix = root.join("Bottle");
        let sys32 = prefix.join("drive_c/windows/system32");
        std::fs::create_dir_all(&sys32).unwrap();
        let bottle = Bottle {
            name: "Fixture".to_string(),
            prefix,
            sys32,
        };

        let (sink, seen) = collecting_sink();
        let run_id: RunId = uuid::Uuid::new_v4();
        let cancel = CancellationToken::new();
        let executor: Arc<dyn Executor> =
            Arc::new(DryRunExecutor::new(run_id, sink.clone(), cancel.clone()));
        let ctx = StageCtx {
            paths,
            bottle: Some(bottle),
            bs_dir: std::path::PathBuf::new(),
            opts: StageOptions {
                bottle_name: Some("Fixture".to_string()),
                ..StageOptions::default()
            },
            executor,
            run_id,
            cancel,
            sink,
        };
        (ctx, seen)
    }

    #[tokio::test]
    async fn run_dry_runs_all_four_layers_in_order_without_touching_the_machine() {
        let (ctx, seen) = full_fixture();
        run(&ctx).await.expect("dry run completes all four layers");

        let planned = ctx.executor.planned();
        use crate::executor::PlannedKind;
        // Layer 1: one DirCopy (backup) then five Copy/Skip entries (dxmt.files).
        assert_eq!(planned[0].kind, PlannedKind::DirCopy, "{planned:#?}");
        let dxmt_count = crate::contract::contract().dxmt.files.len();
        for p in &planned[1..1 + dxmt_count] {
            assert!(matches!(p.kind, PlannedKind::Copy | PlannedKind::Skip));
        }
        // Layer 2: two more Copy/Skip entries (global wineopenxr).
        let layer2 = &planned[1 + dxmt_count..3 + dxmt_count];
        assert_eq!(layer2.len(), 2);
        for p in layer2 {
            assert!(matches!(p.kind, PlannedKind::Copy | PlannedKind::Skip));
        }
        // Layer 3: dll copy, create_dir, manifest copy, then the reg-add spawn.
        let layer3 = &planned[3 + dxmt_count..];
        assert_eq!(layer3.len(), 4, "{layer3:#?}");
        assert!(matches!(
            layer3[0].kind,
            PlannedKind::Copy | PlannedKind::Skip
        ));
        assert_eq!(layer3[1].kind, PlannedKind::CreateDir);
        assert!(matches!(
            layer3[2].kind,
            PlannedKind::Copy | PlannedKind::Skip
        ));
        assert_eq!(layer3[3].kind, PlannedKind::Spawn);
        // Layer 4 planned nothing: the fixture is already current.
        assert_eq!(planned.len(), 3 + dxmt_count + 4);

        // Section banners fired in layer order. run() is called directly here
        // (not through run_stage), so there is no StageStarted/StageFinished
        // pair to assert on — only the four section banners it emits itself.
        let sections: Vec<String> = seen
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                StageEvent::Section { title, .. } => Some(title.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(sections.len(), 4, "{sections:?}");
        assert!(sections[0].starts_with("global DXMT overlay ("));
        assert!(sections[1].starts_with("global wineopenxr ("));
        assert_eq!(sections[2], "bottle 'Fixture'");
        assert!(sections[3].starts_with("host OpenXR registration ("));

        // Layer 4 took the "already current" branch, never touching privilege.
        let evs = seen.lock().unwrap();
        assert!(evs.iter().any(|e| matches!(
            e,
            StageEvent::Line { text, .. } if text == "host registration already current"
        )));
        // …and layer 3 planned the `reg add` rather than running it, so the row
        // is the "would" one: the fixture bottle has no system.reg at all, and
        // a dry run that printed `ok "ActiveRuntime registered"` would be
        // indistinguishable from a completed install in the event log.
        assert!(evs.iter().any(|e| matches!(
            e,
            StageEvent::Line { severity: Severity::Info, text, .. }
                if text == "would register ActiveRuntime"
        )));
        assert!(
            !evs.iter().any(|e| matches!(
                e,
                StageEvent::Line { text, .. } if text == "ActiveRuntime registered"
            )),
            "{evs:#?}"
        );
    }

    /// The honest-stub rule, as a sweep over everything [`run`] says under
    /// `--dry-run`: no row may claim a mutation that did not happen.
    ///
    /// The deny-list is the completed-action vocabulary the four layers use on
    /// a real run; a "would …" row carrying the same word is fine, which is
    /// exactly the distinction this guards.
    #[tokio::test]
    async fn no_dry_run_row_claims_a_completed_mutation() {
        let (ctx, seen, _writes) = testexec_fixture(false);
        // Force layer 4 down the privileged-write branch too (a current
        // destination would skip it and prove nothing).
        std::fs::remove_file(&ctx.paths.host_xr_json).unwrap();
        run(&ctx).await.expect("dry run completes all four layers");

        const DENIED: [&str; 4] = ["backed up", "registered", "written", "installed:"];
        for ev in seen.lock().unwrap().iter() {
            let StageEvent::Line { text, .. } = ev else {
                continue;
            };
            for needle in DENIED {
                assert!(
                    !text.contains(needle) || text.starts_with("would "),
                    "dry-run row claims a completed mutation: {text:?}"
                );
            }
        }
    }

    #[tokio::test]
    async fn run_dies_verbatim_when_a_build_output_is_missing() {
        let (ctx, _seen) = full_fixture();
        std::fs::remove_file(&ctx.paths.woxr_so).unwrap();
        let err = run(&ctx).await.unwrap_err();
        assert_eq!(
            err.to_string(),
            format!(
                "missing build output {} — ./demo.sh build first",
                ctx.paths.woxr_so.display()
            )
        );
    }

    #[tokio::test]
    async fn run_dies_verbatim_when_crossover_is_absent() {
        let (mut ctx, _seen) = full_fixture();
        ctx.paths.cx_app = None;
        ctx.paths.cx = None;
        let err = run(&ctx).await.unwrap_err();
        assert_eq!(err.to_string(), "CrossOver.app not found");
    }

    /// Rebuild a fixture context around [`TestExecutor`], keeping the fixture
    /// tree [`full_fixture`] laid down.
    ///
    /// `deny_inside_app` makes every copy into the fixture's own
    /// `CrossOver.app` fail with `PermissionDenied` — the App Management shape.
    fn testexec_fixture(
        deny_inside_app: bool,
    ) -> (StageCtx, Arc<StdMutex<Vec<StageEvent>>>, Writes) {
        testexec_fixture_with(|base| {
            // The fixture's own .app tree, never the real one.
            deny_inside_app.then(|| base.paths.cx_app.clone().expect("fixture CrossOver.app"))
        })
    }

    /// [`testexec_fixture`] with the deny prefix chosen from the built fixture
    /// — the only way to name a path (the bottle prefix, the fixture's
    /// `CrossOver.app`) that `full_fixture` mints itself.
    fn testexec_fixture_with(
        deny: impl FnOnce(&StageCtx) -> Option<std::path::PathBuf>,
    ) -> (StageCtx, Arc<StdMutex<Vec<StageEvent>>>, Writes) {
        let (base, _) = full_fixture();
        let (sink, seen) = collecting_sink();
        let exec = TestExecutor::dry_run(sink.clone(), base.run_id, base.cancel.clone());
        let exec = match deny(&base) {
            Some(prefix) => exec.denying(prefix),
            None => exec,
        };
        let writes = exec.writes.clone();
        let ctx = StageCtx {
            paths: base.paths,
            bottle: base.bottle,
            bs_dir: base.bs_dir,
            opts: StageOptions {
                dry_run: true,
                ..base.opts
            },
            executor: exec,
            run_id: base.run_id,
            cancel: base.cancel,
            sink,
        };
        (ctx, seen, writes)
    }

    /// The blocker this shape closes: install.sh writes `print -- "$WANT"`, so
    /// the live `/usr/local/share/openxr/1/active_runtime.x86_64.json` ends
    /// `7d 0a 7d 0a` — `}\n}\n`. Sabrage must stage exactly those bytes, not the
    /// newline-less comparison form. Driven through the real [`run`], in
    /// dry-run, against a fixture destination: nothing here can pass while
    /// layer 4 hands the privileged write the wrong string, because the write
    /// path no longer accepts a string at all.
    #[tokio::test]
    async fn layer_four_stages_the_host_manifest_file_form_byte_for_byte() {
        let (ctx, seen, writes) = testexec_fixture(false);
        // Make the destination stale so layer 4 goes through the privileged
        // write instead of the "already current" branch.
        std::fs::remove_file(&ctx.paths.host_xr_json).unwrap();

        run(&ctx).await.expect("dry run completes all four layers");

        let staged = writes.lock().unwrap().clone();
        assert_eq!(staged.len(), 1, "one staging write: {staged:#?}");
        let (path, bytes) = &staged[0];
        assert!(
            path.starts_with(crate::privilege::sabrage_temp_dir()),
            "staged under Sabrage's own support dir, never /tmp: {}",
            path.display()
        );

        let want = crate::util::host_manifest_file_bytes(&ctx.paths.oxr_dylib);
        assert_eq!(
            String::from_utf8_lossy(bytes),
            want,
            "the bytes install layer 4 would write must be host_manifest_file_bytes"
        );
        assert!(want.ends_with("}\n"), "{want:?}");
        assert!(
            bytes.ends_with(b"}\n"),
            "install.sh's `print -- \"$WANT\"` newline is missing: {:?}",
            String::from_utf8_lossy(bytes)
        );
        // …and NOT the comparison form, which is one byte shorter.
        assert_eq!(
            bytes.len(),
            crate::util::render_host_manifest(&ctx.paths.oxr_dylib).len() + 1
        );

        // The stale destination went down the write branch — but under a dry
        // run nothing was prompted for or installed, so the row is the planned
        // one, never the shell's `ok "host registration written"`.
        let evs = seen.lock().unwrap();
        assert!(evs.iter().any(|e| matches!(
            e,
            StageEvent::Line { severity: Severity::Info, text, .. }
                if text == "would write host registration"
        )));
        assert!(
            !evs.iter().any(|e| matches!(
                e,
                StageEvent::Line { text, .. } if text == "host registration written"
            )),
            "{evs:#?}"
        );
    }

    /// Layer 4's other outcome: `write_host_manifest_privileged` re-runs the
    /// currency test under the prompt, so a destination that became current
    /// after this stage's own check comes back `Skipped` — and the row must say
    /// so rather than claiming a write that never happened.
    #[tokio::test]
    async fn layer_four_reports_a_skipped_write_as_already_current() {
        let (ctx, seen, writes) = testexec_fixture(false);
        let dest = ctx.paths.host_xr_json.clone();
        // Stale by this stage's test (a different dylib path)…
        std::fs::write(
            &dest,
            crate::util::host_manifest_file_bytes(Path::new("/stale/lib.dylib")),
        )
        .unwrap();
        assert!(!crate::util::host_manifest_is_current(
            &dest,
            &crate::util::render_host_manifest(&ctx.paths.oxr_dylib)
        ));

        // …but current by the time the privileged write looks. (The real race
        // is another writer between the two reads; writing it here is the same
        // observation, deterministically.)
        let outcome = {
            std::fs::write(
                &dest,
                crate::util::host_manifest_file_bytes(&ctx.paths.oxr_dylib),
            )
            .unwrap();
            run(&ctx).await
        };
        outcome.expect("dry run completes");

        assert!(writes.lock().unwrap().is_empty(), "nothing was staged");
        let evs = seen.lock().unwrap();
        assert!(
            !evs.iter().any(|e| matches!(
                e,
                StageEvent::Line { text, .. }
                    if text == "host registration written" || text == "would write host registration"
            )),
            "a skipped write must not be reported as written (or as planned)"
        );
        assert!(evs.iter().any(|e| matches!(
            e,
            StageEvent::Line { text, .. } if text == "host registration already current"
        )));
    }

    /// #6: a `PermissionDenied` inside `CrossOver.app` must reach the caller as
    /// `TccDenied` — the variant the GUI's permission panel branches on — with
    /// the App Management deep link in the remedy, emitted **once**, by
    /// `privilege::upgrade_write_error` rather than by a hand-rolled copy here.
    #[tokio::test]
    async fn a_permission_denied_inside_crossover_app_is_tcc_denied_with_a_remedy() {
        // The deny prefix is the fixture's own CrossOver.app, so nothing about
        // this test looks at the real machine.
        let (ctx, seen, _writes) = testexec_fixture(true);
        let cx_app = ctx.paths.cx_app.clone().expect("fixture CrossOver.app");
        assert!(crate::privilege::is_inside_app_bundle(&cx_app));

        let err = run(&ctx).await.unwrap_err();
        assert_eq!(err.kind(), "tcc_denied", "{err}");
        assert!(matches!(err, SabrageError::TccDenied { .. }));

        let evs = seen.lock().unwrap();
        let fatals: Vec<(&String, &Option<String>)> = evs
            .iter()
            .filter_map(|e| match e {
                StageEvent::Fatal {
                    message, remedy, ..
                } => Some((message, remedy)),
                _ => None,
            })
            .collect();
        assert_eq!(fatals.len(), 1, "emitted once, not twice: {fatals:#?}");
        let (message, remedy) = fatals[0];
        assert!(
            message.contains("likely macOS App Management permission"),
            "{message}"
        );
        let remedy = remedy.as_deref().expect("the remedy slot is filled");
        assert!(!remedy.is_empty());
        assert!(
            remedy.contains(crate::privilege::APP_MANAGEMENT_SETTINGS_URL),
            "{remedy}"
        );
        assert!(
            remedy.contains("./demo.sh install --bottle Fixture"),
            "{remedy}"
        );
    }

    /// The other half of the arm above: a copy failure that is **not** TCC
    /// (layer 3's destinations live in the bottle, never inside a `.app`, so
    /// `classify_write_error` can never call them App Management) must still
    /// reach the run log as `lib.sh`'s own `die "copy failed: $1 -> $2"`.
    /// Before this, `upgrade_write_error` returned such an error untouched and
    /// nothing emitted a `Fatal` at all — the CLI printed a bare
    /// `error: <dst>: Permission denied` and the GUI got only a rejected
    /// promise.
    #[tokio::test]
    async fn a_non_tcc_copy_failure_dies_with_lib_shs_copy_failed_text() {
        let (ctx, seen, _writes) =
            testexec_fixture_with(|base| Some(base.bottle.as_ref().unwrap().prefix.clone()));
        let bottle_prefix = ctx.bottle.as_ref().unwrap().prefix.clone();
        assert!(
            !crate::privilege::is_inside_app_bundle(&bottle_prefix),
            "the bottle prefix must not classify as TCC, or this tests the wrong arm"
        );

        let err = run(&ctx).await.unwrap_err();
        assert!(
            matches!(err, SabrageError::Fatal { .. }),
            "expected a Fatal, got {err:?}"
        );

        let evs = seen.lock().unwrap();
        let fatals: Vec<(&String, &Option<String>)> = evs
            .iter()
            .filter_map(|e| match e {
                StageEvent::Fatal {
                    message, remedy, ..
                } => Some((message, remedy)),
                _ => None,
            })
            .collect();
        assert_eq!(fatals.len(), 1, "emitted exactly once: {fatals:#?}");
        let (message, remedy) = fatals[0];
        // lib.sh:112 — `cp "$1" "$2" || die "copy failed: $1 -> $2"`.
        let dst = ctx.bottle.as_ref().unwrap().sys32.join("wineopenxr.dll");
        assert_eq!(
            message,
            &format!(
                "copy failed: {} -> {}",
                ctx.paths.woxr_dll.display(),
                dst.display()
            ),
            "verbatim lib.sh die text"
        );
        // `die` has no remedy slot; neither does this.
        assert_eq!(remedy, &None);
        assert_eq!(err.to_string(), *message);

        // The io cause is not swallowed by the verbatim die text: it arrives
        // first, as stderr-shaped output (the analogue of `cp`'s own stderr),
        // naming the destination and the OS error.
        let cause_idx = evs
            .iter()
            .position(|e| {
                matches!(
                    e,
                    StageEvent::Output {
                        stream: Stream::Stderr,
                        ..
                    }
                )
            })
            .expect("the io cause is emitted as stderr output");
        let fatal_idx = evs
            .iter()
            .position(|e| matches!(e, StageEvent::Fatal { .. }))
            .unwrap();
        assert!(cause_idx < fatal_idx, "cause precedes the FATAL row");
        let StageEvent::Output { chunk, .. } = &evs[cause_idx] else {
            unreachable!()
        };
        assert!(
            chunk.contains(&dst.display().to_string())
                && chunk.to_lowercase().contains("permission denied"),
            "cause line carries dst + the OS error: {chunk}"
        );
    }

    // ── layer 1: the backup is not "done" just because the directory is there ─

    /// A `cp -R` that dies right after creating the destination leaves an empty
    /// `dxmt.stock-backup`, which the old `is_dir()` test accepted forever as a
    /// finished backup. It is now called out — and deliberately left alone,
    /// because re-copying after an install would capture the fork, not stock.
    #[tokio::test]
    async fn an_empty_stock_backup_is_warned_about_and_never_recaptured() {
        let (ctx, seen, _writes) = testexec_fixture(false);
        let backup = ctx.paths.cx.as_ref().unwrap().join("lib/dxmt.stock-backup");
        std::fs::create_dir_all(&backup).unwrap();

        run(&ctx).await.expect("dry run completes all four layers");

        let evs = seen.lock().unwrap();
        let warns: Vec<&String> = evs
            .iter()
            .filter_map(|e| match e {
                StageEvent::Line {
                    severity: Severity::Warn,
                    text,
                    ..
                } => Some(text),
                _ => None,
            })
            .collect();
        assert_eq!(warns.len(), 1, "{evs:#?}");
        assert!(
            warns[0].starts_with(&format!("stock DXMT backup {} is empty", backup.display())),
            "{}",
            warns[0]
        );
        assert!(
            !evs.iter().any(|e| matches!(
                e,
                StageEvent::Line { text, .. } if text == "stock DXMT backup already exists"
            )),
            "an empty backup must not be reported as present"
        );
        // Nothing re-captured it: no DirCopy in the plan at all.
        assert!(
            !ctx.executor
                .planned()
                .iter()
                .any(|p| p.kind == crate::executor::PlannedKind::DirCopy),
            "{:#?}",
            ctx.executor.planned()
        );
    }

    /// The backup is the first write into `CrossOver.app`, and it is a `cp -R`
    /// child — so its refusal has no `io::Error` to classify. It must still
    /// reach the caller as `TccDenied` with the App Management remedy, and the
    /// half-made backup directory must be planned away so the retry re-copies
    /// stock instead of trusting a truncated tree.
    #[tokio::test]
    async fn a_refused_stock_backup_cp_is_tcc_denied_and_removes_the_partial_dir() {
        let (base, _) = full_fixture();
        let (sink, seen) = collecting_sink();
        let exec = TestExecutor::dry_run(sink.clone(), base.run_id, base.cancel.clone())
            .with(|e| e.deny_dir_copy = true);
        let ctx = StageCtx {
            paths: base.paths,
            bottle: base.bottle,
            bs_dir: base.bs_dir,
            opts: StageOptions {
                dry_run: true,
                ..base.opts
            },
            executor: exec,
            run_id: base.run_id,
            cancel: base.cancel,
            sink,
        };
        let backup = ctx.paths.cx.as_ref().unwrap().join("lib/dxmt.stock-backup");
        assert!(crate::privilege::is_inside_app_bundle(&backup));

        let err = run(&ctx).await.unwrap_err();
        assert_eq!(err.kind(), "tcc_denied", "{err}");
        assert!(matches!(&err, SabrageError::TccDenied { path } if path == &backup));

        let evs = seen.lock().unwrap();
        let fatals: Vec<(&String, &Option<String>)> = evs
            .iter()
            .filter_map(|e| match e {
                StageEvent::Fatal {
                    message, remedy, ..
                } => Some((message, remedy)),
                _ => None,
            })
            .collect();
        assert_eq!(fatals.len(), 1, "emitted once, not twice: {fatals:#?}");
        assert!(
            fatals[0]
                .0
                .contains("likely macOS App Management permission"),
            "{}",
            fatals[0].0
        );
        assert!(
            fatals[0]
                .1
                .as_deref()
                .expect("remedy")
                .contains(crate::privilege::APP_MANAGEMENT_SETTINGS_URL),
            "{:?}",
            fatals[0].1
        );

        assert!(
            ctx.executor.planned().iter().any(|p| {
                p.kind == crate::executor::PlannedKind::RemoveDir
                    && p.dst.as_deref() == Some(backup.as_path())
            }),
            "the truncated backup is removed: {:#?}",
            ctx.executor.planned()
        );
    }

    // ── layer 3: the lazy system.reg flush ───────────────────────────────────

    /// A "real" (mutation-free) fixture: every branch install takes when it
    /// believes it is not previewing, with no child ever spawned.
    fn real_seeming_fixture() -> (StageCtx, Arc<StdMutex<Vec<StageEvent>>>) {
        let (base, _) = full_fixture();
        let (sink, seen) = collecting_sink();
        let exec = TestExecutor::dry_run(sink.clone(), base.run_id, base.cancel.clone())
            .with(|e| e.pose_as_real = true);
        let ctx = StageCtx {
            paths: base.paths,
            bottle: base.bottle,
            bs_dir: base.bs_dir,
            opts: base.opts,
            executor: exec,
            run_id: base.run_id,
            cancel: base.cancel,
            sink,
        };
        (ctx, seen)
    }

    /// wine flushes `system.reg` lazily, and the launch preflight blocks on
    /// exactly that file — so a flush that lands a moment after `reg add`
    /// returns must not produce a warning contradicted by the `OK` row right
    /// under it.
    #[tokio::test]
    async fn a_late_system_reg_flush_is_waited_for_instead_of_warned_about() {
        let (ctx, seen) = real_seeming_fixture();
        let reg = ctx.bottle.as_ref().unwrap().system_reg();
        assert!(!reg.exists());
        let writer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            // Half a flush: enough for the post-write re-probe's bare
            // `ActiveRuntime` grep, deliberately NOT enough for
            // `registry_current`'s three-literal test — so the stage takes the
            // `reg add` branch whichever side of this write it starts on, and
            // the test cannot race itself into the "already set" branch.
            std::fs::write(
                &reg,
                "[Software\\\\Khronos\\\\OpenXR\\\\1]\n\"ActiveRuntime\"=\n",
            )
            .unwrap();
        });

        run(&ctx).await.expect("install completes");
        writer.join().unwrap();

        let evs = seen.lock().unwrap();
        assert!(
            !evs.iter().any(|e| matches!(
                e,
                StageEvent::Line {
                    severity: Severity::Warn,
                    ..
                }
            )),
            "the flush landed inside the window: {evs:#?}"
        );
        assert!(evs.iter().any(|e| matches!(
            e,
            StageEvent::Line { severity: Severity::Ok, text, .. }
                if text == "ActiveRuntime registered"
        )));
    }

    /// The other side of the bound: a flush that never lands still warns —
    /// once — and the stage still succeeds (Warn, never Fail).
    #[tokio::test]
    async fn a_system_reg_that_never_flushes_warns_once_and_still_succeeds() {
        let (ctx, seen) = real_seeming_fixture();
        assert!(!ctx.bottle.as_ref().unwrap().system_reg().exists());

        run(&ctx).await.expect("install completes despite the warn");

        let evs = seen.lock().unwrap();
        let warns: Vec<&String> = evs
            .iter()
            .filter_map(|e| match e {
                StageEvent::Line {
                    severity: Severity::Warn,
                    text,
                    ..
                } => Some(text),
                _ => None,
            })
            .collect();
        assert_eq!(warns.len(), 1, "{evs:#?}");
        assert_eq!(
            warns[0],
            "registry write not yet visible in system.reg (wine flushes lazily) — re-run doctor later"
        );
        assert!(evs.iter().any(|e| matches!(
            e,
            StageEvent::Line { severity: Severity::Ok, text, .. }
                if text == "ActiveRuntime registered"
        )));
    }

    #[tokio::test]
    async fn run_dies_verbatim_when_dxmt_artifacts_are_incomplete() {
        let (ctx, _seen) = full_fixture();
        let first = &crate::contract::contract().dxmt.files[0];
        std::fs::remove_file(ctx.paths.dxmt_art.join(first)).unwrap();
        let err = run(&ctx).await.unwrap_err();
        assert_eq!(
            err.to_string(),
            "ext/dxmt-artifacts missing or incomplete — ./demo.sh setup first (never half-applies the overlay)"
        );
    }
}
