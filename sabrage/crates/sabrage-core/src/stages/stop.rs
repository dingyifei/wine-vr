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
/// The `-w` bound is a [`tokio::time::timeout`]; the child dies only through
/// [`crate::process::spawn_streamed`]'s `kill_on_drop(true)` when the timed-out
/// future drops. Untested (a timing property); breaking it leaks a `wineserver -w` per stop.
///
/// # Errors
///
/// [`SabrageError::Cancelled`] if `ctx.cancel` fires during either child. A
/// plain non-zero or failed child is swallowed, matching the shell's `|| true`.
/// See tests::{dry_run_stop_wine_plans_the_wineserver_pair_only_when_crossover_is_present,
/// stop_wine_propagates_a_pre_cancelled_token_instead_of_swallowing_it}.
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
/// A kill child that fails — the pid vanished between the scan and the signal —
/// is swallowed, exactly where the shell's `pkill -f … || true` swallows it.
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
mod tests;
