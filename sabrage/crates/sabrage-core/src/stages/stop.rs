//! `demo.sh stop` — cleanly stop the game and the bottle's wine processes.
//!
//! Reference: `scripts/demo/stop.sh`. One step id
//! ([`step::STOP_WINESERVER`]) covers both the kill and the survivor probe;
//! [`step::STOP_PORTS`], [`step::STOP_REAP`] and [`step::STOP_AUDIO`] are one
//! step each. Between the reaps and the audio row,
//! [`crate::session::reconcile::finish_stopped_session`] restores the previous
//! session's guards; it reports its own failures, so the only error it hands
//! back here is [`SabrageError::Cancelled`].
//!
//! Mutations go through [`crate::stages::StageCtx::child`] +
//! [`crate::executor::Executor::run_child`], so `--dry-run` records them
//! instead of touching a live process. Read-only probes (`lsof`,
//! `SwitchAudioSource`) go through [`crate::process::capture_with`] with
//! `ctx.cancel` and a deadline: a wedged probe must not hold this stage — and
//! with it the process-wide operation lock — indefinitely. Cancellation is
//! checked between every step, so a cancelled `stop` returns
//! [`SabrageError::Cancelled`] rather than `StageFinished { ok: true }`.
//! See tests::{a_pre_cancelled_run_yields_cancelled_and_never_reports_stage_finished_ok,
//! cancellation_during_the_reporting_steps_still_fails_the_stage,
//! a_wedged_lsof_warns_instead_of_reporting_free_ports}.
//!
//! Declared divergences: PARITY.md § Stop, "Each reap (leftover encoder helper,
//! leftover ALVR dashboard)"; PARITY.md § Declared by the 2026-08-30
//! adversarial review (round 1 fixes), "**Stop reports probe failures.**";
//! PARITY.md § Session (detach / reconcile), "A **Dead** or
//! **IdentityMismatch** recorded session".

use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

use crate::error::{Result, SabrageError};
use crate::events::{step, StepId};
use crate::paths::{which, Bottle};
use crate::process::{self, ProcInfo};
use crate::stages::{require_bottle, StageCtx};

/// How long to wait for `wineserver -w` to return before giving up on it —
/// 4 s, never fatal.
///
/// Defined in [`crate::stages`] next to its deliberately distinct sibling
/// [`crate::stages::RUN_WINESERVER_WAIT`] (5 s, fatal); never unify the two.
pub use crate::stages::STOP_WINESERVER_WAIT;

/// The substring `pgrep -f 'Beat Saber.exe'` matches on argv — matched the
/// same way here by [`crate::process::find_processes_by_cmdline`]
/// (PARITY.md § Stop, "The Beat Saber survivor probe scans live processes'").
/// Also `format_survivors`'s fallback text when a survivor's exe path has no
/// file name.
const BEAT_SABER_EXE_SUFFIX: &str = "Beat Saber.exe";

/// The exact `lsof` invocation stop.sh (and doctor.sh's `net.ports`) use.
const LSOF_ARGS: [&str; 3] = ["-nP", "-iUDP:9944", "-iTCP:9943"];

/// The staged encoder helper's file name — what a helper from *another*
/// checkout still calls itself. See [`report_foreign_helpers`].
const HELPER_BASENAME: &str = "oxrsys-encoder-helper";

/// stop.sh's `reap_stray "$OXR_HELPER_BIN" … "no leftover encoder helper"`
/// not-found text, emitted by [`report_foreign_helpers`] once the wider scan
/// has confirmed it.
const NO_LEFTOVER_HELPER: &str = "no leftover encoder helper";

/// How long `reap` waits for a signalled process to actually exit before
/// reporting it as a survivor, and how often it re-checks. Deliberately short:
/// this is a report, not a hard guarantee, and `stop` must stay snappy.
const REAP_EXIT_WAIT: std::time::Duration = std::time::Duration::from_millis(1000);
const REAP_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);

/// A reap's three "there was one" rows: the confirmed kill, the process that
/// outlived SIGTERM (plus the surviving `pid name` pairs), and the `--dry-run`
/// row that claims only a plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReapMsg {
    killed: &'static str,
    survived: &'static str,
    would: &'static str,
}

/// stop.sh's helper `reap_stray` text, plus the two rows the shell has no
/// counterpart for.
const HELPER_REAP_MSG: ReapMsg = ReapMsg {
    killed: "encoder helper killed (the runtime owns it — this one outlived its game)",
    survived: "encoder helper still running after SIGTERM",
    would: "would terminate the leftover encoder helper",
};

/// stop.sh's dashboard `reap_stray` text, same shape.
const DASHBOARD_REAP_MSG: ReapMsg = ReapMsg {
    killed: "ALVR dashboard closed (left over from a run that died uncleanly)",
    survived: "ALVR dashboard still running after SIGTERM",
    would: "would close the leftover ALVR dashboard",
};

/// Execute the stage.
pub async fn run(ctx: &StageCtx) -> Result<()> {
    let bottle = require_bottle(ctx)?;
    ctx.section(format!(
        "stopping wineserver for bottle '{}' (takes the game with it)",
        bottle.name
    ));

    stop_wine(ctx, bottle).await?;
    checkpoint(ctx)?;
    // One scan for every probe below (survivors, both reaps, the foreign-helper
    // fallback) instead of one fresh full-table walk each — see
    // `process::ProcessScan`.
    let scan = process::ProcessScan::scan();
    report_survivors(ctx, scan.by_cmdline(BEAT_SABER_EXE_SUFFIX));
    checkpoint(ctx)?;
    report_ports(ctx).await;
    checkpoint(ctx)?;
    // The helper's not-found row is emitted by `report_foreign_helpers` rather
    // than by `reap`: "no leftover encoder helper" may only be said after
    // looking beyond *this* checkout's staged path.
    let helper_matched = reap(
        ctx,
        scan.by_exe(&ctx.paths.oxr_helper_staged),
        step::STOP_REAP,
        Some(HELPER_REAP_MSG),
        None,
    )
    .await?;
    // Unconditional and report-only: a helper staged under another checkout is
    // invisible to the exact-path reap above, so gating this scan on a local
    // match reopens A5-2 (tests::a_foreign_helper_is_reported_whatever_the_local_reap_did).
    report_foreign_helpers(
        ctx,
        &ctx.paths.root,
        scan.by_cmdline(HELPER_BASENAME),
        helper_matched,
    );
    checkpoint(ctx)?;
    reap(
        ctx,
        scan.by_exe(&ctx.paths.alvr_dashboard),
        step::STOP_REAP,
        Some(DASHBOARD_REAP_MSG),
        None,
    )
    .await?;
    checkpoint(ctx)?;
    // Sabrage-only: finish the *previous* session's teardown (restore the audio
    // device, close its dashboard, drop its `--wired` forwards) now that its
    // wine process is gone, so the audio row below reports the restored device.
    // `?` here only ever propagates cancellation — see the module docs.
    crate::session::reconcile::finish_stopped_session(ctx).await?;
    checkpoint(ctx)?;
    report_audio(ctx).await;
    checkpoint(ctx)?;

    Ok(())
}

/// `Err(`[`SabrageError::Cancelled`]`)` the moment `ctx.cancel` has fired.
///
/// Called between every step of [`run`], not only around the mutating
/// children: the reporting steps and the early returns in `stop_wine` and
/// `reap` reach no check of their own, so these calls are what make a
/// cancelled stop fail instead of reporting `StageFinished { ok: true }`.
/// See tests::cancellation_during_the_reporting_steps_still_fails_the_stage.
fn checkpoint(ctx: &StageCtx) -> Result<()> {
    if ctx.cancel.is_cancelled() {
        return Err(SabrageError::Cancelled);
    }
    Ok(())
}

/// `wineserver -k`, then a bounded `wineserver -w`, per `lib.sh`'s `stop_wine`.
///
/// Emits nothing when `ctx.paths.wineserver` is `None`: lib.sh swallows the
/// command-not-found failure, so neither side prints anything.
///
/// # Errors
///
/// [`SabrageError::Cancelled`] if `ctx.cancel` fires during either child. A
/// plain non-zero or failed child is swallowed, matching the shell's `|| true`.
/// See tests::{dry_run_stop_wine_plans_the_wineserver_pair_only_when_crossover_is_present,
/// stop_wine_propagates_a_pre_cancelled_token_instead_of_swallowing_it}.
///
/// The `-w` bound is a [`tokio::time::timeout`]: dropping the timed-out future
/// ends the child via [`crate::process::spawn_streamed`]'s `kill_on_drop(true)`.
/// Nothing tests it (a timing property); breaking it leaks a `wineserver -w`
/// per stop.
async fn stop_wine(ctx: &StageCtx, bottle: &Bottle) -> Result<()> {
    let Some(wineserver) = ctx.paths.wineserver.as_ref() else {
        return Ok(());
    };
    let prefix = bottle.prefix.to_string_lossy().into_owned();

    let kill = ctx
        .child(wineserver.clone(), step::STOP_WINESERVER)
        .arg("-k")
        .env("WINEPREFIX", prefix.clone());
    let _ = ctx.executor.run_child(&kill).await;
    checkpoint(ctx)?;

    let wait = ctx
        .child(wineserver.clone(), step::STOP_WINESERVER)
        .arg("-w")
        .env("WINEPREFIX", prefix);
    let _ = tokio::time::timeout(STOP_WINESERVER_WAIT, ctx.executor.run_child(&wait)).await;
    checkpoint(ctx)
}

/// `"<pid> <exe-basename>"` per survivor, space-joined with a trailing space —
/// the shape of `pgrep -lf 'Beat Saber.exe' | tr '\n' ' '` (one `pid name` pair
/// per line, each line's newline becoming a trailing space).
fn format_survivors(procs: &[ProcInfo]) -> String {
    let mut out = String::new();
    for p in procs {
        let name = p
            .exe
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(BEAT_SABER_EXE_SUFFIX);
        out.push_str(&p.pid.to_string());
        out.push(' ');
        out.push_str(name);
        out.push(' ');
    }
    out
}

/// `warn` naming the survivors when any Beat Saber process is still up, else
/// `ok "game and wineserver down"` — stop.sh's `pgrep -f 'Beat Saber.exe'`
/// branch. See tests::report_survivors_matches_a_direct_probe.
fn report_survivors(ctx: &StageCtx, survivors: Vec<ProcInfo>) {
    let st = ctx.step(step::STOP_WINESERVER);
    if survivors.is_empty() {
        st.ok("game and wineserver down");
    } else {
        st.warn(format!(
            "Beat Saber processes survived: {}",
            format_survivors(&survivors)
        ));
    }
}

/// Did a probe hit its deadline, as opposed to failing the way a missing
/// binary fails? [`process::capture_with`] reports the deadline as an
/// [`SabrageError::Io`] of kind [`std::io::ErrorKind::TimedOut`].
///
/// Only a deadline may print a Sabrage-only row: every other failure is what
/// stop.sh's `2>/dev/null` folds into an empty `$STALE`/`$CUR`, and must keep
/// producing the shell's row.
/// See tests::a_missing_lsof_still_reports_the_shells_free_ports_row.
fn probe_timed_out(e: &SabrageError) -> bool {
    matches!(e, SabrageError::Io { source, .. } if source.kind() == std::io::ErrorKind::TimedOut)
}

/// `COMMAND(PID)` per listener on the streaming ports, deduplicated, sorted,
/// space-joined with a trailing space — the shape stop.sh's
/// `lsof ... | awk ... | sort -u | tr` pipeline produces. Duplicates
/// `checks::network`'s private equivalent, not reachable from here.
///
/// `Ok(None)` means the probe blew `deadline`. `lsof` and `deadline` are
/// parameters so a test can point at a stub that never answers; production
/// passes `lsof` and [`process::DEFAULT_PROBE_TIMEOUT`].
///
/// # Errors
///
/// [`SabrageError::Cancelled`] only. The probe runs through
/// [`process::capture_with`], which SIGKILLs its process group on the token
/// or the deadline: a wedged `lsof` must not hold this stage — and with it
/// the process-wide operation lock — past the reaps and the guard restore.
/// See tests::{stale_listeners_is_well_formed_cmd_pid_pairs_with_a_trailing_space,
/// a_wedged_lsof_warns_instead_of_reporting_free_ports}.
async fn stale_listeners(
    ctx: &StageCtx,
    lsof: &Path,
    deadline: Duration,
) -> Result<Option<String>> {
    let spec = ctx
        .child(lsof.as_os_str().to_os_string(), step::STOP_PORTS)
        .args(LSOF_ARGS)
        .env_path(process::default_child_path());
    let text = match process::capture_with(&spec, &ctx.cancel, deadline).await {
        Ok(out) => out.stdout,
        Err(SabrageError::Cancelled) => return Err(SabrageError::Cancelled),
        Err(e) if probe_timed_out(&e) => return Ok(None),
        // Everything the shell's `2>/dev/null` swallows into an empty `$STALE`.
        Err(_) => String::new(),
    };
    let mut rows: BTreeSet<String> = BTreeSet::new();
    for line in text.lines().skip(1) {
        let mut fields = line.split_whitespace();
        if let (Some(cmd), Some(pid)) = (fields.next(), fields.next()) {
            rows.insert(format!("{cmd}({pid})"));
        }
    }
    let mut out = String::new();
    for row in rows {
        out.push_str(&row);
        out.push(' ');
    }
    Ok(Some(out))
}

/// Sabrage-only (see [`probe_timed_out`]): the row that replaces
/// `ok "streaming ports free"` when the probe never answered. "Free" is a claim
/// about the machine, and an abandoned probe is no evidence for it.
fn ports_unreadable_warn(deadline: Duration) -> String {
    format!(
        "could not read the streaming ports: lsof did not answer within {:.0}s — check them with: lsof {}",
        deadline.as_secs_f32(),
        LSOF_ARGS.join(" ")
    )
}

/// `warn "streaming ports still held by: <stale>"` when `stale_listeners`
/// found any, else `ok "streaming ports free"` — stop.sh's `$STALE` branch. A
/// probe that blew its deadline gets `ports_unreadable_warn` instead; a
/// cancelled one emits nothing.
async fn report_ports(ctx: &StageCtx) {
    report_ports_with(ctx, Path::new("lsof"), process::DEFAULT_PROBE_TIMEOUT).await
}

/// [`report_ports`] with the probe's program and deadline injectable, so a test
/// can exercise the abandoned-probe row against a stub that never answers
/// without waiting out [`process::DEFAULT_PROBE_TIMEOUT`].
async fn report_ports_with(ctx: &StageCtx, lsof: &Path, deadline: Duration) {
    let st = ctx.step(step::STOP_PORTS);
    match stale_listeners(ctx, lsof, deadline).await {
        // Cancelled: emit nothing. [`run`]'s `checkpoint` on the very next line
        // turns the same token into the stage's exit code 130 — this step has
        // nothing truthful left to say.
        Err(_) => {}
        Ok(None) => st.warn(ports_unreadable_warn(deadline)),
        Ok(Some(stale)) if stale.is_empty() => st.ok("streaming ports free"),
        Ok(Some(stale)) => st.warn(format!("streaming ports still held by: {stale}")),
    }
}

/// One `/bin/kill -TERM <pid>` child per already-scanned match, routed through
/// the executor — `lib.sh`'s `reap_stray` specialised to exact-exe-path
/// matching (PARITY.md § Stop, "Each reap (leftover encoder helper, leftover
/// ALVR dashboard)"). The message fires at most once regardless of match
/// count, matching the shell's single `ok` (PARITY.md § Stop, "Each reap sends
/// `/bin/kill -TERM <pid>` once per matched process"); `procs` is the caller's
/// scan, this function never walks the process table itself.
///
/// Returns whether anything was signalled (under `--dry-run`: whether anything
/// matched), so a caller can own the not-found case — the helper's
/// cross-checkout scan does.
///
/// A pid whose `(pid, start_time)` identity no longer matches is not
/// signalled, and the `killed` row is emitted only once every signalled
/// identity is really gone (`wait_for_exit`, bounded by `REAP_EXIT_WAIT`);
/// a process that outlives SIGTERM gets a `warn` naming it instead. Under
/// `--dry-run` nothing is signalled and the row swaps to [`ReapMsg::would`].
/// See tests::{dry_run_reap_plans_a_kill_per_match_and_reports_once,
/// a_real_reap_reports_the_kill_only_once_the_process_is_really_gone,
/// a_term_ignoring_process_gets_a_warn_row_not_a_green_killed_row,
/// reap_never_signals_a_pid_whose_identity_no_longer_matches}.
///
/// # Errors
///
/// [`SabrageError::Cancelled`] the moment `ctx.cancel` fires after any one
/// kill, skipping the remaining kills and the closing message.
async fn reap(
    ctx: &StageCtx,
    procs: Vec<ProcInfo>,
    step_id: StepId,
    found_msg: Option<ReapMsg>,
    not_found_msg: Option<&str>,
) -> Result<bool> {
    if procs.is_empty() {
        if let Some(msg) = not_found_msg {
            ctx.step(step_id).ok(msg);
        }
        return Ok(false);
    }
    let dry_run = ctx.executor.is_dry_run();
    let mut signalled: Vec<ProcInfo> = Vec::new();
    for p in &procs {
        // A dry run signals nothing, so its plan lists every match; a real run
        // re-checks the identity it is about to signal.
        if !dry_run && !p.is_same_process() {
            continue;
        }
        let spec = ctx
            .child("/bin/kill", step_id)
            .arg("-TERM")
            .arg(p.pid.to_string());
        let _ = ctx.executor.run_child(&spec).await;
        signalled.push(p.clone());
        checkpoint(ctx)?;
    }
    if !dry_run && signalled.is_empty() {
        // Every match's identity changed between the scan and the signal — the
        // process exited on its own (or, in theory, its pid was recycled). This
        // stage signalled nothing, so it claims nothing; the caller treats it
        // as the not-found case it now is.
        return Ok(false);
    }
    let Some(msg) = found_msg else {
        return Ok(true);
    };
    let st = ctx.step(step_id);
    if dry_run {
        st.info(msg.would);
        return Ok(true);
    }
    let alive = wait_for_exit(&signalled).await;
    if alive.is_empty() {
        st.ok(msg.killed);
    } else {
        st.warn(format!("{}: {}", msg.survived, format_survivors(&alive)));
    }
    Ok(true)
}

/// The identities out of `procs` that are *still the same live process* after
/// up to [`REAP_EXIT_WAIT`], polled every [`REAP_POLL_INTERVAL`]. Empty means
/// every signalled process is gone.
///
/// Returns as soon as the last one exits, so the common case costs one poll.
async fn wait_for_exit(procs: &[ProcInfo]) -> Vec<ProcInfo> {
    let deadline = tokio::time::Instant::now() + REAP_EXIT_WAIT;
    loop {
        let alive: Vec<ProcInfo> = procs
            .iter()
            .filter(|p| p.is_same_process())
            .cloned()
            .collect();
        if alive.is_empty() || tokio::time::Instant::now() >= deadline {
            return alive;
        }
        tokio::time::sleep(REAP_POLL_INTERVAL).await;
    }
}

/// Sabrage-only and deliberately report-only: a helper staged under another
/// checkout runs from a different absolute path, so neither `reap`'s
/// exact-path match nor the shell's `pkill -f "$OXR_HELPER_BIN"` can see it.
/// Nothing is signalled — a mutating kill may not rely on an argv match
/// (PARITY.md § Stop, "Each reap (leftover encoder helper, leftover ALVR
/// dashboard)") and another checkout's helper is that checkout's `stop` to run.
///
/// `matches` is the caller's `HELPER_BASENAME` cmdline scan; this function
/// narrows it to processes whose resolved executable is *named*
/// `HELPER_BASENAME` and lies outside `root`, so an editor or a `tail -f`
/// that merely mentions the path cannot match.
///
/// `local_matched` gates nothing but the `NO_LEFTOVER_HELPER` row, which may
/// print only when neither scan found anything — never the scan itself, since
/// a local and a foreign helper coexist routinely in a multi-worktree checkout
/// (A5-2).
/// See tests::{a_foreign_helper_is_reported_whatever_the_local_reap_did,
/// the_not_found_row_prints_only_when_nothing_foreign_and_no_local_match}.
fn report_foreign_helpers(
    ctx: &StageCtx,
    root: &Path,
    matches: Vec<ProcInfo>,
    local_matched: bool,
) {
    let st = ctx.step(step::STOP_REAP);
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let foreign: Vec<ProcInfo> = matches
        .into_iter()
        .filter(|p| {
            if p.exe.file_name().and_then(|n| n.to_str()) != Some(HELPER_BASENAME) {
                return false;
            }
            let exe = p.exe.canonicalize().unwrap_or_else(|_| p.exe.clone());
            !exe.starts_with(&root)
        })
        .collect();
    if foreign.is_empty() {
        if !local_matched {
            st.ok(NO_LEFTOVER_HELPER);
        }
        return;
    }
    for p in foreign {
        st.warn(format!(
            "leftover encoder helper from another checkout: {} {} — stop it from that checkout",
            p.pid,
            p.exe.display()
        ));
    }
}

/// stop.sh's `SwitchAudioSource -c -t output`-then-`BlackHole 2ch`-check
/// branch, factored into a pure function of the (already `$(...)`-trimmed)
/// current output device name, so it is testable without the binary installed.
#[derive(Debug, Clone, PartialEq, Eq)]
enum AudioReport {
    /// `warn` + the restore-hint `info`.
    StillBlackhole,
    /// `ok "audio output: <cur>"`.
    Restored(String),
}

const AUDIO_STILL_BLACKHOLE_WARN: &str =
    "Mac audio output is still BlackHole 2ch (a run that died uncleanly could not restore it)";
const AUDIO_RESTORE_HINT: &str = "restore with: SwitchAudioSource -t output -s '<device>'   \
     (list: SwitchAudioSource -a -t output)";

fn audio_report(cur: &str) -> AudioReport {
    if cur == "BlackHole 2ch" {
        AudioReport::StillBlackhole
    } else {
        AudioReport::Restored(cur.to_string())
    }
}

/// stop.sh's audio branch: `warn` plus the restore hint when the Mac's output
/// is still `BlackHole 2ch`, else `ok "audio output: <cur>"`. A probe that blew
/// its deadline gets `audio_unreadable_warn` instead; a cancelled one emits
/// nothing. Entirely silent — no step, no rows — when `SwitchAudioSource` is
/// not on `PATH`.
/// See tests::audio_branch_text_is_verbatim.
async fn report_audio(ctx: &StageCtx) {
    let Some(bin) = which("SwitchAudioSource") else {
        return;
    };
    report_audio_with(ctx, &bin, process::DEFAULT_PROBE_TIMEOUT).await
}

/// [`report_audio`] past the `command -v` gate, with the probe's binary and
/// deadline injectable — see [`report_ports_with`].
async fn report_audio_with(ctx: &StageCtx, bin: &Path, deadline: Duration) {
    let st = ctx.step(step::STOP_AUDIO);
    match current_output_device(ctx, bin, deadline).await {
        // Cancelled — see [`report_ports`]: `run`'s next `checkpoint` reports it.
        Err(_) => {}
        Ok(None) => st.warn(audio_unreadable_warn(deadline)),
        Ok(Some(cur)) => match audio_report(&cur) {
            AudioReport::StillBlackhole => {
                st.warn(AUDIO_STILL_BLACKHOLE_WARN);
                st.info(AUDIO_RESTORE_HINT);
            }
            AudioReport::Restored(cur) => {
                st.ok(format!("audio output: {cur}"));
            }
        },
    }
}

/// `CUR="$(SwitchAudioSource -c -t output 2>/dev/null)"`, trimmed the way `$()`
/// trims — bounded and cancellation-aware for the same reason
/// [`stale_listeners`] is (a `SwitchAudioSource` blocked on a degraded
/// CoreAudio server is the audio-side twin of a wedged `lsof`), and with the
/// same tri-state: `Ok(None)` only for a probe that blew `deadline`, `Err` only
/// for cancellation, and every other failure folded into the empty `$CUR` the
/// shell's `2>/dev/null` produces.
async fn current_output_device(
    ctx: &StageCtx,
    bin: &Path,
    deadline: Duration,
) -> Result<Option<String>> {
    let spec = ctx
        .child(bin.as_os_str().to_os_string(), step::STOP_AUDIO)
        .args(["-c", "-t", "output"])
        .env_path(process::default_child_path());
    match process::capture_with(&spec, &ctx.cancel, deadline).await {
        Ok(out) => Ok(Some(
            crate::util::strip_trailing_newlines(&out.stdout).to_string(),
        )),
        Err(SabrageError::Cancelled) => Err(SabrageError::Cancelled),
        Err(e) if probe_timed_out(&e) => Ok(None),
        Err(_) => Ok(Some(String::new())),
    }
}

/// Sabrage-only, `ports_unreadable_warn`'s twin: a probe that never answered
/// must not print `ok "audio output: "` naming no device at all.
/// See tests::a_wedged_switchaudiosource_warns_instead_of_naming_an_empty_device.
fn audio_unreadable_warn(deadline: Duration) -> String {
    format!(
        "could not read the current audio output device: SwitchAudioSource did not answer within {:.0}s (check it with: SwitchAudioSource -c -t output)",
        deadline.as_secs_f32()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::Severity;
    use crate::executor::PlannedKind;
    use crate::paths::Paths;
    use crate::stages::{StageCtx, StageOptions};
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex as StdMutex};
    use tokio_util::sync::CancellationToken;

    /// The production probe: real `lsof`, real deadline.
    async fn probe_ports(ctx: &StageCtx) -> Option<String> {
        stale_listeners(ctx, Path::new("lsof"), process::DEFAULT_PROBE_TIMEOUT)
            .await
            .expect("not cancelled")
    }

    #[tokio::test]
    async fn stale_listeners_is_well_formed_cmd_pid_pairs_with_a_trailing_space() {
        // Ground truth is machine state (paths.rs's own testing pattern): assert
        // the shape invariant rather than a fixed value.
        let (ctx, _seen) = test_ctx(StageOptions::default());
        let Some(stale) = probe_ports(&ctx).await else {
            return; // lsof did not answer on this machine; covered below
        };
        if stale.is_empty() {
            return;
        }
        assert!(stale.ends_with(' '), "{stale:?} missing trailing space");
        for token in stale.trim_end().split(' ') {
            assert!(
                token.ends_with(')') && token.contains('('),
                "{token:?} is not CMD(PID)"
            );
        }
    }

    #[tokio::test]
    async fn report_ports_matches_a_direct_probe() {
        let (ctx, seen) = test_ctx(StageOptions::default());
        report_ports(&ctx).await;
        let Some(stale) = probe_ports(&ctx).await else {
            return; // lsof did not answer on this machine; covered below
        };
        let evs = seen.lock().unwrap().clone();
        let line = evs.last().expect("one row emitted");
        match (line, stale.is_empty()) {
            (
                crate::events::StageEvent::Line {
                    severity: Severity::Ok,
                    text,
                    step,
                    ..
                },
                true,
            ) => {
                assert_eq!(text, "streaming ports free");
                assert_eq!(step.as_deref(), Some(step::STOP_PORTS));
            }
            (
                crate::events::StageEvent::Line {
                    severity: Severity::Warn,
                    text,
                    ..
                },
                false,
            ) => {
                assert_eq!(text, &format!("streaming ports still held by: {stale}"));
            }
            (other, empty) => panic!("unexpected row {other:?} (stale empty: {empty})"),
        }
    }

    /// An executable that never answers within any test's budget, at a unique
    /// scratch path. Returns `(dir, bin)`; the caller removes `dir`.
    fn never_answers(tag: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "sabrage-stop-probe-{tag}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("wedged-probe.sh");
        std::fs::write(&bin, "#!/bin/sh\nsleep 300\n").unwrap();
        std::fs::set_permissions(&bin, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        (dir, bin)
    }

    /// A budget far below [`process::DEFAULT_PROBE_TIMEOUT`] but far above the
    /// cost of spawning and killing one `/bin/sh`.
    const PROBE_TEST_BUDGET: Duration = Duration::from_secs(5);
    const PROBE_TEST_DEADLINE: Duration = Duration::from_millis(300);

    /// A cancelled token stops the `lsof` probe instead of holding `stop` — and
    /// the process-wide operation lock — until the wedged probe answers.
    #[tokio::test]
    async fn stale_listeners_honors_an_already_cancelled_token() {
        let (dir, bin) = never_answers("ports-cancel");
        let (ctx, _seen) = test_ctx(StageOptions::default());
        ctx.cancel.cancel();

        let started = tokio::time::Instant::now();
        let err = stale_listeners(&ctx, &bin, process::DEFAULT_PROBE_TIMEOUT)
            .await
            .expect_err("a cancelled probe must not answer with a port list");
        assert!(matches!(err, SabrageError::Cancelled), "{err:?}");
        assert!(started.elapsed() < PROBE_TEST_BUDGET, "the probe ran on");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A cancelled ports probe emits no row at all, green or otherwise.
    #[tokio::test]
    async fn a_cancelled_ports_probe_emits_no_row_at_all() {
        let (dir, bin) = never_answers("ports-cancel-row");
        let (ctx, seen) = test_ctx(StageOptions::default());
        ctx.cancel.cancel();

        report_ports_with(&ctx, &bin, process::DEFAULT_PROBE_TIMEOUT).await;
        assert!(
            rows(&seen).is_empty(),
            "a cancelled probe spoke anyway: {:?}",
            rows(&seen)
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// r2:A5-5 regression: a probe that blows its deadline warns instead of
    /// claiming free ports.
    #[tokio::test]
    async fn a_wedged_lsof_warns_instead_of_reporting_free_ports() {
        let (dir, bin) = never_answers("ports-deadline");
        let (ctx, seen) = test_ctx(StageOptions::default());

        let started = tokio::time::Instant::now();
        report_ports_with(&ctx, &bin, PROBE_TEST_DEADLINE).await;
        assert!(started.elapsed() < PROBE_TEST_BUDGET, "the probe ran on");

        assert_eq!(
            rows(&seen),
            vec![(Severity::Warn, ports_unreadable_warn(PROBE_TEST_DEADLINE))],
            "claimed free ports on the strength of a probe that never answered"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The audio twin: `SwitchAudioSource` blocked on a degraded CoreAudio
    /// server used to fall through `unwrap_or_default()` into the green
    /// `"audio output: "` row, naming no device at all.
    ///
    /// r2:A5-5 regression: a probe past its deadline warns instead of naming an
    /// empty current device.
    #[tokio::test]
    async fn a_wedged_switchaudiosource_warns_instead_of_naming_an_empty_device() {
        let (dir, bin) = never_answers("audio-deadline");
        let (ctx, seen) = test_ctx(StageOptions::default());

        let started = tokio::time::Instant::now();
        report_audio_with(&ctx, &bin, PROBE_TEST_DEADLINE).await;
        assert!(started.elapsed() < PROBE_TEST_BUDGET, "the probe ran on");

        assert_eq!(
            rows(&seen),
            vec![(Severity::Warn, audio_unreadable_warn(PROBE_TEST_DEADLINE))],
            "a probe that never answered named the current device"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn current_output_device_honors_an_already_cancelled_token() {
        let (dir, bin) = never_answers("audio-cancel");
        let (ctx, seen) = test_ctx(StageOptions::default());
        ctx.cancel.cancel();

        let started = tokio::time::Instant::now();
        let err = current_output_device(&ctx, &bin, process::DEFAULT_PROBE_TIMEOUT)
            .await
            .expect_err("a cancelled probe must not answer with a device name");
        assert!(matches!(err, SabrageError::Cancelled), "{err:?}");
        assert!(started.elapsed() < PROBE_TEST_BUDGET, "the probe ran on");

        report_audio_with(&ctx, &bin, process::DEFAULT_PROBE_TIMEOUT).await;
        assert!(rows(&seen).is_empty(), "{:?}", rows(&seen));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A missing binary is *not* a wedged one: stop.sh's `2>/dev/null` folds a
    /// command-not-found into an empty `$STALE`, so the shell's green row still
    /// prints. Only the deadline (which the shell cannot reach) gets the warn.
    #[tokio::test]
    async fn a_missing_lsof_still_reports_the_shells_free_ports_row() {
        let (ctx, seen) = test_ctx(StageOptions::default());
        report_ports_with(
            &ctx,
            Path::new("/nonexistent/sabrage/lsof"),
            process::DEFAULT_PROBE_TIMEOUT,
        )
        .await;
        assert_eq!(
            rows(&seen),
            vec![(Severity::Ok, "streaming ports free".to_string())]
        );
    }

    /// (label, (pid, exe path) per survivor, expected line).
    type SurvivorCase<'a> = (&'a str, &'a [(u32, &'a str)], &'a str);

    #[test]
    fn format_survivors_matches_the_pgrep_lf_shape() {
        let cases: &[SurvivorCase<'_>] = &[
            ("no survivors", &[], ""),
            (
                "two survivors, pgrep -lf shape",
                &[
                    (111, "/repo/ext/oxrsys/build-x64/Beat Saber.exe"),
                    (222, "/other/place/Beat Saber.exe"),
                ],
                "111 Beat Saber.exe 222 Beat Saber.exe ",
            ),
            (
                "path with no file name falls back to the suffix",
                &[(7, "/")],
                "7 Beat Saber.exe ",
            ),
        ];
        for (label, input, expected) in cases {
            let procs: Vec<ProcInfo> = input
                .iter()
                .map(|(pid, exe)| ProcInfo {
                    pid: *pid,
                    start_time: 0,
                    exe: PathBuf::from(*exe),
                })
                .collect();
            assert_eq!(format_survivors(&procs), *expected, "{label}");
        }
    }

    /// Finding #8: survivors are matched by argv, not by exe path.

    #[test]
    fn report_survivors_matches_a_direct_probe() {
        let (ctx, seen) = test_ctx(StageOptions::default());
        let survivors = process::find_processes_by_cmdline(BEAT_SABER_EXE_SUFFIX);
        report_survivors(&ctx, survivors.clone());
        let evs = seen.lock().unwrap().clone();
        let line = evs.last().expect("one row emitted");
        let crate::events::StageEvent::Line {
            severity,
            text,
            step,
            ..
        } = line
        else {
            panic!("expected a Line event, got {line:?}");
        };
        assert_eq!(step.as_deref(), Some(step::STOP_WINESERVER));
        if survivors.is_empty() {
            assert_eq!(*severity, Severity::Ok);
            assert_eq!(text, "game and wineserver down");
        } else {
            assert_eq!(*severity, Severity::Warn);
            assert!(text.starts_with("Beat Saber processes survived: "));
        }
    }

    #[test]
    fn audio_report_branches_on_the_exact_blackhole_name() {
        assert_eq!(audio_report("BlackHole 2ch"), AudioReport::StillBlackhole);
        // Whole-string comparison, not substring: a name that merely contains
        // "BlackHole 2ch" does not count (mirrors `[ "$CUR" = "BlackHole 2ch" ]`,
        // not a grep).
        assert_eq!(
            audio_report("BlackHole 2ch (aggregate)"),
            AudioReport::Restored("BlackHole 2ch (aggregate)".to_string())
        );
        assert_eq!(
            audio_report("MacBook Pro Speakers"),
            AudioReport::Restored("MacBook Pro Speakers".to_string())
        );
        assert_eq!(audio_report(""), AudioReport::Restored(String::new()));
    }

    #[test]
    fn audio_branch_text_is_verbatim() {
        assert_eq!(
            AUDIO_STILL_BLACKHOLE_WARN,
            "Mac audio output is still BlackHole 2ch (a run that died uncleanly could not \
             restore it)"
        );
        assert_eq!(
            AUDIO_RESTORE_HINT,
            "restore with: SwitchAudioSource -t output -s '<device>'   (list: SwitchAudioSource \
             -a -t output)"
        );
    }

    fn test_ctx(opts: StageOptions) -> (StageCtx, Arc<StdMutex<Vec<crate::events::StageEvent>>>) {
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let s = seen.clone();
        let sink: crate::stages::EventSink = Arc::new(move |ev| s.lock().unwrap().push(ev));
        let mut paths = Paths::new("/nonexistent/sabrage-stop-test");
        // Load-bearing: `Paths::new` derives `session-state.json` from the real
        // `$HOME`, so without this override a stop test would read — and with a
        // real executor delete — the developer's own live session record.
        paths.sabrage_appsup = scratch_dir();
        let ctx = StageCtx::new(paths, opts, sink, CancellationToken::new());
        (ctx, seen)
    }

    /// A unique path under the system temp dir. **Not** created: tests that only
    /// need "no session record here" want it absent, and the one test that
    /// writes a record creates it itself.
    fn scratch_dir() -> PathBuf {
        std::env::temp_dir().join(format!(
            "sabrage-stop-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ))
    }

    /// (label, `ctx.paths.wineserver`, the WHOLE recorded plan as (kind, reason)).
    type StopWineCase<'a> = (&'a str, Option<&'a str>, &'a [(PlannedKind, &'a str)]);

    /// Both machines are simulated by overriding `ctx.paths.wineserver`:
    /// `Paths::new` probes for a real CrossOver.app unconditionally, so neither
    /// row may depend on whether this Mac has one.
    #[tokio::test]
    async fn dry_run_stop_wine_plans_the_wineserver_pair_only_when_crossover_is_present() {
        let cases: &[StopWineCase<'_>] = &[
            (
                "wineserver present",
                Some("/nonexistent/sabrage/wineserver"),
                &[
                    (PlannedKind::Spawn, "/nonexistent/sabrage/wineserver -k"),
                    (PlannedKind::Spawn, "/nonexistent/sabrage/wineserver -w"),
                ],
            ),
            ("no wineserver on this machine", None, &[]),
        ];
        for (label, wineserver, expected) in cases {
            let (mut ctx, _seen) = test_ctx(StageOptions {
                dry_run: true,
                ..Default::default()
            });
            ctx.paths.wineserver = wineserver.map(PathBuf::from);
            let bottle = Bottle::unvalidated("SabrageStopTest");

            stop_wine(&ctx, &bottle).await.expect("not cancelled");

            let planned: Vec<(PlannedKind, String)> = ctx
                .executor
                .planned()
                .into_iter()
                .map(|p| (p.kind, p.reason))
                .collect();
            let want: Vec<(PlannedKind, String)> = expected
                .iter()
                .map(|(kind, reason)| (*kind, (*reason).to_string()))
                .collect();
            assert_eq!(planned, want, "{label}");
        }
    }

    /// A [`ReapMsg`] whose three texts are distinguishable at a glance.
    const TEST_REAP_MSG: ReapMsg = ReapMsg {
        killed: "found",
        survived: "survived",
        would: "would find",
    };

    #[tokio::test]
    async fn dry_run_reap_plans_a_kill_per_match_and_reports_once() {
        // find_processes_by_exe on a path nothing runs from: not-found branch.
        let (ctx, seen) = test_ctx(StageOptions {
            dry_run: true,
            ..Default::default()
        });
        let matched = reap(
            &ctx,
            process::find_processes_by_exe(Path::new("/nonexistent/sabrage/helper")),
            step::STOP_REAP,
            Some(TEST_REAP_MSG),
            Some("not found"),
        )
        .await
        .expect("not cancelled");
        assert!(!matched);
        let evs = seen.lock().unwrap().clone();
        assert_eq!(evs.len(), 1);
        assert!(matches!(
            &evs[0],
            crate::events::StageEvent::Line { text, .. } if text == "not found"
        ));
        assert!(ctx.executor.planned().is_empty(), "no kill for no match");
    }

    #[tokio::test]
    async fn dry_run_reap_matches_this_test_binary_by_exact_path() {
        let exe = std::env::current_exe().expect("test binary path");
        let (ctx, seen) = test_ctx(StageOptions {
            dry_run: true,
            ..Default::default()
        });
        let matched = reap(
            &ctx,
            process::find_processes_by_exe(&exe),
            step::STOP_REAP,
            Some(TEST_REAP_MSG),
            Some("not found"),
        )
        .await
        .expect("not cancelled");
        assert!(matched);
        let evs = seen.lock().unwrap().clone();
        assert_eq!(evs.len(), 1);
        // A dry run signalled nothing, so it may not claim a kill: the row is
        // the future-tense variant, at Info.
        assert!(
            matches!(
                &evs[0],
                crate::events::StageEvent::Line { text, severity, .. }
                    if text == TEST_REAP_MSG.would && *severity == Severity::Info
            ),
            "{evs:?}"
        );
        let planned = ctx.executor.planned();
        assert!(
            !planned.is_empty(),
            "this test process itself should match its own exe path"
        );
        assert!(planned
            .iter()
            .all(|p| p.kind == PlannedKind::Spawn && p.reason.contains("/bin/kill -TERM")));
    }

    /// Spawned as a child by the reap tests below, never as part of the suite
    /// (`#[ignore]` plus the env gate). The child is a **copy** of this test
    /// binary placed at a unique temp path named [`HELPER_BASENAME`], so
    /// `find_processes_by_exe` matches exactly that copy and nothing else on
    /// the machine — this harness process included.
    #[test]
    #[ignore = "spawned as a child by the reap tests; not a test of its own"]
    fn sleeper_child() {
        let Ok(secs) = std::env::var("SABRAGE_TEST_SLEEP_SECS") else {
            return;
        };
        if std::env::var("SABRAGE_TEST_IGNORE_TERM").is_ok() {
            use nix::sys::signal::{signal, SigHandler, Signal};
            // SAFETY: installing SIG_IGN in a freshly spawned single-purpose
            // child, before it does anything else.
            unsafe { signal(Signal::SIGTERM, SigHandler::SigIgn) }.expect("ignore SIGTERM");
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                let _ = std::fs::write(dir.join("ready"), b"1");
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(
            secs.parse().unwrap_or(10).min(60),
        ));
    }

    /// A live child at a unique `…/oxrsys-encoder-helper` path. Killed and
    /// removed on drop, whatever the test did.
    struct Sleeper {
        child: std::process::Child,
        dir: PathBuf,
        exe: PathBuf,
    }

    impl Drop for Sleeper {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
            std::fs::remove_dir_all(&self.dir).ok();
        }
    }

    fn spawn_sleeper(ignore_term: bool) -> Sleeper {
        let dir = scratch_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join(HELPER_BASENAME);
        std::fs::copy(std::env::current_exe().unwrap(), &exe).unwrap();

        let mut cmd = std::process::Command::new(&exe);
        cmd.args([
            "--exact",
            "stages::stop::tests::sleeper_child",
            "--ignored",
            "--nocapture",
        ])
        .env("SABRAGE_TEST_SLEEP_SECS", "20")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
        if ignore_term {
            cmd.env("SABRAGE_TEST_IGNORE_TERM", "1");
        }
        let child = cmd.spawn().expect("spawn the copied test binary");
        let sleeper = Sleeper { child, dir, exe };

        // Wait for the child to have installed its disposition and reached the
        // sleep — it writes `ready` next to itself just before.
        let ready = sleeper.dir.join("ready");
        for _ in 0..200 {
            if ready.is_file() && !process::find_processes_by_exe(&sleeper.exe).is_empty() {
                return sleeper;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        panic!("the sleeper child never became ready at {:?}", sleeper.exe);
    }

    fn rows(seen: &Arc<StdMutex<Vec<crate::events::StageEvent>>>) -> Vec<(Severity, String)> {
        seen.lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                crate::events::StageEvent::Line { severity, text, .. } => {
                    Some((*severity, text.clone()))
                }
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn a_real_reap_reports_the_kill_only_once_the_process_is_really_gone() {
        let sleeper = spawn_sleeper(false);
        let (ctx, seen) = test_ctx(StageOptions::default());

        let matched = reap(
            &ctx,
            process::find_processes_by_exe(&sleeper.exe),
            step::STOP_REAP,
            Some(TEST_REAP_MSG),
            None,
        )
        .await
        .expect("not cancelled");

        assert!(matched);
        assert_eq!(rows(&seen), vec![(Severity::Ok, "found".to_string())]);
        // The row is only allowed to exist because the process is gone.
        assert!(
            process::find_processes_by_exe(&sleeper.exe).is_empty(),
            "the killed row printed while the process was still alive"
        );
    }

    #[tokio::test]
    async fn a_term_ignoring_process_gets_a_warn_row_not_a_green_killed_row() {
        let sleeper = spawn_sleeper(true);
        let pid = sleeper.child.id();
        let (ctx, seen) = test_ctx(StageOptions::default());

        reap(
            &ctx,
            process::find_processes_by_exe(&sleeper.exe),
            step::STOP_REAP,
            Some(TEST_REAP_MSG),
            None,
        )
        .await
        .expect("not cancelled");

        let rows = rows(&seen);
        assert_eq!(rows.len(), 1, "{rows:?}");
        let (severity, text) = &rows[0];
        assert_eq!(*severity, Severity::Warn, "{rows:?}");
        assert!(
            text.starts_with("survived: ") && text.contains(&pid.to_string()),
            "{text:?} should name the surviving pid {pid}"
        );
        // Still alive: the whole point of the warn.
        assert!(!process::find_processes_by_exe(&sleeper.exe).is_empty());
    }

    /// A stale identity — same pid, a start time that process never had — must
    /// not be signalled at all: `reap` skips it.
    ///
    /// r1:A5-5 regression: a pid whose start time no longer matches fails
    /// `is_same_process`, so `wait_for_exit` never counts it alive.
    #[tokio::test]
    async fn reap_never_signals_a_pid_whose_identity_no_longer_matches() {
        let mut mismatched = ProcInfo::observe(std::process::id()).expect("observe self");
        mismatched.start_time += 1;
        assert!(!mismatched.is_same_process());
        // Sanity that the guard, not some other branch, is what protects us:
        // this pid *is* alive.
        assert!(ProcInfo::observe(std::process::id()).is_some());
        // Exercised through `wait_for_exit`'s own predicate rather than by
        // asking `reap` to kill this very test process.
        assert!(wait_for_exit(&[mismatched]).await.is_empty());
    }

    /// r1:A5-7 regression: a helper left over in another checkout is reported
    /// whether or not this checkout's own reap matched.
    #[test]
    fn a_foreign_helper_is_reported_whatever_the_local_reap_did() {
        let sleeper = spawn_sleeper(false);
        let pid = sleeper.child.id();
        // (label, local_matched — whether this checkout's own exact-path
        // reap already killed a helper just before the scan)
        let cases: &[(&str, bool)] = &[
            (
                "r1:A5-7 regression: a foreign helper is reported instead of the not-found row",
                false,
            ),
            (
                "r2:A5-2 regression: a local helper match must not suppress the cross-checkout warn",
                true,
            ),
        ];
        for (label, local_matched) in cases {
            let (mut ctx, seen) = test_ctx(StageOptions::default());
            // "Another checkout": this repo root contains neither the sleeper
            // nor anything else.
            ctx.paths.root = PathBuf::from("/nonexistent/sabrage-stop-test");
            report_foreign_helpers(
                &ctx,
                &ctx.paths.root.clone(),
                process::find_processes_by_cmdline(HELPER_BASENAME),
                *local_matched,
            );

            let rows = rows(&seen);
            assert!(
                rows.iter().any(|(sev, t)| *sev == Severity::Warn
                    && t.starts_with("leftover encoder helper from another checkout: ")
                    && t.contains(&pid.to_string())),
                "{label}: {rows:?}"
            );
            assert!(
                !rows.iter().any(|(_, t)| t == NO_LEFTOVER_HELPER),
                "{label}: {rows:?}"
            );
        }
    }

    /// (label, cmdline matches by pid, `local_matched`, expected rows).
    type ForeignHelperCase<'a> = (&'a str, &'a [u32], bool, &'a [(Severity, &'a str)]);

    /// The gate on the shell's not-found row: it prints only when neither
    /// scan found anything, so the killed row and `NO_LEFTOVER_HELPER` can
    /// never both appear — and a cmdline match whose exe is not the helper
    /// binary was never a foreign helper to begin with.
    #[test]
    fn the_not_found_row_prints_only_when_nothing_foreign_and_no_local_match() {
        // A live pid whose executable is NOT named oxrsys-encoder-helper: the
        // basename filter must drop it.
        let not_the_helper = [std::process::id()];
        let cases: &[ForeignHelperCase<'_>] = &[
            (
                "nothing foreign, no local match: the shell's not-found row",
                &[],
                false,
                &[(Severity::Ok, NO_LEFTOVER_HELPER)],
            ),
            (
                "nothing foreign, local reap already reported a kill: no row at all",
                &[],
                true,
                &[],
            ),
            (
                "a cmdline match that is not the helper binary is filtered out",
                &not_the_helper,
                false,
                &[(Severity::Ok, NO_LEFTOVER_HELPER)],
            ),
        ];
        for (label, pids, local_matched, expected) in cases {
            let (mut ctx, seen) = test_ctx(StageOptions::default());
            ctx.paths.root = PathBuf::from("/nonexistent/sabrage-stop-test");
            let matches: Vec<ProcInfo> = pids
                .iter()
                .map(|pid| ProcInfo::observe(*pid).expect("observe a live pid"))
                .collect();
            report_foreign_helpers(&ctx, &ctx.paths.root.clone(), matches, *local_matched);
            let want: Vec<(Severity, String)> = expected
                .iter()
                .map(|(sev, text)| (*sev, (*text).to_string()))
                .collect();
            assert_eq!(rows(&seen), want, "{label}");
        }
    }

    /// Finding #2: cancellation propagates out of the stop helpers instead of
    /// being swallowed.

    #[tokio::test]
    async fn stop_wine_propagates_a_pre_cancelled_token_instead_of_swallowing_it() {
        let (mut ctx, _seen) = test_ctx(StageOptions {
            dry_run: true,
            ..Default::default()
        });
        ctx.paths.wineserver = Some(PathBuf::from("/nonexistent/sabrage/wineserver"));
        ctx.cancel.cancel();
        let bottle = Bottle::unvalidated("SabrageStopTest");

        let err = stop_wine(&ctx, &bottle).await.unwrap_err();
        assert!(matches!(err, SabrageError::Cancelled));
        // Under DryRunExecutor `run_child` never itself errors — the check must
        // come from `ctx.cancel` directly, not from `run_child`'s result.
        assert!(
            !ctx.executor.planned().is_empty(),
            "the -k spawn was still planned before the check"
        );
    }

    #[tokio::test]
    async fn reap_propagates_a_pre_cancelled_token_instead_of_swallowing_it() {
        let exe = std::env::current_exe().expect("test binary path");
        let (ctx, seen) = test_ctx(StageOptions {
            dry_run: true,
            ..Default::default()
        });
        ctx.cancel.cancel();

        let err = reap(
            &ctx,
            process::find_processes_by_exe(&exe),
            step::STOP_REAP,
            Some(TEST_REAP_MSG),
            Some("not found"),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, SabrageError::Cancelled));
        // The closing "found" message must NOT have been emitted — cancellation
        // short-circuits before it.
        assert!(seen.lock().unwrap().is_empty());
    }

    /// Both wineserver shapes, because they exercise different code: with a
    /// `wineserver` path `stop_wine` spawns and hits its own post-`run_child`
    /// check; with `None` it returns `Ok(())` immediately and only [`run`]'s
    /// between-step `checkpoint` can catch the cancellation.
    #[tokio::test]
    async fn a_pre_cancelled_run_yields_cancelled_and_never_reports_stage_finished_ok() {
        for wineserver in [Some(PathBuf::from("/nonexistent/sabrage/wineserver")), None] {
            let (mut ctx, seen) = test_ctx(StageOptions {
                dry_run: true,
                bottle_name: Some("SabrageStopTest".to_string()),
                ..Default::default()
            });
            // Bypass the real `~/Library/.../CrossOver/Bottles` filesystem check —
            // `require_bottle` only needs `ctx.bottle` to be `Some`.
            ctx.bottle = Some(Bottle::unvalidated("SabrageStopTest"));
            ctx.paths.wineserver = wineserver.clone();
            ctx.cancel.cancel();

            let err = crate::stages::run_stage_holding_lock(crate::stages::Stage::Stop, &ctx)
                .await
                .unwrap_err();
            assert!(matches!(err, SabrageError::Cancelled), "{wineserver:?}");

            let evs = seen.lock().unwrap().clone();
            assert!(
                !evs.iter().any(|e| matches!(
                    e,
                    crate::events::StageEvent::StageFinished { ok: true, .. }
                )),
                "a cancelled stop must never report StageFinished{{ok:true}} \
                 (wineserver={wineserver:?}): {evs:?}"
            );
            assert!(
                evs.iter().any(|e| matches!(
                    e,
                    crate::events::StageEvent::StageFinished {
                        ok: false,
                        exit_code_equiv: 130,
                        ..
                    }
                )),
                "expected a failed StageFinished carrying exit code 130 \
                 (wineserver={wineserver:?}): {evs:?}"
            );
        }
    }

    /// A pid no process can have on macOS (`kern.maxproc` is five digits), so
    /// the recorded wine process classifies as `Dead` without a pid-reuse race.
    /// Deliberately not `u32::MAX`, which is `-1` as an `i32`.
    const DEAD_PID: u32 = 2_147_483_646;

    /// Finding #6, at the stage level: the reconcile pass between steps 3 and 4
    /// is *additive*, so a failure inside it is reported rather than aborting the
    /// stage before the audio row — `stop.sh` has no step that can end the script.
    ///
    /// Deterministic and machine-independent: the record carries a `--wired`
    /// forward and `adb` points at a nonexistent path, so `forward --remove`
    /// fails at `spawn` with `ENOENT`. Nothing is spawned, signalled, or written
    /// on the machine.
    #[tokio::test]
    async fn a_failed_reconcile_is_reported_and_the_stage_still_reaches_its_audio_row() {
        use crate::session::state::{SessionState, WiredForward};

        let dir = scratch_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let (mut ctx, seen) = test_ctx(StageOptions {
            dry_run: false,
            bottle_name: Some("SabrageStopTest".to_string()),
            ..Default::default()
        });
        assert!(!ctx.executor.is_dry_run());
        ctx.bottle = Some(Bottle::unvalidated("SabrageStopTest"));
        ctx.paths.sabrage_appsup = dir.clone();
        ctx.paths.wineserver = None;
        ctx.paths.oxr_helper_staged = PathBuf::from("/nonexistent/sabrage/helper");
        ctx.paths.alvr_dashboard = PathBuf::from("/nonexistent/sabrage/dashboard");
        ctx.paths.adb = Some(dir.join("bin/adb"));

        let mut state = SessionState::new(
            uuid::Uuid::new_v4(),
            "SabrageStopTest",
            "/games/Beat Saber 1294",
            "/repo/logs/beatsaber-20260829-101112.log",
            1_786_300_214_181,
        );
        state.wine = Some(ProcInfo {
            pid: DEAD_PID,
            start_time: 1,
            exe: PathBuf::from("/nonexistent/sabrage/wine"),
        });
        state.wired_forwards = vec![WiredForward {
            serial: "1WMHH000X00000".to_string(),
            port: 9943,
        }];
        let path = ctx.paths.session_state_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut bytes = serde_json::to_vec_pretty(&state).unwrap();
        bytes.push(b'\n');
        std::fs::write(&path, bytes).unwrap();

        crate::stages::run_stage_holding_lock(crate::stages::Stage::Stop, &ctx)
            .await
            .expect("a failed reconcile must not fail the stop stage");

        let evs = seen.lock().unwrap().clone();
        assert!(
            evs.iter()
                .any(|e| matches!(e, crate::events::StageEvent::StageFinished { ok: true, .. })),
            "{evs:?}"
        );
        let lines: Vec<(Severity, Option<String>, String)> = evs
            .iter()
            .filter_map(|e| match e {
                crate::events::StageEvent::Line {
                    severity,
                    step,
                    text,
                    ..
                } => Some((*severity, step.clone(), text.clone())),
                _ => None,
            })
            .collect();
        assert!(
            lines.iter().any(|(sev, step, text)| *sev == Severity::Warn
                && step.as_deref() == Some(crate::session::reconcile::STEP)
                && text.starts_with("previous session not fully restored: ")),
            "the failure is reported: {lines:?}"
        );
        // …and the audio row that comes after it still ran.
        let audio_rows = lines
            .iter()
            .filter(|(_, step, _)| step.as_deref() == Some(step::STOP_AUDIO))
            .count();
        if which("SwitchAudioSource").is_some() {
            // One row normally; two when this Mac's output happens to be sitting
            // on BlackHole 2ch (warn + restore hint). Either proves step 4 ran.
            assert!(audio_rows >= 1, "the audio row is the point: {lines:?}");
        } else {
            assert_eq!(audio_rows, 0, "silent without the tool: {lines:?}");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The narrower window finding #3 named: cancellation after the wineserver
    /// kill, in the *reporting* half where no executor child is spawned and
    /// nothing else observes the token. Deterministic: cancel **from the event
    /// sink** the instant the first row (`report_survivors`'s) is emitted —
    /// [`run`]'s next call is its own `checkpoint`.
    #[tokio::test]
    async fn cancellation_during_the_reporting_steps_still_fails_the_stage() {
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let cancel = CancellationToken::new();
        let s = seen.clone();
        let c = cancel.clone();
        let sink: crate::stages::EventSink = Arc::new(move |ev| {
            if matches!(ev, crate::events::StageEvent::Line { .. }) {
                c.cancel();
            }
            s.lock().unwrap().push(ev);
        });
        let mut ctx = StageCtx::new(
            Paths::new("/nonexistent/sabrage-stop-test"),
            StageOptions {
                dry_run: true,
                bottle_name: Some("SabrageStopTest".to_string()),
                ..Default::default()
            },
            sink,
            cancel,
        );
        ctx.bottle = Some(Bottle::unvalidated("SabrageStopTest"));
        // No wineserver and nothing to reap, so every `run_child`-adjacent
        // check is unreachable: only `run`'s between-step checkpoints are left.
        ctx.paths.wineserver = None;
        ctx.paths.oxr_helper_staged = PathBuf::from("/nonexistent/sabrage/helper");
        ctx.paths.alvr_dashboard = PathBuf::from("/nonexistent/sabrage/dashboard");

        let err = crate::stages::run_stage_holding_lock(crate::stages::Stage::Stop, &ctx)
            .await
            .unwrap_err();
        assert!(matches!(err, SabrageError::Cancelled), "{err:?}");

        let evs = seen.lock().unwrap().clone();
        assert!(
            !evs.iter()
                .any(|e| matches!(e, crate::events::StageEvent::StageFinished { ok: true, .. })),
            "{evs:?}"
        );
        // The steps after the cancellation never ran.
        assert!(
            !evs.iter().any(|e| matches!(
                e,
                crate::events::StageEvent::Line { text, .. }
                    if text.starts_with("streaming ports")
            )),
            "report_ports ran past a cancelled token: {evs:?}"
        );
    }
}
