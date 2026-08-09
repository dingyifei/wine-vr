//! The parity tap channel: `"<slug> <status>"`, one line per check.
//!
//! lib.sh:
//!
//! ```zsh
//! tap() { # slug status
//!   [ -n "${WINEVR_DOCTOR_TAP:-}" ] && print -r -- "$1 $2" >> "$WINEVR_DOCTOR_TAP"
//!   :
//! }
//! ```
//!
//! This is what the tier-2 live differ (`scripts/dev/parity.sh`) compares between
//! a zsh doctor run and a native one. It carries **slug + status only** — never
//! prose — which is exactly why check message text can stay impl-owned.
//!
//! Status vocabulary is fixed by the zsh side: `ok`, `warn`, `fail`, `info`,
//! `skipped`. Nothing else may appear on this channel.

use std::io::Write;
use std::path::Path;

use crate::checks::{CheckOutcome, CheckStatus};

/// The tap word for a status.
///
/// [`CheckStatus::NotImplemented`] maps to `skipped` because the differ's
/// vocabulary is the zsh one and has no fourth state. That is deliberately the
/// *honest* mapping while Phase 1 is in flight: an unbound slug behaves like a
/// row that did not run, so the differ reports a real mismatch against a zsh
/// `ok`/`fail` instead of silently agreeing. When every evaluator is bound this
/// arm becomes unreachable.
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
mod tests {
    use super::*;

    #[test]
    fn words_match_the_zsh_vocabulary() {
        assert_eq!(tap_word(CheckStatus::Pass), "ok");
        assert_eq!(tap_word(CheckStatus::Warn), "warn");
        assert_eq!(tap_word(CheckStatus::Fail), "fail");
        assert_eq!(tap_word(CheckStatus::Info), "info");
        assert_eq!(tap_word(CheckStatus::Skipped), "skipped");
        assert_eq!(tap_word(CheckStatus::NotImplemented), "skipped");
    }

    #[test]
    fn renders_one_line_per_outcome() {
        let outcomes = vec![
            CheckOutcome::pass("sys.arch", "Apple Silicon"),
            CheckOutcome::fail(
                "cx.version",
                "CrossOver 26.1 < 26.2",
                "upgrade CrossOver to 26.2+",
            ),
            CheckOutcome::skipped("hs.client", "no adb device".into()),
        ];
        assert_eq!(
            render_tap(&outcomes),
            "sys.arch ok\ncx.version fail\nhs.client skipped\n"
        );
    }
}
