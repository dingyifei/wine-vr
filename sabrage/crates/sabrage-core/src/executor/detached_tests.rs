use super::*;
use crate::events::step;

fn sinks() -> (RunId, EventSink, CancellationToken) {
    let sink: EventSink = Arc::new(|_| {});
    (uuid::Uuid::new_v4(), sink, CancellationToken::new())
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sabrage-detach-{name}-{}", std::process::id()));
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
