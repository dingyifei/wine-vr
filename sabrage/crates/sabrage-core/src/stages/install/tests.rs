use super::*;
use crate::error::SabrageError;
use crate::events::{RunId, Severity, StageEvent};
use crate::executor::DryRunExecutor;
use crate::paths::{Bottle, Paths};
use crate::stages::{EventSink, StageOptions};
use std::sync::{Arc, Mutex as StdMutex};
use tokio_util::sync::CancellationToken;

/// A fresh scratch directory, unique per call. Several tests share
/// `scratch("full")` through `full_fixture` and `cargo test` runs them
/// concurrently, so a shared path would race their fixture trees.
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

/// Every `write_atomic` the stage performs, with its bytes.
type Writes = Arc<StdMutex<Vec<(std::path::PathBuf, Vec<u8>)>>>;

/// [`DryRunExecutor`] with the test affordances it lacks: every
/// `write_atomic` is kept **with its bytes** (the plan records only a byte
/// count, and the host manifest is defined by its bytes), `copy_if_changed`
/// can fail with `PermissionDenied` under a path prefix — the shape a macOS
/// App Management refusal arrives in — and the per-field knobs below. Every
/// method no knob touches delegates, so `run()` behaves as under a plain dry
/// run and still touches nothing.
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
    /// on-disk end state of the failure path is observable.
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

    fn remove_dir_all<'a>(&'a self, path: &'a Path) -> crate::executor::BoxFuture<'a, Result<()>> {
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

    fn create_dir_all<'a>(&'a self, path: &'a Path) -> crate::executor::BoxFuture<'a, Result<()>> {
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
    ) -> crate::executor::BoxFuture<'a, Result<Option<crate::executor::DetachedChild>>> {
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

#[test]
fn host_manifest_skip_decision_matches_cat_semantics() {
    let dir = scratch("host-manifest");
    let dest = dir.join("active_runtime.x86_64.json");
    let dylib =
        std::path::PathBuf::from("/repo/ext/oxrsys/build-x64/runtime/liboxrsys-runtime.dylib");
    let want = crate::util::render_host_manifest(&dylib);

    // Missing file: not current.
    assert!(!crate::util::host_manifest_is_current(&dest, &want));

    // On-disk with the shell's single trailing newline (`print -r -- "$WANT"`).
    std::fs::write(&dest, format!("{want}\n")).unwrap();
    assert!(crate::util::host_manifest_is_current(&dest, &want));

    // No trailing newline: `$(cat …)` has nothing to strip, still current.
    std::fs::write(&dest, &want).unwrap();
    assert!(crate::util::host_manifest_is_current(&dest, &want));

    // Two trailing newlines: `$(cat …)` strips *all* of them, still current.
    std::fs::write(&dest, format!("{want}\n\n")).unwrap();
    assert!(crate::util::host_manifest_is_current(&dest, &want));

    // A stale dylib path: not current.
    let other_want = crate::util::render_host_manifest(std::path::Path::new("/other/lib.dylib"));
    assert_ne!(want, other_want);
    assert!(!crate::util::host_manifest_is_current(&dest, &other_want));

    std::fs::remove_dir_all(&dir).unwrap();
}

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

    std::fs::remove_dir_all(&dir).unwrap();
}

/// Builds a complete on-disk fixture (build outputs, DXMT artifacts, a
/// fake CrossOver.app tree, a fake bottle, and a host manifest already
/// current) so [`run`] can execute all four layers without touching the
/// real machine and without reaching
/// `privilege::write_host_manifest_privileged` (layer 4 takes the
/// "already current" branch, so no test can prompt for authorization).
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

    // DXMT artifacts: every file the contract lists.
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
    // `run()` never reaches the privileged write.
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
    // Layer 1: one DirCopy (the backup), then one Copy/Skip per dxmt.files entry.
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
fn testexec_fixture(deny_inside_app: bool) -> (StageCtx, Arc<StdMutex<Vec<StageEvent>>>, Writes) {
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

/// install.sh writes `print -r -- "$WANT"`, so the live
/// `/usr/local/share/openxr/1/active_runtime.x86_64.json` ends `7d 0a 7d
/// 0a`. Layer 4 stages exactly those bytes, not the newline-less
/// comparison form. Driven through the real [`run`] in dry-run against a
/// fixture destination.
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
/// `classify_write_error` can never call them App Management) still reaches
/// the run log as one `Fatal` carrying `lib.sh`'s own
/// `die "copy failed: $1 -> $2"`, with the io cause emitted before it.
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
    // lib.sh's copy helper: `cp "$1" "$2" || die "copy failed: $1 -> $2"`.
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

/// A `cp -R` that dies right after creating the destination leaves an empty
/// `dxmt.stock-backup`. It is warned about and deliberately left alone:
/// re-copying after an install has landed would capture the fork, not stock.
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
                        && d.file_name()
                            .is_some_and(|n| n.to_string_lossy().starts_with(PARTIAL_BACKUP_PREFIX))
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

/// Stop pressed while layer 3's `reg add` runs. Cancellation is distinct
/// from a timed-out flush, so the stage neither warns nor claims the
/// registration and never enters layer 4 — the pipeline's only privileged
/// write.
#[tokio::test]
async fn a_cancel_during_the_registry_wait_stops_before_the_privileged_layer() {
    let (base, _) = full_fixture();
    let (sink, seen) = collecting_sink();
    let exec = TestExecutor::dry_run(sink.clone(), base.run_id, base.cancel.clone()).with(|e| {
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

/// The timeout arm of r1:A6-4's wait: when the flush never lands the stage
/// warns, even for a bottle whose stale `ActiveRuntime` value satisfies
/// the shell's `grep -q ActiveRuntime`. One warn more than the shell
/// prints for this state, in the honest direction.
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
