//! `demo.sh stop` — cleanly stop the game and the bottle's wine processes.
//!
//! Reference: `scripts/demo/stop.sh`. One step id
//! ([`step::STOP_WINESERVER`]) covers both the kill and the survivor probe
//! (they are one shell block); the remaining three steps are one each:
//!
//! 1. [`step::STOP_WINESERVER`] — `wineserver -k` (ignore failure) then a
//!    bounded `wineserver -w` (4 s, non-fatal — [`STOP_WINESERVER_WAIT`]),
//!    then: any survivor whose command line contains `"Beat Saber.exe"` →
//!    warn; else → `ok "game and wineserver down"`.
//! 2. [`step::STOP_PORTS`] — `lsof -nP -iUDP:9944 -iTCP:9943`, parsed the way
//!    doctor's `net.ports` is, but with stop.sh's own message text.
//! 3. [`step::STOP_REAP`] — reap a leftover encoder helper, then a leftover
//!    ALVR dashboard, by **exact** executable path.
//! 4. [`step::STOP_AUDIO`] — warn (plus a restore hint) when the Mac's audio
//!    output is still `BlackHole 2ch`; otherwise report the current device.
//!    Silent entirely when `SwitchAudioSource` is not installed.
//!
//! Between 3 and 4 sits the one thing stop.sh cannot do:
//! [`crate::session::reconcile::finish_stopped_session`] reads
//! `session-state.json` and, when the wine pid it names is now dead, actually
//! **restores** the previous session's guards — the audio device above all —
//! instead of only warning about them. It runs *before* [`report_audio`] on
//! purpose, so the `stop.4.audio` row reports the device it just put back.
//! Its rows carry their own step (`session.reconcile`) and its own section
//! banner; a `stop` with no leftover record emits nothing at all and step 4
//! reads exactly as it always did. Being additive, it is also not allowed to
//! *cost* this stage anything: it reports its own failures and returns
//! `Ok(())`, so the only error it can hand back is
//! [`SabrageError::Cancelled`] (its own "Failure policy" section). The `?`
//! below is therefore the cancellation path, not an abort path — a broken
//! `SwitchAudioSource` or `adb` recorded by the previous session must never
//! stop this one from reporting the ports and the audio device.
//!
//! # Mutations go through the executor
//!
//! `wineserver -k`/`-w` and every reap kill are spawned via
//! [`crate::stages::StageCtx::child`] + [`crate::executor::Executor::run_child`]
//! — never [`crate::process::terminate`] directly — so `--dry-run` records them
//! as [`crate::executor::PlannedKind::Spawn`] instead of touching a live
//! process. A reap kill is therefore `/bin/kill -TERM <pid>` run as a child,
//! which is the one primitive the [`crate::executor::Executor`] trait already
//! offers for "run something that mutates the machine" — there is no bespoke
//! "signal a pid" method on it, and adding one is outside this file's
//! ownership.
//!
//! The bounded `-w` wait has no cancellation hook of its own on the
//! [`crate::executor::Executor`] trait either, so it is raced against
//! [`STOP_WINESERVER_WAIT`] with [`tokio::time::timeout`]: on a real run this
//! relies on [`crate::process::spawn_streamed`]'s `kill_on_drop(true)` child —
//! dropping the timed-out future drops the still-running `tokio::process::Child`
//! in place, which kills it. A dry run's child never really spawns, so the
//! timeout never fires there.
//!
//! # Cancellation
//!
//! Every mutating [`crate::executor::Executor::run_child`] call in this file
//! ([`stop_wine`]'s `-k` and `-w`, [`reap`]'s per-pid kill) is followed by a
//! `checkpoint`, and [`run`] additionally checkpoints **between every step**.
//! If the token has fired, the function returns
//! `Err(`[`SabrageError::Cancelled`]`)` immediately instead of continuing on
//! to the remaining steps and reporting success — a Cancel during `stop` must
//! surface as [`SabrageError::Cancelled`]'s exit code 130, not a quiet
//! `StageFinished { ok: true }`. Checking the token directly (rather than only
//! matching the `run_child` result) also covers a pre-cancelled token under
//! [`crate::executor::DryRunExecutor`], whose `run_child` never spawns
//! anything and so never itself produces a cancellation error. The
//! between-step checkpoints cover the rest of the stage: the three reporting
//! steps spawn no executor child at all (they run `lsof`/`SwitchAudioSource`
//! and walk the process table) yet are where a live `stop` spends most of its
//! wall clock, and both `stop_wine` with no `wineserver` on the machine and
//! `reap` with nothing to kill return before reaching any check of their
//! own. A plain child failure — `wineserver -k`/`-w` exiting non-zero, a reap
//! kill racing a pid that already exited — is still swallowed exactly where
//! the shell's `|| true` swallows it; only cancellation short-circuits.
//!
//! # Declared divergences (see `sabrage/PARITY.md`)
//!
//! * The survivor probe is **argv-based by design**:
//!   [`crate::process::find_processes_by_cmdline`] (moved there in Phase 3 —
//!   run.sh's wineserver-timeout warning and the cancellation teardown need
//!   the same probe)
//!   which scans each process's command line (`sysinfo`'s `cmd()`, refreshed
//!   alongside `exe()`) for the substring `"Beat Saber.exe"` — the same shape
//!   `pgrep -f 'Beat Saber.exe'` matches, including the `Z:\...\Beat
//!   Saber.exe` Windows-path form Wine puts on the game's argv even though the
//!   OS-level executable for the process is CrossOver's own wine loader, not
//!   literally `Beat Saber.exe`. Exec-path matching (`ends_with` on the
//!   resolved exe) was tried first and rejected: under Wine the exe path can
//!   never end in `Beat Saber.exe`, so that probe's warn branch was
//!   unreachable and the row would silently always say "down" even with the
//!   game running — the one safety row this stage exists to report correctly.
//! * The two **reap** steps keep exact exe-path matching
//!   ([`crate::process::find_processes_by_exe`]) against a known staged
//!   binary, not argv — a narrower, still-intentional divergence from
//!   `pkill -f` (PARITY.md, "Stop"): a false-positive kill
//!   from an argv substring match (a `tail -f` of the log, an editor
//!   mentioning the path) is a worse failure mode there than an
//!   under-detecting probe would be here, so the two steps make opposite
//!   trade-offs on purpose.
//! * The **persisted guard restore** between steps 3 and 4
//!   ([`crate::session::reconcile::finish_stopped_session`]) has no shell
//!   counterpart at all: `run.sh`'s guards are shell traps, which a `SIGKILL`
//!   or a power loss skips entirely, and `stop.sh` can therefore only *warn*
//!   that the Mac's output is still `BlackHole 2ch` with nothing on the machine
//!   able to say what it was before. This is additive — it changes no shell
//!   text, and with no `session-state.json` on disk the stage behaves exactly
//!   as before.
//! * [`stale_listeners`] and its `lsof` invocation duplicate
//!   `checks::network`'s private `stale_listeners()` byte-for-byte (same args,
//!   same `awk`-shaped parse). That function is private to its module and this
//!   file's ownership does not extend to `checks/network.rs`; folding the two
//!   into one shared helper is a one-line follow-up for whoever next owns
//!   either file.

use std::collections::BTreeSet;
use std::path::Path;

use crate::error::{Result, SabrageError};
use crate::events::{step, StepId};
use crate::paths::{which, Bottle};
use crate::process::{self, find_processes_by_cmdline, ProcInfo};
use crate::stages::{require_bottle, StageCtx};

/// How long to wait for `wineserver -w` to return before giving up on it —
/// 4 s, never fatal.
///
/// Relocated to [`crate::stages`] in Phase 3, next to its deliberately
/// distinct sibling [`crate::stages::RUN_WINESERVER_WAIT`] (5 s, fatal), and
/// re-exported here so `stop::STOP_WINESERVER_WAIT` keeps resolving.
pub use crate::stages::STOP_WINESERVER_WAIT;

/// The substring `pgrep -f 'Beat Saber.exe'` matches on argv — matched the
/// same way here by [`crate::process::find_processes_by_cmdline`] (see the module docs'
/// "argv-based by design" note). Also the exe-basename fallback text in
/// [`format_survivors`] when a survivor's exe path has no file name.
const BEAT_SABER_EXE_SUFFIX: &str = "Beat Saber.exe";

/// The exact `lsof` invocation stop.sh (and doctor.sh's `net.ports`) use.
const LSOF_ARGS: [&str; 3] = ["-nP", "-iUDP:9944", "-iTCP:9943"];

/// Execute the stage.
pub async fn run(ctx: &StageCtx) -> Result<()> {
    let bottle = require_bottle(ctx)?;
    ctx.section(format!(
        "stopping wineserver for bottle '{}' (takes the game with it)",
        bottle.name
    ));

    stop_wine(ctx, bottle).await?;
    checkpoint(ctx)?;
    report_survivors(ctx);
    checkpoint(ctx)?;
    report_ports(ctx).await;
    checkpoint(ctx)?;
    reap(
        ctx,
        &ctx.paths.oxr_helper_staged,
        step::STOP_REAP,
        Some("encoder helper killed (the runtime owns it — this one outlived its game)"),
        Some("no leftover encoder helper"),
    )
    .await?;
    checkpoint(ctx)?;
    reap(
        ctx,
        &ctx.paths.alvr_dashboard,
        step::STOP_REAP,
        Some("ALVR dashboard closed (left over from a run that died uncleanly)"),
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
/// Called between **every** step of [`run`], not only around the mutating
/// children: the three reporting steps ([`report_survivors`],
/// [`report_ports`], [`report_audio`]) each shell out to a real subprocess
/// (`lsof`, `SwitchAudioSource`) or walk the whole process table, and on a live
/// `stop` they dominate the stage's wall-clock time — a Cancel landing there
/// used to run to the end and still report `StageFinished { ok: true }`,
/// exit 0. [`stop_wine`] returning early (no `wineserver` on this machine) and
/// [`reap`] finding nothing to kill are the same hole: neither reaches its own
/// post-`run_child` check, so the checkpoints in [`run`] are what make the
/// guarantee unconditional.
fn checkpoint(ctx: &StageCtx) -> Result<()> {
    if ctx.cancel.is_cancelled() {
        return Err(SabrageError::Cancelled);
    }
    Ok(())
}

// ── step 1: wineserver kill + survivor probe ─────────────────────────────────

/// `lib.sh`'s `stop_wine`:
///
/// ```zsh
/// stop_wine() {
///   WINEPREFIX="$PREFIX" "$WINESERVER" -k 2>/dev/null || true
///   ( WINEPREFIX="$PREFIX" "$WINESERVER" -w 2>/dev/null ) &
///   local _wp=$!
///   for _i in {1..40}; do kill -0 $_wp 2>/dev/null || break; sleep 0.1; done
///   kill $_wp 2>/dev/null || true
/// }
/// ```
///
/// No CrossOver on this machine means `ctx.paths.wineserver` is `None` — lib.sh
/// would still "run" a bogus path and swallow the resulting command-not-found
/// failure, so skipping outright here is behaviourally the same (no output
/// either way).
///
/// Returns `Err(`[`SabrageError::Cancelled`]`)` if `ctx.cancel` fires during
/// either child — see the module doc's Cancellation section. A plain
/// non-zero/failed child is still swallowed, matching the shell's `|| true`.
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

/// stop.sh:
/// ```zsh
/// if pgrep -f 'Beat Saber.exe' >/dev/null 2>&1; then
///   warn "Beat Saber processes survived: $(pgrep -lf 'Beat Saber.exe' | tr '\n' ' ')"
/// else
///   ok "game and wineserver down"
/// fi
/// ```
fn report_survivors(ctx: &StageCtx) {
    let st = ctx.step(step::STOP_WINESERVER);
    let survivors = find_processes_by_cmdline(BEAT_SABER_EXE_SUFFIX);
    if survivors.is_empty() {
        st.ok("game and wineserver down");
    } else {
        st.warn(format!(
            "Beat Saber processes survived: {}",
            format_survivors(&survivors)
        ));
    }
}

// ── step 2: streaming ports ───────────────────────────────────────────────────

/// `lsof -nP -iUDP:9944 -iTCP:9943 2>/dev/null | awk 'NR>1{print $1"("$2")"}' |
/// sort -u | tr '\n' ' '` — `COMMAND(PID)` per listener, deduplicated, sorted,
/// space-joined with a trailing space when non-empty. See the module docs for
/// why this duplicates `checks::network`'s private equivalent.
async fn stale_listeners() -> String {
    let out = match tokio::process::Command::new("lsof")
        .args(LSOF_ARGS)
        .output()
        .await
    {
        Ok(o) => o,
        Err(_) => return String::new(),
    };
    let text = String::from_utf8_lossy(&out.stdout);
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
    out
}

/// stop.sh:
/// ```zsh
/// STALE="$(lsof -nP -iUDP:9944 -iTCP:9943 2>/dev/null | awk '…' | sort -u | tr '\n' ' ')"
/// if [ -n "$STALE" ]; then warn "streaming ports still held by: $STALE"
/// else ok "streaming ports free"; fi
/// ```
async fn report_ports(ctx: &StageCtx) {
    let st = ctx.step(step::STOP_PORTS);
    let stale = stale_listeners().await;
    if stale.is_empty() {
        st.ok("streaming ports free");
    } else {
        st.warn(format!("streaming ports still held by: {stale}"));
    }
}

// ── step 3: reap leftovers ────────────────────────────────────────────────────

/// `lib.sh`'s `reap_stray`, specialised to exact-exe-path matching (see the
/// module docs) and routed through the executor:
///
/// ```zsh
/// reap_stray() { # bin_path [found_msg] [not_found_msg]
///   if pgrep -f "$1" >/dev/null 2>&1; then
///     pkill -f "$1" 2>/dev/null || true
///     [ -n "${2:-}" ] && ok "$2"
///   else
///     [ -n "${3:-}" ] && ok "$3"
///   fi
/// }
/// ```
///
/// One `/bin/kill -TERM <pid>` child per match (`pkill -f` kills every match at
/// once; a matched-then-vanished pid's failing kill is swallowed exactly as
/// `pkill`'s is). `found_msg`/`not_found_msg` fire at most once regardless of
/// match count, matching the shell's single `ok` call.
///
/// Returns `Err(`[`SabrageError::Cancelled`]`)` — skipping any remaining kills
/// and the closing message — the moment `ctx.cancel` fires after any one kill;
/// see the module doc's Cancellation section.
async fn reap(
    ctx: &StageCtx,
    exe_path: &Path,
    step_id: StepId,
    found_msg: Option<&str>,
    not_found_msg: Option<&str>,
) -> Result<()> {
    let procs = process::find_processes_by_exe(exe_path);
    if procs.is_empty() {
        if let Some(msg) = not_found_msg {
            ctx.step(step_id).ok(msg);
        }
        return Ok(());
    }
    for p in &procs {
        let spec = ctx
            .child("/bin/kill", step_id)
            .arg("-TERM")
            .arg(p.pid.to_string());
        let _ = ctx.executor.run_child(&spec).await;
        checkpoint(ctx)?;
    }
    if let Some(msg) = found_msg {
        ctx.step(step_id).ok(msg);
    }
    Ok(())
}

// ── step 4: audio ─────────────────────────────────────────────────────────────

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

/// stop.sh:
/// ```zsh
/// if command -v SwitchAudioSource >/dev/null 2>&1; then
///   CUR="$(SwitchAudioSource -c -t output 2>/dev/null)"
///   if [ "$CUR" = "BlackHole 2ch" ]; then
///     warn "Mac audio output is still BlackHole 2ch (a run that died uncleanly could not restore it)"
///     info "restore with: SwitchAudioSource -t output -s '<device>'   (list: SwitchAudioSource -a -t output)"
///   else
///     ok "audio output: $CUR"
///   fi
/// fi
/// ```
/// Entirely silent — no step, no rows — when `SwitchAudioSource` is not on `PATH`.
async fn report_audio(ctx: &StageCtx) {
    let Some(bin) = which("SwitchAudioSource") else {
        return;
    };
    let raw = tokio::process::Command::new(&bin)
        .args(["-c", "-t", "output"])
        .output()
        .await
        .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
        .unwrap_or_default();
    let cur = crate::util::strip_trailing_newlines(&raw);

    let st = ctx.step(step::STOP_AUDIO);
    match audio_report(cur) {
        AudioReport::StillBlackhole => {
            st.warn(AUDIO_STILL_BLACKHOLE_WARN);
            st.info(AUDIO_RESTORE_HINT);
        }
        AudioReport::Restored(cur) => {
            st.ok(format!("audio output: {cur}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Moved to `crate::process` in Phase 3; the tests below still cover it
    // here because this file is where its one production caller lives.
    use crate::events::Severity;
    use crate::executor::{DryRunExecutor, Executor, PlannedKind};
    use crate::paths::Paths;
    use crate::process::cmdline_contains;
    use crate::stages::{null_sink, StageCtx, StageOptions};
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex as StdMutex};
    use tokio_util::sync::CancellationToken;

    // ── STALE string builder ─────────────────────────────────────────────────

    #[tokio::test]
    async fn stale_listeners_is_well_formed_cmd_pid_pairs_with_a_trailing_space() {
        // Ground truth is machine state (paths.rs's own testing pattern): assert
        // the shape invariant rather than a fixed value.
        let stale = stale_listeners().await;
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
        let stale = stale_listeners().await;
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

    // ── survivor-line formatting ──────────────────────────────────────────────

    #[test]
    fn format_survivors_matches_the_pgrep_lf_shape() {
        assert_eq!(format_survivors(&[]), "");
        let procs = vec![
            ProcInfo {
                pid: 111,
                start_time: 0,
                exe: PathBuf::from("/repo/ext/oxrsys/build-x64/Beat Saber.exe"),
            },
            ProcInfo {
                pid: 222,
                start_time: 0,
                exe: PathBuf::from("/other/place/Beat Saber.exe"),
            },
        ];
        assert_eq!(
            format_survivors(&procs),
            "111 Beat Saber.exe 222 Beat Saber.exe "
        );
    }

    #[test]
    fn format_survivors_falls_back_to_the_suffix_when_a_path_has_no_file_name() {
        let procs = vec![ProcInfo {
            pid: 7,
            start_time: 0,
            exe: PathBuf::from("/"),
        }];
        assert_eq!(format_survivors(&procs), "7 Beat Saber.exe ");
    }

    // ── argv-based survivor matcher (finding #8) ─────────────────────────────

    #[test]
    fn cmdline_contains_matches_the_pgrep_f_shape_including_the_windows_path_form() {
        // The real case: one argv element is the whole `Z:\...` Windows path,
        // which itself contains an embedded space ("Beat" + "Saber.exe").
        assert!(cmdline_contains(
            &["Z:\\repo\\ext\\oxrsys\\build-x64\\Beat Saber.exe".to_string()],
            BEAT_SABER_EXE_SUFFIX
        ));
        assert!(cmdline_contains(
            &[
                "wine64-preloader".to_string(),
                "Z:\\Beat Saber.exe".to_string(),
            ],
            BEAT_SABER_EXE_SUFFIX
        ));
        // A hypothetical split across two argv elements still matches via the
        // whitespace-joined whole line, the same shape `pgrep -f` scans.
        assert!(cmdline_contains(
            &["Beat".to_string(), "Saber.exe".to_string()],
            BEAT_SABER_EXE_SUFFIX
        ));
        assert!(!cmdline_contains(
            &["wineserver".to_string(), "-k".to_string()],
            BEAT_SABER_EXE_SUFFIX
        ));
        assert!(!cmdline_contains(&[], BEAT_SABER_EXE_SUFFIX));
    }

    #[test]
    fn finds_by_cmdline_using_this_test_binarys_own_argv() {
        let exe = std::env::current_exe().expect("test binary path");
        let name = exe
            .file_name()
            .and_then(|n| n.to_str())
            .expect("utf8 test binary name");
        // A short, distinctive suffix of the real binary name, not the whole
        // path — proving this is a *substring-of-cmdline* match, unlike
        // `find_processes_by_exe`'s exact-path equality.
        let needle = &name[name.len().saturating_sub(6)..];
        let found = find_processes_by_cmdline(needle);
        let me = std::process::id();
        assert!(
            found.iter().any(|p| p.pid == me),
            "own pid {me} not found by cmdline needle {needle:?} among {found:?}"
        );
        assert!(find_processes_by_cmdline("nonexistent-sabrage-needle.exe").is_empty());
    }

    #[test]
    fn report_survivors_matches_a_direct_probe() {
        let (ctx, seen) = test_ctx(StageOptions::default());
        report_survivors(&ctx);
        let survivors = find_processes_by_cmdline(BEAT_SABER_EXE_SUFFIX);
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

    // ── audio branch texts ────────────────────────────────────────────────────

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

    // ── stop_wine / reap route through the executor ──────────────────────────

    fn test_ctx(opts: StageOptions) -> (StageCtx, Arc<StdMutex<Vec<crate::events::StageEvent>>>) {
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let s = seen.clone();
        let sink: crate::stages::EventSink = Arc::new(move |ev| s.lock().unwrap().push(ev));
        let mut paths = Paths::new("/nonexistent/sabrage-stop-test");
        // Load-bearing: `run` reconciles `session-state.json` between steps 3
        // and 4, and `Paths::new` derives that path from the real `$HOME`.
        // Without this override a stop test would read — and with a real
        // executor delete — the developer's own live session record.
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

    #[tokio::test]
    async fn dry_run_stop_wine_never_spawns_a_real_wineserver() {
        let (mut ctx, _seen) = test_ctx(StageOptions {
            dry_run: true,
            ..Default::default()
        });
        // Force a wineserver path regardless of whether CrossOver is installed
        // on this machine, so the test exercises the spawn path either way.
        ctx.paths.wineserver = Some(PathBuf::from("/nonexistent/sabrage/wineserver"));
        let bottle = Bottle::unvalidated("SabrageStopTest");

        stop_wine(&ctx, &bottle).await.expect("not cancelled");

        let planned = ctx.executor.planned();
        let spawns: Vec<_> = planned
            .into_iter()
            .filter(|p| p.kind == PlannedKind::Spawn)
            .collect();
        assert_eq!(spawns.len(), 2, "expected -k and -w to both be planned");
        assert!(spawns[0].reason.ends_with("wineserver -k"));
        assert!(spawns[1].reason.ends_with("wineserver -w"));
    }

    #[tokio::test]
    async fn dry_run_stop_wine_is_a_no_op_without_crossover() {
        let (mut ctx, _seen) = test_ctx(StageOptions {
            dry_run: true,
            ..Default::default()
        });
        // Deterministic regardless of whether this machine happens to have a
        // real CrossOver.app (Paths::new probes for one unconditionally).
        ctx.paths.wineserver = None;
        let bottle = Bottle::unvalidated("SabrageStopTest");
        stop_wine(&ctx, &bottle).await.expect("not cancelled");
        assert!(ctx.executor.planned().is_empty());
    }

    #[tokio::test]
    async fn dry_run_reap_plans_a_kill_per_match_and_reports_once() {
        // find_processes_by_exe on a path nothing runs from: not-found branch.
        let (ctx, seen) = test_ctx(StageOptions {
            dry_run: true,
            ..Default::default()
        });
        reap(
            &ctx,
            Path::new("/nonexistent/sabrage/helper"),
            step::STOP_REAP,
            Some("found"),
            Some("not found"),
        )
        .await
        .expect("not cancelled");
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
        reap(
            &ctx,
            &exe,
            step::STOP_REAP,
            Some("found"),
            Some("not found"),
        )
        .await
        .expect("not cancelled");
        let evs = seen.lock().unwrap().clone();
        assert_eq!(evs.len(), 1);
        assert!(matches!(
            &evs[0],
            crate::events::StageEvent::Line { text, .. } if text == "found"
        ));
        let planned = ctx.executor.planned();
        assert!(
            !planned.is_empty(),
            "this test process itself should match its own exe path"
        );
        assert!(planned
            .iter()
            .all(|p| p.kind == PlannedKind::Spawn && p.reason.contains("/bin/kill -TERM")));
    }

    #[test]
    fn dry_run_executor_is_dry_run() {
        // Sanity: the fixtures above rely on StageOptions{dry_run:true} wiring
        // up a DryRunExecutor.
        let sink = null_sink();
        let ex = DryRunExecutor::new(uuid::Uuid::nil(), sink, CancellationToken::new());
        assert!(ex.is_dry_run());
    }

    // ── cancellation propagation (finding #2) ────────────────────────────────

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
            &exe,
            step::STOP_REAP,
            Some("found"),
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
    /// `wineserver` path [`stop_wine`] spawns and hits its own post-`run_child`
    /// check; with `None` it returns `Ok(())` immediately and only [`run`]'s
    /// between-step [`checkpoint`] can catch the cancellation. The `None`
    /// variant reported `StageFinished { ok: true }`, exit 0, before those
    /// checkpoints existed.
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

    // ── the reconcile pass may not cost the stage its last rows (finding #6) ──

    /// A pid no process can have on macOS (`kern.maxproc` is five digits), so
    /// the recorded wine process classifies as `Dead` without a pid-reuse race.
    /// Deliberately not `u32::MAX`, which is `-1` as an `i32`.
    const DEAD_PID: u32 = 2_147_483_646;

    /// Finding #6, at the stage level. The reconcile pass between steps 3 and 4
    /// is *additive*, so a failure inside it must be reported rather than abort
    /// the stage on the way to the ports and audio rows — `stop.sh` has no step
    /// that can end the script.
    ///
    /// Made to fail deterministically and machine-independently: the record
    /// carries a `--wired` forward and `adb` points at a path that does not
    /// exist, so the `forward --remove` child fails at `spawn` with `ENOENT`.
    /// That is the exact `Err` shape `stop::run`'s `?` used to hand upward.
    /// Nothing is spawned, signalled or written anywhere on the machine: no
    /// wineserver, nothing matching either reap path, and the store is a fresh
    /// temp directory.
    #[tokio::test]
    async fn a_failed_reconcile_is_reported_and_the_stage_still_reaches_its_audio_row() {
        use crate::session::state::{self, SessionState, WiredForward};

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
        // …and the two steps that come after it still ran.
        assert!(
            lines
                .iter()
                .any(|(_, step, _)| step.as_deref() == Some(step::STOP_PORTS)),
            "report_ports never ran: {lines:?}"
        );
        let audio_rows = lines
            .iter()
            .filter(|(_, step, _)| step.as_deref() == Some(step::STOP_AUDIO))
            .count();
        if which("SwitchAudioSource").is_some() {
            // One row normally; two when this Mac's output happens to be sitting
            // on BlackHole 2ch (warn + restore hint). Either proves step 4 ran,
            // which is what a failed reconcile used to cost.
            assert!(audio_rows >= 1, "the audio row is the point: {lines:?}");
        } else {
            assert_eq!(audio_rows, 0, "silent without the tool: {lines:?}");
        }
        assert!(
            state::load(&path).unwrap().is_some(),
            "the record is kept so the next stop can retry"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The narrower window finding #3 named: cancellation landing after the
    /// wineserver kill, in the *reporting* half of the stage, where no
    /// executor child is ever spawned and so nothing else can observe the
    /// token. Made deterministic by cancelling **from the event sink** the
    /// instant the first row is emitted (`report_survivors`'s), rather than
    /// racing a timer: the next thing [`run`] does is its own [`checkpoint`].
    /// Without those checkpoints this ran to the end and reported
    /// `StageFinished { ok: true }`, exit 0.
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
