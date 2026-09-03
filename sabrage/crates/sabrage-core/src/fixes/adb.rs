//! `fix.remove-adb-forwards` — drop the two `--wired` port forwards.
//!
//! Leftover `tcp:9943`/`tcp:9944` forwards persist across sessions and silently
//! break WiFi discovery ("searching for streamer"). A normal (non-wired) run
//! removes exactly those two in preflight.
//!
//! Invariants (PARITY.md, "must NOT change"):
//! * `adb forward --remove tcp:9943` and `tcp:9944`, **per serial**
//!   (`adb -s <serial> forward --remove …`) — never `--remove-all`, which would
//!   also delete unrelated forwards;
//! * distinct from `adb reverse --remove-all`, which `run.sh` does use;
//! * the port pair comes from the contract (`ports.stream`), never a literal;
//! * `paths.adb` is `Option` — absent adb is "nothing to do", not an error.
//!
//! Ported verbatim from `run.sh` (lines 113–124):
//!
//! ```zsh
//! elif [ -n "$ADB" ]; then
//!   "$ADB" forward --list 2>/dev/null | while read -r fwd_ser fwd_local fwd_remote; do
//!     case "$fwd_local" in
//!       tcp:9943|tcp:9944)
//!         "$ADB" -s "$fwd_ser" forward --remove "$fwd_local" 2>/dev/null && \
//!           info "cleared stale adb forward $fwd_local on $fwd_ser (left over from a --wired launch — would otherwise break WiFi discovery)"
//!         ;;
//!     esac
//!   done
//! fi
//! ```
//!
//! Note the `&&`: a failed removal prints nothing at all (no warn, no fail, no
//! die). The `info` row is reproduced exactly — it appears only for a removal
//! that really happened — but the shell's *silence* is not: a structured
//! [`FixReport`] has to say something, and "no stale adb port forwards to clear"
//! would be a lie on a device whose forward is still installed. A failed
//! removal, and a `forward --list` that could not be read at all, therefore warn
//! and report what is still (possibly) there. PARITY.md carries the divergence.

use std::path::Path;
use std::time::Duration;

use crate::contract::contract;
use crate::error::Result;
use crate::events::{StageEvent, StepId};
use crate::fixes::{FixAction, FixReport};
use crate::stages::{EventSink, StageCtx};

/// `adb forward --list` starts adb's background server on a cold run, which can
/// block for a few seconds — long enough that a bare `.output().await` on the
/// async worker is worth bounding. A timeout is reported as a query failure
/// (see [`list_forwards`]), never as an empty forwarding table.
const ADB_LIST_TIMEOUT: Duration = Duration::from_secs(5);

/// This fix's own step id, used when it runs **as a fix** — from the doctor's
/// fix list or `fixes::apply`, where there is no stage step to belong to.
///
/// The launch path is the other caller and must *not* use it: run.sh's
/// adb-forward hygiene is a numbered step of the run stage, and its rows have
/// to sort and group with the rest of that stage's. It passes
/// [`crate::events::step::RUN_ADB_FORWARDS`] to
/// [`remove_adb_forwards_at`] instead.
const STEP: StepId = "fix.remove-adb-forwards";

/// Parse `adb forward --list`'s stdout: `<serial> <local> <remote>` rows, one
/// per line. Rows with fewer than two whitespace-separated fields are ignored
/// (there should never be any, but a short/blank line must not panic).
fn parse_forward_list(stdout: &str) -> Vec<(String, String)> {
    stdout
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let serial = fields.next()?;
            let local = fields.next()?;
            Some((serial.to_string(), local.to_string()))
        })
        .collect()
}

/// `"$ADB" forward --list 2>/dev/null`. A read-only query — like
/// `checks::network`'s own adb probe, this bypasses the executor (dry-run
/// gating only matters for mutations, and listing forwards changes nothing).
///
/// Runs on `tokio::process::Command` rather than a blocking
/// `std::process::Command::output()` (this fix's `remove_adb_forwards` is
/// itself async, spawned on the async worker) and bounded by
/// [`ADB_LIST_TIMEOUT`], since a cold `adb` starting its own server can block
/// for seconds.
///
/// `Err(reason)` for a query that could not be answered — a spawn failure, the
/// timeout, or a non-zero `adb`. It is deliberately **not** folded into an empty
/// list: "adb could not tell us" and "there is nothing to clear" are different
/// facts, and only one of them justifies telling the user their forwarding
/// table is clean.
async fn list_forwards(adb: &Path) -> std::result::Result<Vec<(String, String)>, String> {
    let output = tokio::time::timeout(
        ADB_LIST_TIMEOUT,
        tokio::process::Command::new(adb)
            .args(["forward", "--list"])
            .output(),
    )
    .await;
    match output {
        Ok(Ok(out)) if out.status.success() => {
            Ok(parse_forward_list(&String::from_utf8_lossy(&out.stdout)))
        }
        Ok(Ok(out)) => Err(format!("adb forward --list exited {}", out.status)),
        Ok(Err(e)) => Err(format!("could not run {}: {e}", adb.display())),
        Err(_) => Err(format!(
            "adb forward --list timed out after {}s",
            ADB_LIST_TIMEOUT.as_secs()
        )),
    }
}

/// run.sh's `info` row for one cleared forward, verbatim:
///
/// ```zsh
/// info "cleared stale adb forward $fwd_local on $fwd_ser (left over from a --wired launch — would otherwise break WiFi discovery)"
/// ```
///
/// `verb` is `"cleared"` for a real removal and `"would clear"` for a dry run
/// (Sabrage-only — the shell has no dry run).
///
/// `pub` so `sabrage-parity` can compare the **live** native string against
/// `run.sh` rather than a fragment copied into the parity crate: tier 1 runs
/// `-p sabrage-parity` only, so a native literal edited without touching run.sh
/// is otherwise ungated (A1-3).
pub fn cleared_forward_line(verb: &str, local: &str, serial: &str) -> String {
    format!(
        "{verb} stale adb forward {local} on {serial} (left over from a --wired launch — would \
         otherwise break WiFi discovery)"
    )
}

/// The `tcp:<port>` local-forward specs this fix targets, from the contract's
/// `[ports] stream` — never a literal `"tcp:9943"` (PARITY.md).
fn stale_local_specs() -> Vec<String> {
    contract()
        .ports
        .stream
        .iter()
        .map(|p| format!("tcp:{p}"))
        .collect()
}

/// Remove the stale `tcp:9943`/`tcp:9944` forwards, per serial, as the
/// standalone fix — every row stamped [`STEP`].
///
/// Refuses while a session is live. During a `--wired` session the two forwards
/// this fix removes are the ones the stream is running over, and doctor cannot
/// tell them apart from stale ones (it does not know the launch's `wired`
/// state), so its row offers this remedy either way. [`crate::fixes::apply`]
/// already gates every fix the same way; the check is repeated here because this
/// function is the one a caller could reach directly, and disconnecting the
/// headset is not a mistake worth leaving one door open for.
///
/// The launch path calls [`remove_adb_forwards_at`], which is deliberately
/// **not** gated: clearing leftovers is a preflight step of the very session
/// being started.
pub async fn remove_adb_forwards(ctx: &StageCtx, sink: &EventSink) -> Result<FixReport> {
    if let Some(reason) = crate::stages::live_session_block(&ctx.paths) {
        return Err(ctx.fatal(
            format!(
                "refusing to remove adb port forwards while a session is live — {reason}; a \
                 --wired session is streaming over these forwards"
            ),
            Some(format!(
                "./demo.sh stop --bottle {}",
                ctx.opts.bottle_name.as_deref().unwrap_or("<name>")
            )),
        ));
    }
    remove_adb_forwards_at(ctx, sink, STEP).await
}

/// The same removal, stamped with a caller-supplied step id.
///
/// The behaviour is identical — same per-serial `adb forward --remove`, same
/// `info` text, same tolerant "a failed removal prints nothing" rule; only the
/// step the rows are attributed to changes. The launch path
/// ([`crate::stages::run::actions::adb_forward_hygiene`]) passes
/// [`crate::events::step::RUN_ADB_FORWARDS`], so its rows belong to the run
/// stage's step 2 rather than to a fix that is not running.
pub async fn remove_adb_forwards_at(
    ctx: &StageCtx,
    sink: &EventSink,
    step: StepId,
) -> Result<FixReport> {
    let Some(adb) = ctx.paths.adb.clone() else {
        return Ok(FixReport::unchanged(
            FixAction::RemoveAdbForwards,
            "adb not found — nothing to clear",
        ));
    };

    let stale_locals = stale_local_specs();
    let dry_run = ctx.executor.is_dry_run();
    let executor = ctx.executor_for(step);

    let listed = match list_forwards(&adb).await {
        Ok(rows) => rows,
        Err(reason) => {
            // Nothing was removed and nothing is known: say both. Reporting the
            // clean-table string here is how a WiFi-breaking forward survives a
            // fix the user watched succeed.
            let text = format!(
                "could not query adb forwards ({reason}) — stale {} forwards may still be \
                 installed",
                stale_locals.join("/")
            );
            sink(StageEvent::warn(ctx.run_id, Some(step), text.clone()));
            return Ok(FixReport::unchanged(FixAction::RemoveAdbForwards, text));
        }
    };

    let mut cleared: Vec<String> = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    for (serial, local) in listed {
        if !stale_locals.contains(&local) {
            continue;
        }
        let spec = ctx
            .child(adb.clone(), step)
            .args(["-s", &serial, "forward", "--remove", &local]);
        // NEVER `--remove-all` here — see this module's header. `run_child`
        // reports a non-zero exit as `Ok`, matching the shell's tolerant `&&`:
        // a failed removal is not an error and never aborts the launch that
        // calls this. Unlike the shell, it is not silent either — the pair is
        // remembered so the report cannot claim the table is clean.
        let status = executor.run_child(&spec).await?;
        if !status.success() {
            let text = format!("could not clear adb forward {local} on {serial} ({status})");
            sink(StageEvent::warn(ctx.run_id, Some(step), text));
            failed.push(format!("{serial} {local}"));
            continue;
        }
        let verb = if dry_run { "would clear" } else { "cleared" };
        let line = cleared_forward_line(verb, &local, &serial);
        sink(StageEvent::info(ctx.run_id, Some(step), line.clone()));
        cleared.push(line);
    }

    if !failed.is_empty() {
        let still = failed.join(", ");
        let description = if cleared.is_empty() {
            format!("adb forwards still installed: {still}")
        } else {
            format!("{}; still installed: {still}", cleared.join("; "))
        };
        // `changed` follows what actually happened: a partial clear did change
        // the machine, a total failure did not.
        return Ok(if cleared.is_empty() {
            FixReport::unchanged(FixAction::RemoveAdbForwards, description)
        } else {
            FixReport::changed(FixAction::RemoveAdbForwards, description)
        });
    }

    if cleared.is_empty() {
        Ok(FixReport::unchanged(
            FixAction::RemoveAdbForwards,
            "no stale adb port forwards to clear",
        ))
    } else {
        Ok(FixReport::changed(
            FixAction::RemoveAdbForwards,
            cleared.join("; "),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::Paths;
    use crate::stages::{StageCtx, StageOptions};
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use tokio_util::sync::CancellationToken;

    // ── pure parsing ─────────────────────────────────────────────────────────

    #[test]
    fn parse_forward_list_takes_serial_and_local_and_skips_unusable_lines() {
        type Expected = &'static [(&'static str, &'static str)];
        let cases: &[(&str, &str, Expected)] = &[
            (
                "serial and local kept, the remote field dropped",
                "192.168.1.5:5555 tcp:9943 tcp:9943\n192.168.1.5:5555 tcp:9944 tcp:9944\n",
                &[
                    ("192.168.1.5:5555", "tcp:9943"),
                    ("192.168.1.5:5555", "tcp:9944"),
                ],
            ),
            ("empty input", "", &[]),
            ("blank lines only", "\n\n", &[]),
            (
                "short line is skipped, the next row still parses",
                "onlyone\nser tcp:1 tcp:2\n",
                &[("ser", "tcp:1")],
            ),
        ];
        for (label, input, expected) in cases {
            let expected: Vec<(String, String)> = expected
                .iter()
                .map(|(serial, local)| ((*serial).to_string(), (*local).to_string()))
                .collect();
            assert_eq!(parse_forward_list(input), expected, "{label}");
        }
    }

    // ── remove_adb_forwards (the async fix) ─────────────────────────────────

    fn scratch(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("sabrage-adb-fix-{tag}-{}", std::process::id()))
    }

    /// A `/bin/sh` fixture standing in for `adb`: a plain shell script, not a
    /// copied Mach-O binary — this crate's sandboxed test runner has been
    /// observed to `SIGKILL` the latter before it can even run (see
    /// `fixes::backend`'s test module header), but a script executes fine.
    fn write_fake_adb(script_path: &Path, list_stdout: &str, log_path: &Path) {
        std::fs::create_dir_all(script_path.parent().unwrap()).unwrap();
        let script = format!(
            "#!/bin/sh\n\
             if [ \"$1\" = forward ] && [ \"$2\" = --list ]; then\n\
             \x20\x20cat <<'SABRAGE_EOF'\n{list_stdout}SABRAGE_EOF\n\
             \x20\x20exit 0\n\
             fi\n\
             if [ \"$1\" = -s ]; then\n\
             \x20\x20echo \"$2 $3 $4 $5\" >> {log}\n\
             \x20\x20exit 0\n\
             fi\n\
             exit 1\n",
            log = log_path.display(),
        );
        std::fs::write(script_path, script).unwrap();
        let mut perms = std::fs::metadata(script_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(script_path, perms).unwrap();
    }

    /// A fake `adb` whose exit codes are the fixture: `list_exit` for
    /// `forward --list`, `remove_exit` for every `-s <serial> forward --remove`.
    fn write_fake_adb_failing(
        script_path: &Path,
        list_stdout: &str,
        list_exit: i32,
        remove_exit: i32,
        log_path: &Path,
    ) {
        std::fs::create_dir_all(script_path.parent().unwrap()).unwrap();
        let script = format!(
            "#!/bin/sh\n\
             if [ \"$1\" = forward ] && [ \"$2\" = --list ]; then\n\
             \x20\x20cat <<'SABRAGE_EOF'\n{list_stdout}SABRAGE_EOF\n\
             \x20\x20exit {list_exit}\n\
             fi\n\
             if [ \"$1\" = -s ]; then\n\
             \x20\x20echo \"$2 $3 $4 $5\" >> {log}\n\
             \x20\x20exit {remove_exit}\n\
             fi\n\
             exit 1\n",
            log = log_path.display(),
        );
        std::fs::write(script_path, script).unwrap();
        let mut perms = std::fs::metadata(script_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(script_path, perms).unwrap();
    }

    fn ctx_with_adb(root: &Path, adb: PathBuf, dry_run: bool) -> StageCtx {
        let mut paths = Paths::new(root);
        paths.adb = Some(adb);
        // The standalone fix consults the live-session policy; point both stores
        // at the scratch root so it reads fixtures, never the real machine.
        paths.sabrage_appsup = root.join("Sabrage");
        paths.oxr_appsup = root.join("OXRSys");
        let opts = StageOptions {
            dry_run,
            ..StageOptions::default()
        };
        let sink: EventSink = std::sync::Arc::new(|_| {});
        StageCtx::new(paths, opts, sink, CancellationToken::new())
    }

    #[tokio::test]
    async fn no_adb_is_a_silent_noop() {
        let root = scratch("no-adb");
        let mut paths = Paths::new(&root);
        paths.adb = None;
        paths.sabrage_appsup = root.join("Sabrage");
        paths.oxr_appsup = root.join("OXRSys");
        let sink: EventSink = std::sync::Arc::new(|_| {});
        let ctx = StageCtx::new(
            paths,
            StageOptions::default(),
            sink.clone(),
            CancellationToken::new(),
        );
        let report = remove_adb_forwards(&ctx, &sink).await.unwrap();
        assert!(!report.changed);
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn clean_forward_list_is_a_noop() {
        let root = scratch("clean");
        let adb = root.join("adb.sh");
        let log = root.join("removed.log");
        write_fake_adb(&adb, "SER tcp:5555 tcp:5555\n", &log);

        let ctx = ctx_with_adb(&root, adb, false);
        let sink: EventSink = ctx.sink.clone();
        let report = remove_adb_forwards(&ctx, &sink).await.unwrap();
        assert!(!report.changed);
        assert!(!log.exists(), "nothing stale -> nothing removed");
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn removes_exactly_the_two_stale_ports_per_serial_never_remove_all() {
        let root = scratch("remove");
        let adb = root.join("adb.sh");
        let log = root.join("removed.log");
        write_fake_adb(
            &adb,
            "SERIALX tcp:9943 tcp:9943\nSERIALX tcp:9944 tcp:9944\nSERIALX tcp:5555 tcp:5555\n",
            &log,
        );

        let ctx = ctx_with_adb(&root, adb, false);
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let s = seen.clone();
        let sink: EventSink = std::sync::Arc::new(move |ev| s.lock().unwrap().push(ev));

        let report = remove_adb_forwards(&ctx, &sink).await.unwrap();
        assert!(report.changed);
        assert!(report.description.contains("tcp:9943"));
        assert!(report.description.contains("tcp:9944"));
        assert!(!report.description.contains("5555"));

        let log_text = std::fs::read_to_string(&log).unwrap();
        let mut lines: Vec<&str> = log_text.lines().collect();
        lines.sort_unstable();
        assert_eq!(
            lines,
            vec![
                "SERIALX forward --remove tcp:9943",
                "SERIALX forward --remove tcp:9944",
            ],
            "must remove exactly the two stale ports, per-serial, never --remove-all"
        );
        assert!(!log_text.contains("--remove-all"));

        let texts: Vec<String> = seen
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                StageEvent::Line { text, severity, .. } => {
                    assert_eq!(
                        *severity,
                        crate::events::Severity::Info,
                        "run.sh uses `info`, not `ok`, here"
                    );
                    Some(text.clone())
                }
                _ => None,
            })
            .collect();
        assert!(texts.iter().any(|t| t
            == "cleared stale adb forward tcp:9943 on SERIALX (left over from a --wired launch — would otherwise break WiFi discovery)"));
        assert!(texts.iter().any(|t| t
            == "cleared stale adb forward tcp:9944 on SERIALX (left over from a --wired launch — would otherwise break WiFi discovery)"));

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn dry_run_reports_would_clear_and_still_invokes_the_planned_spawn() {
        let root = scratch("dry");
        let adb = root.join("adb.sh");
        let log = root.join("removed.log");
        write_fake_adb(&adb, "SERIALX tcp:9943 tcp:9943\n", &log);

        let ctx = ctx_with_adb(&root, adb, true);
        let sink: EventSink = ctx.sink.clone();
        let report = remove_adb_forwards(&ctx, &sink).await.unwrap();
        assert!(report.changed);
        assert!(report
            .description
            .starts_with("would clear stale adb forward tcp:9943"));
        assert!(!log.exists(), "dry run must not actually spawn the removal");

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn the_step_id_is_the_fixs_own_by_default_and_the_callers_with_at() {
        // #16c: the standalone fix keeps `fix.remove-adb-forwards`; the launch
        // path stamps the run stage's step instead, without forking the code.
        let root = scratch("step-id");
        let adb = root.join("adb.sh");
        let log = root.join("removed.log");
        write_fake_adb(&adb, "SERIALX tcp:9943 tcp:9943\n", &log);

        let ctx = ctx_with_adb(&root, adb, true);
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let s = seen.clone();
        let sink: EventSink = std::sync::Arc::new(move |ev| s.lock().unwrap().push(ev));

        let steps = |seen: &std::sync::Mutex<Vec<StageEvent>>| -> Vec<Option<String>> {
            seen.lock()
                .unwrap()
                .iter()
                .filter(|e| matches!(e, StageEvent::Line { .. }))
                .map(|e| e.step().map(str::to_string))
                .collect()
        };

        remove_adb_forwards(&ctx, &sink).await.unwrap();
        assert_eq!(steps(&seen), vec![Some(STEP.to_string())]);
        assert_eq!(STEP, "fix.remove-adb-forwards");

        seen.lock().unwrap().clear();
        remove_adb_forwards_at(&ctx, &sink, crate::events::step::RUN_ADB_FORWARDS)
            .await
            .unwrap();
        assert_eq!(steps(&seen), vec![Some("run.2.adb-forwards".to_string())]);

        std::fs::remove_dir_all(&root).ok();
    }

    /// A device that disappeared mid-removal used to be reported exactly like a
    /// clean forwarding table — "no stale adb port forwards to clear" — while
    /// the WiFi-breaking forward was still installed.
    #[tokio::test]
    async fn a_failed_removal_is_never_reported_as_a_clean_table() {
        let root = scratch("remove-fails");
        let adb = root.join("adb.sh");
        let log = root.join("removed.log");
        write_fake_adb_failing(
            &adb,
            "SERIALX tcp:9943 tcp:9943\nSERIALX tcp:9944 tcp:9944\n",
            0,
            1,
            &log,
        );

        let ctx = ctx_with_adb(&root, adb, false);
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let s = seen.clone();
        let sink: EventSink = std::sync::Arc::new(move |ev| s.lock().unwrap().push(ev));

        let report = remove_adb_forwards(&ctx, &sink).await.unwrap();
        assert!(!report.changed, "nothing was actually cleared");
        assert_ne!(report.description, "no stale adb port forwards to clear");
        assert!(
            report.description.contains("SERIALX tcp:9943"),
            "{report:?}"
        );
        assert!(
            report.description.contains("SERIALX tcp:9944"),
            "{report:?}"
        );

        let warns: Vec<String> = seen
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                StageEvent::Line { text, severity, .. }
                    if *severity == crate::events::Severity::Warn =>
                {
                    Some(text.clone())
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            warns.len(),
            2,
            "one warn per unremovable forward: {warns:?}"
        );
        assert!(warns[0].starts_with("could not clear adb forward tcp:9943 on SERIALX"));

        std::fs::remove_dir_all(&root).ok();
    }

    /// `adb forward --list` that cannot be answered is not an empty table.
    #[tokio::test]
    async fn a_query_failure_is_reported_as_a_query_failure() {
        let root = scratch("list-fails");
        let adb = root.join("adb.sh");
        let log = root.join("removed.log");
        write_fake_adb_failing(&adb, "", 1, 0, &log);

        let ctx = ctx_with_adb(&root, adb, false);
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let s = seen.clone();
        let sink: EventSink = std::sync::Arc::new(move |ev| s.lock().unwrap().push(ev));

        let report = remove_adb_forwards(&ctx, &sink).await.unwrap();
        assert!(!report.changed);
        assert!(
            report
                .description
                .starts_with("could not query adb forwards ("),
            "{report:?}"
        );
        assert!(
            report.description.contains("tcp:9943/tcp:9944"),
            "{report:?}"
        );
        assert!(!log.exists(), "a failed query must remove nothing");
        assert!(seen.lock().unwrap().iter().any(|e| matches!(
            e,
            StageEvent::Line { severity, .. } if *severity == crate::events::Severity::Warn
        )));

        std::fs::remove_dir_all(&root).ok();
    }

    /// An `adb` that cannot be spawned at all (deleted between the probe that
    /// found it and this fix, or not executable) is the other half of "adb
    /// could not tell us" — and the one the GUI must render.
    ///
    /// This pins the payload Doctor has to keep on screen: exactly one `warn`
    /// naming the query failure and the two ports that may still be installed,
    /// plus an `unchanged` report carrying the same text. Doctor currently
    /// records only `fatal` events and repaints the row from a fresh check
    /// pass, so this warn is dropped and the row goes green over a forwarding
    /// table nobody could read (review A4-5; the UI half is
    /// `ui/src/screens/Doctor.svelte`).
    #[tokio::test]
    async fn an_unspawnable_adb_warns_once_and_never_reports_a_clean_table() {
        let root = scratch("list-unspawnable");
        std::fs::create_dir_all(&root).unwrap();
        // Nothing is ever written here: the path does not exist.
        let adb = root.join("no-such-adb");
        assert!(!adb.exists());

        let ctx = ctx_with_adb(&root, adb, false);
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let s = seen.clone();
        let sink: EventSink = std::sync::Arc::new(move |ev| s.lock().unwrap().push(ev));

        let report = remove_adb_forwards(&ctx, &sink).await.unwrap();
        assert!(!report.changed);
        assert_ne!(report.description, "no stale adb port forwards to clear");
        assert!(
            report
                .description
                .starts_with("could not query adb forwards ("),
            "{report:?}"
        );
        assert!(
            report
                .description
                .ends_with("— stale tcp:9943/tcp:9944 forwards may still be installed"),
            "{report:?}"
        );

        let warns: Vec<String> = seen
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                StageEvent::Line { text, severity, .. }
                    if *severity == crate::events::Severity::Warn =>
                {
                    Some(text.clone())
                }
                _ => None,
            })
            .collect();
        assert_eq!(warns, vec![report.description.clone()], "{warns:?}");

        std::fs::remove_dir_all(&root).ok();
    }

    /// The `info` row is rendered in one place, so `sabrage-parity` can compare
    /// the live string against run.sh instead of a copy (A1-3).
    #[test]
    fn the_cleared_forward_line_is_the_one_renderer() {
        assert_eq!(
            cleared_forward_line("cleared", "tcp:9943", "SERIALX"),
            "cleared stale adb forward tcp:9943 on SERIALX (left over from a --wired launch — would otherwise break WiFi discovery)"
        );
        assert!(cleared_forward_line("would clear", "tcp:9944", "SerB")
            .starts_with("would clear stale adb forward tcp:9944 on SerB "));
    }

    /// The `--wired` session's own forwards: the standalone fix (a Doctor
    /// button) must refuse, while the launch path — which clears leftovers
    /// *before* a session exists — keeps working.
    #[tokio::test]
    async fn the_standalone_fix_refuses_during_a_live_session_but_the_launch_path_does_not() {
        let _g = crate::session::lock_session_globals();
        let root = scratch("live");
        let adb = root.join("adb.sh");
        let log = root.join("removed.log");
        write_fake_adb(&adb, "SERIALX tcp:9943 tcp:9943\n", &log);

        let ctx = ctx_with_adb(&root, adb, false);
        let state_path = ctx.paths.session_state_path();
        std::fs::create_dir_all(state_path.parent().unwrap()).unwrap();
        let mut state = crate::session::state::SessionState::new(
            uuid::Uuid::new_v4(),
            "FixtureBottle",
            "/bs",
            "/log",
            0,
        );
        state.wine = crate::process::ProcInfo::observe(std::process::id());
        std::fs::write(&state_path, serde_json::to_string(&state).unwrap()).unwrap();

        let sink: EventSink = ctx.sink.clone();
        let err = remove_adb_forwards(&ctx, &sink).await.unwrap_err();
        assert!(
            err.to_string()
                .starts_with("refusing to remove adb port forwards while a session is live"),
            "{err}"
        );
        assert!(!log.exists(), "adb must not be spawned while refusing");

        // The launch path is the same removal without the gate.
        let report = remove_adb_forwards_at(&ctx, &sink, crate::events::step::RUN_ADB_FORWARDS)
            .await
            .unwrap();
        assert!(report.changed);
        assert!(log.exists());

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn multiple_serials_are_each_targeted_independently() {
        let root = scratch("multi-serial");
        let adb = root.join("adb.sh");
        let log = root.join("removed.log");
        write_fake_adb(
            &adb,
            "SerA tcp:9943 tcp:9943\nSerB tcp:9944 tcp:9944\n",
            &log,
        );

        let ctx = ctx_with_adb(&root, adb, false);
        let sink: EventSink = ctx.sink.clone();
        let report = remove_adb_forwards(&ctx, &sink).await.unwrap();
        assert!(report.changed);

        let log_text = std::fs::read_to_string(&log).unwrap();
        let mut lines: Vec<&str> = log_text.lines().collect();
        lines.sort_unstable();
        assert_eq!(
            lines,
            vec![
                "SerA forward --remove tcp:9943",
                "SerB forward --remove tcp:9944"
            ]
        );

        std::fs::remove_dir_all(&root).ok();
    }
}
