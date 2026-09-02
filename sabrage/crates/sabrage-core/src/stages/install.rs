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

/// Name prefix of an **uncommitted** stock-DXMT capture (layer 1).
///
/// `cp -R` is not atomic, so the copy lands under
/// `dxmt.stock-backup.partial-<uuid>` and is renamed onto `dxmt.stock-backup`
/// only once it has returned. Anything still carrying this prefix is a
/// truncated tree from an interrupted run: never trusted, always swept.
const PARTIAL_BACKUP_PREFIX: &str = "dxmt.stock-backup.partial-";

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
    let dxmt_lib = cx.join("lib");
    let dxmt_backup = cx.join("lib/dxmt.stock-backup");
    // Whatever an interrupted capture left behind. A partial is incomplete by
    // construction — it only becomes `dxmt.stock-backup` once `cp -R` has
    // returned — so it is swept, never inspected: the sweep runs before the
    // branch below because a partial outlives the run that created it (a
    // cancelled `remove_dir_all`, a SIGKILL) and nothing else ever collects it.
    sweep_partial_backups(ctx, exec1.as_ref(), &dxmt_lib).await;
    if dxmt_backup.is_dir() {
        if dir_is_empty(&dxmt_backup) {
            // An empty backup predates the commit-by-rename below (or came
            // from install.sh, whose `cp -R` has no cleanup at all). It is
            // deliberately *not* re-copied: by the time anyone sees this,
            // `lib/dxmt` may already hold the fork, so "re-capture the backup"
            // would overwrite the stock copy with fork binaries and destroy the
            // only rollback there is — which is also why the remedy is not
            // "remove it and re-run install": following that after this stage
            // has overlaid the fork captures the fork as the alleged stock.
            ctx.step(step::INSTALL_DXMT_OVERLAY).warn(format!(
                "stock DXMT backup {} is empty — the stock copy is gone; reinstall CrossOver to restore it (left alone: re-copying now would back up the fork)",
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
        //
        // It copies to a sibling nothing trusts, because `cp -R` is not atomic:
        // a failure (or a Stop, or a SIGKILL) part-way through leaves a
        // truncated tree, and a truncated tree under the committed name is
        // indistinguishable from a finished backup — every later install would
        // accept it, and the rollback it promises would restore half of stock.
        let partial = dxmt_lib.join(format!(
            "{PARTIAL_BACKUP_PREFIX}{}",
            uuid::Uuid::new_v4().as_simple()
        ));
        if let Err(e) = exec1.dir_copy(&dxmt_dir, &partial).await {
            discard_partial_backup(ctx, exec1.as_ref(), &partial).await;
            return Err(privilege::upgrade_child_write_error(ctx, e, &dxmt_backup));
        }
        // The commit. `mv` within one directory is `rename(2)` — atomic — so
        // `dxmt.stock-backup` exists only for a `cp -R` that ran to completion,
        // which is what makes the `is_dir()` test above a completeness test
        // rather than a guess. Same end state as install.sh's plain `cp -R`, so
        // a backup made by either front-end reads as complete to the other.
        //
        // Skipped when nothing was copied: a dry run (and the mutation-free
        // test executors) planned the copy instead of performing it, so there
        // is no partial to rename and the `mv` would be a lie about the disk.
        if partial.is_dir() && !dxmt_backup.exists() {
            let spec = ctx
                .child("/bin/mv", step::INSTALL_DXMT_OVERLAY)
                .arg(&partial)
                .arg(&dxmt_backup);
            let committed = exec1.run_child(&spec).await;
            // The tail is empty because `run_child` streams rather than
            // collects; `upgrade_child_write_error` therefore cannot call this
            // App Management, which is right — the `cp -R` into this very
            // directory succeeded a moment ago, so TCC is not the cause.
            let failed = match committed {
                Ok(status) if status.success() => None,
                Ok(status) => Some(SabrageError::ChildFailed {
                    argv0: "/bin/mv".to_string(),
                    status: crate::process::exit_code_of(status),
                    tail: Vec::new(),
                }),
                Err(e) => Some(e),
            };
            if let Some(e) = failed {
                discard_partial_backup(ctx, exec1.as_ref(), &partial).await;
                return Err(privilege::upgrade_child_write_error(ctx, e, &dxmt_backup));
            }
        } else if partial.is_dir() {
            // Another writer — an unserialized `./demo.sh install`, which does
            // not take the operation lock — captured stock between the test
            // above and now. `mv` would move the partial *inside* it; drop it
            // instead. The backup that won the race is stock either way.
            discard_partial_backup(ctx, exec1.as_ref(), &partial).await;
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
            // `?`: a Stop during the wait ends the stage here. Reporting the
            // wait's *timeout* and its *cancellation* as the same `false` used
            // to turn Stop into a warn, an `OK` row, and a fall-through into
            // layer 4 — the privileged one.
            if !wait_for_registry_flush(ctx, &bottle.system_reg()).await? {
                ctx.step(step::INSTALL_BOTTLE).warn(
                    "registry write not yet visible in system.reg (wine flushes lazily) — re-run doctor later",
                );
            }
            ctx.step(step::INSTALL_BOTTLE)
                .ok("ActiveRuntime registered");
        }
    }

    // ── 4. host OpenXR registration ──────────────────────────────────────────
    // The last chance to stop before the pipeline's only privileged write: a
    // Stop pressed anywhere in layers 1–3 (including while the `reg add` child
    // was running, which is where the executor's own guard hands cancellation
    // back) must not be followed by an authorization prompt.
    ensure_not_cancelled(ctx)?;
    // A repo path with a control character in it is a path install.sh cannot
    // represent in the manifest's JSON string literal (its two `${//}`
    // substitutions escape `\` and `"` and nothing else). Rather than install
    // invalid JSON as root over the working host registration, refuse — before
    // the currency test, because a destination that "matches" invalid JSON is
    // not current in any useful sense.
    privilege::reject_unrepresentable_manifest_path(ctx, &ctx.paths.oxr_dylib)?;
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
                    end: crate::process::ChunkEnd::Lf,
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

/// install.sh's bare post-write re-probe:
/// `grep -q 'ActiveRuntime' "$PREFIX/system.reg"` — looser than
/// [`registry_current`], and no longer this stage's predicate for anything:
/// [`wait_for_registry_flush`] waits *and* warns on the strict one, because
/// the loose test calls a bottle still holding a stale `ActiveRuntime` value
/// registered. Kept as a test-only pin on the shell semantics the strict
/// predicate is contrasted against.
#[cfg(test)]
fn system_reg_contains(system_reg: &Path, needle: &str) -> bool {
    std::fs::read(system_reg)
        .map(|bytes| String::from_utf8_lossy(&bytes).contains(needle))
        .unwrap_or(false)
}

/// Wait for wine's lazy `system.reg` flush; `Ok(false)` when it never landed.
///
/// Two predicates, deliberately:
///
/// * the **wait** is on [`registry_current`] — the same three-literal test the
///   launch preflight blocks on (`bottle.registry`). Waiting on the shell's
///   bare `ActiveRuntime` grep instead ended the poll on its first probe
///   whenever the bottle still held a *stale* ActiveRuntime value, so install
///   reported success against a file the very next launch rejected;
/// * the **warn text** is install.sh's, unchanged — but the timeout arm no
///   longer falls back to its looser `grep -q 'ActiveRuntime'` probe. Doing so
///   turned "present, but still the *stale* value" into `Ok(true)`: no warn,
///   an `OK ActiveRuntime registered` row, and the very next launch rejecting
///   the same file — the false green this function exists to remove, just
///   `REGISTRY_FLUSH_TIMEOUT` later. A timeout is now always a warn, which is
///   one more warn than the shell prints for a stale-value bottle (a
///   divergence in the honest direction; the shell's own row is unchanged for
///   every state where the flush landed).
///
/// Cancellation is [`SabrageError::Cancelled`], never `Ok(false)`: the caller's
/// next steps are an `OK` row and the pipeline's one privileged write, and a
/// Stop must not be reported as a completed registration.
async fn wait_for_registry_flush(ctx: &StageCtx, system_reg: &Path) -> Result<bool> {
    let deadline = std::time::Instant::now() + REGISTRY_FLUSH_TIMEOUT;
    loop {
        if registry_current(system_reg) {
            return Ok(true);
        }
        ensure_not_cancelled(ctx)?;
        if std::time::Instant::now() >= deadline {
            // The loop only reaches here with `registry_current` false.
            return Ok(false);
        }
        tokio::time::sleep(REGISTRY_FLUSH_POLL).await;
    }
}

/// True when `dir` holds no entries at all — the shape an install.sh `cp -R`
/// (or a pre-rename sabrage) interrupted immediately after creating the
/// directory leaves behind. A backup this stage committed is complete by
/// construction; this catches the ones it did not.
fn dir_is_empty(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_none())
}

/// Delete an uncommitted stock-DXMT capture, cancellation included.
///
/// The executor is tried first (so a dry run records the removal and performs
/// nothing), but a truncated tree is *exactly* what a cancelled `cp -R` leaves,
/// and every [`crate::executor::RealExecutor`] mutation refuses once the token
/// is cancelled — so the fallback goes straight to the filesystem. It is safe
/// there and only there: the path carries [`PARTIAL_BACKUP_PREFIX`] and a uuid
/// this run minted, so nothing else can be looking at it.
///
/// A removal that still fails is a `warn`, not a failure: the leftover is
/// inert (nothing ever reads a partial) and the next install sweeps it.
async fn discard_partial_backup(ctx: &StageCtx, exec: &dyn Executor, partial: &Path) {
    if exec.remove_dir_all(partial).await.is_ok() || !partial.exists() {
        return;
    }
    if let Err(e) = std::fs::remove_dir_all(partial) {
        ctx.step(step::INSTALL_DXMT_OVERLAY).warn(format!(
            "could not remove the partial DXMT backup {} ({e}) — it is never trusted as the backup, and the next install sweeps it",
            partial.display()
        ));
    }
}

/// Drop every `dxmt.stock-backup.partial-*` under `lib_dir`.
async fn sweep_partial_backups(ctx: &StageCtx, exec: &dyn Executor, lib_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(lib_dir) else {
        return;
    };
    let stale: Vec<std::path::PathBuf> = entries
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with(PARTIAL_BACKUP_PREFIX)
        })
        .map(|e| e.path())
        .collect();
    for partial in stale {
        discard_partial_backup(ctx, exec, &partial).await;
    }
}

/// [`SabrageError::Cancelled`] once Stop has been pressed.
///
/// Layer 4 is the pipeline's only privileged write: a Stop that arrived during
/// layer 3 must end the stage *before* the authorization prompt, not after the
/// user has typed a password into it.
fn ensure_not_cancelled(ctx: &StageCtx) -> Result<()> {
    if ctx.cancel.is_cancelled() {
        return Err(SabrageError::Cancelled);
    }
    Ok(())
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
    /// tests in this module share `scratch("full")` via `full_fixture`, and
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
        /// `dir_copy` fails the way an **interrupted** `cp -R` does: the
        /// destination is really created and really left half-populated, then
        /// the copy reports failure. `remove_dir_all` becomes real too, so the
        /// on-disk end state of the failure path is observable — which is the
        /// whole question ("is a truncated tree trusted next time?").
        truncating_dir_copy: bool,
        /// Report `is_dry_run() == false` while still mutating nothing: the
        /// only way to exercise the real-run branches (the registry re-probe,
        /// the completed-action rows) without spawning wine.
        pose_as_real: bool,
        /// Cancel this token from inside `run_child` — the shape of a Stop
        /// pressed while layer 3's `reg add` is running.
        cancel_in_run_child: Option<CancellationToken>,
    }

    impl TestExecutor {
        fn dry_run(sink: EventSink, run_id: RunId, cancel: CancellationToken) -> Arc<TestExecutor> {
            Arc::new(TestExecutor {
                inner: Arc::new(DryRunExecutor::new(run_id, sink, cancel)),
                writes: Arc::new(StdMutex::new(Vec::new())),
                deny_prefix: None,
                deny_dir_copy: false,
                truncating_dir_copy: false,
                pose_as_real: false,
                cancel_in_run_child: None,
            })
        }

        /// A copy of this executor with one knob changed.
        fn with(self: &Arc<Self>, f: impl FnOnce(&mut TestExecutor)) -> Arc<TestExecutor> {
            let mut next = TestExecutor {
                inner: self.inner.clone(),
                writes: self.writes.clone(),
                deny_prefix: self.deny_prefix.clone(),
                deny_dir_copy: self.deny_dir_copy,
                truncating_dir_copy: self.truncating_dir_copy,
                pose_as_real: self.pose_as_real,
                cancel_in_run_child: self.cancel_in_run_child.clone(),
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
                truncating_dir_copy: self.truncating_dir_copy,
                pose_as_real: self.pose_as_real,
                cancel_in_run_child: self.cancel_in_run_child.clone(),
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
            if self.truncating_dir_copy {
                return Box::pin(async move {
                    match std::fs::remove_dir_all(path) {
                        Ok(()) => Ok(()),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                        Err(e) => Err(SabrageError::io(path, e)),
                    }
                });
            }
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
            if self.truncating_dir_copy {
                return Box::pin(async move {
                    // What `cp -R` leaves when it is killed after its first
                    // entry: the destination exists and is NOT empty.
                    std::fs::create_dir_all(dst).unwrap();
                    std::fs::write(dst.join("d3d11.dll"), b"half a tree").unwrap();
                    Err(SabrageError::ChildFailed {
                        argv0: "cp".to_string(),
                        status: 130,
                        tail: vec!["cp: interrupted".to_string()],
                    })
                });
            }
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
            if let Some(cancel) = &self.cancel_in_run_child {
                cancel.cancel();
            }
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

    /// A non-empty `dxmt.stock-backup` is a finished capture: the stage reports
    /// it and never re-copies, because a second `cp -R` after the overlay has
    /// landed would capture the fork as the alleged stock.
    #[tokio::test]
    async fn a_complete_stock_backup_is_reported_and_never_recaptured() {
        let (ctx, seen, _writes) = testexec_fixture(false);
        let backup = ctx.paths.cx.as_ref().unwrap().join("lib/dxmt.stock-backup");
        std::fs::create_dir_all(&backup).unwrap();
        std::fs::write(backup.join("d3d11.dll"), b"stock").unwrap();

        run(&ctx).await.expect("dry run completes all four layers");

        assert!(
            seen.lock().unwrap().iter().any(|e| matches!(
                e,
                StageEvent::Line { severity: Severity::Info, text, .. }
                    if text == "stock DXMT backup already exists"
            )),
            "{:#?}",
            seen.lock().unwrap()
        );
        assert!(
            !ctx.executor
                .planned()
                .iter()
                .any(|p| p.kind == crate::executor::PlannedKind::DirCopy),
            "an existing backup is never re-captured: {:#?}",
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

        // The refused copy went to an uncommitted sibling, and that is what is
        // removed. The committed name was never involved at all — which is the
        // stronger statement: it exists only for a `cp -R` that finished.
        let plan = ctx.executor.planned();
        assert!(
            plan.iter().any(|p| {
                p.kind == crate::executor::PlannedKind::RemoveDir
                    && p.dst.as_deref().is_some_and(|d| {
                        d.parent() == backup.parent()
                            && d.file_name().is_some_and(|n| {
                                n.to_string_lossy().starts_with(PARTIAL_BACKUP_PREFIX)
                            })
                    })
            }),
            "the truncated capture is removed: {plan:#?}"
        );
        assert!(
            !plan
                .iter()
                .any(|p| p.dst.as_deref() == Some(backup.as_path())),
            "nothing is planned against the committed backup name: {plan:#?}"
        );
        assert!(!backup.exists());
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

    /// A `system.reg` flush that never lands: the wait times out, the stage
    /// warns exactly once and still succeeds (Warn, never Fail).
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

    /// The one thing an existing `dxmt.stock-backup` has to mean: `cp -R` ran to
    /// completion. A copy interrupted after its first entry leaves a *non-empty*
    /// truncated tree, which the emptiness test cannot tell from a finished
    /// backup — so the copy lands under a name nothing trusts and is renamed
    /// onto the committed one only on success.
    #[tokio::test]
    async fn an_interrupted_backup_never_becomes_the_trusted_stock_backup() {
        let (base, _) = full_fixture();
        let (sink, seen) = collecting_sink();
        let exec = TestExecutor::dry_run(sink.clone(), base.run_id, base.cancel.clone())
            .with(|e| e.truncating_dir_copy = true);
        let ctx = StageCtx {
            executor: exec,
            sink,
            ..base.clone()
        };
        let lib = ctx.paths.cx.as_ref().unwrap().join("lib");
        let backup = lib.join("dxmt.stock-backup");

        run(&ctx).await.unwrap_err();

        // The truncated tree is neither the backup nor left lying around under
        // a name a later run could mistake for one.
        assert!(
            !backup.exists(),
            "a half-copied tree must not become the stock backup"
        );
        assert!(
            partial_backups(&lib).is_empty(),
            "the partial capture is cleaned up: {:?}",
            partial_backups(&lib)
        );
        assert!(
            !seen.lock().unwrap().iter().any(|e| matches!(
                e,
                StageEvent::Line { text, .. } if text == "stock DXMT backup already exists"
            )),
            "nothing may report the interrupted capture as a backup"
        );

        // …and the next run re-copies stock instead of trusting the wreckage.
        let (sink2, seen2) = collecting_sink();
        let exec2 = TestExecutor::dry_run(sink2.clone(), base.run_id, base.cancel.clone());
        let ctx2 = StageCtx {
            executor: exec2,
            sink: sink2,
            opts: StageOptions {
                dry_run: true,
                ..base.opts.clone()
            },
            ..base.clone()
        };
        run(&ctx2).await.expect("the retry completes");
        assert!(
            ctx2.executor
                .planned()
                .iter()
                .any(|p| p.kind == crate::executor::PlannedKind::DirCopy),
            "the retry re-captures stock: {:#?}",
            ctx2.executor.planned()
        );
        assert!(
            !seen2.lock().unwrap().iter().any(|e| matches!(
                e,
                StageEvent::Line { text, .. } if text == "stock DXMT backup already exists"
            )),
            "{:#?}",
            seen2.lock().unwrap()
        );
    }

    /// A partial left by a run that was killed outright (no cleanup ran at all)
    /// is swept on the next install, never inspected and never promoted.
    #[tokio::test]
    async fn a_leftover_partial_capture_is_swept_not_promoted() {
        let (ctx, seen, _writes) = testexec_fixture(false);
        let lib = ctx.paths.cx.as_ref().unwrap().join("lib");
        let leftover = lib.join(format!("{PARTIAL_BACKUP_PREFIX}deadbeef"));
        std::fs::create_dir_all(&leftover).unwrap();
        std::fs::write(leftover.join("d3d11.dll"), b"half a tree").unwrap();

        run(&ctx).await.expect("dry run completes all four layers");

        let plan = ctx.executor.planned();
        assert!(
            plan.iter()
                .any(|p| p.kind == crate::executor::PlannedKind::RemoveDir
                    && p.dst.as_deref() == Some(leftover.as_path())),
            "the leftover is swept: {plan:#?}"
        );
        assert!(
            plan.iter()
                .any(|p| p.kind == crate::executor::PlannedKind::DirCopy),
            "and stock is still captured: {plan:#?}"
        );
        assert!(
            !seen.lock().unwrap().iter().any(|e| matches!(
                e,
                StageEvent::Line { text, .. } if text == "stock DXMT backup already exists"
            )),
            "a partial is never reported as the backup"
        );
        // A dry run swept nothing for real.
        assert!(leftover.is_dir());
    }

    /// Every `dxmt.stock-backup.partial-*` under `lib`.
    fn partial_backups(lib: &Path) -> Vec<std::path::PathBuf> {
        let Ok(entries) = std::fs::read_dir(lib) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(PARTIAL_BACKUP_PREFIX)
            })
            .map(|e| e.path())
            .collect()
    }

    /// Stop pressed while layer 3's `reg add` runs. The flush poll used to
    /// report cancellation and timeout as the same `false`, so the stage warned,
    /// printed `OK ActiveRuntime registered`, and walked into layer 4 — the
    /// pipeline's only privileged write — with the run already over.
    #[tokio::test]
    async fn a_cancel_during_the_registry_wait_stops_before_the_privileged_layer() {
        let (base, _) = full_fixture();
        let (sink, seen) = collecting_sink();
        let exec =
            TestExecutor::dry_run(sink.clone(), base.run_id, base.cancel.clone()).with(|e| {
                e.pose_as_real = true;
                e.cancel_in_run_child = Some(base.cancel.clone());
            });
        let ctx = StageCtx {
            executor: exec,
            sink,
            ..base.clone()
        };

        let err = run(&ctx).await.unwrap_err();
        assert!(matches!(err, SabrageError::Cancelled), "{err:?}");

        let evs = seen.lock().unwrap();
        assert!(
            !evs.iter().any(|e| matches!(
                e,
                StageEvent::Line {
                    severity: Severity::Warn,
                    ..
                }
            )),
            "a Stop is not a lazy-flush warning: {evs:#?}"
        );
        assert!(
            !evs.iter().any(|e| matches!(
                e,
                StageEvent::Line { text, .. } if text == "ActiveRuntime registered"
            )),
            "a cancelled run never claims the registration completed: {evs:#?}"
        );
        assert!(
            !evs.iter()
                .any(|e| matches!(e, StageEvent::NeedsAdmin { .. })),
            "no authorization is announced after Stop: {evs:#?}"
        );
        assert!(
            !evs.iter().any(|e| matches!(
                e,
                StageEvent::Section { title, .. } if title.starts_with("host OpenXR registration")
            )),
            "layer 4 is never entered: {evs:#?}"
        );
    }

    /// r1:A6-4 regression: a late `system.reg` flush is waited for, not warned about.
    /// wine flushes `system.reg` lazily, and the launch preflight blocks on
    /// exactly that file — so a flush that lands a moment after `reg add`
    /// returns must not produce a warning contradicted by the `OK` row right
    /// under it. The wait's predicate is the launch gate's, not the shell's
    /// looser grep: a bottle still holding a *stale* `ActiveRuntime` value
    /// satisfies `grep -q ActiveRuntime` on the first probe, so waiting on that
    /// ended the poll instantly and install reported success against a file the
    /// very next launch preflight (`bottle.registry`) blocks on.
    #[tokio::test]
    async fn a_stale_active_runtime_value_does_not_end_the_flush_wait() {
        let (ctx, seen) = real_seeming_fixture();
        let reg = ctx.bottle.as_ref().unwrap().system_reg();
        std::fs::create_dir_all(reg.parent().unwrap()).unwrap();
        std::fs::write(
            &reg,
            "[Software\\Khronos\\OpenXR\\1]\n\"ActiveRuntime\"=\"C:\\\\other\\\\someruntime.json\"\n",
        )
        .unwrap();
        assert!(system_reg_contains(&reg, "ActiveRuntime"));
        assert!(!registry_current(&reg), "stale, so `reg add` still runs");

        let target = reg.clone();
        let writer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            let mut text = std::fs::read_to_string(&target).unwrap();
            text.push_str("\"ActiveRuntime\"=\"C:\\\\openxr\\\\wineopenxr64.json\"\n");
            std::fs::write(&target, text).unwrap();
        });

        let started = std::time::Instant::now();
        run(&ctx).await.expect("install completes");
        let elapsed = started.elapsed();
        writer.join().unwrap();

        assert!(
            elapsed >= Duration::from_millis(150),
            "the wait ended before the real value flushed: {elapsed:?}"
        );
        assert!(registry_current(&reg));
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

    /// The timeout arm of the same wait. When the flush never lands, the row
    /// has to say so: falling back to the shell's `grep -q 'ActiveRuntime'`
    /// there reported a bottle whose value is still *stale* as registered —
    /// no warn, an `OK` row, and the next launch preflight blocking on the
    /// very same file. One warn more than the shell prints for this state, in
    /// the honest direction.
    #[tokio::test]
    async fn a_flush_that_never_lands_warns_even_with_a_stale_active_runtime() {
        let (ctx, seen) = real_seeming_fixture();
        let reg = ctx.bottle.as_ref().unwrap().system_reg();
        std::fs::create_dir_all(reg.parent().unwrap()).unwrap();
        // Present for the shell's loose grep, wrong for the launch gate — and
        // nothing ever rewrites it, so the wait runs out.
        std::fs::write(
            &reg,
            "[Software\\Khronos\\OpenXR\\1]\n\"ActiveRuntime\"=\"C:\\\\other\\\\someruntime.json\"\n",
        )
        .unwrap();
        assert!(system_reg_contains(&reg, "ActiveRuntime"));

        run(&ctx).await.expect("install completes");
        assert!(!registry_current(&reg), "the flush never landed");

        let evs = seen.lock().unwrap();
        assert!(
            evs.iter().any(|e| matches!(
                e,
                StageEvent::Line { severity: Severity::Warn, text, .. }
                    if text.starts_with("registry write not yet visible in system.reg")
            )),
            "a stale value that never flushed is not a silent success: {evs:#?}"
        );
    }

    /// A repo path with a control character in it cannot be rendered as valid
    /// JSON (the escape helper is install.sh's two substitutions, by design), and
    /// the host manifest is installed as `root:wheel` over the file the OpenXR
    /// loader reads. Layer 4 refuses before the currency test, so nothing is
    /// compared, staged or prompted for.
    #[tokio::test]
    async fn a_control_character_in_the_dylib_path_refuses_layer_four() {
        let (mut ctx, seen, writes) = testexec_fixture(false);
        let nasty = ctx
            .paths
            .oxr_dylib
            .parent()
            .unwrap()
            .join("libo\nxrsys-runtime.dylib");
        std::fs::write(&nasty, b"dylib").unwrap();
        ctx.paths.oxr_dylib = nasty.clone();
        // Stale, so layer 4 would otherwise go down the privileged-write branch.
        std::fs::remove_file(&ctx.paths.host_xr_json).unwrap();

        let err = run(&ctx).await.unwrap_err();
        assert!(matches!(err, SabrageError::Fatal { .. }), "{err:?}");
        assert!(err.to_string().contains("control character"), "{err}");
        assert!(writes.lock().unwrap().is_empty(), "nothing was staged");
        let evs = seen.lock().unwrap();
        assert!(
            !evs.iter()
                .any(|e| matches!(e, StageEvent::NeedsAdmin { .. })),
            "{evs:#?}"
        );
        let fatals: Vec<&String> = evs
            .iter()
            .filter_map(|e| match e {
                StageEvent::Fatal { message, .. } => Some(message),
                _ => None,
            })
            .collect();
        assert_eq!(fatals.len(), 1, "{fatals:#?}");
        assert!(
            fatals[0].contains(&nasty.display().to_string()),
            "{}",
            fatals[0]
        );
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
