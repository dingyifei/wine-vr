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
/// overwrite the first. Restoring that backup is the documented recovery
/// from this fix's own known-bad outcome.
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
    assert!(first.description.contains(&backups[0].0));
    assert!(second.description.contains(&backups[1].0));

    std::fs::remove_dir_all(&root).ok();
}

/// The mutation decision belongs to the executor, never to `opts.dry_run`:
/// with a preview executor and `dry_run: false` this fix deleted
/// `session.json` for real while its backup was only planned.
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
    // process, as `fixes::backend`'s equivalent test does:
    // `any_wineserver_alive` ignores WINEPREFIX, so no spawn is needed.
    ctx.paths.wineserver = Some(std::env::current_exe().expect("current_exe resolves"));

    let sink: EventSink = Arc::new(|_| {});
    let err = delete_session_json(&ctx, &sink).await.unwrap_err();
    assert!(err.to_string().contains("refusing to delete"));
    assert!(session_json.is_file(), "must not delete while refusing");

    std::fs::remove_dir_all(&root).ok();
}
