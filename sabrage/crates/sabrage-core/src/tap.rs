//! The parity tap channel: `"<slug> <status>"`, one line per check.
//!
//! The channel carries slug and status only (see scripts/demo/lib.sh, `tap()`),
//! never prose, which is why check message text can stay implementation-owned;
//! the tier-2 live differ (`scripts/dev/parity.sh`) compares it between a zsh
//! doctor run and a native one.
//!
//! Status vocabulary is fixed by the zsh side: `ok`, `warn`, `fail`, `info`,
//! `skipped`. Nothing else may appear on this channel
//! (tests::words_match_the_zsh_vocabulary).

use std::io::Write;
use std::path::Path;

use crate::checks::{CheckOutcome, CheckStatus};

/// The tap word for a status.
///
/// [`CheckStatus::NotImplemented`] maps to `skipped` so an unbound slug reads as
/// a row that did not run and the differ reports a real mismatch against a zsh
/// `ok`/`fail` instead of silently agreeing (tests::words_match_the_zsh_vocabulary).
pub fn tap_word(status: CheckStatus) -> &'static str {
    match status {
        CheckStatus::Pass => "ok",
        CheckStatus::Warn => "warn",
        CheckStatus::Fail => "fail",
        CheckStatus::Info => "info",
        CheckStatus::Skipped => "skipped",
        CheckStatus::NotImplemented => "skipped",
    }
}

/// One tap line, without the trailing newline: `"<slug> <status>"`.
pub fn tap_line(outcome: &CheckOutcome) -> String {
    format!("{} {}", outcome.slug, tap_word(outcome.status))
}

/// The whole tap payload: one line per outcome, each newline-terminated
/// (`print -r --` appends a newline per row).
pub fn render_tap<'a>(outcomes: impl IntoIterator<Item = &'a CheckOutcome>) -> String {
    let mut out = String::new();
    for o in outcomes {
        out.push_str(&tap_line(o));
        out.push('\n');
    }
    out
}

/// Append the tap payload to `path`, matching zsh's `>>` (the differ runs both
/// sides into fresh files, and appending keeps repeated sections additive).
///
/// This is a *renderer* API, called by the CLI after a run — never from check
/// code, which must not touch the filesystem.
pub fn append_tap<'a>(
    path: &Path,
    outcomes: impl IntoIterator<Item = &'a CheckOutcome>,
) -> std::io::Result<()> {
    let payload = render_tap(outcomes);
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    f.write_all(payload.as_bytes())
}

#[cfg(test)]
mod tests;
