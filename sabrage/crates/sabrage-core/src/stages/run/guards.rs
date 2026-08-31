//! The two guarded launch actions — the only mutations `run` undoes.
//!
//! run.sh installs three traps at lines 179–181, and what they cover is
//! exactly this file:
//!
//! ```zsh
//! trap 'stop_dashboard; stop_helper; restore_audio' EXIT
//! trap 'print ""; print -r -- "-- interrupted: stopping wine"; stop_wine; stop_dashboard; stop_helper; restore_audio; trap - INT;  kill -INT  $$' INT
//! trap 'print -r -- "-- terminated: stopping wine"; stop_wine; stop_dashboard; stop_helper; restore_audio; trap - TERM; kill -TERM $$' TERM
//! ```
//!
//! Everything earlier in the script — the backend fix, the helper restage, the
//! adb forwards, the Goldberg swap — is **permanent** and stays permanent
//! (parity decision 17). Everything here is undone on every exit path.
//!
//! # The lifecycle, and why `Drop` is only a fallback
//!
//! ```text
//! arm(ctx, facts, state)      →  persist first  (apply_switch mutates, never the reverse)
//! release(self, ctx, state)   →  undo, set the flag, save    (the normal path)
//! disarm(self)                →  forget without undoing      (detach only)
//! Drop                        →  best-effort sync fallback, only if neither ran
//! ```
//!
//! The orchestrator always calls the async `release`; `Drop` exists for the
//! panic/early-return path, where an `.await` is impossible. It must therefore
//! stay synchronous and best-effort, and it must do **nothing** once the guard
//! has been released or disarmed (design-core §3.2).
//!
//! `disarm` is what makes detach honest: leaving the session running means
//! leaving the audio device on BlackHole and the dashboard open, deliberately,
//! with `session-state.json` still describing both so a later Sabrage can
//! finish the job.
//!
//! # Persist-before-mutate
//!
//! `acquire` writes [`SessionState`] **before** performing its mutation — the
//! previous audio device is recorded before `SwitchAudioSource -t output -s`
//! runs, the dashboard's identity immediately after its spawn. See
//! [`crate::session::state`]'s header for why the other order leaves an
//! unrecoverable window.
//!
//! # `Drop` bypasses the executor, and must not fire under `--dry-run`
//!
//! The async `release` mutates through [`crate::executor::Executor`], so a dry
//! run plans the undo instead of performing it. `Drop` cannot: it has no
//! `.await` and therefore no access to the trait's boxed futures, so it
//! shells out synchronously. Each guard remembers whether its run was a dry
//! run and does nothing at all in that case — otherwise `--dry-run` would
//! switch the user's audio device back on an early return, which is the exact
//! opposite of what the flag promises.

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

// ── audio ─────────────────────────────────────────────────────────────────────

/// Which of run.sh's four audio branches applies (lines 182–200).
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

/// run.sh:182-184 as a pure decision.
///
/// ```zsh
/// if   [ -n "${WINEVR_NO_AUDIO:-}" ]; then info "…"
/// elif [ "$PROTOCOL" = "alvr" ] && command -v SwitchAudioSource >/dev/null 2>&1; then …
/// fi
/// ```
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

/// run.sh:183.
pub const AUDIO_DISABLED_LINE: &str =
    "audio routing disabled (--no-audio) — sound stays on the Mac";

/// run.sh:198.
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

/// `launch-action: audio-route` — run.sh lines 154–200.
///
/// Skipped entirely (an `info` row, no guard state) when `--no-audio`, when
/// `protocol != "alvr"`, when `SwitchAudioSource` is absent, or when
/// `BlackHole 2ch` is not among the output devices. Otherwise: remember
/// `SwitchAudioSource -c -t output`, switch to `BlackHole 2ch`, and set the
/// device volume to 100 (BlackHole applies the device volume to the loopback
/// samples, so anything less reaches the headset attenuated — and volume is
/// per-device, so the speakers we restore are untouched).
///
/// Emits, verbatim:
/// `audio: default output -> BlackHole 2ch (was: <dev>)` on success,
/// `audio: restored output -> <dev>` on release.
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
    /// A dry run never mutates, so its `Drop` must not either.
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
    /// The split from [`AudioGuard::apply_switch`] is what lets the caller
    /// install the armed guard in its held set *before* the one call that can
    /// come back `Cancelled` (A8-3). Arming it inside `acquire` and returning
    /// the switch's `Cancelled` through `?` dropped the guard on the floor:
    /// `Drop` would then restore the device synchronously and say so, but —
    /// having neither `&mut SessionState` nor an executor — it could not set
    /// `guards.audio_restored` or save, so the teardown kept the record and
    /// reported a guard still pending over a device that was already back.
    /// Everything up to and including the pre-mutation save lives here; the
    /// mutation itself lives there.
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
            // run.sh:183
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
            // run.sh:198
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

        // A device carried forward from an earlier session whose restore never
        // finished (`run` seeds it from the kept record — see
        // `stages::run::unfinished_audio_restore`) outranks the current
        // reading, because in exactly that case the reading IS `BlackHole 2ch`:
        // recording it would pin the loopback as the device to "restore" and
        // lose the real one for good. Sabrage-only — run.sh has no record to
        // carry anything forward from.
        let carried = state.prev_audio_output.is_some() && !state.guards.audio_restored;
        let previous = match state.prev_audio_output.clone() {
            Some(pending) if carried => pending,
            _ => reading,
        };

        // Write BEFORE the mutation: a crash in the window that follows must
        // still leave a machine-readable record of the device to restore.
        state.prev_audio_output = Some(previous.clone());
        state::save(&*ctx.executor, &ctx.paths.session_state_path(), state).await?;

        // Arm the guard BEFORE the switch, not after it. `run_child` can report
        // `Cancelled` for a child that already applied the CoreAudio change
        // (`process::spawn_streamed_inner`'s select has no pre-spawn check and
        // signals a child that may have finished), and returning that through
        // `?` from a function that still owns the guard would drop it — the
        // Mac left on BlackHole with the `Drop` fallback disabled. Armed here
        // and handed back to the caller, the switch's cancellation unwinds
        // through the ordinary teardown instead.
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
            // run.sh:192 — BlackHole applies the macOS device volume to the
            // loopback samples; anything under 100% reaches the headset
            // attenuated. Per-device, so the speakers we restore are untouched.
            // Failure is swallowed exactly as the shell's `|| true` swallows it.
            let volume = ctx
                .child("osascript", step::RUN_AUDIO)
                .arg("-e")
                .arg("set volume output volume 100")
                .env_path(process::default_child_path());
            let _ = ctx.executor.run_child(&volume).await;
        } else {
            // run.sh:194-195 — warn, and clear the remembered device again so
            // the exit trap restores nothing.
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
    /// A guard that never switched writes nothing: `prev_audio_output` is
    /// `None`, so [`SessionState::has_pending_guards`] is already false and a
    /// save would only create a state file for a run that never touched audio.
    ///
    /// When the recorded device is **gone** — Bluetooth headphones that
    /// disconnected mid-session — the switch back exits non-zero and the Mac
    /// is left on `BlackHole 2ch`, i.e. silent. That is not an outcome to
    /// swallow, so this falls back to the built-in output
    /// ([`crate::session::fallback_output_device`]) and says so, or prints the
    /// remedy and leaves `guards.audio_restored` **false** so the record
    /// survives for a later restore. Either way it is rows only: a clean quit's
    /// exit code is wine's, and a device that will not switch cannot change it.
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

// ── dashboard ─────────────────────────────────────────────────────────────────

/// Which of run.sh's four dashboard branches applies (lines 207–217).
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

/// run.sh:207-217 as a pure decision.
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
/// Duplicates `paths::is_executable`, which is private to its module and
/// outside this file's ownership (PARITY.md's `is_executable` note applies to
/// both: mode bits `0o111`, not effective access).
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// `launch-action: dashboard` — run.sh lines 202–217.
///
/// Skipped (an `info` row) for `--no-dashboard`; silent for
/// `protocol != "alvr"`; a warn when `alvr_dashboard` is not built
/// (`./demo.sh build` — continuing without the dashboard). Otherwise spawned
/// detached with both pipes on `/dev/null`
/// ([`crate::executor::DetachedStdio::Null`], the shell's `>/dev/null 2>&1 &`).
/// Launching it before the game is fine: it polls `127.0.0.1:8082` until the
/// embedded server appears.
///
/// Emits `dashboard: ALVR server dashboard opening (connects once the game is
/// up)` on acquire and `dashboard: closed` on release, both verbatim.
///
/// The spawned [`crate::executor::DetachedChild`] is **not** kept in the
/// guard: it is moved into a small task that `wait()`s on it, so a dashboard
/// the user closes themselves is reaped instead of becoming a zombie
/// (`spawn_detached` sets `kill_on_drop(false)`). What the guard keeps is the
/// identity, which is what `release` needs and what survives into
/// `session-state.json`.
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
/// run.sh:208.
pub const DASHBOARD_DISABLED_LINE: &str = "ALVR dashboard disabled (--no-dashboard)";
/// run.sh:216.
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
            // run.sh:208
            DashboardEligibility::Disabled => {
                st.info(DASHBOARD_DISABLED_LINE);
                return Ok(guard);
            }
            // run.sh:209-210 — the bare `:`.
            DashboardEligibility::Skip => return Ok(guard),
            // run.sh:216
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
    /// The kill is guarded by [`ProcInfo::is_same_process`] — pid **and**
    /// start time — where the shell only has `kill -0`. A recycled pid is
    /// never signalled.
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
mod tests {
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

    // ── pure decisions ───────────────────────────────────────────────────────

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

    // ── audio guard ──────────────────────────────────────────────────────────

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
        // Nothing planned, nothing written, nothing to restore.
        assert!(ctx.executor.planned().is_empty());
        guard.release(&ctx, &mut state).await.unwrap();
        assert!(!state.guards.audio_restored, "no guard, no flag, no save");
        assert!(ctx.executor.planned().is_empty());
        assert!(!ctx.paths.session_state_path().exists());
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
        ) -> crate::executor::BoxFuture<'a, Result<Option<crate::executor::DetachedChild>>>
        {
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

    #[tokio::test]
    async fn a_dry_run_guard_drop_restores_nothing() {
        // The Drop fallback shells out directly (no executor), so it must be
        // inert under --dry-run or the flag would be a lie.
        let root = scratch("audio-drop");
        let (ctx, seen) = dry_ctx(&root, StageOptions::default());
        {
            let mut g = AudioGuard::inert(&ctx);
            g.previous_output = Some("MacBook Pro Speakers".into());
            g.switch_bin = Some(PathBuf::from("/nonexistent/SwitchAudioSource"));
            assert!(g.dry_run);
        }
        assert!(seen.lock().unwrap().is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[tokio::test]
    async fn a_released_or_disarmed_guard_drops_silently() {
        let root = scratch("audio-disarm");
        let (ctx, seen) = dry_ctx(&root, StageOptions::default());
        let mut state = fresh_state();

        let mut g = AudioGuard::inert(&ctx);
        g.previous_output = Some("Speakers".into());
        g.dry_run = false; // pretend a real run …
        g.disarm(); // … that detached: the device stays on BlackHole.
        assert!(seen.lock().unwrap().is_empty());

        let mut g = AudioGuard::inert(&ctx);
        g.dry_run = false;
        g.release(&ctx, &mut state).await.unwrap();
        assert!(seen.lock().unwrap().is_empty());
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
        ) -> crate::executor::BoxFuture<'a, Result<Option<crate::executor::DetachedChild>>>
        {
            self.inner.spawn_detached(spec, stdio)
        }
    }

    /// The device of the 2026-08-29 finding: recorded at launch, disconnected
    /// before the session was torn down.
    const AIRPODS: &str = "Yifei\u{2019}s AirPods Pro";

    /// `SwitchAudioSource -a -t output` on that machine, verbatim and in order.
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
        // What `acquire` recorded before it switched: the record and the guard
        // name the same device, which is what makes the pending flag below
        // mean anything to `teardown`.
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

    // ── dashboard guard ──────────────────────────────────────────────────────

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
}
