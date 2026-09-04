//! The two guarded launch actions — audio routing and the ALVR dashboard —
//! the only mutations `run` undoes. Reference: scripts/demo/run.sh
//! (`launch-action: audio-route`, `launch-action: dashboard`); everything the
//! script does before them is permanent (parity decision 17).
//!
//! Each guard persists its record into [`SessionState`] around its mutation —
//! the previous audio device before the switch, the dashboard's identity
//! immediately after its spawn (see [`crate::session::state`]). `release` undoes the mutation, sets the
//! flag and saves; `disarm` forgets without undoing (detach — `session-state.json`
//! still describes the device and dashboard for a later Sabrage); `Drop` is a
//! synchronous best-effort fallback for panics and early returns, inert once
//! released, disarmed, or under `--dry-run`.

use std::future::Future;
use std::path::{Path, PathBuf};

use crate::error::{Result, SabrageError};
use crate::events::{step, RunId, StageEvent};
use crate::executor::DetachedStdio;
use crate::paths::which;
use crate::process::{self, ProcInfo};
use crate::session::state::{self, SessionState};
use crate::stages::{EventSink, StageCtx};

use super::actions::BLACKHOLE_DEVICE;
use super::PreflightFacts;

/// Which of run.sh's audio branches applies (`launch-action: audio-route`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AudioEligibility {
    /// `--no-audio`: one `info` row, no guard.
    Disabled,
    /// `protocol != "alvr"`, or `SwitchAudioSource` is not installed — run.sh
    /// prints **nothing at all** for either.
    Skip,
    /// Eligible: probe the device list, then switch.
    Probe(PathBuf),
}

/// run.sh's audio if/elif chain as a pure decision
/// (`launch-action: audio-route`).
pub(crate) fn audio_eligibility(
    no_audio: bool,
    protocol: &str,
    switch_bin: Option<PathBuf>,
) -> AudioEligibility {
    if no_audio {
        return AudioEligibility::Disabled;
    }
    match switch_bin {
        Some(bin) if protocol == "alvr" => AudioEligibility::Probe(bin),
        _ => AudioEligibility::Skip,
    }
}

/// `SwitchAudioSource -a -t output | grep -qx "BlackHole 2ch"` — an **exact
/// whole-line** match (`-x`), so a device merely containing the name does not
/// count.
pub(crate) fn blackhole_listed(list_stdout: &str) -> bool {
    list_stdout.lines().any(|l| l == BLACKHOLE_DEVICE)
}

// ── verbatim text (A1-3: `pub` so `sabrage-parity` can pin these against
// `run.sh` by calling the real renderer rather than copying a substring) ────

/// run.sh's `--no-audio` info line, verbatim.
pub const AUDIO_DISABLED_LINE: &str =
    "audio routing disabled (--no-audio) — sound stays on the Mac";

/// run.sh's BlackHole-not-present warn line, verbatim.
pub fn blackhole_not_present_line() -> String {
    format!(
        "{BLACKHOLE_DEVICE} not present (brew install blackhole-2ch + reboot) — audio stays on \
         the Mac"
    )
}

/// run.sh's failed-switch branch, which clears `PREV_AUDIO_OUT` again.
pub fn blackhole_switch_failed_line() -> String {
    format!("could not switch output to {BLACKHOLE_DEVICE} — audio stays on the Mac")
}

/// `launch-action: audio-route` — Reference: scripts/demo/run.sh.
///
/// Routes the Mac's default output to `BlackHole 2ch` for the session and
/// restores it on release. Inert for `--no-audio` (an `info` row), for
/// `protocol != "alvr"` or a missing `SwitchAudioSource` (silent), and for a
/// machine with no `BlackHole 2ch` output device (a `warn`).
pub struct AudioGuard {
    /// The device to restore. `None` = nothing was switched, so `release` is a
    /// no-op (the `PREV_AUDIO_OUT=""` case, including the failed-switch branch
    /// where run.sh explicitly clears it again).
    previous_output: Option<String>,
    /// `SwitchAudioSource`'s resolved path, kept so `release` and `Drop` do not
    /// have to re-`which` it.
    switch_bin: Option<PathBuf>,
    /// Was `previous_output` inherited from an EARLIER session's unfinished
    /// restore rather than read off this machine? Decided in
    /// [`AudioGuard::arm`] and consumed by [`AudioGuard::apply_switch`], which
    /// is the other half of the split those two make.
    carried: bool,
    run_id: RunId,
    sink: EventSink,
    /// A dry run never mutates, so its `Drop` must not either
    /// (tests::a_dry_runs_guard_restores_nothing_when_dropped).
    dry_run: bool,
    released: bool,
    disarmed: bool,
}

/// `audio: default output -> BlackHole 2ch (was: <dev>)`
///
/// `pub` (A1-3), same reason as the block above.
pub fn audio_switched_line(previous: &str) -> String {
    format!("audio: default output -> {BLACKHOLE_DEVICE} (was: {previous})")
}

/// `audio: restored output -> <dev>`
///
/// `pub` (A1-3), same reason as the block above.
pub fn audio_restored_line(previous: &str) -> String {
    format!("audio: restored output -> {previous}")
}

/// `SwitchAudioSource -t output -s <device>` through the executor, as a bool.
///
/// A child that cannot even be spawned counts as a failed switch rather than
/// an error: the teardown's job is to report what the machine is left in, and
/// [`AudioGuard::restore_output`]'s remedy row says it better than a
/// propagated `Err` that would turn a clean quit into exit 1.
/// [`SabrageError::Cancelled`] is the exception — it is never a switch result.
async fn switch_output(ctx: &StageCtx, bin: &Path, device: &str) -> Result<bool> {
    let spec = ctx
        .child(bin.to_path_buf(), step::RUN_TEARDOWN)
        .arg("-t")
        .arg("output")
        .arg("-s")
        .arg(device);
    match ctx.executor.run_child(&spec).await {
        Ok(status) => Ok(status.success()),
        Err(SabrageError::Cancelled) => Err(SabrageError::Cancelled),
        Err(_) => Ok(false),
    }
}

/// `SwitchAudioSource -a -t output` as one device name per line — read-only,
/// hence [`crate::process::capture_with`] rather than the executor (the same
/// exception `AudioGuard::arm`'s two probes take), carrying `ctx.cancel`
/// so a Cancel during teardown does not wait out the probe's full timeout.
///
/// An absent or failing binary yields an empty list, which simply means "no
/// fallback": the caller then prints the remedy.
async fn list_output_devices(ctx: &StageCtx, bin: &Path) -> Vec<String> {
    let spec = ctx
        .child(bin.to_path_buf(), step::RUN_TEARDOWN)
        .arg("-a")
        .arg("-t")
        .arg("output")
        .env_path(process::default_child_path());
    match process::capture_with(&spec, &ctx.cancel, process::DEFAULT_PROBE_TIMEOUT).await {
        Ok(out) if out.status.success() => out
            .stdout
            .lines()
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

impl AudioGuard {
    /// An inert guard: nothing was switched, so nothing will be restored.
    fn inert(ctx: &StageCtx) -> AudioGuard {
        AudioGuard {
            previous_output: None,
            switch_bin: None,
            carried: false,
            run_id: ctx.run_id,
            sink: ctx.sink.clone(),
            dry_run: ctx.executor.is_dry_run(),
            released: false,
            disarmed: false,
        }
    }

    /// Test-only: an *armed* guard — one whose `release` actually restores a
    /// device and says so — built without [`AudioGuard::arm`]'s three
    /// machine probes (`which("SwitchAudioSource")` and two `SwitchAudioSource`
    /// captures). `super`'s teardown-order test needs a guard that speaks on
    /// release; it must not need `SwitchAudioSource` installed to get one.
    #[cfg(test)]
    pub(super) fn armed_for_test(
        ctx: &StageCtx,
        previous: impl Into<String>,
        switch_bin: impl Into<PathBuf>,
    ) -> AudioGuard {
        AudioGuard {
            previous_output: Some(previous.into()),
            switch_bin: Some(switch_bin.into()),
            carried: false,
            run_id: ctx.run_id,
            sink: ctx.sink.clone(),
            dry_run: ctx.executor.is_dry_run(),
            released: false,
            disarmed: false,
        }
    }

    /// Take the guard, persisting `previous_output` into `state` **before**
    /// switching the device — but **without** switching it.
    ///
    /// The split from [`AudioGuard::apply_switch`] lets the caller install the
    /// armed guard in its held set before the one call that can come back
    /// `Cancelled` (A8-3), so a cancelled switch unwinds through the ordinary
    /// teardown — the only path that can set `guards.audio_restored` and save
    /// (tests::a_cancelled_switch_leaves_the_guard_armed_for_the_teardown).
    pub async fn arm(
        ctx: &StageCtx,
        facts: &PreflightFacts,
        state: &mut SessionState,
    ) -> Result<Self> {
        let st = ctx.step(step::RUN_AUDIO);
        let mut guard = AudioGuard::inert(ctx);

        let bin = match audio_eligibility(
            ctx.opts.no_audio,
            &facts.protocol,
            which("SwitchAudioSource"),
        ) {
            AudioEligibility::Disabled => {
                st.info(AUDIO_DISABLED_LINE);
                return Ok(guard);
            }
            AudioEligibility::Skip => return Ok(guard),
            AudioEligibility::Probe(bin) => bin,
        };
        guard.switch_bin = Some(bin.clone());

        // Read-only probes: they bypass the executor and therefore run under a
        // dry run too, so the plan is accurate rather than optimistic.
        let listing = process::capture_with(
            &ctx.child(bin.clone(), step::RUN_AUDIO)
                .arg("-a")
                .arg("-t")
                .arg("output")
                .env_path(process::default_child_path()),
            &ctx.cancel,
            process::DEFAULT_PROBE_TIMEOUT,
        )
        .await?;
        if !blackhole_listed(&listing.stdout) {
            st.warn(blackhole_not_present_line());
            return Ok(guard);
        }

        let current = process::capture_with(
            &ctx.child(bin.clone(), step::RUN_AUDIO)
                .arg("-c")
                .arg("-t")
                .arg("output")
                .env_path(process::default_child_path()),
            &ctx.cancel,
            process::DEFAULT_PROBE_TIMEOUT,
        )
        .await?;
        // `$(…)` capture semantics: trailing newlines stripped, nothing else.
        let reading = crate::util::strip_trailing_newlines(&current.stdout).to_string();

        // A device carried forward from an earlier session's unfinished restore
        // (`stages::run::unfinished_audio_restore`) outranks the current
        // reading: in exactly that case the reading IS `BlackHole 2ch`, and
        // recording it would lose the real device for good. Sabrage-only.
        let carried = state.prev_audio_output.is_some() && !state.guards.audio_restored;
        let previous = match state.prev_audio_output.clone() {
            Some(pending) if carried => pending,
            _ => reading,
        };

        // Write BEFORE the mutation: a crash in the window that follows must
        // still leave a machine-readable record of the device to restore.
        state.prev_audio_output = Some(previous.clone());
        state::save(&*ctx.executor, &ctx.paths.session_state_path(), state).await?;

        // Armed BEFORE the switch: `run_child` can report `Cancelled` for a child
        // that already applied the CoreAudio change, and returning that through `?`
        // here would drop the guard (A8-3; tests::a_cancelled_switch_leaves_the_guard_armed_for_the_teardown).
        guard.previous_output = Some(previous);
        guard.carried = carried;
        Ok(guard)
    }

    /// The mutation [`AudioGuard::arm`] deliberately left undone: switch the
    /// default output to `BlackHole 2ch` and pin the device volume.
    ///
    /// A guard that armed nothing (`--no-audio`, a non-ALVR protocol, no
    /// `SwitchAudioSource`, no BlackHole) switches nothing — this is then a
    /// no-op, and the caller does not have to know which of the four it was.
    ///
    /// Must be called with the guard already installed in the caller's held
    /// set: an `Err` out of here is a guard to release, not one to forget.
    pub async fn apply_switch(&mut self, ctx: &StageCtx, state: &mut SessionState) -> Result<()> {
        let (Some(previous), Some(bin)) = (self.previous_output.clone(), self.switch_bin.clone())
        else {
            return Ok(());
        };
        let st = ctx.step(step::RUN_AUDIO);

        let switch = ctx
            .child(bin, step::RUN_AUDIO)
            .arg("-t")
            .arg("output")
            .arg("-s")
            .arg(BLACKHOLE_DEVICE);
        if ctx.executor.run_child(&switch).await?.success() {
            ctx.emit(StageEvent::text(
                ctx.run_id,
                Some(step::RUN_AUDIO),
                audio_switched_line(&previous),
            ));
            // BlackHole applies the macOS device volume to loopback samples, so
            // anything under 100% reaches the headset attenuated. Volume is
            // per-device (speakers untouched), and failure is swallowed as
            // run.sh's `|| true` does.
            let volume = ctx
                .child("osascript", step::RUN_AUDIO)
                .arg("-e")
                .arg("set volume output volume 100")
                .env_path(process::default_child_path());
            let _ = ctx.executor.run_child(&volume).await;
        } else {
            // run.sh's failed-switch branch: warn, and clear the remembered
            // device again so the teardown restores nothing.
            st.warn(blackhole_switch_failed_line());
            self.previous_output = None;
            // …but never clear a device this run only inherited: that record is
            // an EARLIER session's unfinished restore, and this launch failing
            // to switch is no reason to forget it.
            if !self.carried {
                state.prev_audio_output = None;
                state::save(&*ctx.executor, &ctx.paths.session_state_path(), state).await?;
            }
        }
        Ok(())
    }

    /// Restore the device, set `guards.audio_restored`, and save.
    ///
    /// A guard that never switched writes nothing — no state file for a run
    /// that never touched audio.
    ///
    /// When the recorded device is gone, falls back to the built-in output
    /// ([`crate::session::fallback_output_device`]) and says so, or prints the
    /// remedy and leaves `guards.audio_restored` false so the record survives.
    /// Either outcome is rows only, never a failed stage
    /// (tests::a_recorded_device_that_vanished_falls_back_to_the_built_in_output,
    /// tests::an_unrestorable_device_prints_the_remedy_and_leaves_the_guard_pending).
    pub async fn release(self, ctx: &StageCtx, state: &mut SessionState) -> Result<()> {
        let bin = self.switch_bin.clone();
        self.release_with(ctx, state, move || async move {
            match bin {
                Some(b) => list_output_devices(ctx, &b).await,
                None => Vec::new(),
            }
        })
        .await
    }

    /// [`AudioGuard::release`] with the device-list probe injected, so a test
    /// can exercise the fallback without `SwitchAudioSource` on the machine
    /// (the same shape `session::reconcile` uses for its probe).
    ///
    /// The probe is only called when the recorded switch has already failed —
    /// a normal teardown still runs exactly one child.
    async fn release_with<F, Fut>(
        mut self,
        ctx: &StageCtx,
        state: &mut SessionState,
        outputs: F,
    ) -> Result<()>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Vec<String>>,
    {
        self.released = true;
        let Some(previous) = self.previous_output.take() else {
            return Ok(());
        };
        let restored = match self.switch_bin.clone() {
            Some(bin) => self.restore_output(ctx, &bin, &previous, outputs).await?,
            // Nothing to switch with means nothing was ever switched: there is
            // no pending work for a later process to inherit.
            None => true,
        };
        state.guards.audio_restored = restored;
        state::save(&*ctx.executor, &ctx.paths.session_state_path(), state).await
    }

    /// The recorded device, then the fallback, then the remedy. `true` when the
    /// Mac ended up on something audible.
    async fn restore_output<F, Fut>(
        &self,
        ctx: &StageCtx,
        bin: &Path,
        previous: &str,
        outputs: F,
    ) -> Result<bool>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Vec<String>>,
    {
        // run.sh's `restore_audio` prints only when the switch succeeded.
        if switch_output(ctx, bin, previous).await? {
            ctx.emit(StageEvent::text(
                ctx.run_id,
                Some(step::RUN_TEARDOWN),
                audio_restored_line(previous),
            ));
            return Ok(true);
        }
        let st = ctx.step(step::RUN_TEARDOWN);
        if let Some(alt) = crate::session::fallback_output_device(&outputs().await) {
            if switch_output(ctx, bin, &alt).await? {
                st.warn(crate::session::audio_fallback_line(
                    ctx.executor.is_dry_run(),
                    previous,
                    &alt,
                ));
                return Ok(true);
            }
        }
        st.warn(crate::session::audio_unrestorable_line(previous));
        Ok(false)
    }

    /// Detach: forget the guard **without** restoring. The device stays on
    /// BlackHole on purpose, and `session-state.json` keeps describing it.
    pub fn disarm(mut self) {
        self.disarmed = true;
    }
}

impl Drop for AudioGuard {
    fn drop(&mut self) {
        // Released or disarmed: there is nothing left to undo, and undoing it
        // anyway is exactly the bug detach exists to avoid.
        if self.released || self.disarmed || self.dry_run {
            return;
        }
        // Fallback only — the orchestrator's `release` is the normal path.
        // Synchronous and best-effort by necessity: `Drop` cannot `.await`,
        // and a panic here would abort during unwinding.
        let (Some(previous), Some(bin)) = (self.previous_output.take(), self.switch_bin.as_ref())
        else {
            return;
        };
        let ok = std::process::Command::new(bin)
            .args(["-t", "output", "-s", &previous])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            (self.sink)(StageEvent::text(
                self.run_id,
                Some(step::RUN_TEARDOWN),
                audio_restored_line(&previous),
            ));
        }
    }
}

/// Which of run.sh's dashboard branches applies (`launch-action: dashboard`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DashboardEligibility {
    /// `--no-dashboard`: one `info` row.
    Disabled,
    /// `protocol != "alvr"`: the shell's bare `:` — nothing at all.
    Skip,
    /// The binary is missing or not executable: one `warn` row.
    NotBuilt,
    /// Spawn it.
    Spawn,
}

/// run.sh's dashboard if/elif chain as a pure decision
/// (`launch-action: dashboard`).
pub(crate) fn dashboard_eligibility(
    no_dashboard: bool,
    protocol: &str,
    binary_executable: bool,
) -> DashboardEligibility {
    if no_dashboard {
        DashboardEligibility::Disabled
    } else if protocol != "alvr" {
        DashboardEligibility::Skip
    } else if binary_executable {
        DashboardEligibility::Spawn
    } else {
        DashboardEligibility::NotBuilt
    }
}

/// `[ -x "$ALVR_DASHBOARD_BIN" ]`.
///
/// Duplicates `paths::is_executable`, which is private to its module; both
/// follow PARITY.md § Doctor / checks, "`is_executable` tests mode bits
/// `0o111`, not effective access".
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// `launch-action: dashboard` — Reference: scripts/demo/run.sh.
///
/// Spawns the ALVR dashboard detached with pipes on `/dev/null`
/// ([`crate::executor::DetachedStdio::Null`]) and closes it on release. Inert
/// for `--no-dashboard` (an `info` row), for `protocol != "alvr"` (silent),
/// and for an unbuilt `alvr_dashboard` (a warn). Safe to launch before the
/// game: it polls `127.0.0.1:8082` until the embedded server appears.
///
/// The guard keeps the child's identity, not the child: the child is moved
/// into a task that `wait()`s on it, so a user-closed dashboard is reaped
/// instead of becoming a zombie (`spawn_detached` sets `kill_on_drop(false)`).
pub struct DashboardGuard {
    /// Its identity, mirrored into [`SessionState`] so a later process can
    /// close it by identity rather than by bare pid.
    identity: Option<ProcInfo>,
    run_id: RunId,
    sink: EventSink,
    dry_run: bool,
    released: bool,
    disarmed: bool,
}

// `pub` (A1-3): see this file's audio-line block above for why.
pub const DASHBOARD_OPENING_LINE: &str =
    "dashboard: ALVR server dashboard opening (connects once the game is up)";
pub const DASHBOARD_CLOSED_LINE: &str = "dashboard: closed";
/// run.sh's `--no-dashboard` info line, verbatim.
pub const DASHBOARD_DISABLED_LINE: &str = "ALVR dashboard disabled (--no-dashboard)";
/// run.sh's unbuilt-dashboard warn line, verbatim.
pub const DASHBOARD_NOT_BUILT_LINE: &str =
    "alvr_dashboard not built — ./demo.sh build (continuing without the dashboard)";

impl DashboardGuard {
    fn inert(ctx: &StageCtx) -> DashboardGuard {
        DashboardGuard {
            identity: None,
            run_id: ctx.run_id,
            sink: ctx.sink.clone(),
            dry_run: ctx.executor.is_dry_run(),
            released: false,
            disarmed: false,
        }
    }

    /// Spawn the dashboard (when eligible) and record its identity in `state`
    /// immediately.
    pub async fn acquire(
        ctx: &StageCtx,
        facts: &PreflightFacts,
        state: &mut SessionState,
    ) -> Result<Self> {
        let st = ctx.step(step::RUN_DASHBOARD);
        let mut guard = DashboardGuard::inert(ctx);

        match dashboard_eligibility(
            ctx.opts.no_dashboard,
            &facts.protocol,
            is_executable(&ctx.paths.alvr_dashboard),
        ) {
            DashboardEligibility::Disabled => {
                st.info(DASHBOARD_DISABLED_LINE);
                return Ok(guard);
            }
            // The shell's bare `:` — nothing is printed at all.
            DashboardEligibility::Skip => return Ok(guard),
            DashboardEligibility::NotBuilt => {
                st.warn(DASHBOARD_NOT_BUILT_LINE);
                return Ok(guard);
            }
            DashboardEligibility::Spawn => {}
        }

        let spec = ctx
            .child(ctx.paths.alvr_dashboard.clone(), step::RUN_DASHBOARD)
            .env_path(process::default_child_path());
        if let Some(child) = ctx
            .executor
            .spawn_detached(&spec, DetachedStdio::Null)
            .await?
        {
            guard.identity = Some(child.identity.clone());
            state.dashboard = Some(child.identity.clone());
            state::save(&*ctx.executor, &ctx.paths.session_state_path(), state).await?;
            // Reap it whenever it exits — ours by descent, but never ours to
            // keep alive: `kill_on_drop` is false, so waiting is the only way
            // it stops being a zombie in this process.
            let mut child = child;
            tokio::spawn(async move {
                let _ = child.child.wait().await;
            });
        }
        // A dry run has no child but still says what it would have done — the
        // shell prints this line unconditionally once it reaches the branch.
        ctx.emit(StageEvent::text(
            ctx.run_id,
            Some(step::RUN_DASHBOARD),
            DASHBOARD_OPENING_LINE,
        ));
        Ok(guard)
    }

    /// Close the dashboard, set `guards.dashboard_closed`, and save.
    ///
    /// The kill is guarded by [`ProcInfo::is_same_process`] — pid **and** start
    /// time, where the shell only has `kill -0` — so a recycled pid is never
    /// signalled (tests::release_never_signals_an_identity_that_no_longer_matches).
    pub async fn release(mut self, ctx: &StageCtx, state: &mut SessionState) -> Result<()> {
        self.released = true;
        let Some(identity) = self.identity.take() else {
            return Ok(());
        };
        if identity.is_same_process() {
            let spec = ctx
                .child("/bin/kill", step::RUN_TEARDOWN)
                .arg("-TERM")
                .arg(identity.pid.to_string());
            // run.sh's `stop_dashboard` prints only when the kill succeeded.
            if ctx.executor.run_child(&spec).await?.success() {
                ctx.emit(StageEvent::text(
                    ctx.run_id,
                    Some(step::RUN_TEARDOWN),
                    DASHBOARD_CLOSED_LINE,
                ));
            }
        }
        state.guards.dashboard_closed = true;
        state::save(&*ctx.executor, &ctx.paths.session_state_path(), state).await
    }

    /// Detach: leave the dashboard open on purpose.
    pub fn disarm(mut self) {
        self.disarmed = true;
    }
}

impl Drop for DashboardGuard {
    fn drop(&mut self) {
        if self.released || self.disarmed || self.dry_run {
            return;
        }
        // See `AudioGuard::drop`. Signalling by identity, never by bare pid.
        let Some(identity) = self.identity.take() else {
            return;
        };
        if identity.is_same_process() && process::terminate(identity.pid).is_ok() {
            (self.sink)(StageEvent::text(
                self.run_id,
                Some(step::RUN_TEARDOWN),
                DASHBOARD_CLOSED_LINE,
            ));
        }
    }
}

#[cfg(test)]
mod tests;
