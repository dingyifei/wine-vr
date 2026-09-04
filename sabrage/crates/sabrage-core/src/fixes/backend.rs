//! `fix.set-graphics-backend` — force `CX_GRAPHICS_BACKEND` to `dxmt` in the
//! bottle's `cxbottle.conf`.
//!
//! The CrossOver GUI writes `""` (= auto), which does not select DXMT: the game
//! spins forever before D3D11 device creation, with no DXMT banner and no
//! streamer (`docs/troubleshooting.md`). The edit is a permanent mutation that
//! is never unwound (design-core §3.2).
//!
//! Reference: `scripts/demo/run.sh`.

use std::path::Path;

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use crate::error::Result;
use crate::fixes::FixReport;
use crate::stages::{require_bottle, EventSink, StageCtx};

/// The exact line doctor's `bottle.gfx-dxmt` greps for, anchored at both ends
/// (`^"CX_GRAPHICS_BACKEND" = "dxmt"$`). `pub(crate)` so
/// `checks::bottle::bottle_gfx_dxmt` compares against this literal instead of
/// its own copy — one byte-critical string, not two kept in sync by hand.
pub(crate) const TARGET_LINE: &str = "\"CX_GRAPHICS_BACKEND\" = \"dxmt\"";
/// `run.sh`'s prefix test for "some `CX_GRAPHICS_BACKEND` line exists, whatever
/// its value" (`grep -q '^"CX_GRAPHICS_BACKEND"'`).
const KEY_PREFIX: &str = "\"CX_GRAPHICS_BACKEND\"";
/// Everything up to and including the opening quote of the value, for the
/// per-line rewrite test (`^"CX_GRAPHICS_BACKEND" = ".*"$`).
const VALUE_PREFIX: &str = "\"CX_GRAPHICS_BACKEND\" = \"";
const ENV_SECTION_HEADER: &str = "[EnvironmentVariables]";

/// The console text `run.sh` prints after a successful edit, verbatim. Also
/// [`FixReport::changed`]'s description, so a CLI renderer and a structured
/// `AutoFixed` consumer show the same words.
const FORCED_DESCRIPTION: &str =
    "bottle graphics backend forced to dxmt (was auto/other — the CrossOver GUI can reset this)";

/// Which of [`rewrite_graphics_backend`]'s three branches it took.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Branch {
    /// An existing `"CX_GRAPHICS_BACKEND" = "..."` line was rewritten in place.
    Rewrote,
    /// No line started with the key at all, but `[EnvironmentVariables]` did
    /// exist; the target line was inserted immediately after that header.
    InsertedAfterEnvSection,
    /// Neither existed: a new `[EnvironmentVariables]` section was appended.
    AppendedSection,
}

/// Introduce `TARGET_LINE` into a `cxbottle.conf` body; returns the new bytes
/// and which branch produced them. Callers have established the line is not
/// present verbatim — this only decides how to introduce it.
///
/// Rewrite and insert branches are line-oriented: every untouched line and the
/// trailing-newline state survive. Append is a raw concatenation.
///
/// Like `sed`, the rewrite branch only touches a `CX_GRAPHICS_BACKEND` line
/// shaped `"CX_GRAPHICS_BACKEND" = "..."`; any other shape is left untouched
/// and the caller does not re-verify, so the result can lack the target line
/// (tests::branch_rewrite_cases).
///
/// Not byte-parity with BSD `sed` in one cell: with `[EnvironmentVariables]`
/// as the last line and no trailing newline, sed's `a\` concatenates onto the
/// header; this joins them with a real line break and keeps the newline absent
/// (tests::branch_rewrite_cases).
pub fn rewrite_graphics_backend(conf: &str) -> (String, Branch) {
    if conf.lines().any(|l| l.starts_with(KEY_PREFIX)) {
        let (mut lines, trailing_newline) = split_lines(conf);
        for line in lines.iter_mut() {
            if let Some(rest) = line.strip_prefix(VALUE_PREFIX) {
                if rest.ends_with('"') {
                    *line = TARGET_LINE.to_string();
                }
            }
        }
        return (rejoin(&lines, trailing_newline), Branch::Rewrote);
    }

    if conf.lines().any(|l| l == ENV_SECTION_HEADER) {
        let (lines, trailing_newline) = split_lines(conf);
        let mut out = Vec::with_capacity(lines.len() + 1);
        for line in lines {
            let is_header = line == ENV_SECTION_HEADER;
            out.push(line);
            if is_header {
                out.push(TARGET_LINE.to_string());
            }
        }
        return (
            rejoin(&out, trailing_newline),
            Branch::InsertedAfterEnvSection,
        );
    }

    // A raw append, indifferent to whether `conf` already ended in a newline.
    let appended = format!("{conf}\n{ENV_SECTION_HEADER}\n{TARGET_LINE}\n");
    (appended, Branch::AppendedSection)
}

fn split_lines(conf: &str) -> (Vec<String>, bool) {
    let trailing_newline = conf.ends_with('\n');
    (conf.lines().map(str::to_string).collect(), trailing_newline)
}

fn rejoin(lines: &[String], trailing_newline: bool) -> String {
    let mut out = lines.join("\n");
    if trailing_newline {
        out.push('\n');
    }
    out
}

/// One running process whose resolved executable is a CrossOver `wineserver`.
struct WineserverProc {
    /// The `WINEPREFIX` value from its environment. `None` covers both "the
    /// variable was absent" and "the environment could not be read at all" —
    /// no caller here needs to tell those apart, since both mean "cannot
    /// positively rule this process out."
    wineprefix: Option<String>,
}

/// Every live process whose resolved executable equals `wineserver_exe`,
/// canonicalized on both sides like [`crate::process::find_processes_by_exe`]
/// (not reused: it skips `environ`, and a second full scan would repeat the
/// same syscalls).
fn scan_wineservers(wineserver_exe: &Path) -> Vec<WineserverProc> {
    let want = wineserver_exe
        .canonicalize()
        .unwrap_or_else(|_| wineserver_exe.to_path_buf());
    let refresh = ProcessRefreshKind::nothing()
        .with_exe(UpdateKind::Always)
        .with_environ(UpdateKind::Always);
    // `System::new()` loads nothing; `new_with_specifics` would walk the whole
    // process table once more before the explicit scan below.
    let mut sys = System::new();
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh);

    sys.processes()
        .values()
        .filter_map(|proc_| {
            let exe = proc_.exe()?;
            let resolved = exe.canonicalize().unwrap_or_else(|_| exe.to_path_buf());
            if resolved != want {
                return None;
            }
            let wineprefix = proc_.environ().iter().find_map(|kv| {
                kv.to_str()
                    .and_then(|s| s.strip_prefix("WINEPREFIX="))
                    .map(str::to_string)
            });
            Some(WineserverProc { wineprefix })
        })
        .collect()
}

/// The `bottle_wineserver_is_live` decision as a pure function of the
/// `WINEPREFIX` values observed on live wineserver processes; `None` (absent or
/// unreadable environment) cannot be ruled out and counts as live.
///
/// Separate from `scan_wineservers` so the "when in doubt, refuse" rule has a
/// test independent of system-wide process state
/// (tests::wineservers_indicate_live_decides_by_wineprefix).
fn wineservers_indicate_live(observed_wineprefixes: &[Option<String>], want_prefix: &str) -> bool {
    observed_wineprefixes.iter().any(|wp| match wp.as_deref() {
        Some(p) => p == want_prefix,
        None => true,
    })
}

/// Whether a CrossOver wineserver appears to be alive **for `bottle_prefix`
/// specifically**, read from process state because `wineserver -w` would block
/// indefinitely against a live server: every process whose resolved executable
/// is `wineserver_exe`, matched on its `WINEPREFIX` environment variable, compared as an exact string — `run.sh` and this crate's launch code both set it verbatim to the bottle prefix, neither side canonicalizes.
///
/// A matching process whose `WINEPREFIX` cannot be read, or that lacks it, is
/// treated as live: a false "clear" would let this fix edit a file the CrossOver
/// GUI still has open (tests::wineservers_indicate_live_decides_by_wineprefix).
pub(crate) fn bottle_wineserver_is_live(wineserver_exe: &Path, bottle_prefix: &Path) -> bool {
    let observed: Vec<Option<String>> = scan_wineservers(wineserver_exe)
        .into_iter()
        .map(|p| p.wineprefix)
        .collect();
    wineservers_indicate_live(&observed, &bottle_prefix.to_string_lossy())
}

/// Whether **any** CrossOver wineserver is alive, for callers with no single
/// bottle to narrow the probe against (ALVR's `session.json` is machine-global,
/// not scoped to one bottle — see [`crate::fixes::session_json`]).
pub(crate) fn any_wineserver_alive(wineserver_exe: &Path) -> bool {
    !scan_wineservers(wineserver_exe).is_empty()
}

/// What the shared body does when the bottle's own wineserver is alive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WineserverPolicy {
    /// Refuse with a Fatal — [`set_graphics_backend`], the doctor Fix button.
    Refuse,
    /// Edit anyway — [`set_graphics_backend_for_launch`]. See its doc for why
    /// that is safe there and only there.
    EditAnyway,
}

/// Rewrite the bottle's graphics backend to `dxmt`.
///
/// Refuses while the bottle's wineserver is live: a standalone fix (the doctor
/// Fix button, `sabrage fix set-graphics-backend`) races the CrossOver GUI,
/// which rewrites `cxbottle.conf` from memory on exit and would silently
/// clobber the edit. The launch preflight uses
/// [`set_graphics_backend_for_launch`] instead.
pub async fn set_graphics_backend(ctx: &StageCtx, sink: &EventSink) -> Result<FixReport> {
    rewrite(ctx, sink, WineserverPolicy::Refuse).await
}

/// [`set_graphics_backend`] **without** the live-wineserver refusal — the
/// launch preflight's variant. Same three-branch rewrite, same `FixReport`,
/// same console text; only the liveness gate differs.
///
/// The refusal protects an edit that must survive alongside a running CrossOver.
/// A launch's edit does not: run.sh's `wineserver-reset` kills that wineserver
/// before anything reads the file again, so refusing here would block
/// `./demo.sh run` after a crashed session
/// (tests::for_launch_edits_even_while_the_bottles_wineserver_is_live).
pub async fn set_graphics_backend_for_launch(
    ctx: &StageCtx,
    sink: &EventSink,
) -> Result<FixReport> {
    rewrite(ctx, sink, WineserverPolicy::EditAnyway).await
}

/// The body both entry points share.
async fn rewrite(ctx: &StageCtx, sink: &EventSink, policy: WineserverPolicy) -> Result<FixReport> {
    let bottle = require_bottle(ctx)?;
    let conf_path = bottle.conf_path();
    let conf = std::fs::read_to_string(&conf_path)
        .map_err(|e| crate::error::SabrageError::io(&conf_path, e))?;

    if conf.lines().any(|l| l == TARGET_LINE) {
        return Ok(FixReport::unchanged(
            crate::fixes::FixAction::SetGraphicsBackend,
            format!("{} already forces dxmt", ctx.paths.rel_display(&conf_path)),
        ));
    }

    if policy == WineserverPolicy::Refuse {
        if let Some(wineserver) = &ctx.paths.wineserver {
            if bottle_wineserver_is_live(wineserver, &bottle.prefix) {
                return Err(ctx.fatal(
                    format!(
                        "refusing to edit {} while bottle '{}' has a live wineserver — CrossOver \
                         may rewrite this file from memory on exit and clobber the change; stop \
                         the session first",
                        conf_path.display(),
                        bottle.name
                    ),
                    Some(format!("./demo.sh stop --bottle {}", bottle.name)),
                ));
            }
        }
    }

    let (rewritten, _branch) = rewrite_graphics_backend(&conf);

    // The sed-faithful rewrite branch can return bytes without the target line, so
    // verify it before writing — else the fix claims a success doctor still fails
    // (tests::a_line_the_rewrite_cannot_canonicalize_is_a_failure_not_a_success).
    if !rewritten.lines().any(|l| l == TARGET_LINE) {
        return Err(ctx.fatal(
            format!(
                "could not force graphics backend to dxmt in {}",
                conf_path.display()
            ),
            None,
        ));
    }

    ctx.executor
        .write_atomic(&conf_path, rewritten.as_bytes())
        .await?;

    sink(crate::events::StageEvent::ok(
        ctx.run_id,
        None,
        FORCED_DESCRIPTION,
    ));
    Ok(FixReport::changed(
        crate::fixes::FixAction::SetGraphicsBackend,
        FORCED_DESCRIPTION,
    ))
}

#[cfg(test)]
mod tests;
