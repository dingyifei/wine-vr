use super::*;
use crate::events::{Severity, StageEvent};
use crate::executor::{DetachedChild, Executor, PlannedAction, PlannedKind};
use crate::paths::{Bottle, Paths};
use crate::process::ProcInfo;
use crate::session::state::WiredForward;
use crate::stages::{EventSink, StageCtx, StageOptions};
use std::path::Path;
use std::sync::{Arc, Mutex as StdMutex};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// A pid no process can have on macOS (the default `kern.maxproc` ceiling
/// is five digits), so the "dead" case is deterministic and free of a
/// pid-reuse race. Not `u32::MAX`: that is `-1` as an `i32`, and
/// `kill(-1, …)` addresses every process the user can signal.
const DEAD_PID: u32 = 2_147_483_646;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "sabrage-reconcile-{tag}-{}-{}",
        std::process::id(),
        Uuid::new_v4().simple()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A context whose Sabrage store and adb both point **inside the fixture**.
///
/// `sabrage_appsup` is the load-bearing override: without it `Paths::new`
/// derives it from the real `$HOME`, and these tests read — and with a real
/// executor write — the developer's own `session-state.json`.
fn test_ctx(dir: &Path, dry_run: bool) -> (StageCtx, Arc<StdMutex<Vec<StageEvent>>>) {
    let seen = Arc::new(StdMutex::new(Vec::new()));
    let s = seen.clone();
    let sink: EventSink = Arc::new(move |ev| s.lock().unwrap().push(ev));
    let mut paths = Paths::new(dir.join("repo"));
    paths.sabrage_appsup = dir.join("appsup");
    paths.adb = Some(dir.join("bin/adb"));
    let ctx = StageCtx::new(
        paths,
        StageOptions {
            dry_run,
            ..StageOptions::default()
        },
        sink,
        CancellationToken::new(),
    );
    (ctx, seen)
}

fn me() -> ProcInfo {
    ProcInfo::observe(std::process::id()).expect("this test process is observable")
}

fn recycled() -> ProcInfo {
    let mut p = me();
    p.start_time += 1;
    p
}

fn dead() -> ProcInfo {
    ProcInfo {
        pid: DEAD_PID,
        start_time: 1,
        exe: PathBuf::from("/Applications/CrossOver.app/…/wine"),
    }
}

/// A record with every guard still pending.
fn pending(wine: Option<ProcInfo>, dashboard: Option<ProcInfo>) -> SessionState {
    SessionState {
        wine,
        dashboard,
        prev_audio_output: Some("MacBook Pro Speakers".into()),
        wired_forwards: vec![
            WiredForward {
                serial: "1WMHH000X00000".into(),
                port: 9943,
            },
            WiredForward {
                serial: "1WMHH000X00000".into(),
                port: 9944,
            },
        ],
        ..SessionState::new(
            Uuid::new_v4(),
            "Steam",
            "/games/Beat Saber 1294",
            "/repo/logs/beatsaber-20260829-101112.log",
            1_786_300_214_181,
        )
    }
}

fn write_state(ctx: &StageCtx, state: &SessionState) {
    let path = ctx.paths.session_state_path();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut bytes = serde_json::to_vec_pretty(state).unwrap();
    bytes.push(b'\n');
    std::fs::write(&path, bytes).unwrap();
}

/// Probe stub: the tool is installed, reports `device`, and lists nothing
/// (the fallback pool only matters where a test says it does).
fn probing(device: &str) -> impl FnOnce() -> std::future::Ready<Result<Option<AudioProbe>>> + '_ {
    probing_list(device, &[])
}

/// [`probing`] with a device list — `SwitchAudioSource -a -t output`.
fn probing_list<'a>(
    device: &'a str,
    outputs: &'a [&'a str],
) -> impl FnOnce() -> std::future::Ready<Result<Option<AudioProbe>>> + 'a {
    move || {
        std::future::ready(Ok(Some(AudioProbe {
            bin: PathBuf::from("/fixture/SwitchAudioSource"),
            current: device.into(),
            outputs: outputs.iter().map(|o| o.to_string()).collect(),
        })))
    }
}

/// Probe stub: no `SwitchAudioSource` on this machine.
fn no_probe() -> impl FnOnce() -> std::future::Ready<Result<Option<AudioProbe>>> {
    || std::future::ready(Ok(None))
}

/// Probe stub reporting a binary **inside the fixture that does not exist**,
/// so a *real* executor's `run_child` fails at `spawn` with `ENOENT`. The
/// cheapest way to make one reconcile mutation fail for real without
/// touching the machine — nothing is executed, and the device is untouched.
fn probing_a_vanished_binary(
    dir: &Path,
) -> impl FnOnce() -> std::future::Ready<Result<Option<AudioProbe>>> {
    let bin = dir.join("bin/SwitchAudioSource");
    move || {
        std::future::ready(Ok(Some(AudioProbe {
            bin,
            current: BLACKHOLE.to_string(),
            outputs: Vec::new(),
        })))
    }
}

/// The recorded output device of the disconnected-device defect: connected
/// at launch, gone by the time `stop` ran.
const AIRPODS: &str = "Yifei\u{2019}s AirPods Pro";

/// `SwitchAudioSource -a -t output` on that machine, verbatim and in order.
const LIVE_OUTPUTS: [&str; 6] = [
    "BlackHole 2ch",
    "MacBook Pro Speakers",
    "Steam Streaming Microphone",
    "Steam Streaming Speakers",
    "Virtual Desktop Mic",
    "Virtual Desktop Speakers",
];

/// A [`crate::executor::DryRunExecutor`] whose children come back
/// **non-zero** whenever `device` is one of their arguments: the machine
/// failure the audio fallback exists for, a disconnected recorded output
/// device.
///
/// Everything else delegates, and the inner executor still *sees* the
/// child, so the plan records every attempt in order.
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

/// `exit 1` as an [`std::process::ExitStatus`], the way `wait(2)` encodes it.
fn exit_1() -> std::process::ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    std::process::ExitStatus::from_raw(1 << 8)
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
    fn planned(&self) -> Vec<PlannedAction> {
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
            Ok(if fails { exit_1() } else { status })
        })
    }
    fn spawn_detached<'a>(
        &'a self,
        spec: &'a crate::process::ChildSpec,
        stdio: crate::executor::DetachedStdio,
    ) -> crate::executor::BoxFuture<'a, Result<Option<DetachedChild>>> {
        self.inner.spawn_detached(spec, stdio)
    }
}

fn rows(seen: &Arc<StdMutex<Vec<StageEvent>>>) -> Vec<(Severity, String)> {
    seen.lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            StageEvent::Line {
                severity,
                text,
                step,
                ..
            } => {
                assert_eq!(
                    step.as_deref(),
                    Some(STEP),
                    "every reconcile row is attributed"
                );
                Some((*severity, text.clone()))
            }
            _ => None,
        })
        .collect()
}

fn sections(seen: &Arc<StdMutex<Vec<StageEvent>>>) -> Vec<String> {
    seen.lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            StageEvent::Section { title, .. } => Some(title.clone()),
            _ => None,
        })
        .collect()
}

fn spawns(planned: &[PlannedAction]) -> Vec<String> {
    planned
        .iter()
        .filter(|p| p.kind == PlannedKind::Spawn)
        .map(|p| p.reason.clone())
        .collect()
}

#[test]
fn classify_reads_pid_and_start_time_only() {
    assert_eq!(classify(&pending(None, None)), Classification::Dead);
    assert_eq!(classify(&pending(Some(dead()), None)), Classification::Dead);
    assert_eq!(classify(&pending(Some(me()), None)), Classification::Live);
    assert_eq!(
        classify(&pending(Some(recycled()), None)),
        Classification::IdentityMismatch,
    );
}

#[test]
fn a_zero_pid_is_dead_never_a_process_group() {
    let zero = ProcInfo {
        pid: 0,
        start_time: 0,
        exe: PathBuf::from("/"),
    };
    assert_eq!(
        classify(&pending(Some(zero.clone()), None)),
        Classification::Dead
    );
    assert!(!signalable(&zero), "pid 0 must never be signalled");
    assert!(!signalable(&dead()), "a gone pid must never be signalled");
    assert!(
        !signalable(&recycled()),
        "a recycled pid must never be signalled"
    );
    assert!(signalable(&me()));
}

#[test]
fn the_spawn_fallback_start_time_can_never_match() {
    // `spawn_detached` records start_time 0 when it cannot observe the pid;
    // that identity gets its own classification, which restores nothing.
    // `IdentityMismatch` would undo the guards of a possibly-running
    // session (A9-5).
    let unobserved = ProcInfo {
        start_time: 0,
        ..me()
    };
    assert!(
        !unobserved.is_same_process(),
        "0 never matches a real start"
    );
    assert_eq!(
        classify(&pending(Some(unobserved.clone()), None)),
        Classification::Unverifiable
    );
    assert_eq!(restore_mode(Classification::Unverifiable), None);
    assert!(
        !signalable(&unobserved),
        "and nothing may be signalled on that identity"
    );
    // A start time that IS observed and differs really is a recycled pid.
    assert_eq!(
        classify(&pending(Some(recycled()), None)),
        Classification::IdentityMismatch
    );
}

#[tokio::test]
async fn no_state_file_is_no_session_and_says_nothing() {
    let dir = scratch("nosession");
    let (ctx, seen) = test_ctx(&dir, true);
    let out = reconcile_with(&ctx, None, None, no_probe()).await.unwrap();
    assert_eq!(out, Reconciled::NoSession);
    assert!(seen.lock().unwrap().is_empty());
    assert!(ctx.executor.planned().is_empty());
    std::fs::remove_dir_all(&dir).ok();
}

/// Every live-shaped record is adopted untouched, whatever kind of live it is.
#[tokio::test]
async fn live_records_are_adopted_without_touching_anything() {
    let unobserved = ProcInfo {
        start_time: 0,
        ..me()
    };
    let cases: &[(&str, ProcInfo, Option<ProcInfo>, Classification)] = &[
        ("live", me(), Some(me()), Classification::Live),
        (
            "unverifiable [r1:A9-5]",
            unobserved,
            None,
            Classification::Unverifiable,
        ),
    ];
    for (i, (label, wine, dashboard, class)) in cases.iter().enumerate() {
        let dir = scratch(&format!("live-reconcile-{i}"));
        let (ctx, seen) = test_ctx(&dir, true);
        let state = pending(Some(wine.clone()), dashboard.clone());
        assert_eq!(classify(&state), *class, "{label}: classification");
        write_state(&ctx, &state);

        let out = reconcile_with(&ctx, None, None, probing(BLACKHOLE))
            .await
            .unwrap();
        assert_eq!(out, Reconciled::Live { state }, "{label}");
        assert!(
            seen.lock().unwrap().is_empty(),
            "{label}: adoption is silent"
        );
        assert!(
            ctx.executor.planned().is_empty(),
            "{label}: no SwitchAudioSource, no adb forward --remove, no clear"
        );
        assert!(
            ctx.paths.session_state_path().exists(),
            "{label}: a live session's record must survive"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}

#[tokio::test]
async fn a_dead_session_restores_every_guard_then_clears_the_record() {
    let dir = scratch("dead");
    let (ctx, seen) = test_ctx(&dir, true);
    // Dashboard identity = this test process, so `is_same_process` holds and
    // the kill is *planned*. Under DryRunExecutor nothing is ever spawned.
    write_state(&ctx, &pending(Some(dead()), Some(me())));

    let out = reconcile_with(&ctx, None, None, probing(BLACKHOLE))
        .await
        .unwrap();
    let Reconciled::Dead {
        state,
        restored,
        pending,
    } = out
    else {
        panic!("expected Dead, got {out:?}");
    };
    assert!(!pending, "every guard was released, so the record went");
    assert!(state.guards.audio_restored);
    assert!(state.guards.dashboard_closed);
    assert!(state.guards.forwards_cleared);
    assert!(!state.has_pending_guards());

    assert_eq!(
        restored,
        vec![
            "audio: would restore output -> MacBook Pro Speakers (previous session did not \
                 shut down cleanly)",
            "ALVR dashboard would be closed (left over from the previous session)",
            "would clear adb forward tcp:9943 on 1WMHH000X00000",
            "would clear adb forward tcp:9944 on 1WMHH000X00000",
        ]
    );
    assert_eq!(sections(&seen), vec![RECONCILE_SECTION], "banner once");
    assert_eq!(
        rows(&seen).into_iter().map(|(s, _)| s).collect::<Vec<_>>(),
        vec![Severity::Ok, Severity::Ok, Severity::Info, Severity::Info],
    );

    // Every flag hits disk before the next guard is touched: progress has
    // to be crash-durable (A9-4). See
    // tests::a_removal_that_took_is_on_disk_before_the_next_one_is_tried.
    let planned = ctx.executor.planned();
    let kinds: Vec<PlannedKind> = planned.iter().map(|p| p.kind).collect();
    assert_eq!(
        kinds,
        vec![
            PlannedKind::Spawn,
            PlannedKind::CreateDir,
            PlannedKind::Write,
            PlannedKind::Spawn,
            PlannedKind::CreateDir,
            PlannedKind::Write,
            PlannedKind::Spawn,
            PlannedKind::CreateDir,
            PlannedKind::Write,
            PlannedKind::Spawn,
            PlannedKind::CreateDir,
            PlannedKind::Write,
            PlannedKind::CreateDir,
            PlannedKind::Write,
            PlannedKind::RemoveFile,
        ]
    );
    let argv = spawns(&planned);
    assert_eq!(
        argv[0],
        "/fixture/SwitchAudioSource -t output -s MacBook Pro Speakers"
    );
    assert_eq!(argv[1], format!("/bin/kill -TERM {}", std::process::id()));
    assert!(argv[2].ends_with("-s 1WMHH000X00000 forward --remove tcp:9943"));
    assert!(argv[3].ends_with("-s 1WMHH000X00000 forward --remove tcp:9944"));
    assert!(
        !argv.iter().any(|a| a.contains("--remove-all")),
        "never --remove-all: {argv:?}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn an_identity_mismatch_restores_the_pid_free_guards_and_signals_nothing() {
    let dir = scratch("mismatch");
    let (ctx, seen) = test_ctx(&dir, true);
    write_state(&ctx, &pending(Some(recycled()), Some(me())));

    let out = reconcile_with(&ctx, None, None, probing(BLACKHOLE))
        .await
        .unwrap();
    let Reconciled::IdentityMismatch {
        state,
        restored,
        pending,
    } = out
    else {
        panic!("expected IdentityMismatch, got {out:?}");
    };
    assert!(!pending);
    assert!(state.guards.audio_restored);
    assert!(
        !state.guards.dashboard_closed,
        "SafeOnly leaves the dashboard untouched and unflagged"
    );
    assert!(state.guards.forwards_cleared);
    assert_eq!(restored.len(), 3, "audio + two forwards, no dashboard");

    let argv = spawns(&ctx.executor.planned());
    assert!(
        !argv.iter().any(|a| a.contains("/bin/kill")),
        "a recycled pid must never be signalled: {argv:?}"
    );
    assert_eq!(sections(&seen), vec![RECONCILE_SECTION]);
    std::fs::remove_dir_all(&dir).ok();
}

/// A live process that is not this one, for the ownership guard. Killed on
/// drop, so no fixture can leak a process onto the machine.
struct ForeignProcess(std::process::Child);

impl ForeignProcess {
    fn spawn() -> ForeignProcess {
        ForeignProcess(
            std::process::Command::new("/bin/sleep")
                .arg("30")
                .spawn()
                .expect("/bin/sleep is on every macOS"),
        )
    }
    fn pid(&self) -> u32 {
        self.0.id()
    }
}

impl Drop for ForeignProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A9-1. Between the first guard and the wine spawn the record exists with
/// `wine` still `None`, so `classify` says `Dead` and only the run stage's
/// published phase knows the launch is happening. Reconciling it would
/// restore the audio device mid-launch, `SIGTERM` the dashboard this launch
/// just spawned, pull its `--wired` forwards and delete its record, under a
/// launch that keeps going.
#[tokio::test]
async fn a_record_belonging_to_the_launch_in_progress_is_never_touched() {
    for phase in [
        crate::session::SessionPhase::Preflight,
        crate::session::SessionPhase::Launching,
        crate::session::SessionPhase::Stopping,
    ] {
        let dir = scratch("in-flight");
        let (ctx, seen) = test_ctx(&dir, true);
        // Exactly the pre-spawn record: guards taken, no wine child.
        let state = pending(None, Some(me()));
        write_state(&ctx, &state);

        let out = reconcile_with(
            &ctx,
            None,
            Some(crate::session::RunPhaseInfo {
                phase,
                run_id: state.run_id,
                bottle: "Steam".into(),
                exit_code: None,
            }),
            probing(BLACKHOLE),
        )
        .await
        .unwrap();

        assert_eq!(
            out,
            Reconciled::Busy {
                state,
                reason: RECORD_IN_FLIGHT.to_string(),
                silent: true,
            },
            "{phase:?}"
        );
        assert!(
            ctx.executor.planned().is_empty(),
            "{phase:?}: no switch, no kill, no removal, no write"
        );
        assert!(
            seen.lock().unwrap().is_empty(),
            "{phase:?}: our own launch is not a warning"
        );
        assert!(
            ctx.paths.session_state_path().exists(),
            "{phase:?}: the launch still needs its record"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}

/// …and a phase published for a *different* run does not shield a stale
/// record: that is the ordinary "recover before launching" case, and it is
/// what `run` calls reconcile for.
#[tokio::test]
async fn a_launch_in_progress_does_not_shield_some_other_runs_record() {
    let dir = scratch("in-flight-other");
    let (ctx, _seen) = test_ctx(&dir, true);
    let mut state = pending(Some(dead()), None);
    state.wired_forwards.clear();
    write_state(&ctx, &state);

    let out = reconcile_with(
        &ctx,
        None,
        Some(crate::session::RunPhaseInfo {
            phase: crate::session::SessionPhase::Preflight,
            run_id: Uuid::new_v4(),
            bottle: "Steam".into(),
            exit_code: None,
        }),
        probing(BLACKHOLE),
    )
    .await
    .unwrap();

    assert!(matches!(out, Reconciled::Dead { .. }), "{out:?}");
    assert!(
        !ctx.executor.planned().is_empty(),
        "the previous session's guards are still this launch's to clean up"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A9-3. The other front-end's record: `sabrage run` in a terminal next to
/// an open Sabrage. `owner_pid` names the process running it, and reconcile
/// must not touch its guards.
#[tokio::test]
async fn a_record_a_live_foreign_process_owns_is_reported_and_left_alone() {
    let dir = scratch("owned-elsewhere");
    let (ctx, seen) = test_ctx(&dir, true);
    let foreign = ForeignProcess::spawn();
    let mut state = pending(None, Some(me()));
    state.set_owner(foreign.pid());
    write_state(&ctx, &state);

    let out = reconcile_with(&ctx, None, None, probing(BLACKHOLE))
        .await
        .unwrap();

    assert_eq!(
        out,
        Reconciled::Busy {
            state,
            reason: owned_elsewhere_row(foreign.pid()),
            silent: false,
        }
    );
    assert!(ctx.executor.planned().is_empty());
    assert_eq!(
        rows(&seen),
        vec![(Severity::Warn, owned_elsewhere_row(foreign.pid()))],
        "the user is told why nothing was restored"
    );
    assert!(ctx.paths.session_state_path().exists());
    std::fs::remove_dir_all(&dir).ok();
}

/// A9-8. A record from a newer Sabrage may describe a guard this build
/// cannot undo: rewriting it through this struct would drop that
/// description, and clearing it would throw it away entirely.
#[tokio::test]
async fn a_newer_schema_record_is_reported_and_never_rewritten() {
    let dir = scratch("newer-schema");
    let (ctx, seen) = test_ctx(&dir, true);
    let path = ctx.paths.session_state_path();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut state = pending(Some(dead()), None);
    state.version = state::SESSION_STATE_VERSION + 1;
    let mut json = serde_json::to_value(&state).unwrap();
    json["futureGuard"] = serde_json::json!({ "somethingWeCannotUndo": true });
    std::fs::write(&path, serde_json::to_vec_pretty(&json).unwrap()).unwrap();

    let out = reconcile_with(&ctx, None, None, probing(BLACKHOLE))
        .await
        .unwrap();

    assert!(
        matches!(&out, Reconciled::Busy { reason, .. } if *reason == newer_schema_row(2)),
        "{out:?}"
    );
    assert!(
        ctx.executor.planned().is_empty(),
        "a guard we do not understand is not a guard we may release"
    );
    assert_eq!(rows(&seen), vec![(Severity::Warn, newer_schema_row(2))]);
    assert!(path.exists());
    std::fs::remove_dir_all(&dir).ok();
}

/// A9-1. Every `Busy` except this process's own in-flight record is a
/// record somebody is still using, and the launch path has to be able to
/// *tell*: otherwise it carries on into preflight's auto-fixes, `adb forward
/// --remove` and the bottle's `wineserver -k`, taking down the session the
/// classification just refused to touch.
#[tokio::test]
async fn every_busy_but_our_own_in_flight_record_is_a_refusal() {
    let dir = scratch("busy-refusal");
    let (ctx, _seen) = test_ctx(&dir, true);

    // Our own launch, mid-flight: nothing is wrong, nothing to refuse.
    let mine = pending(Some(dead()), None);
    write_state(&ctx, &mine);
    let out = reconcile_with(
        &ctx,
        None,
        Some(crate::session::RunPhaseInfo {
            phase: crate::session::SessionPhase::Preflight,
            run_id: mine.run_id,
            bottle: "Steam".into(),
            exit_code: None,
        }),
        probing(BLACKHOLE),
    )
    .await
    .unwrap();
    assert!(matches!(out, Reconciled::Busy { silent: true, .. }));
    assert_eq!(out.busy_refusal(), None);

    // Another live front-end's record: a refusal, carrying the row the
    // user saw.
    let foreign = ForeignProcess::spawn();
    let mut theirs = pending(None, None);
    theirs.set_owner(foreign.pid());
    write_state(&ctx, &theirs);
    let out = reconcile_with(&ctx, None, None, probing(BLACKHOLE))
        .await
        .unwrap();
    assert_eq!(
        out.busy_refusal(),
        Some(owned_elsewhere_row(foreign.pid()).as_str())
    );

    // A newer schema: likewise.
    let mut newer = pending(Some(dead()), None);
    newer.version = state::SESSION_STATE_VERSION + 1;
    write_state(&ctx, &newer);
    let out = reconcile_with(&ctx, None, None, probing(BLACKHOLE))
        .await
        .unwrap();
    assert_eq!(
        out.busy_refusal(),
        Some(newer_schema_row(newer.version).as_str())
    );

    // And the outcomes a launch may carry on over say nothing.
    assert_eq!(Reconciled::NoSession.busy_refusal(), None);
    assert_eq!(
        (Reconciled::Dead {
            state: mine.clone(),
            restored: Vec::new(),
            pending: false,
        })
        .busy_refusal(),
        None
    );

    assert!(ctx.executor.planned().is_empty(), "nothing was touched");
    std::fs::remove_dir_all(&dir).ok();
}

/// A9-4. `adb forward --remove` that comes back non-zero is
/// indeterminate — usually the device is gone and took the forward with
/// it, but it may be a still-installed `tcp:9943` that will silently break
/// the next WiFi discovery. Flagging the guard released on that clears the
/// record and leaves nothing on the machine that knows the port is there.
#[tokio::test]
async fn a_forward_that_could_not_be_removed_keeps_the_record() {
    let dir = scratch("forward-stuck");
    let (mut ctx, seen) = test_ctx(&dir, true);
    // Only the 9943 removal fails; 9944's succeeds.
    ctx.executor = FailSwitchTo::around(ctx.executor.clone(), "tcp:9943");
    let mut state = pending(Some(dead()), None);
    state.prev_audio_output = None;
    write_state(&ctx, &state);

    let out = reconcile_with(&ctx, None, None, no_probe()).await.unwrap();
    let Reconciled::Dead {
        state,
        restored,
        pending,
    } = out
    else {
        panic!("expected Dead, got {out:?}");
    };

    assert!(pending, "the record is kept, so the caller is told");
    assert!(
        !state.guards.forwards_cleared,
        "one removal did not take, so the guard is not released"
    );
    assert_eq!(
        state
            .wired_forwards
            .iter()
            .map(|f| f.port)
            .collect::<Vec<_>>(),
        vec![9943],
        "the port that is still installed stays on the record; the removed one goes"
    );
    assert_eq!(
        restored,
        vec!["would clear adb forward tcp:9944 on 1WMHH000X00000".to_string()],
        "only what really happened is reported"
    );
    assert_eq!(
        rows(&seen).last().cloned(),
        Some((Severity::Info, RECORD_KEPT.to_string()))
    );

    let planned = ctx.executor.planned();
    assert!(
        !planned.iter().any(|p| p.kind == PlannedKind::RemoveFile),
        "the record is what the next stop reads: {planned:?}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A9-4. The removals are not atomic with the record that describes them:
/// a crash (or a Cancel, or a power loss) between two `adb forward
/// --remove`s must leave a record naming only what is *still* installed. A
/// record that keeps an already-removed port can never be completed,
/// because the retry's `--remove` of an absent listener exits non-zero and
/// this module reads that as "still installed".
#[tokio::test]
async fn a_removal_that_took_is_on_disk_before_the_next_one_is_tried() {
    let dir = scratch("forward-crash-resume");
    let (ctx, _seen) = test_ctx(&dir, false);
    let record = ctx.paths.session_state_path();
    let snapshot = dir.join("record-as-the-second-removal-saw-it.json");

    // A stub `adb` that fails the *second* removal — and, before failing,
    // copies the record exactly as it stands at that moment. That copy is
    // what a crash right there would have left behind.
    let adb = ctx
        .paths
        .adb
        .clone()
        .expect("test_ctx points adb at the fixture");
    std::fs::create_dir_all(adb.parent().unwrap()).unwrap();
    std::fs::write(
        &adb,
        format!(
            "#!/bin/sh\ncase \"$*\" in\n  *9944*) cp '{}' '{}'; exit 1;;\nesac\nexit 0\n",
            record.display(),
            snapshot.display()
        ),
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&adb, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let mut state = pending(Some(dead()), None);
    state.prev_audio_output = None;
    write_state(&ctx, &state);

    let out = reconcile_with(&ctx, None, None, no_probe()).await.unwrap();
    let Reconciled::Dead { pending, .. } = out else {
        panic!("expected Dead, got {out:?}");
    };
    assert!(pending, "9944 is still installed, so the record is kept");

    let mid = state::load(&snapshot)
        .unwrap()
        .expect("the stub copied the record");
    assert_eq!(
        mid.wired_forwards
            .iter()
            .map(|f| f.port)
            .collect::<Vec<_>>(),
        vec![9944],
        "a crash after the 9943 removal must not leave 9943 on the record"
    );
    assert!(!mid.guards.forwards_cleared);

    let after = state::load(&record).unwrap().expect("the record is kept");
    assert_eq!(
        after
            .wired_forwards
            .iter()
            .map(|f| f.port)
            .collect::<Vec<_>>(),
        vec![9944]
    );
    assert!(!after.guards.forwards_cleared);

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_record_this_process_still_supervises_is_reported_but_never_touched() {
    let dir = scratch("owned");
    let (ctx, seen) = test_ctx(&dir, true);
    let state = pending(Some(dead()), Some(me()));
    write_state(&ctx, &state);

    let out = reconcile_with(&ctx, Some(state.run_id), None, probing(BLACKHOLE))
        .await
        .unwrap();
    assert_eq!(
        out,
        Reconciled::Dead {
            state,
            restored: Vec::new(),
            pending: false,
        }
    );
    assert!(
        ctx.executor.planned().is_empty(),
        "the supervise loop owns it"
    );
    assert!(seen.lock().unwrap().is_empty());
    assert!(ctx.paths.session_state_path().exists());
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_corrupt_record_warns_instead_of_blocking_the_next_launch() {
    let dir = scratch("corrupt");
    let (ctx, seen) = test_ctx(&dir, true);
    let path = ctx.paths.session_state_path();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"{not json").unwrap();

    let out = reconcile_with(&ctx, None, None, no_probe()).await.unwrap();
    assert_eq!(out, Reconciled::NoSession);

    let seen_rows = rows(&seen);
    assert_eq!(seen_rows[0].0, Severity::Warn);
    assert!(seen_rows[0]
        .1
        .starts_with("previous session state unreadable: "));
    assert_eq!(seen_rows[1].0, Severity::Info);
    assert_eq!(
        seen_rows[1].1,
        format!("delete {} to clear this warning", path.display())
    );
    assert!(path.exists(), "a record we could not read is not deleted");
    std::fs::remove_dir_all(&dir).ok();
}

/// The disconnected-device defect: the recorded output (AirPods) is gone,
/// so the switch to it fails. Without a fallback the Mac is left on
/// BlackHole — silent — with the record cleared as if it had been restored;
/// the built-in speakers take over instead.
#[tokio::test]
async fn a_recorded_device_that_is_gone_falls_back_to_the_built_in_output() {
    let dir = scratch("dead-audio-fallback");
    let (mut ctx, seen) = test_ctx(&dir, true);
    ctx.executor = FailSwitchTo::around(ctx.executor.clone(), AIRPODS);
    let mut state = pending(Some(dead()), None);
    state.prev_audio_output = Some(AIRPODS.into());
    state.wired_forwards.clear();
    write_state(&ctx, &state);

    let out = reconcile_with(&ctx, None, None, probing_list(BLACKHOLE, &LIVE_OUTPUTS))
        .await
        .unwrap();
    let Reconciled::Dead {
        state,
        restored,
        pending,
    } = out
    else {
        panic!("expected Dead, got {out:?}");
    };
    assert!(!pending, "the fallback IS a restore, so the record goes");
    let expected = format!(
        "recorded output device '{AIRPODS}' is not connected — would restore output -> \
             MacBook Pro Speakers instead (previous session did not shut down cleanly)"
    );
    assert_eq!(restored, vec![expected.clone()]);
    assert!(
        state.guards.audio_restored,
        "landing somewhere audible IS a restore"
    );
    // A warn, not an ok: this is not the device the user had.
    assert_eq!(rows(&seen), vec![(Severity::Warn, expected)]);
    assert_eq!(sections(&seen), vec![RECONCILE_SECTION], "it is a recovery");

    let planned = ctx.executor.planned();
    assert_eq!(
        spawns(&planned),
        vec![
            format!("/fixture/SwitchAudioSource -t output -s {AIRPODS}"),
            "/fixture/SwitchAudioSource -t output -s MacBook Pro Speakers".to_string(),
        ],
        "the recorded device is always tried first"
    );
    assert!(
        planned.iter().any(|p| p.kind == PlannedKind::RemoveFile),
        "every guard is done, so the record goes: {planned:?}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// Nothing audible to fall back to: warn with the unrestorable-device row,
/// keep the record so the next launch or stop can try again, and re-save it
/// with the audio guard still pending.
#[tokio::test]
async fn with_nothing_to_fall_back_to_the_record_is_kept_with_the_remedy() {
    let dir = scratch("dead-audio-stuck");
    let (mut ctx, seen) = test_ctx(&dir, true);
    ctx.executor = FailSwitchTo::around(ctx.executor.clone(), AIRPODS);
    let mut state = pending(Some(dead()), None);
    state.prev_audio_output = Some(AIRPODS.into());
    state.wired_forwards.clear();
    write_state(&ctx, &state);

    let out = reconcile_with(
        &ctx,
        None,
        None,
        // Only the loopback and the streaming virtuals are connected.
        probing_list(
            BLACKHOLE,
            &[
                "BlackHole 2ch",
                "Virtual Desktop Mic",
                "Virtual Desktop Speakers",
            ],
        ),
    )
    .await
    .unwrap();
    let Reconciled::Dead {
        state,
        restored,
        pending,
    } = out
    else {
        panic!("expected Dead, got {out:?}");
    };
    assert!(
        pending,
        "a kept record must be distinguishable from a finished one: the launch that \
             follows has to know `prev_audio_output` is still owed"
    );
    assert!(restored.is_empty(), "nothing was restored");
    assert!(!state.guards.audio_restored, "the guard stays pending");
    assert_eq!(
        rows(&seen),
        vec![
            (
                Severity::Warn,
                crate::session::audio_unrestorable_line(AIRPODS)
            ),
            (Severity::Info, RECORD_KEPT.to_string()),
        ]
    );
    assert!(
        sections(&seen).is_empty(),
        "no recovery happened, so no banner"
    );

    let planned = ctx.executor.planned();
    assert_eq!(
        spawns(&planned),
        vec![format!("/fixture/SwitchAudioSource -t output -s {AIRPODS}")],
        "no fallback was attempted: every device on offer is virtual"
    );
    assert!(
        !planned.iter().any(|p| p.kind == PlannedKind::RemoveFile),
        "the record is what a later restore reads: {planned:?}"
    );
    assert!(
        planned.iter().any(|p| p.kind == PlannedKind::Write),
        "…and it is re-saved, pending guard and all: {planned:?}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// The same audio failure through `stop`, the other entry point into
/// `restore_with`.
#[tokio::test]
async fn stop_keeps_the_record_when_the_audio_could_not_be_restored() {
    let dir = scratch("stop-audio-stuck");
    let (mut ctx, seen) = stop_ctx(&dir, true);
    ctx.executor = FailSwitchTo::around(ctx.executor.clone(), AIRPODS);
    let mut state = pending(Some(dead()), None);
    state.prev_audio_output = Some(AIRPODS.into());
    state.wired_forwards.clear();
    write_state(&ctx, &state);

    finish_stopped_session_with(
        &ctx,
        None,
        None,
        probing_list(BLACKHOLE, &["BlackHole 2ch"]),
    )
    .await
    .expect("a stuck audio guard is a row, never a failed stage");

    assert_eq!(
        rows(&seen),
        vec![
            (
                Severity::Warn,
                crate::session::audio_unrestorable_line(AIRPODS)
            ),
            (Severity::Info, RECORD_KEPT.to_string()),
        ]
    );
    let planned = ctx.executor.planned();
    assert!(
        !planned.iter().any(|p| p.kind == PlannedKind::RemoveFile),
        "the next stop must still know what to restore: {planned:?}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// The other half of the keep-the-record rule: a guard that was never taken
/// counts as done, so a record with nothing in it is still cleared.
#[tokio::test]
async fn a_record_with_no_guards_at_all_is_still_cleared_in_silence() {
    let dir = scratch("dead-inert");
    let (ctx, seen) = test_ctx(&dir, true);
    let mut state = pending(Some(dead()), None);
    state.prev_audio_output = None;
    state.wired_forwards.clear();
    write_state(&ctx, &state);

    let out = reconcile_with(&ctx, None, None, no_probe()).await.unwrap();
    let Reconciled::Dead { restored, .. } = out else {
        panic!("expected Dead, got {out:?}");
    };
    assert!(restored.is_empty());
    assert!(seen.lock().unwrap().is_empty(), "nothing to announce");
    assert_eq!(
        ctx.executor
            .planned()
            .iter()
            .map(|p| p.kind)
            .collect::<Vec<_>>(),
        vec![PlannedKind::RemoveFile],
        "an inert guard is a released guard"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_missing_switchaudiosource_leaves_the_audio_guard_pending() {
    let dir = scratch("no-tool");
    let (ctx, _seen) = test_ctx(&dir, true);
    let mut state = pending(Some(dead()), None);
    state.wired_forwards.clear();

    let restored = restore_with(&ctx, &mut state, RestoreMode::Full, no_probe())
        .await
        .unwrap();
    assert!(restored.is_empty());
    assert!(
        !state.guards.audio_restored,
        "'could not look' is not 'nothing to do'"
    );
    assert!(ctx.executor.planned().is_empty());
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn an_already_released_record_restores_nothing_twice() {
    let dir = scratch("idempotent");
    let (ctx, seen) = test_ctx(&dir, true);
    let mut state = pending(Some(dead()), Some(me()));
    state.guards.audio_restored = true;
    state.guards.dashboard_closed = true;
    state.guards.forwards_cleared = true;

    let restored = restore_with(&ctx, &mut state, RestoreMode::Full, probing(BLACKHOLE))
        .await
        .unwrap();
    assert!(restored.is_empty());
    assert!(ctx.executor.planned().is_empty());
    assert!(sections(&seen).is_empty());
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_dead_dashboard_identity_is_flagged_without_a_signal() {
    let dir = scratch("dead-dashboard");
    let (ctx, _seen) = test_ctx(&dir, true);
    let mut state = pending(Some(dead()), Some(dead()));
    state.prev_audio_output = None;
    state.wired_forwards.clear();

    let restored = restore_with(&ctx, &mut state, RestoreMode::Full, no_probe())
        .await
        .unwrap();
    assert!(
        restored.is_empty(),
        "nothing was closed, so nothing is reported"
    );
    assert!(state.guards.dashboard_closed);
    assert!(spawns(&ctx.executor.planned()).is_empty());
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn without_adb_the_forwards_stay_pending() {
    let dir = scratch("no-adb");
    let (mut ctx, _seen) = test_ctx(&dir, true);
    ctx.paths.adb = None;
    let mut state = pending(Some(dead()), None);
    state.prev_audio_output = None;

    let restored = restore_with(&ctx, &mut state, RestoreMode::Full, no_probe())
        .await
        .unwrap();
    assert!(restored.is_empty());
    assert!(!state.guards.forwards_cleared);
    assert!(ctx.executor.planned().is_empty());
    std::fs::remove_dir_all(&dir).ok();
}

/// The one `restore_with` test with a real executor. It is deliberately shaped so that no
/// child can be spawned: the only pending guard is the audio device, and
/// the probe reports it is already back — so the whole run is a flag flip
/// plus one atomic write into the fixture directory.
#[tokio::test]
async fn each_flag_reaches_disk_as_its_guard_is_released() {
    let dir = scratch("persist");
    let (ctx, seen) = test_ctx(&dir, false);
    assert!(!ctx.executor.is_dry_run());

    let mut state = pending(Some(dead()), None);
    state.wired_forwards.clear();
    write_state(&ctx, &state);

    let restored = restore_with(
        &ctx,
        &mut state,
        RestoreMode::Full,
        probing("MacBook Pro Speakers"),
    )
    .await
    .unwrap();
    assert!(
        restored.is_empty(),
        "nothing was performed, so nothing is reported"
    );
    assert!(sections(&seen).is_empty(), "no banner for a silent pass");

    let path = ctx.paths.session_state_path();
    let on_disk = state::load(&path).unwrap().expect("record still present");
    assert!(
        on_disk.guards.audio_restored,
        "the flag was saved, not just set"
    );
    assert_eq!(on_disk.run_id, state.run_id);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_row_texts_are_stable_in_both_verbs() {
    assert_eq!(
        audio_row(false, "MacBook Pro Speakers"),
        "audio: restored output -> MacBook Pro Speakers (previous session did not shut down \
             cleanly)"
    );
    assert_eq!(
        audio_row(true, "MacBook Pro Speakers"),
        "audio: would restore output -> MacBook Pro Speakers (previous session did not shut \
             down cleanly)"
    );
    assert_eq!(
        audio_fallback_row(false, AIRPODS, "MacBook Pro Speakers"),
        format!(
            "recorded output device '{AIRPODS}' is not connected — restored output -> \
                 MacBook Pro Speakers instead (previous session did not shut down cleanly)"
        )
    );
    assert_eq!(
        audio_fallback_row(true, AIRPODS, "Built-in Output"),
        format!(
            "recorded output device '{AIRPODS}' is not connected — would restore output -> \
                 Built-in Output instead (previous session did not shut down cleanly)"
        )
    );
    assert_eq!(
        dashboard_row(false),
        "ALVR dashboard closed (left over from the previous session)"
    );
    assert_eq!(
        dashboard_row(true),
        "ALVR dashboard would be closed (left over from the previous session)"
    );
    assert_eq!(
        forward_row(false, 9943, "1WMHH000X00000"),
        "cleared adb forward tcp:9943 on 1WMHH000X00000"
    );
    assert_eq!(
        forward_row(true, 9944, "1WMHH000X00000"),
        "would clear adb forward tcp:9944 on 1WMHH000X00000"
    );
    assert_eq!(RECONCILE_SECTION, "reconciling the previous session");
    assert_eq!(RECONCILE_FAILED, "previous session not fully restored");
    assert_eq!(
        RECONCILE_RETRY_HINT,
        "the record is kept; stop again to retry"
    );
    assert_eq!(
        RECORD_KEPT,
        "previous session record kept for a later restore"
    );
    assert_eq!(STEP, "session.reconcile");
}

// These camelCase wire shapes are what sabrage/ui/src/ipc.ts mirrors.

#[test]
fn the_reconcile_types_serialize_camel_case() {
    assert_eq!(
        serde_json::to_value(Classification::IdentityMismatch).unwrap(),
        "identityMismatch"
    );
    assert_eq!(
        serde_json::to_value(RestoreMode::SafeOnly).unwrap(),
        "safeOnly"
    );
    assert_eq!(
        serde_json::to_value(Reconciled::NoSession).unwrap(),
        serde_json::json!({ "kind": "noSession" })
    );
    let ev = Reconciled::Dead {
        state: pending(Some(dead()), None),
        restored: vec!["audio: restored output -> X".into()],
        pending: true,
    };
    let j = serde_json::to_value(&ev).unwrap();
    assert_eq!(j["kind"], "dead");
    assert_eq!(j["pending"], true);
    assert_eq!(j["state"]["prevAudioOutput"], "MacBook Pro Speakers");
}

fn stop_ctx(dir: &Path, dry_run: bool) -> (StageCtx, Arc<StdMutex<Vec<StageEvent>>>) {
    let (mut ctx, seen) = test_ctx(dir, dry_run);
    ctx.bottle = Some(Bottle::unvalidated("Steam"));
    (ctx, seen)
}

/// `stop` names a surviving wine pid in its own words — one sentence per
/// live classification — and keeps the record either way.
#[tokio::test]
async fn stop_names_a_surviving_wine_pid_and_keeps_the_record() {
    let pid = std::process::id();
    let cases: &[(&str, ProcInfo, String)] = &[
        (
            "live",
            me(),
            format!("previous session state kept: wine pid {pid} still alive"),
        ),
        (
            "unverifiable",
            ProcInfo {
                start_time: 0,
                ..me()
            },
            format!(
                "previous session state kept: wine pid {pid} is alive but could not be identified"
            ),
        ),
    ];
    for (i, (label, wine, warning)) in cases.iter().enumerate() {
        let dir = scratch(&format!("stop-alive-{i}"));
        let (ctx, seen) = stop_ctx(&dir, true);
        write_state(&ctx, &pending(Some(wine.clone()), None));

        finish_stopped_session_with(&ctx, None, None, probing(BLACKHOLE))
            .await
            .unwrap();

        assert_eq!(
            rows(&seen),
            vec![(Severity::Warn, warning.clone())],
            "{label}"
        );
        assert!(
            ctx.executor.planned().is_empty(),
            "{label}: nothing restored while it lives"
        );
        assert!(
            ctx.paths.session_state_path().exists(),
            "{label}: the record survives"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}

#[tokio::test]
async fn stop_restores_and_clears_a_session_it_did_not_start() {
    let dir = scratch("stop-dead");
    let (ctx, seen) = stop_ctx(&dir, true);
    let mut state = pending(Some(dead()), None);
    state.wired_forwards.clear();
    write_state(&ctx, &state);

    finish_stopped_session_with(&ctx, None, None, probing(BLACKHOLE))
        .await
        .unwrap();

    assert_eq!(
        rows(&seen),
        vec![(
            Severity::Ok,
            "audio: would restore output -> MacBook Pro Speakers (previous session did not \
                 shut down cleanly)"
                .to_string()
        )]
    );
    assert_eq!(sections(&seen), vec![RECONCILE_SECTION]);
    let kinds: Vec<PlannedKind> = ctx.executor.planned().iter().map(|p| p.kind).collect();
    assert_eq!(
        kinds,
        vec![
            PlannedKind::Spawn,
            PlannedKind::CreateDir,
            PlannedKind::Write,
            PlannedKind::RemoveFile,
        ],
        "switch, save the flag, then clear the record"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn stop_ignores_a_record_belonging_to_another_bottle() {
    let dir = scratch("stop-other-bottle");
    let (ctx, seen) = stop_ctx(&dir, true);
    let mut state = pending(Some(dead()), None);
    state.bottle = "SomeOtherBottle".into();
    write_state(&ctx, &state);

    finish_stopped_session_with(&ctx, None, None, probing(BLACKHOLE))
        .await
        .unwrap();

    assert!(seen.lock().unwrap().is_empty());
    assert!(ctx.executor.planned().is_empty());
    assert!(
        ctx.paths.session_state_path().exists(),
        "another bottle's record is not this stop's to clear"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn stop_leaves_a_session_this_process_supervises_to_its_own_teardown() {
    let dir = scratch("stop-owned");
    let (ctx, seen) = stop_ctx(&dir, true);
    let state = pending(Some(dead()), Some(me()));
    write_state(&ctx, &state);

    finish_stopped_session_with(&ctx, Some(state.run_id), None, probing(BLACKHOLE))
        .await
        .unwrap();

    assert!(seen.lock().unwrap().is_empty());
    assert!(ctx.executor.planned().is_empty());
    assert!(ctx.paths.session_state_path().exists());
    std::fs::remove_dir_all(&dir).ok();
}

/// Finding #6. A reconcile mutation that genuinely fails — here the
/// recorded `SwitchAudioSource` is gone, so the switch cannot even be
/// spawned — must be **reported**, not propagated: `stop` has rows left to
/// print (ports, audio) and `stop.sh` has no step that can end the script.
#[tokio::test]
async fn a_failed_restore_is_reported_and_the_record_is_kept_for_the_next_stop() {
    let dir = scratch("stop-restore-failed");
    // A real executor, deliberately shaped so the only child it can reach is
    // a path that does not exist: the spawn fails, nothing runs.
    let (ctx, seen) = stop_ctx(&dir, false);
    assert!(!ctx.executor.is_dry_run());
    let mut state = pending(Some(dead()), None);
    state.wired_forwards.clear();
    write_state(&ctx, &state);

    finish_stopped_session_with(&ctx, None, None, probing_a_vanished_binary(&dir))
        .await
        .expect("a failed restore must not fail the caller");

    let seen_rows = rows(&seen);
    assert_eq!(seen_rows.len(), 2, "{seen_rows:?}");
    assert_eq!(seen_rows[0].0, Severity::Warn);
    let detail = seen_rows[0]
        .1
        .strip_prefix("previous session not fully restored: ")
        .unwrap_or_else(|| panic!("the warn must carry the prefix: {:?}", seen_rows[0].1));
    assert!(
        !detail.is_empty(),
        "the underlying error is appended after the prefix: {:?}",
        seen_rows[0].1
    );
    assert_eq!(
        seen_rows[1],
        (Severity::Info, RECONCILE_RETRY_HINT.to_string())
    );
    assert!(
        sections(&seen).is_empty(),
        "nothing was restored to announce"
    );

    let on_disk = state::load(&ctx.paths.session_state_path())
        .unwrap()
        .expect("the record survives so the next stop can retry");
    assert!(
        !on_disk.guards.audio_restored,
        "the guard that could not be released stays pending"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// The one error the policy above must **not** absorb: a Cancel during
/// `stop` has to reach the stage and become exit 130.
#[tokio::test]
async fn a_cancelled_reconcile_still_reaches_the_caller() {
    let dir = scratch("stop-cancelled");
    let (ctx, seen) = stop_ctx(&dir, true);

    let err = tolerate_reconcile_failure(&ctx, Err(SabrageError::Cancelled)).unwrap_err();
    assert!(matches!(err, SabrageError::Cancelled));
    assert!(
        seen.lock().unwrap().is_empty(),
        "cancellation is not a partial-restore report"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn stop_without_a_record_is_a_silent_noop() {
    let dir = scratch("stop-none");
    let (ctx, seen) = stop_ctx(&dir, true);
    finish_stopped_session_with(&ctx, None, None, no_probe())
        .await
        .unwrap();
    assert!(seen.lock().unwrap().is_empty());
    assert!(ctx.executor.planned().is_empty());
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn detach_fires_only_the_detach_token_and_marks_the_record() {
    let dir = scratch("detach");
    let (ctx, _seen) = test_ctx(&dir, false);
    let state = pending(Some(me()), Some(me()));
    write_state(&ctx, &state);

    let handle = LiveSessionHandle {
        run_id: state.run_id,
        bottle: state.bottle.clone(),
        identity: me(),
        log_path: state.log_path.clone(),
        started_at_unix_ms: state.started_at_unix_ms,
        cancel: CancellationToken::new(),
        detach: CancellationToken::new(),
    };

    detach(&ctx.paths, &handle).await.unwrap();

    assert!(handle.detach.is_cancelled());
    assert!(
        !handle.cancel.is_cancelled(),
        "detaching must never trigger the teardown path"
    );
    let on_disk = state::load(&ctx.paths.session_state_path())
        .unwrap()
        .expect("the record survives a detach");
    assert!(on_disk.detached);
    assert_eq!(on_disk.guards, state.guards, "guards are left in place");
    assert_eq!(
        on_disk.prev_audio_output.as_deref(),
        Some("MacBook Pro Speakers"),
        "the device stays on BlackHole, and the record still says what it was"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A9-9. Stop is terminal. Both tokens feed one unbiased `select!` in the
/// supervisor, so a detach fired after a Stop can win that race, disarm the
/// guards and leave wine running — while the Stop caller watches the live
/// slot empty and reports success to the user.
#[tokio::test]
async fn detach_does_nothing_once_stop_has_already_fired() {
    let dir = scratch("detach-after-stop");
    let (ctx, _seen) = test_ctx(&dir, false);
    let state = pending(Some(me()), Some(me()));
    write_state(&ctx, &state);

    let handle = LiveSessionHandle {
        run_id: state.run_id,
        bottle: state.bottle.clone(),
        identity: me(),
        log_path: state.log_path.clone(),
        started_at_unix_ms: state.started_at_unix_ms,
        cancel: CancellationToken::new(),
        detach: CancellationToken::new(),
    };
    handle.cancel.cancel(); // Stop got there first.

    detach(&ctx.paths, &handle).await.unwrap();

    assert!(
        !handle.detach.is_cancelled(),
        "a Stop that has fired cannot be superseded by a detach"
    );
    let on_disk = state::load(&ctx.paths.session_state_path())
        .unwrap()
        .expect("the record is the teardown\u{2019}s to clear, not ours to rewrite");
    assert!(
        !on_disk.detached,
        "the record must not claim a detach that did not happen"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// A9-9. Stop can also win *during* detach's wait: it fires the terminal
/// token and then releases the live slot, and its teardown legitimately
/// **keeps** the record when a guard could not be released (a disconnected
/// output device). Writing `detached: true` over that record makes the app
/// tell the user a session it had just stopped "detached instead of
/// stopping — it is still running, unsupervised".
#[tokio::test]
async fn detach_does_not_relabel_a_session_stopped_during_the_wait() {
    let dir = scratch("detach-stop-wins-the-race");
    let (ctx, _seen) = test_ctx(&dir, false);
    let state = pending(Some(me()), Some(me()));
    write_state(&ctx, &state);

    let handle = LiveSessionHandle {
        run_id: state.run_id,
        bottle: state.bottle.clone(),
        identity: me(),
        log_path: state.log_path.clone(),
        started_at_unix_ms: state.started_at_unix_ms,
        cancel: CancellationToken::new(),
        detach: CancellationToken::new(),
    };

    // The supervisor holds the slot for the first few polls; then a Stop
    // fires its terminal token and the slot empties — exactly that order,
    // which is what the supervisor's teardown does.
    let polls = std::sync::atomic::AtomicU32::new(0);
    let cancel = handle.cancel.clone();
    detach_with(&ctx.paths, &handle, Duration::from_secs(5), |_| {
        let n = polls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if n < 3 {
            return true;
        }
        cancel.cancel();
        false
    })
    .await
    .unwrap();

    let on_disk = state::load(&ctx.paths.session_state_path())
        .unwrap()
        .expect("the teardown kept the record");
    assert!(
        !on_disk.detached,
        "a session the Stop path tore down must not read as detached"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// The other half: the wait can simply run out. A supervisor that still
/// holds the live slot is still writing to that record, so the safety net
/// must not fire — its whole justification is that the supervisor has
/// already let go.
#[tokio::test]
async fn detach_that_times_out_leaves_the_record_alone() {
    let dir = scratch("detach-timeout");
    let (ctx, _seen) = test_ctx(&dir, false);
    let state = pending(Some(me()), Some(me()));
    write_state(&ctx, &state);
    let path = ctx.paths.session_state_path();
    let before = std::fs::read(&path).unwrap();

    let handle = LiveSessionHandle {
        run_id: state.run_id,
        bottle: state.bottle.clone(),
        identity: me(),
        log_path: state.log_path.clone(),
        started_at_unix_ms: state.started_at_unix_ms,
        cancel: CancellationToken::new(),
        detach: CancellationToken::new(),
    };

    // Never released: the supervisor is wedged, or slower than the wait.
    detach_with(&ctx.paths, &handle, Duration::from_millis(120), |_| true)
        .await
        .unwrap();

    assert!(handle.detach.is_cancelled(), "the token still fires");
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "a timed-out wait writes nothing at all"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn detach_creates_nothing_when_the_supervisor_already_cleared_the_record() {
    let dir = scratch("detach-cleared");
    let (ctx, _seen) = test_ctx(&dir, false);
    std::fs::create_dir_all(ctx.paths.sabrage_appsup.clone()).unwrap();

    let handle = LiveSessionHandle {
        run_id: Uuid::new_v4(),
        bottle: "Steam".into(),
        identity: me(),
        log_path: PathBuf::from("/repo/logs/x.log"),
        started_at_unix_ms: 1,
        cancel: CancellationToken::new(),
        detach: CancellationToken::new(),
    };

    detach(&ctx.paths, &handle).await.unwrap();
    assert!(handle.detach.is_cancelled());
    assert!(!ctx.paths.session_state_path().exists());
    std::fs::remove_dir_all(&dir).ok();
}
