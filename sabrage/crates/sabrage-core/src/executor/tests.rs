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
