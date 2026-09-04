use super::*;
use crate::events::Severity;
use crate::executor::{DryRunExecutor, Executor, PlannedKind};
use crate::paths::Paths;
use crate::stages::StageOptions;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "sabrage-run-guards-{tag}-{}-{}",
        std::process::id(),
        Uuid::new_v4().as_simple()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Fixture context: every path under `root`, a [`DryRunExecutor`], and no
/// `alvr_dashboard` on disk. Nothing here can touch the real machine.
fn dry_ctx(root: &Path, opts: StageOptions) -> (StageCtx, Arc<Mutex<Vec<StageEvent>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s = seen.clone();
    let sink: EventSink = Arc::new(move |ev| s.lock().unwrap().push(ev));
    let run_id = Uuid::new_v4();
    let cancel = CancellationToken::new();
    let executor: Arc<dyn Executor> =
        Arc::new(DryRunExecutor::new(run_id, sink.clone(), cancel.clone()));
    let mut paths = Paths::new(root);
    paths.oxr_appsup = root.join("appsup-oxrsys");
    paths.sabrage_appsup = root.join("appsup-sabrage");
    paths.adb = None;
    let ctx = StageCtx::with_executor(paths, opts, sink, cancel, executor, run_id);
    (ctx, seen)
}

fn facts(protocol: &str) -> PreflightFacts {
    PreflightFacts {
        protocol: protocol.to_string(),
        encoder_process: "auto".to_string(),
    }
}

fn fresh_state() -> SessionState {
    SessionState::new(Uuid::nil(), "Steam", "/games/bs", "/repo/logs/x.log", 1)
}

fn rows(evs: &[StageEvent]) -> Vec<String> {
    evs.iter()
        .filter_map(|e| match e {
            StageEvent::Text { text, .. } => Some(text.clone()),
            StageEvent::Line { severity, text, .. } => Some(format!("[{severity}] {text}")),
            _ => None,
        })
        .collect()
}

#[test]
fn audio_eligibility_is_run_shs_if_elif_chain() {
    let bin = || Some(PathBuf::from("/opt/homebrew/bin/SwitchAudioSource"));
    // --no-audio wins over everything, binary present or not.
    assert_eq!(
        audio_eligibility(true, "alvr", bin()),
        AudioEligibility::Disabled
    );
    assert_eq!(
        audio_eligibility(true, "oxrsys", None),
        AudioEligibility::Disabled
    );
    // Both conditions must hold for the switch to be attempted.
    assert_eq!(
        audio_eligibility(false, "alvr", bin()),
        AudioEligibility::Probe(bin().unwrap())
    );
    assert_eq!(
        audio_eligibility(false, "oxrsys", bin()),
        AudioEligibility::Skip
    );
    assert_eq!(
        audio_eligibility(false, "alvr", None),
        AudioEligibility::Skip
    );
}

#[test]
fn blackhole_is_matched_as_a_whole_line() {
    assert!(blackhole_listed("MacBook Pro Speakers\nBlackHole 2ch\n"));
    assert!(blackhole_listed("BlackHole 2ch"));
    // grep -qx: a substring or a longer name is NOT a match.
    assert!(!blackhole_listed("BlackHole 2ch (Aggregate)\n"));
    assert!(!blackhole_listed("My BlackHole 2ch\n"));
    assert!(!blackhole_listed("BlackHole 16ch\n"));
    assert!(!blackhole_listed(""));
}

#[test]
fn dashboard_eligibility_is_run_shs_if_elif_chain() {
    use DashboardEligibility::*;
    assert_eq!(dashboard_eligibility(true, "alvr", true), Disabled);
    assert_eq!(dashboard_eligibility(true, "oxrsys", false), Disabled);
    assert_eq!(dashboard_eligibility(false, "oxrsys", true), Skip);
    assert_eq!(dashboard_eligibility(false, "alvr", true), Spawn);
    assert_eq!(dashboard_eligibility(false, "alvr", false), NotBuilt);
}

#[test]
fn the_guard_texts_are_run_shs_verbatim() {
    assert_eq!(
        audio_switched_line("MacBook Pro Speakers"),
        "audio: default output -> BlackHole 2ch (was: MacBook Pro Speakers)"
    );
    assert_eq!(
        audio_restored_line("MacBook Pro Speakers"),
        "audio: restored output -> MacBook Pro Speakers"
    );
    assert_eq!(
        DASHBOARD_OPENING_LINE,
        "dashboard: ALVR server dashboard opening (connects once the game is up)"
    );
    assert_eq!(DASHBOARD_CLOSED_LINE, "dashboard: closed");
}

#[tokio::test]
async fn no_audio_yields_an_inert_guard_and_one_info_row() {
    let root = scratch("audio-off");
    let (ctx, seen) = dry_ctx(
        &root,
        StageOptions {
            no_audio: true,
            ..Default::default()
        },
    );
    let mut state = fresh_state();
    let guard = AudioGuard::arm(&ctx, &facts("alvr"), &mut state)
        .await
        .unwrap();
    assert_eq!(
        rows(&seen.lock().unwrap()),
        vec!["[info] audio routing disabled (--no-audio) — sound stays on the Mac"]
    );
    assert!(state.prev_audio_output.is_none());
    assert!(guard.dry_run, "a dry run's guard never restores from Drop");
    // Nothing planned, nothing written, nothing to restore.
    assert!(ctx.executor.planned().is_empty());
    guard.release(&ctx, &mut state).await.unwrap();
    assert!(!state.guards.audio_restored, "no guard, no flag, no save");
    assert!(ctx.executor.planned().is_empty());
    std::fs::remove_dir_all(&root).unwrap();
}

/// Dropping an armed guard built over a dry-run context runs no child process and emits no restore row.
#[tokio::test]
async fn a_dry_runs_guard_restores_nothing_when_dropped() {
    let root = scratch("dry-drop");
    let marker = root.join("should-not-exist.marker");
    let script = root.join("fake-switch");
    std::fs::write(
        &script,
        format!("#!/bin/sh\ntouch '{}'\n", marker.display()),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let (ctx, seen) = dry_ctx(&root, StageOptions::default());
    let guard = AudioGuard::armed_for_test(&ctx, "MacBook Pro Speakers", &script);
    drop(guard);

    assert!(
        !marker.exists(),
        "dry-run Drop must not spawn the restore child"
    );
    assert!(
        seen.lock().unwrap().is_empty(),
        "dry-run Drop must emit no row"
    );
    std::fs::remove_dir_all(&root).unwrap();
}

/// An [`Executor`] that reports [`SabrageError::Cancelled`] for every
/// child — a Stop landing on the `SwitchAudioSource` call. Everything else
/// delegates, so nothing here reaches the machine.
#[derive(Debug)]
struct CancelChildren {
    inner: Arc<dyn Executor>,
    calls: Arc<Mutex<Vec<String>>>,
}

impl Executor for CancelChildren {
    fn with_step(&self, step: crate::events::StepId) -> Arc<dyn Executor> {
        Arc::new(CancelChildren {
            inner: self.inner.with_step(step),
            calls: self.calls.clone(),
        })
    }
    fn is_dry_run(&self) -> bool {
        self.inner.is_dry_run()
    }
    fn planned(&self) -> Vec<crate::executor::PlannedAction> {
        self.inner.planned()
    }
    fn copy_if_changed<'a>(
        &'a self,
        src: &'a Path,
        dst: &'a Path,
    ) -> crate::executor::BoxFuture<'a, Result<crate::executor::Copied>> {
        self.inner.copy_if_changed(src, dst)
    }
    fn write_atomic<'a>(
        &'a self,
        path: &'a Path,
        bytes: &'a [u8],
    ) -> crate::executor::BoxFuture<'a, Result<()>> {
        self.inner.write_atomic(path, bytes)
    }
    fn remove_dir_all<'a>(&'a self, p: &'a Path) -> crate::executor::BoxFuture<'a, Result<()>> {
        self.inner.remove_dir_all(p)
    }
    fn remove_file<'a>(&'a self, p: &'a Path) -> crate::executor::BoxFuture<'a, Result<()>> {
        self.inner.remove_file(p)
    }
    fn create_dir_all<'a>(&'a self, p: &'a Path) -> crate::executor::BoxFuture<'a, Result<()>> {
        self.inner.create_dir_all(p)
    }
    fn dir_copy<'a>(
        &'a self,
        src: &'a Path,
        dst: &'a Path,
    ) -> crate::executor::BoxFuture<'a, Result<()>> {
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
    fn touch<'a>(&'a self, p: &'a Path) -> crate::executor::BoxFuture<'a, Result<()>> {
        self.inner.touch(p)
    }
    fn run_child<'a>(
        &'a self,
        spec: &'a crate::process::ChildSpec,
    ) -> crate::executor::BoxFuture<'a, Result<std::process::ExitStatus>> {
        self.calls
            .lock()
            .unwrap()
            .push(spec.program.display().to_string());
        Box::pin(async move { Err(SabrageError::Cancelled) })
    }
    fn spawn_detached<'a>(
        &'a self,
        spec: &'a crate::process::ChildSpec,
        stdio: crate::executor::DetachedStdio,
    ) -> crate::executor::BoxFuture<'a, Result<Option<crate::executor::DetachedChild>>> {
        self.inner.spawn_detached(spec, stdio)
    }
}

/// A8-3: `run_child` can report `Cancelled` for a switch CoreAudio has
/// already applied. Arming and switching in one call meant that `?` threw
/// the guard away before the caller could hold it: `Drop` then restored
/// the device and said so, but — with no `&mut SessionState` and no
/// executor — could set neither `guards.audio_restored` nor the record, so
/// the teardown reported a pending guard over a device already back.
#[tokio::test]
async fn a_cancelled_switch_leaves_the_guard_armed_for_the_teardown() {
    let root = scratch("audio-switch-cancelled");
    let (ctx, seen) = dry_ctx(&root, StageOptions::default());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let cancelling = StageCtx {
        executor: Arc::new(CancelChildren {
            inner: ctx.executor.clone(),
            calls: calls.clone(),
        }),
        ..ctx.clone()
    };
    let mut state = fresh_state();

    // The shape `arm` hands back: the device recorded, the record saved,
    // the switch not yet run.
    let mut guard = AudioGuard::armed_for_test(
        &cancelling,
        "MacBook Pro Speakers",
        "/opt/homebrew/bin/SwitchAudioSource",
    );
    let err = guard
        .apply_switch(&cancelling, &mut state)
        .await
        .unwrap_err();
    assert!(matches!(err, SabrageError::Cancelled), "{err}");
    assert_eq!(
        calls.lock().unwrap().len(),
        1,
        "the switch was attempted exactly once"
    );
    assert_eq!(
        guard.previous_output.as_deref(),
        Some("MacBook Pro Speakers"),
        "the guard is still armed — this is what the caller keeps"
    );
    assert!(seen.lock().unwrap().is_empty(), "and nothing was announced");

    // The teardown's bounded path — not `Drop` — is what runs next, and it
    // is the only one that can record the restore.
    guard.release(&ctx, &mut state).await.unwrap();
    assert_eq!(
        rows(&seen.lock().unwrap()),
        vec!["audio: restored output -> MacBook Pro Speakers".to_string()],
        "exactly one restore row, from the release rather than from Drop"
    );
    assert!(state.guards.audio_restored);
    std::fs::remove_dir_all(&root).unwrap();
}

#[tokio::test]
async fn a_non_alvr_protocol_touches_audio_not_at_all() {
    let root = scratch("audio-legacy");
    let (ctx, seen) = dry_ctx(&root, StageOptions::default());
    let mut state = fresh_state();
    let guard = AudioGuard::arm(&ctx, &facts("oxrsys"), &mut state)
        .await
        .unwrap();
    assert!(seen.lock().unwrap().is_empty(), "the shell prints nothing");
    guard.release(&ctx, &mut state).await.unwrap();
    assert!(ctx.executor.planned().is_empty());
    std::fs::remove_dir_all(&root).unwrap();
}

/// A [`DryRunExecutor`] whose children come back **non-zero** whenever
/// `device` is one of their arguments — a `SwitchAudioSource -t output -s
/// "…AirPods Pro"` for headphones that are no longer connected. Same shape
/// as `super`'s `DenyWriteTo`; everything else delegates, so the plan still
/// records every attempt in order.
#[derive(Debug)]
struct FailSwitchTo {
    inner: Arc<dyn Executor>,
    device: std::ffi::OsString,
}

impl FailSwitchTo {
    fn around(inner: Arc<dyn Executor>, device: &str) -> Arc<FailSwitchTo> {
        Arc::new(FailSwitchTo {
            inner,
            device: device.into(),
        })
    }
}

impl Executor for FailSwitchTo {
    fn with_step(&self, step: crate::events::StepId) -> Arc<dyn Executor> {
        Arc::new(FailSwitchTo {
            inner: self.inner.with_step(step),
            device: self.device.clone(),
        })
    }
    fn is_dry_run(&self) -> bool {
        self.inner.is_dry_run()
    }
    fn planned(&self) -> Vec<crate::executor::PlannedAction> {
        self.inner.planned()
    }
    fn copy_if_changed<'a>(
        &'a self,
        src: &'a Path,
        dst: &'a Path,
    ) -> crate::executor::BoxFuture<'a, Result<crate::executor::Copied>> {
        self.inner.copy_if_changed(src, dst)
    }
    fn write_atomic<'a>(
        &'a self,
        path: &'a Path,
        bytes: &'a [u8],
    ) -> crate::executor::BoxFuture<'a, Result<()>> {
        self.inner.write_atomic(path, bytes)
    }
    fn remove_dir_all<'a>(&'a self, p: &'a Path) -> crate::executor::BoxFuture<'a, Result<()>> {
        self.inner.remove_dir_all(p)
    }
    fn remove_file<'a>(&'a self, p: &'a Path) -> crate::executor::BoxFuture<'a, Result<()>> {
        self.inner.remove_file(p)
    }
    fn create_dir_all<'a>(&'a self, p: &'a Path) -> crate::executor::BoxFuture<'a, Result<()>> {
        self.inner.create_dir_all(p)
    }
    fn dir_copy<'a>(
        &'a self,
        src: &'a Path,
        dst: &'a Path,
    ) -> crate::executor::BoxFuture<'a, Result<()>> {
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
    fn touch<'a>(&'a self, p: &'a Path) -> crate::executor::BoxFuture<'a, Result<()>> {
        self.inner.touch(p)
    }
    fn run_child<'a>(
        &'a self,
        spec: &'a crate::process::ChildSpec,
    ) -> crate::executor::BoxFuture<'a, Result<std::process::ExitStatus>> {
        let fails = spec.args.contains(&self.device);
        Box::pin(async move {
            let status = self.inner.run_child(spec).await?;
            Ok(if fails {
                use std::os::unix::process::ExitStatusExt;
                std::process::ExitStatus::from_raw(1 << 8)
            } else {
                status
            })
        })
    }
    fn spawn_detached<'a>(
        &'a self,
        spec: &'a crate::process::ChildSpec,
        stdio: DetachedStdio,
    ) -> crate::executor::BoxFuture<'a, Result<Option<crate::executor::DetachedChild>>> {
        self.inner.spawn_detached(spec, stdio)
    }
}

/// A device recorded at launch and disconnected before teardown.
const AIRPODS: &str = "Yifei\u{2019}s AirPods Pro";

/// One machine's `SwitchAudioSource -a -t output`, verbatim and in order.
fn live_outputs() -> Vec<String> {
    [
        "BlackHole 2ch",
        "MacBook Pro Speakers",
        "Steam Streaming Microphone",
        "Steam Streaming Speakers",
        "Virtual Desktop Mic",
        "Virtual Desktop Speakers",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn spawn_reasons(ctx: &StageCtx) -> Vec<String> {
    ctx.executor
        .planned()
        .into_iter()
        .filter(|p| p.kind == PlannedKind::Spawn)
        .map(|p| p.reason)
        .collect()
}

/// The recorded device is gone, so the switch back exits non-zero and the
/// Mac would stay on BlackHole — silent. Land on the built-in speakers and
/// say so.
#[tokio::test]
async fn a_recorded_device_that_vanished_falls_back_to_the_built_in_output() {
    let root = scratch("audio-fallback");
    let (mut ctx, seen) = dry_ctx(&root, StageOptions::default());
    ctx.executor = FailSwitchTo::around(ctx.executor.clone(), AIRPODS);
    let mut state = fresh_state();

    AudioGuard::armed_for_test(&ctx, AIRPODS, "/opt/homebrew/bin/SwitchAudioSource")
        .release_with(&ctx, &mut state, || std::future::ready(live_outputs()))
        .await
        .unwrap();

    assert_eq!(
        rows(&seen.lock().unwrap()),
        vec![format!(
            "[warn] recorded output device '{AIRPODS}' is not connected — would restore \
                 output -> MacBook Pro Speakers instead"
        )],
        "no `audio: restored output -> …`: that device is not what came back"
    );
    assert!(
        state.guards.audio_restored,
        "landing somewhere audible IS a restore"
    );
    assert_eq!(
        spawn_reasons(&ctx),
        vec![
            format!("/opt/homebrew/bin/SwitchAudioSource -t output -s {AIRPODS}"),
            "/opt/homebrew/bin/SwitchAudioSource -t output -s MacBook Pro Speakers".to_string(),
        ],
        "the recorded device is always tried first"
    );
    std::fs::remove_dir_all(&root).unwrap();
}

/// Nothing on the list is audible: print the remedy, leave the guard
/// pending so `session-state.json` survives for a later restore — and do
/// not fail the stage over it (run.sh's EXIT trap cannot change `exit $rc`
/// either).
#[tokio::test]
async fn an_unrestorable_device_prints_the_remedy_and_leaves_the_guard_pending() {
    let root = scratch("audio-stuck");
    let (mut ctx, seen) = dry_ctx(&root, StageOptions::default());
    ctx.executor = FailSwitchTo::around(ctx.executor.clone(), AIRPODS);
    let mut state = fresh_state();
    // What `arm` recorded before the switch: the record and the guard name
    // the same device, which is what makes the pending flag below mean
    // anything to `teardown`.
    state.prev_audio_output = Some(AIRPODS.to_string());

    AudioGuard::armed_for_test(&ctx, AIRPODS, "/opt/homebrew/bin/SwitchAudioSource")
        .release_with(&ctx, &mut state, || {
            // Only the loopback and the streaming virtuals are connected.
            std::future::ready(vec![
                "BlackHole 2ch".to_string(),
                "Virtual Desktop Speakers".to_string(),
            ])
        })
        .await
        .expect("a device that will not switch is a row, never a failed stage");

    assert_eq!(
        rows(&seen.lock().unwrap()),
        vec![format!(
            "[warn] {}",
            crate::session::audio_unrestorable_line(AIRPODS)
        )]
    );
    assert!(
        !state.guards.audio_restored,
        "the guard stays pending, so the record is kept for a later restore"
    );
    assert!(
        super::super::teardown_pending(&state),
        "…and `teardown` has to agree: this is the state that keeps the file"
    );
    assert_eq!(
        spawn_reasons(&ctx),
        vec![format!(
            "/opt/homebrew/bin/SwitchAudioSource -t output -s {AIRPODS}"
        )],
        "no fallback was attempted: every device on offer is virtual"
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[tokio::test]
async fn no_dashboard_yields_an_inert_guard_and_one_info_row() {
    let root = scratch("dash-off");
    let (ctx, seen) = dry_ctx(
        &root,
        StageOptions {
            no_dashboard: true,
            ..Default::default()
        },
    );
    let mut state = fresh_state();
    let guard = DashboardGuard::acquire(&ctx, &facts("alvr"), &mut state)
        .await
        .unwrap();
    assert_eq!(
        rows(&seen.lock().unwrap()),
        vec!["[info] ALVR dashboard disabled (--no-dashboard)"]
    );
    assert!(state.dashboard.is_none());
    guard.release(&ctx, &mut state).await.unwrap();
    assert!(ctx.executor.planned().is_empty());
    std::fs::remove_dir_all(&root).unwrap();
}

#[tokio::test]
async fn a_missing_dashboard_binary_warns_and_continues() {
    let root = scratch("dash-missing");
    let (ctx, seen) = dry_ctx(&root, StageOptions::default());
    let mut state = fresh_state();
    DashboardGuard::acquire(&ctx, &facts("alvr"), &mut state)
        .await
        .unwrap();
    let evs = seen.lock().unwrap().clone();
    assert_eq!(
        rows(&evs),
        vec![
            "[warn] alvr_dashboard not built — ./demo.sh build (continuing without the dashboard)"
        ]
    );
    assert!(matches!(
        &evs[0],
        StageEvent::Line {
            severity: Severity::Warn,
            ..
        }
    ));
    assert_eq!(evs[0].step(), Some(step::RUN_DASHBOARD));
    std::fs::remove_dir_all(&root).unwrap();
}

#[tokio::test]
async fn a_dry_run_plans_the_dashboard_spawn_and_still_prints_the_line() {
    use std::os::unix::fs::PermissionsExt;
    let root = scratch("dash-dry");
    let (ctx, seen) = dry_ctx(&root, StageOptions::default());
    std::fs::create_dir_all(ctx.paths.alvr_dashboard.parent().unwrap()).unwrap();
    std::fs::write(&ctx.paths.alvr_dashboard, b"#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(
        &ctx.paths.alvr_dashboard,
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();

    let mut state = fresh_state();
    let guard = DashboardGuard::acquire(&ctx, &facts("alvr"), &mut state)
        .await
        .unwrap();

    // A dry run spawns nothing, so there is no identity to record.
    assert!(state.dashboard.is_none());
    let plan = ctx.executor.planned();
    assert_eq!(plan.len(), 1);
    assert_eq!(plan[0].kind, PlannedKind::SpawnDetached);
    assert!(
        plan[0].describe().ends_with("> /dev/null"),
        "{}",
        plan[0].describe()
    );
    assert_eq!(rows(&seen.lock().unwrap()), vec![DASHBOARD_OPENING_LINE]);

    guard.release(&ctx, &mut state).await.unwrap();
    assert!(!state.guards.dashboard_closed, "nothing was opened");
    std::fs::remove_dir_all(&root).unwrap();
}

#[tokio::test]
async fn release_never_signals_an_identity_that_no_longer_matches() {
    let root = scratch("dash-recycled");
    let (ctx, seen) = dry_ctx(&root, StageOptions::default());
    let mut state = fresh_state();
    let mut guard = DashboardGuard::inert(&ctx);
    // start_time 0 is the "could not observe" sentinel: never a real
    // process, so `is_same_process` is false and no kill is planned.
    guard.identity = Some(ProcInfo {
        pid: std::process::id(),
        start_time: 0,
        exe: PathBuf::from("/nope"),
    });
    guard.release(&ctx, &mut state).await.unwrap();
    assert!(
        !ctx.executor
            .planned()
            .iter()
            .any(|p| p.reason.contains("kill")),
        "a mismatched identity must never be signalled"
    );
    assert!(state.guards.dashboard_closed);
    assert!(!rows(&seen.lock().unwrap()).contains(&DASHBOARD_CLOSED_LINE.to_string()));
    std::fs::remove_dir_all(&root).unwrap();
}

/// A2-3: `list_output_devices` carries `ctx.cancel` into
/// [`crate::process::capture_with`] rather than [`crate::process::capture`],
/// so a Cancel during teardown does not have to wait out the probe's full
/// [`crate::process::DEFAULT_PROBE_TIMEOUT`]. A wedged `SwitchAudioSource`
/// (here, a script that sleeps far longer than the test's own budget) must
/// return promptly — with an empty list, exactly as a missing binary
/// would — once the token is already cancelled.
#[tokio::test]
async fn list_output_devices_honors_an_already_cancelled_token() {
    let root = scratch("audio-list-cancel");
    let (ctx, _seen) = dry_ctx(&root, StageOptions::default());
    ctx.cancel.cancel();

    let slow_bin = root.join("SwitchAudioSource-slow.sh");
    std::fs::write(&slow_bin, "#!/bin/sh\nsleep 30\necho 'BlackHole 2ch'\n").unwrap();
    std::fs::set_permissions(
        &slow_bin,
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .unwrap();

    let started = tokio::time::Instant::now();
    let devices = list_output_devices(&ctx, &slow_bin).await;
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "the probe should abort on the cancelled token instead of running to completion"
    );
    assert!(devices.is_empty(), "a cancelled probe yields no devices");
    std::fs::remove_dir_all(&root).unwrap();
}
