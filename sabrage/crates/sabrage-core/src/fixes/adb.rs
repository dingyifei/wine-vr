//! `fix.remove-adb-forwards` — drop the two `--wired` stream port forwards.
//!
//! Leftover `tcp:9943`/`tcp:9944` forwards persist across sessions and silently
//! break WiFi discovery ("searching for streamer"), so a normal (non-wired) run
//! clears exactly those, per serial (`adb -s <serial> forward --remove`), never
//! with `--remove-all` — distinct from the `adb reverse --remove-all` that
//! run.sh does use: PARITY.md § Invariants that must NOT change (byte/behavior
//! parity), "adb `forward --remove` per-serial". The port pair comes from the
//! contract (`ports.stream`), never a literal; an absent `paths.adb` is
//! "nothing to do", not an error. Reference: `scripts/demo/run.sh`.
//!
//! Where the shell is silent about a removal that failed, a [`FixReport`] warns
//! and names what may still be installed: PARITY.md § Declared by the
//! 2026-08-30 adversarial review (round 1 fixes), "`net.adb-forwards` on a
//! failed probe". Enforced by
//! tests::removes_exactly_the_two_stale_ports_per_serial_never_remove_all and
//! tests::a_failed_removal_is_never_reported_as_a_clean_table.

use std::path::Path;
use std::time::Duration;

use crate::contract::contract;
use crate::error::Result;
use crate::events::{StageEvent, StepId};
use crate::fixes::{FixAction, FixReport};
use crate::stages::{EventSink, StageCtx};

/// Bound on `adb forward --list`: a cold run starts adb's background server and
/// can block for seconds. A timeout is a query failure, never an empty table;
/// the spawn must stay `tokio::process` — `timeout` cannot bound a blocking one.
const ADB_LIST_TIMEOUT: Duration = Duration::from_secs(5);

/// This fix's own step id, used when it runs as a fix (doctor's fix list or
/// `fixes::apply`). The launch path must not use it: it passes
/// [`crate::events::step::RUN_ADB_FORWARDS`] to [`remove_adb_forwards_at`] so
/// its rows sort and group with the run stage's
/// (tests::the_step_id_is_the_fixs_own_by_default_and_the_callers_with_at).
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

/// `adb forward --list` as `(serial, local)` rows. A read-only query, so it
/// bypasses the executor: dry-run gating only matters for mutations.
///
/// # Errors
/// `Err(reason)` when the query could not be answered — a spawn failure,
/// [`ADB_LIST_TIMEOUT`], or a non-zero `adb`. Never folded into an empty list:
/// "adb could not tell us" and "there is nothing to clear" are different facts
/// (tests::a_query_failure_is_reported_as_a_query_failure).
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

/// run.sh's `info` row for one cleared forward. `verb` is `"cleared"` for a
/// real removal and `"would clear"` for a dry run (Sabrage-only; the shell has
/// no dry run) (tests::the_cleared_forward_line_is_the_one_renderer).
///
/// `pub` for A1-3, so `sabrage-parity` can pin this live literal rather than a
/// fragment copied into the parity crate: CI's tier 1 runs `-p sabrage-parity
/// -p sabrage-contract-gen` only, so this module's own frozen-text test does
/// not gate a native literal edited without touching run.sh.
pub fn cleared_forward_line(verb: &str, local: &str, serial: &str) -> String {
    format!(
        "{verb} stale adb forward {local} on {serial} (left over from a --wired launch — would \
         otherwise break WiFi discovery)"
    )
}

/// The `tcp:<port>` local-forward specs this fix targets, from the contract's
/// `[ports] stream` — never a literal `"tcp:9943"`.
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
/// # Errors
/// Refuses while a session is live — duplicating [`crate::fixes::apply`]'s
/// gate, because this function is also reachable directly: a `--wired` session
/// streams over these very forwards, and doctor offers this remedy without
/// knowing the launch's `wired` state. The launch path calls
/// [`remove_adb_forwards_at`], which is not gated
/// (tests::the_standalone_fix_refuses_during_a_live_session_but_the_launch_path_does_not).
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

/// The same removal as [`remove_adb_forwards`], stamped with a caller-supplied
/// step id and without the live-session gate: the launch path clears leftovers
/// before the session exists ([`crate::stages::run::actions::adb_forward_hygiene`];
/// tests::the_step_id_is_the_fixs_own_by_default_and_the_callers_with_at).
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
        // Never `--remove-all` here. A non-zero exit comes back `Ok`, matching
        // the shell's tolerant `&&`, but the pair is remembered so the report
        // cannot claim a clean table (tests::a_failed_removal_is_never_reported_as_a_clean_table).
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
mod tests;
