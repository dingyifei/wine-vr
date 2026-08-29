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
//! die) — reproduced here by simply not emitting a row for that pair.

use std::path::Path;
use std::time::Duration;

use crate::contract::contract;
use crate::error::Result;
use crate::events::{StageEvent, StepId};
use crate::fixes::{FixAction, FixReport};
use crate::stages::{EventSink, StageCtx};

/// `adb forward --list` starts adb's background server on a cold run, which can
/// block for a few seconds — long enough that a bare `.output().await` on the
/// async worker is worth bounding. A timeout here degrades to "nothing to
/// clear" (see [`list_forwards`]), the same as any other query failure.
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
/// for seconds — a hang here degrades to "nothing to clear", the same
/// no-forwards-found shape a genuine empty list produces.
async fn list_forwards(adb: &Path) -> Vec<(String, String)> {
    let output = tokio::time::timeout(
        ADB_LIST_TIMEOUT,
        tokio::process::Command::new(adb)
            .args(["forward", "--list"])
            .output(),
    )
    .await;
    match output {
        Ok(Ok(out)) => parse_forward_list(&String::from_utf8_lossy(&out.stdout)),
        Ok(Err(_)) | Err(_) => Vec::new(),
    }
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
pub async fn remove_adb_forwards(ctx: &StageCtx, sink: &EventSink) -> Result<FixReport> {
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

    let mut cleared: Vec<String> = Vec::new();
    for (serial, local) in list_forwards(&adb).await {
        if !stale_locals.contains(&local) {
            continue;
        }
        let spec = ctx
            .child(adb.clone(), step)
            .args(["-s", &serial, "forward", "--remove", &local]);
        // NEVER `--remove-all` here — see this module's header. `run_child`
        // reports a non-zero exit as `Ok`, matching the shell's tolerant `&&`
        // (a failed removal prints nothing and is not an error).
        let status = executor.run_child(&spec).await?;
        if !status.success() {
            continue;
        }
        let verb = if dry_run { "would clear" } else { "cleared" };
        let line = format!(
            "{verb} stale adb forward {local} on {serial} (left over from a --wired launch — \
             would otherwise break WiFi discovery)"
        );
        sink(StageEvent::info(ctx.run_id, Some(step), line.clone()));
        cleared.push(line);
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
    fn parse_forward_list_splits_serial_and_local() {
        let rows = parse_forward_list(
            "192.168.1.5:5555 tcp:9943 tcp:9943\n192.168.1.5:5555 tcp:9944 tcp:9944\n",
        );
        assert_eq!(
            rows,
            vec![
                ("192.168.1.5:5555".to_string(), "tcp:9943".to_string()),
                ("192.168.1.5:5555".to_string(), "tcp:9944".to_string()),
            ]
        );
    }

    #[test]
    fn parse_forward_list_ignores_blank_and_short_lines() {
        assert_eq!(parse_forward_list(""), Vec::<(String, String)>::new());
        assert_eq!(parse_forward_list("\n\n"), Vec::<(String, String)>::new());
        assert_eq!(
            parse_forward_list("onlyone\nser tcp:1 tcp:2\n"),
            vec![("ser".to_string(), "tcp:1".to_string())]
        );
    }

    #[test]
    fn stale_local_specs_come_from_the_contract_ports() {
        let specs = stale_local_specs();
        assert_eq!(specs, vec!["tcp:9943".to_string(), "tcp:9944".to_string()]);
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

    fn ctx_with_adb(root: &Path, adb: PathBuf, dry_run: bool) -> StageCtx {
        let mut paths = Paths::new(root);
        paths.adb = Some(adb);
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
        assert!(ctx.executor.is_dry_run());
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
