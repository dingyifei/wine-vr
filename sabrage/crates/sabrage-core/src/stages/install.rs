//! `demo.sh install` — install the bridge into CrossOver, the bottle, and the
//! host loader. Idempotent (hash-gated copies); the ONLY stage that can prompt
//! for administrator authorization.
//!
//! Reference: `scripts/demo/install.sh`. Preconditions first (`require_bottle`,
//! CrossOver present, the three build outputs, a complete DXMT artifact set),
//! then four layers — 1. the global DXMT overlay, 2. global wineopenxr, 3. the
//! bottle, 4. the host registration — each opening with a
//! [`crate::stages::StageCtx::section`] banner whose text matches the shell's
//! `print -r --` line verbatim.
//!
//! Layers 1–2 write inside `CrossOver.app` and need macOS App Management
//! (TCC), not root. [`crate::privilege::upgrade_write_error`] is called
//! uniformly — a safe no-op outside a `.app`, and no layer-3 destination is
//! inside one. Layer 4 is the pipeline's only privileged write, skipped when
//! the on-disk bytes already match: that comparison is literal, so one extra
//! byte makes the two front-ends rewrite the root-owned file after each other.
//!
//! Order, TCC classification and layer 4's file form are pinned by
//! tests::{run_dry_runs_all_four_layers_in_order_without_touching_the_machine,
//! a_permission_denied_inside_crossover_app_is_tcc_denied_with_a_remedy,
//! layer_four_stages_the_host_manifest_file_form_byte_for_byte}.

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
/// Native-only: PARITY.md § Declared by the 2026-08-30 adversarial review (round 1 fixes),
/// "**Registry flush re-probe after `reg add`.**".
const REGISTRY_FLUSH_TIMEOUT: Duration = Duration::from_secs(2);

/// Re-probe interval inside [`REGISTRY_FLUSH_TIMEOUT`].
const REGISTRY_FLUSH_POLL: Duration = Duration::from_millis(50);

/// Name prefix of an **uncommitted** stock-DXMT capture (layer 1).
///
/// `cp -R` is not atomic, so anything still carrying this prefix is a truncated
/// tree: never trusted as the backup, always swept
/// (tests::an_interrupted_backup_never_becomes_the_trusted_stock_backup).
const PARTIAL_BACKUP_PREFIX: &str = "dxmt.stock-backup.partial-";

/// Execute the stage.
pub async fn run(ctx: &StageCtx) -> Result<()> {
    let bottle = require_bottle(ctx)?;

    if ctx.paths.cx_app.is_none() {
        return Err(ctx.fatal("CrossOver.app not found", None));
    }
    // Invariant (paths.rs): cx/wine are Some exactly when cx_app is.
    let cx = ctx
        .paths
        .cx
        .as_ref()
        .expect("cx present whenever cx_app is present");

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

    if !crate::util::dxmt_files_ok(&ctx.paths) {
        return Err(ctx.fatal(
            "ext/dxmt-artifacts missing or incomplete — ./demo.sh setup first (never half-applies the overlay)",
            None,
        ));
    }

    // A preview must not read like completed work in the event log: mutation
    // rows say "would" when nothing was mutated (vocabulary shared with
    // build.rs / setup.rs; tests::no_dry_run_row_claims_a_completed_mutation).
    let dry_run = ctx.executor.is_dry_run();

    ctx.section(format!("global DXMT overlay ({}/lib/dxmt)", cx.display()));
    let exec1 = ctx.executor_for(step::INSTALL_DXMT_OVERLAY);
    let dxmt_dir = cx.join("lib/dxmt");
    let dxmt_lib = cx.join("lib");
    let dxmt_backup = cx.join("lib/dxmt.stock-backup");
    // Swept before the branch below, never inspected: a partial outlives the
    // run that created it (a cancelled `remove_dir_all`, a SIGKILL) and nothing
    // else ever collects it
    // (tests::a_leftover_partial_capture_is_swept_not_promoted).
    sweep_partial_backups(ctx, exec1.as_ref(), &dxmt_lib).await;
    if dxmt_backup.is_dir() {
        if dir_is_empty(&dxmt_backup) {
            // Never re-copied: `lib/dxmt` may already hold the fork, so a
            // re-capture would destroy the only rollback there is
            // (tests::an_empty_stock_backup_is_warned_about_and_never_recaptured).
            ctx.step(step::INSTALL_DXMT_OVERLAY).warn(format!(
                "stock DXMT backup {} is empty — the stock copy is gone; reinstall CrossOver to restore it (left alone: re-copying now would back up the fork)",
                dxmt_backup.display()
            ));
        } else {
            ctx.step(step::INSTALL_DXMT_OVERLAY)
                .info("stock DXMT backup already exists");
        }
    } else {
        // A shelled `cp -R`, the pipeline's first write into `CrossOver.app`:
        // no `io::Error` to classify, so TCC upgrading goes through
        // `upgrade_child_write_error` (tests::a_refused_stock_backup_cp_is_tcc_denied_and_removes_the_partial_dir).
        // It copies to a sibling nothing trusts: a truncated tree under the
        // committed name would be indistinguishable from a finished backup
        // (tests::an_interrupted_backup_never_becomes_the_trusted_stock_backup).
        let partial = dxmt_lib.join(format!(
            "{PARTIAL_BACKUP_PREFIX}{}",
            uuid::Uuid::new_v4().as_simple()
        ));
        if let Err(e) = exec1.dir_copy(&dxmt_dir, &partial).await {
            discard_partial_backup(ctx, exec1.as_ref(), &partial).await;
            return Err(privilege::upgrade_child_write_error(ctx, e, &dxmt_backup));
        }
        // The commit: `mv` within one directory is `rename(2)`, so
        // `dxmt.stock-backup` exists only for a `cp -R` that ran to completion,
        // which makes the `is_dir()` test above a completeness test rather than
        // a guess. Skipped when nothing was copied: a dry run planned the copy
        // instead of performing it, so there is no partial to rename.
        if partial.is_dir() && !dxmt_backup.exists() {
            let spec = ctx
                .child("/bin/mv", step::INSTALL_DXMT_OVERLAY)
                .arg(&partial)
                .arg(&dxmt_backup);
            let committed = exec1.run_child(&spec).await;
            // The tail is empty (`run_child` streams rather than collects), so
            // `upgrade_child_write_error` cannot call this App Management —
            // right, since the `cp -R` into this directory just succeeded.
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
            // Another writer (an unserialized `./demo.sh install`, which takes
            // no operation lock) captured stock between the test above and now:
            // `mv` would move the partial inside it, so drop it instead.
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
        // Output is streamed into the run's event log rather than the shell's
        // `>/dev/null 2>&1` — PARITY.md § Install (the one privileged write),
        // "`wine … reg add`'s output is captured into the event stream".
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
            // Warn, never Fail: wine flushes system.reg lazily, so the probe
            // is retried for REGISTRY_FLUSH_TIMEOUT before warning. `?`: a Stop
            // during the wait ends the stage here, before layer 4.
            if !wait_for_registry_flush(ctx, &bottle.system_reg()).await? {
                ctx.step(step::INSTALL_BOTTLE).warn(
                    "registry write not yet visible in system.reg (wine flushes lazily) — re-run doctor later",
                );
            }
            ctx.step(step::INSTALL_BOTTLE)
                .ok("ActiveRuntime registered");
        }
    }

    ensure_not_cancelled(ctx)?;
    // Refuse before the currency test: install.sh's two `${//}` substitutions
    // cannot represent a control character, so the manifest would be invalid
    // JSON, written as root (tests::a_control_character_in_the_dylib_path_refuses_layer_four).
    privilege::reject_unrepresentable_manifest_path(ctx, &ctx.paths.oxr_dylib)?;
    ctx.section(format!(
        "host OpenXR registration ({})",
        ctx.paths.host_xr_json.display()
    ));
    // The *comparison* form (no trailing newline), used only for install.sh's
    // `[ "$(cat "$HOST_XR_JSON")" = "$WANT" ]` currency test. The bytes that
    // land on disk are rendered inside the privileged write from the dylib path
    // (tests::layer_four_stages_the_host_manifest_file_form_byte_for_byte).
    let want = crate::util::render_host_manifest(&ctx.paths.oxr_dylib);
    if crate::util::host_manifest_is_current(&ctx.paths.host_xr_json, &want) {
        ctx.step(step::INSTALL_HOST_MANIFEST)
            .info("host registration already current");
    } else {
        ctx.step(step::INSTALL_HOST_MANIFEST).info(format!(
            "writing {} (needs sudo)...",
            ctx.paths.host_xr_json.display()
        ));
        // `write_host_manifest_privileged` emits StageEvent::NeedsAdmin itself
        // and re-runs the currency test under the prompt, so it can still come
        // back Skipped; saying "written" here would be a lie.
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
/// intends: the executor does the byte compare and the copy, the caller prints
/// the row — `info "unchanged: <dst>"` / `ok "installed: <dst>"`, verbatim,
/// `<dst>` the full destination path exactly as the shell prints `$2`.
///
/// # Errors
///
/// A `PermissionDenied` under a `.app` bundle is upgraded by
/// [`privilege::upgrade_write_error`] to [`SabrageError::TccDenied`], propagated
/// as-is. Every other copy failure emits the io cause as stderr-shaped output
/// and dies with `lib.sh`'s verbatim `copy failed: $1 -> $2`. See
/// tests::{a_permission_denied_inside_crossover_app_is_tcc_denied_with_a_remedy,
/// a_non_tcc_copy_failure_dies_with_lib_shs_copy_failed_text}.
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
            // Everything else: the io cause as stderr-shaped output ahead of
            // lib.sh's verbatim die text, so a plain PermissionDenied, a
            // read-only volume and ENOSPC stay distinguishable. PARITY.md §
            // Install (the one privileged write), "A copy failure prints the OS
            // error as one stderr-shaped output line".
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

/// `grep -q 'ActiveRuntime.*openxr.*wineopenxr64.json' "$PREFIX/system.reg"`:
/// true when one line carries all three literals in that order. Duplicated from
/// `checks::bridge`'s private `registry_has_active_runtime` because this stage
/// needs the read before deciding whether to run `reg add`, not only afterward
/// (tests::registry_current_requires_all_three_literals_in_order_on_one_line).
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

/// install.sh's bare post-write re-probe, `grep -q 'ActiveRuntime'`: looser
/// than `registry_current`, because it calls a bottle still holding a *stale*
/// `ActiveRuntime` value registered. Test-only pin on the shell semantics the
/// strict predicate is contrasted against.
#[cfg(test)]
fn system_reg_contains(system_reg: &Path, needle: &str) -> bool {
    std::fs::read(system_reg)
        .map(|bytes| String::from_utf8_lossy(&bytes).contains(needle))
        .unwrap_or(false)
}

/// Wait for wine's lazy `system.reg` flush; `Ok(false)` when it never landed.
///
/// The wait is on `registry_current`, the same three-literal test the launch
/// preflight blocks on (`bottle.registry`); the timeout arm never falls back to
/// install.sh's looser `ActiveRuntime` grep, so a stale value neither ends the
/// poll early nor earns an `OK` row the next launch rejects. A timeout is
/// always a warn — one more than the shell prints for a stale-value bottle.
///
/// # Errors
///
/// Cancellation is [`SabrageError::Cancelled`], never `Ok(false)`: the caller's
/// next steps are an `OK` row and the pipeline's one privileged write, so a
/// Stop must not be reported as a completed registration.
///
/// tests::{a_stale_active_runtime_value_does_not_end_the_flush_wait,
/// a_flush_that_never_lands_warns_even_with_a_stale_active_runtime,
/// a_cancel_during_the_registry_wait_stops_before_the_privileged_layer};
/// PARITY.md § Declared by the 2026-08-30 adversarial review (round 1 fixes),
/// "**Registry flush re-probe after `reg add`.**".
async fn wait_for_registry_flush(ctx: &StageCtx, system_reg: &Path) -> Result<bool> {
    let deadline = std::time::Instant::now() + REGISTRY_FLUSH_TIMEOUT;
    loop {
        if registry_current(system_reg) {
            return Ok(true);
        }
        ensure_not_cancelled(ctx)?;
        if std::time::Instant::now() >= deadline {
            return Ok(false);
        }
        tokio::time::sleep(REGISTRY_FLUSH_POLL).await;
    }
}

/// True when `dir` holds no entries at all — the shape an interrupted `cp -R`
/// leaves behind. A backup this stage committed is complete by construction;
/// this catches the ones it did not.
fn dir_is_empty(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_none())
}

/// Delete an uncommitted stock-DXMT capture, cancellation included.
///
/// The executor is tried first (so a dry run records the removal); every
/// [`crate::executor::RealExecutor`] mutation refuses once cancelled, so the
/// fallback goes straight to the filesystem — safe only because the path
/// carries `PARTIAL_BACKUP_PREFIX` and a uuid this run minted. A removal that
/// still fails is a `warn`: the leftover is inert and the next install sweeps it.
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
/// layer 3 must end the stage before the authorization prompt, not after
/// (tests::a_cancel_during_the_registry_wait_stops_before_the_privileged_layer).
fn ensure_not_cancelled(ctx: &StageCtx) -> Result<()> {
    if ctx.cancel.is_cancelled() {
        return Err(SabrageError::Cancelled);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
