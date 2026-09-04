//! `fix.delete-session-json` — delete ALVR's `session.json` to clear stale
//! manual client IP pins.
//!
//! Known-bad remedy, marked `destructive`: deleting the file has been observed
//! to leave the client at an 800x900 black screen; editing the pinned IPs in
//! place is the recovery that works. Listed in
//! [`crate::fixes::DEFERRED_CONTRACT_FIX_IDS`], so no GUI Doctor row offers a
//! button for it.
//!
//! Backs [`crate::paths::Paths::alvr_session_json`] up under
//! `ctx.paths.sabrage_appsup`'s `backups/`, routes every write and the removal
//! through [`crate::executor::Executor`], treats an absent file as
//! [`FixReport::unchanged`], and refuses while any CrossOver wineserver is
//! alive — `session.json` is machine-global, so there is no per-bottle
//! narrowing as in [`crate::fixes::backend`] (`backend::any_wineserver_alive`
//! is this crate's one home for wineserver-liveness scanning). See
//! tests::{deletes_after_backing_up_and_reports_the_backup_location,
//! refuses_while_any_wineserver_is_alive}. No shell equivalent: `run.sh`
//! never deletes this file, and no message text here is a verbatim shell
//! string.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{Result, SabrageError};
use crate::events::{StageEvent, StepId};
use crate::executor::Executor;
use crate::fixes::backend::any_wineserver_alive;
use crate::fixes::{FixAction, FixReport};
use crate::stages::{EventSink, StageCtx};

const STEP: StepId = "fix.delete-session-json";

/// Remove ALVR's `session.json` (after backing it up).
///
/// Backup and removal both go through [`crate::executor::Executor`], so
/// mutation is decided by the executor, never by `ctx.opts.dry_run`: a preview
/// context built with [`StageCtx::with_executor`] plans both. See
/// tests::a_preview_executor_beats_opts_dry_run_false. The removal uses
/// `Executor::remove_file`, not `remove_dir_all` on `alvr/`, which would take
/// the trusted-client state with it. A dry run says what it *would* do and
/// still reports `changed`.
///
/// # Errors
/// Fails when `ctx.paths.wineserver` is known and any CrossOver wineserver is
/// alive, and on a read, backup, or removal I/O failure.
pub async fn delete_session_json(ctx: &StageCtx, sink: &EventSink) -> Result<FixReport> {
    let path = ctx.paths.alvr_session_json();
    if !path.is_file() {
        return Ok(FixReport::unchanged(
            FixAction::DeleteSessionJson,
            format!("{} not present — nothing to delete", path.display()),
        ));
    }

    if let Some(wineserver) = &ctx.paths.wineserver {
        if any_wineserver_alive(wineserver) {
            return Err(ctx.fatal(
                format!(
                    "refusing to delete {} while a CrossOver session may still be running — the \
                     server core reads this file once at init and rewrites it at shutdown; stop \
                     the running session first",
                    path.display()
                ),
                Some("./demo.sh stop --bottle <name>".to_string()),
            ));
        }
    }

    // Reading is not a mutation: the dry run reads the file too, so its plan
    // carries the real byte count the backup would have.
    let bytes = std::fs::read(&path).map_err(|e| SabrageError::io(&path, e))?;
    // `ctx.paths.sabrage_appsup`, not the global `sabrage_support_dir()`: the
    // field exists so a caller can redirect Sabrage's own store away from the
    // real `$HOME` without mutating the process environment.
    let backup_dir = ctx.paths.sabrage_appsup.join("backups");

    let executor = ctx.executor_for(STEP);
    executor.create_dir_all(&backup_dir).await?;
    let backup_path = write_backup(&*executor, &backup_dir, &bytes).await?;
    executor.remove_file(&path).await?;

    if executor.is_dry_run() {
        let description = format!("would back up and delete {}", path.display());
        sink(StageEvent::info(
            ctx.run_id,
            Some(STEP),
            description.clone(),
        ));
        return Ok(FixReport::changed(
            FixAction::DeleteSessionJson,
            description,
        ));
    }

    let description = format!(
        "deleted {} (backed up to {} first — if the client goes to an 800x900 black screen, \
         restore that backup and edit the pinned IP in place instead of deleting again)",
        path.display(),
        backup_path.display()
    );
    sink(StageEvent::ok(ctx.run_id, Some(STEP), description.clone()));
    Ok(FixReport::changed(
        FixAction::DeleteSessionJson,
        description,
    ))
}

/// Write the backup under a name **no existing backup owns**, and return it.
///
/// The `session.json.<secs>` suffix is whole seconds, so two deletions inside
/// one second collide; [`Executor::create_new`] (`O_EXCL`) makes the kernel
/// allocate the `-2`, `-3`, … name instead of a probe another writer can win,
/// so the earlier backup — the one that recovers this fix's known-bad outcome
/// — is never replaced. See
/// tests::a_second_deletion_in_the_same_second_does_not_overwrite_the_first_backup.
///
/// # Errors
/// Propagates the executor's write failures.
async fn write_backup(executor: &dyn Executor, backup_dir: &Path, bytes: &[u8]) -> Result<PathBuf> {
    let base = format!("session.json.{}", unix_timestamp());
    for n in 1u32.. {
        let candidate = if n == 1 {
            backup_dir.join(&base)
        } else {
            backup_dir.join(format!("{base}-{n}"))
        };
        if executor.create_new(&candidate, bytes).await? {
            return Ok(candidate);
        }
    }
    unreachable!("u32 exhausted")
}

/// Seconds since the epoch, for the backup filename suffix. `0` on a clock
/// that reports before 1970 (never happens in practice; better than panicking
/// inside a destructive fix).
fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests;
