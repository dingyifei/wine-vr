//! `fix.delete-session-json` — delete ALVR's `session.json` to clear stale
//! manual client IP pins.
//!
//! **This remedy is known to be broken** and is marked `destructive` for that
//! reason: deleting the file has been observed to leave the client at an
//! 800x900 black screen (the runtime recreates a session with no display
//! parameters). Editing the pinned IPs in place is the recovery that works, and
//! it supersedes this fix once the comment-preserving config editor lands
//! (design-core §4.1).
//!
//! It is also **withheld from the GUI**: `fix.delete-session-json` is in
//! [`crate::fixes::DEFERRED_CONTRACT_FIX_IDS`], so no Doctor row renders a
//! button for it, and [`crate::fixes::FixDef::consequence`] carries the
//! black-screen warning for any door that does offer it.
//!
//! Implementation notes for the fixes agent:
//! * the file is [`crate::paths::Paths::alvr_session_json`]
//!   (`~/Library/Application Support/OXRSys/alvr/session.json`);
//! * back it up under `ctx.paths.sabrage_appsup`'s `backups/` before removing
//!   anything, under a name no existing backup owns;
//! * every write **and the removal itself** go through
//!   [`crate::executor::Executor`], so a dry run plans them and mutates
//!   nothing;
//! * absent file ⇒ [`FixReport::unchanged`], never an error;
//! * refuse while a session is live (the server core reads the file once at
//!   init and rewrites it at shutdown).
//!
//! There is no shell equivalent to port byte-for-byte here — `run.sh` never
//! deletes this file itself; deletion is a manual troubleshooting step this
//! fix merely automates (with a backup the shell-driven workflow never took).
//! Nothing in this module's message text is a verbatim shell string.
//!
//! `session.json` is machine-global (`~/Library/Application Support/OXRSys/`
//! is not per-bottle), so unlike [`crate::fixes::backend`]'s edit — which can
//! narrow its liveness probe to one bottle's `WINEPREFIX` — this fix refuses
//! while **any** CrossOver wineserver is alive, via `backend::any_wineserver_alive`
//! (that module is this crate's one home for wineserver-liveness scanning; see
//! its header).

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
/// Backup and removal both go through [`crate::executor::Executor`]
/// (`create_dir_all` + `write_atomic` + [`crate::executor::Executor::remove_file`]),
/// so what mutates is decided by the executor the context carries and by
/// nothing else. Reading `ctx.opts.dry_run` here instead would be a real,
/// irreversible delete for any caller that built a preview context with
/// [`StageCtx::with_executor`] — and the backup, which *does* go through the
/// executor, would have been planned rather than written. `remove_file` exists
/// on the trait precisely so this fix is not an exception: `remove_dir_all` on
/// `alvr/` would take the trusted-client state with it.
///
/// Only the wording branches on the executor's mode: a dry run says what it
/// *would* do, at `info`, and reports `changed` all the same so the caller can
/// show what a real apply would achieve.
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
    // field exists so a caller (a test above all) can redirect Sabrage's own
    // store away from the real `$HOME`, and the crate's other backup writer
    // (`config::runtime_toml`) already uses it.
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
/// The suffix is whole seconds, so two deletions inside one second — a
/// restore/recreate/delete cycle, or two front-ends at once — collide. The
/// earlier file is the one the user is most likely to want back (it is the
/// recovery from this fix's own known-bad outcome), and `write_atomic` is
/// replace-by-rename, so it would be the one lost.
///
/// [`Executor::create_new`] makes "did I create it?" the kernel's answer
/// (`O_EXCL`) rather than a probe another process can win between, so the loop
/// is a real allocation of names, not a check-then-write race. Same
/// `<name>.<secs>`, `-2`, `-3`, … shape as `config::runtime_toml`'s
/// `next_backup_path`, which the toml writer uses for the same reason.
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
mod tests {
    use super::*;
    use crate::executor::PlannedKind;
    use crate::paths::Paths;
    use crate::stages::{StageCtx, StageOptions};
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex as StdMutex};
    use tokio_util::sync::CancellationToken;

    fn scratch(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "sabrage-session-json-fix-{tag}-{}",
            std::process::id()
        ))
    }

    /// A ctx whose OXRSys store **and** Sabrage store live under a scratch
    /// dir — never the real `~/Library/Application Support`.
    ///
    /// `sabrage_appsup` is a field rather than a call precisely so this is a
    /// redirection and not a process-global `$HOME` swap: the fix reads the
    /// field, so nothing here has to mutate the environment other tests are
    /// reading at the same instant.
    fn ctx_with_session_json(root: &Path, dry_run: bool) -> (StageCtx, PathBuf) {
        let mut paths = Paths::new(root);
        paths.oxr_appsup = root.join("OXRSys");
        paths.sabrage_appsup = root.join("Sabrage");
        let opts = StageOptions {
            dry_run,
            ..StageOptions::default()
        };
        let sink: EventSink = Arc::new(|_| {});
        let ctx = StageCtx::new(paths, opts, sink, CancellationToken::new());
        let session_json = ctx.paths.alvr_session_json();
        (ctx, session_json)
    }

    /// The backups directory the fix writes into, for the injected store.
    fn backups_dir(ctx: &StageCtx) -> PathBuf {
        ctx.paths.sabrage_appsup.join("backups")
    }

    #[tokio::test]
    async fn missing_file_is_a_noop() {
        let root = scratch("missing");
        let (ctx, _session_json) = ctx_with_session_json(&root, false);

        let sink: EventSink = Arc::new(|_| {});
        let report = delete_session_json(&ctx, &sink).await.unwrap();
        assert!(!report.changed);
        assert_eq!(report.action, FixAction::DeleteSessionJson);

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn deletes_after_backing_up_and_reports_the_backup_location() {
        let root = scratch("delete");
        let (ctx, session_json) = ctx_with_session_json(&root, false);
        std::fs::create_dir_all(session_json.parent().unwrap()).unwrap();
        std::fs::write(&session_json, b"{\"client_connections\":{}}").unwrap();

        let seen = Arc::new(StdMutex::new(Vec::new()));
        let s = seen.clone();
        let sink: EventSink = Arc::new(move |ev| s.lock().unwrap().push(ev));

        let report = delete_session_json(&ctx, &sink).await.unwrap();

        assert!(report.changed);
        assert!(!session_json.exists(), "the original must be gone");
        assert!(
            report.description.contains("800x900 black screen"),
            "must carry the known-broken-remedy caveat: {}",
            report.description
        );

        let backups_dir = backups_dir(&ctx);
        let backups: Vec<_> = std::fs::read_dir(&backups_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(backups.len(), 1, "exactly one backup file expected");
        assert_eq!(
            std::fs::read(backups[0].path()).unwrap(),
            b"{\"client_connections\":{}}"
        );
        assert!(backups[0]
            .file_name()
            .to_string_lossy()
            .starts_with("session.json."));

        assert!(seen.lock().unwrap().iter().any(|e| matches!(
            e,
            StageEvent::Line { severity, text, .. }
                if *severity == crate::events::Severity::Ok
                && text.starts_with("deleted ")
                && text.contains(&backups_dir.display().to_string())
        )));

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn dry_run_neither_backs_up_nor_deletes() {
        let root = scratch("dry");
        let (ctx, session_json) = ctx_with_session_json(&root, true);
        std::fs::create_dir_all(session_json.parent().unwrap()).unwrap();
        std::fs::write(&session_json, b"{}").unwrap();

        let sink: EventSink = Arc::new(|_| {});
        let report = delete_session_json(&ctx, &sink).await.unwrap();

        assert!(report.changed, "dry run still reports what WOULD change");
        assert!(session_json.is_file(), "dry run must not delete");
        assert_eq!(
            report.description,
            format!("would back up and delete {}", session_json.display())
        );
        assert!(
            !backups_dir(&ctx).exists(),
            "dry run must not create the backup directory either"
        );

        // Every mutation is planned, none performed — including the removal,
        // which is what `Executor::remove_file` exists for.
        let kinds: Vec<PlannedKind> = ctx.executor.planned().iter().map(|p| p.kind).collect();
        assert_eq!(
            kinds,
            vec![
                PlannedKind::CreateDir,
                PlannedKind::Write,
                PlannedKind::RemoveFile
            ]
        );
        let removal = ctx.executor.planned().pop().expect("a planned removal");
        assert_eq!(removal.dst.as_deref(), Some(session_json.as_path()));
        assert_eq!(
            removal.describe(),
            format!("would remove {} (exists)", session_json.display())
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// Two deletions inside one wall-clock second: the second backup must not
    /// overwrite the first. Restoring a backup is the documented recovery from
    /// this fix's own known-bad outcome, so the copy it silently replaced was
    /// the one the user most needed — a `session.json.<secs>` name derived from
    /// whole seconds and written with replace-by-rename semantics lost it.
    #[tokio::test]
    async fn a_second_deletion_in_the_same_second_does_not_overwrite_the_first_backup() {
        let root = scratch("backup-collision");
        let (ctx, session_json) = ctx_with_session_json(&root, false);
        std::fs::create_dir_all(session_json.parent().unwrap()).unwrap();
        let sink: EventSink = Arc::new(|_| {});

        std::fs::write(&session_json, b"first").unwrap();
        let first = delete_session_json(&ctx, &sink).await.unwrap();
        std::fs::write(&session_json, b"second").unwrap();
        let second = delete_session_json(&ctx, &sink).await.unwrap();
        assert!(first.changed && second.changed);

        let mut backups: Vec<(String, Vec<u8>)> = std::fs::read_dir(backups_dir(&ctx))
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| {
                (
                    e.file_name().to_string_lossy().into_owned(),
                    std::fs::read(e.path()).unwrap(),
                )
            })
            .collect();
        backups.sort();
        assert_eq!(backups.len(), 2, "both backups must survive: {backups:?}");
        let contents: Vec<&[u8]> = backups.iter().map(|(_, b)| b.as_slice()).collect();
        assert!(contents.contains(&b"first".as_slice()));
        assert!(contents.contains(&b"second".as_slice()));
        // Same second (the overwhelmingly likely case) ⇒ the collision suffix.
        // Across a second boundary the names differ on their own, which the
        // count above already proved.
        let shared_second = backups[0].0 == backups[1].0.trim_end_matches("-2");
        assert!(
            !shared_second || backups[1].0.ends_with("-2"),
            "expected a `-2` suffix: {backups:?}"
        );
        // Each report names the backup it actually wrote.
        assert!(first.description.contains(&backups[0].0));
        assert!(second.description.contains(&backups[1].0));

        std::fs::remove_dir_all(&root).ok();
    }

    /// The mutation decision belongs to the executor, never to `opts.dry_run`.
    ///
    /// [`StageCtx::with_executor`] lets the two disagree — `build.rs`'s tests
    /// already build contexts exactly this way — and when they did, this fix
    /// really deleted `session.json` while its backup was merely *planned*.
    #[tokio::test]
    async fn a_preview_executor_beats_opts_dry_run_false() {
        let root = scratch("preview-executor");
        let mut paths = Paths::new(&root);
        paths.oxr_appsup = root.join("OXRSys");
        paths.sabrage_appsup = root.join("Sabrage");
        let opts = StageOptions {
            dry_run: false,
            ..Default::default()
        };
        let sink: EventSink = Arc::new(|_| {});
        let run_id = uuid::Uuid::new_v4();
        let cancel = CancellationToken::new();
        let executor = Arc::new(crate::executor::DryRunExecutor::new(
            run_id,
            sink.clone(),
            cancel.clone(),
        ));
        let ctx = StageCtx::with_executor(paths, opts, sink.clone(), cancel, executor, run_id);

        let session_json = ctx.paths.alvr_session_json();
        std::fs::create_dir_all(session_json.parent().unwrap()).unwrap();
        std::fs::write(&session_json, b"{\"client_connections\":{}}").unwrap();

        let report = delete_session_json(&ctx, &sink).await.unwrap();

        assert!(report.changed);
        assert!(
            session_json.is_file(),
            "a DryRunExecutor must never delete, whatever opts.dry_run says"
        );
        assert_eq!(
            std::fs::read(&session_json).unwrap(),
            b"{\"client_connections\":{}}"
        );
        assert!(!backups_dir(&ctx).exists(), "no backup written");
        assert!(ctx
            .executor
            .planned()
            .iter()
            .any(|p| p.kind == PlannedKind::RemoveFile
                && p.dst.as_deref() == Some(session_json.as_path())));

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn refuses_while_any_wineserver_is_alive() {
        let root = scratch("refuse");
        let (mut ctx, session_json) = ctx_with_session_json(&root, false);
        std::fs::create_dir_all(session_json.parent().unwrap()).unwrap();
        std::fs::write(&session_json, b"{}").unwrap();

        // Stand in for a live wineserver with this test binary's own running
        // process, exactly like `fixes::backend`'s equivalent test — no
        // spawning needed, and `any_wineserver_alive` does not care about
        // WINEPREFIX at all, so there is no environment ambiguity to reason
        // about here.
        ctx.paths.wineserver = Some(std::env::current_exe().expect("current_exe resolves"));

        let sink: EventSink = Arc::new(|_| {});
        let err = delete_session_json(&ctx, &sink).await.unwrap_err();
        assert!(err.to_string().contains("refusing to delete"));
        assert!(session_json.is_file(), "must not delete while refusing");

        std::fs::remove_dir_all(&root).ok();
    }
}
