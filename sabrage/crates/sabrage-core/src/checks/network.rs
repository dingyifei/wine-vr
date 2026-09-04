//! Doctor evaluators for the `network` group: `net.ports` and
//! `net.adb-forwards`, bound in contract order by [`defs`]. Message and
//! remedy strings track `scripts/demo/doctor.sh` sections 16/16b verbatim
//! (see [`super`] for why). Reference: scripts/demo/doctor.sh.
//!
//! One declared divergence (A4-4 / A3b packet): a failed `adb forward --list`
//! probe Warns here and taps `ok` in zsh. PARITY.md § Declared by the
//! 2026-08-30 adversarial review (round 1 fixes), "**`net.adb-forwards` on a
//! failed probe.**"

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use super::Evaluator;
#[allow(unused_imports)]
use super::{CheckCtx, CheckOutcome, CheckStatus, SkipReason};

const PROBES_DISABLED: &str = "adb probing disabled (Sabrage setting)";

/// The exact literal ports `scripts/demo/doctor.sh` section 16 passes. Pinned
/// to doctor.sh's text rather than derived from `contract().ports.stream`
/// (`[9943, 9944]` today), because doctor.sh does not derive it either.
const LSOF_ARGS: [&str; 3] = ["-nP", "-iUDP:9944", "-iTCP:9943"];

/// `COMMAND(PID)` for every `lsof` row on the streaming ports: deduplicated,
/// sorted, space-joined **with a trailing space** when non-empty (the caller
/// concatenates directly before doctor.sh's em dash). Empty when nothing is
/// listening or `lsof` cannot be spawned.
fn stale_listeners() -> String {
    let out = match Command::new("lsof").args(LSOF_ARGS).output() {
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

/// `net.ports`: Warn naming the busy listeners, Pass when [`stale_listeners`]
/// reports none — including when `lsof` cannot be spawned, matching doctor.sh.
/// Reference: scripts/demo/doctor.sh section 16.
/// tests::net_ports_matches_a_direct_lsof_probe
fn net_ports(_ctx: &CheckCtx) -> CheckOutcome {
    let stale = stale_listeners();
    if stale.is_empty() {
        CheckOutcome::pass("net.ports", "streaming ports free")
    } else {
        CheckOutcome::warn(
            "net.ports",
            format!("ports 9943/9944 busy: {stale}— a previous session may still be running"),
        )
    }
}

/// The local side (`tcp:<port>`) of every `adb forward --list` row.
///
/// # Errors
/// `Err` when `adb` cannot be spawned or exits non-zero — distinct from
/// `Ok(vec![])` (no forwards). Callers must not fold the two (A4-4).
/// tests::adb_forward_local_specs_reports_nonzero_exit_as_err
fn adb_forward_local_specs(adb: &Path) -> Result<Vec<String>, String> {
    let out = Command::new(adb)
        .args(["forward", "--list"])
        .output()
        .map_err(|e| format!("failed to run '{}': {e}", adb.display()))?;
    if !out.status.success() {
        return Err(format!(
            "'{} forward --list' exited with {}",
            adb.display(),
            out.status
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .map(str::to_string)
        .collect())
}

/// `net.adb-forwards`: Skipped when adb probes are disabled or no adb is
/// found, Warn when `tcp:9943`/`tcp:9944` are forwarded or the probe failed,
/// Pass otherwise — a row zsh does not print (PARITY.md § Doctor / checks,
/// "`net.adb-forwards` renders a green"). Reference:
/// scripts/demo/doctor.sh section 16b.
fn net_adb_forwards(ctx: &CheckCtx) -> CheckOutcome {
    if !ctx.opts.allow_adb_probes {
        return CheckOutcome::skipped("net.adb-forwards", SkipReason::new(PROBES_DISABLED));
    }
    let Some(adb) = ctx.paths.adb.as_deref() else {
        return CheckOutcome::skipped("net.adb-forwards", SkipReason::new("adb not found"));
    };
    let specs = match adb_forward_local_specs(adb) {
        Ok(specs) => specs,
        // A4-4 / A3b packet: a failed probe is not "no stale forwards"; stale
        // tcp:9943/9944 would silently break WiFi discovery, so never Pass here.
        // tests::net_adb_forwards_warns_not_passes_when_the_probe_cannot_spawn_adb
        Err(e) => {
            return CheckOutcome::warn(
                "net.adb-forwards",
                format!(
                    "could not query adb port forwards ({e}) — stale tcp:9943/tcp:9944 \
                     forwards may still be present and would break WiFi discovery; check \
                     manually with '{} forward --list'",
                    adb.display()
                ),
            )
            .with_detail(e);
        }
    };
    let stale = specs.iter().any(|s| s == "tcp:9943" || s == "tcp:9944");
    if stale {
        CheckOutcome::warn(
            "net.adb-forwards",
            "adb forward tcp:9943/tcp:9944 present — expected only for a wired launch \
             (--wired); stale forwards break WiFi discovery — remedy: adb forward --remove \
             tcp:9943 (and tcp:9944), or just a normal ./demo.sh run",
        )
    } else {
        CheckOutcome::pass("net.adb-forwards", "no stale adb port forwards")
    }
}

/// Evaluators this module binds, keyed by contract slug.
pub fn defs() -> Vec<(&'static str, Evaluator)> {
    vec![
        ("net.ports", net_ports as Evaluator),
        ("net.adb-forwards", net_adb_forwards as Evaluator),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::CheckOptions;
    use crate::paths::Paths;

    fn ctx() -> CheckCtx {
        CheckCtx::new(
            Paths::new("/nonexistent/sabrage-network-probe"),
            CheckOptions::new(),
        )
    }

    #[test]
    fn net_ports_matches_a_direct_lsof_probe() {
        let o = net_ports(&ctx());
        let stale = stale_listeners();
        if stale.is_empty() {
            assert_eq!(o.status, CheckStatus::Pass);
            assert_eq!(o.message, "streaming ports free");
        } else {
            assert_eq!(o.status, CheckStatus::Warn);
            assert_eq!(
                o.message,
                format!("ports 9943/9944 busy: {stale}— a previous session may still be running")
            );
        }
    }

    #[test]
    fn probes_disabled_skips_net_adb_forwards() {
        let opts = CheckOptions {
            allow_adb_probes: false,
            ..CheckOptions::new()
        };
        let ctx = CheckCtx::new(Paths::new("/nonexistent/sabrage-network-probe"), opts);
        assert_eq!(net_adb_forwards(&ctx).status, CheckStatus::Skipped);
    }

    #[test]
    fn no_adb_binary_skips_net_adb_forwards() {
        let c = ctx();
        if c.paths.adb.is_some() {
            return; // this machine has adb on PATH; nothing to assert here
        }
        assert_eq!(net_adb_forwards(&c).status, CheckStatus::Skipped);
    }

    #[test]
    fn net_adb_forwards_matches_a_direct_probe_when_adb_is_present() {
        let c = ctx();
        let Some(adb) = c.paths.adb.clone() else {
            return; // no adb on this machine; covered by the skip test above
        };
        let Ok(specs) = adb_forward_local_specs(&adb) else {
            return; // probe itself failed on this machine; covered below
        };
        let stale = specs.iter().any(|s| s == "tcp:9943" || s == "tcp:9944");
        let o = net_adb_forwards(&c);
        if stale {
            assert_eq!(o.status, CheckStatus::Warn);
        } else {
            assert_eq!(o.status, CheckStatus::Pass);
        }
    }

    /// A4-4: a failed adb probe must not read as "clean".
    #[test]
    fn net_adb_forwards_warns_not_passes_when_the_probe_cannot_spawn_adb() {
        let opts = CheckOptions::new();
        let mut c = CheckCtx::new(Paths::new("/nonexistent/sabrage-network-probe"), opts);
        c.paths.adb = Some(std::path::PathBuf::from(
            "/nonexistent/sabrage-network-probe/not-a-real-adb-binary",
        ));
        let o = net_adb_forwards(&c);
        assert_eq!(o.status, CheckStatus::Warn);
        assert!(
            o.message.contains("could not query adb port forwards"),
            "message must say the probe failed, not that forwards are clean: {}",
            o.message
        );
        assert!(
            o.message.contains("failed to run"),
            "the raw spawn-error text must reach the message verbatim: {}",
            o.message
        );
        assert_ne!(o.message, "no stale adb port forwards");
    }

    #[test]
    fn adb_forward_local_specs_reports_nonzero_exit_as_err() {
        // `false` always exits 1 without touching stdout — exercises the
        // exit-status branch, distinct from the spawn-failure branch that
        // net_adb_forwards_warns_not_passes_when_the_probe_cannot_spawn_adb covers.
        let false_bin = Path::new("/usr/bin/false");
        if !false_bin.is_file() {
            // not present on this machine; the Result plumbing is covered by
            // net_adb_forwards_warns_not_passes_when_the_probe_cannot_spawn_adb.
            return;
        }
        let err = adb_forward_local_specs(false_bin).expect_err("non-zero exit must be Err");
        assert!(err.contains("exited with"), "unexpected error text: {err}");
    }
}
