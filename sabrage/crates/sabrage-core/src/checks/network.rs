//! Group `network` — doctor.sh section 16, 16b: streaming ports and stale adb forwards.
//!
//! Slugs owned here, in contract order:
//!
//! * `net.ports` — nothing already listening on the `[ports] stream` pair.
//!   Volatile
//! * `net.adb-forwards` — no `tcp:9943`/`tcp:9944` in `adb forward --list` —
//!   legitimate only for a `--wired` launch; left behind they squat the ports
//!   and break WiFi discovery. Volatile; silent-when-clean in zsh (`tap
//!   net.adb-forwards ok`)
//!
//! Every evaluator is `fn(&CheckCtx) -> CheckOutcome`: a **read-only probe**.
//! Message and remedy strings must match `scripts/demo/doctor.sh` verbatim,
//! with one recorded exception (below).
//!
//! ## `net.adb-forwards` failed-probe divergence (A4-4)
//!
//! doctor.sh discards adb's stderr and ignores its exit status
//! (`FWD="$("$ADB" forward --list 2>/dev/null | awk …)"`), so a probe that
//! *failed* produces an empty `$FWD` and taps `ok` — "no stale forwards" —
//! silently. [`net_adb_forwards`] Warns instead: a failed query is not
//! evidence that `tcp:9943`/`tcp:9944` are absent, and left behind they break
//! WiFi discovery. That makes the two doctors disagree on this slug's tap
//! channel (`ok` vs `warn`) for one machine state — adb present, its server
//! unreachable — which `scripts/dev/parity.sh` tier-2 diffs, and prints a
//! console row doctor.sh never emits. Needs either a matching
//! `scripts/demo/doctor.sh` change (an exit-status test on the `adb forward
//! --list` pipeline) or a `sabrage/PARITY.md` row declaring the divergence
//! (cross-area — this module cannot make either edit).

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use super::Evaluator;
#[allow(unused_imports)]
use super::{CheckCtx, CheckOutcome, CheckStatus, SkipReason};

const PROBES_DISABLED: &str = "adb probing disabled (Sabrage setting)";

// ── section 16: stale streaming listeners ────────────────────────────────────

/// `lsof -nP -iUDP:9944 -iTCP:9943` — the exact literal ports doctor.sh's
/// (non-generated) reference passes; they are also `contract().ports.stream`
/// today (`[9943, 9944]`), but this call is pinned to doctor.sh's own literal
/// text rather than re-derived, since doctor.sh itself does not derive it.
const LSOF_ARGS: [&str; 3] = ["-nP", "-iUDP:9944", "-iTCP:9943"];

/// `awk 'NR>1{print $1"("$2")"}' | sort -u | tr '\n' ' '` over `lsof`'s
/// output: for every row after the header, `COMMAND(PID)`, deduplicated,
/// sorted, space-joined **with a trailing space** when non-empty — the same
/// trailing-space quirk `cfg.session-pins` reproduces, so the caller can
/// concatenate this directly before an em dash with no extra space.
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

/// doctor.sh section 16:
/// ```sh
/// STALE="$(lsof -nP -iUDP:9944 -iTCP:9943 2>/dev/null | awk '…' | sort -u | tr '\n' ' ')"
/// if [ -n "$STALE" ]; then chk warn net.ports "ports 9943/9944 busy: $STALE— a previous session may still be running"
/// else chk ok net.ports "streaming ports free"; fi
/// ```
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

// ── section 16b: stale adb forwards ──────────────────────────────────────────

/// `"$ADB" forward --list 2>/dev/null | awk '{print $2}'` — the local side
/// (`tcp:<port>`) of every forward, one per line of `adb forward --list`'s
/// `<serial> <local> <remote>` rows.
///
/// `Err` means the probe itself failed (couldn't spawn `adb`, or `adb`
/// exited non-zero) — distinct from `Ok(vec![])`, which means the probe
/// ran cleanly and genuinely found no forwards. Callers must not fold the
/// two together: a failed probe is not evidence of a clean state (A4-4 /
/// A3b packet — the previous `Vec::new()`-on-any-error return made an ADB
/// query failure indistinguishable from "no forwards", so `net_adb_forwards`
/// reported Pass on a broken probe).
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

/// doctor.sh section 16b:
/// ```sh
/// if [ -n "$ADB" ]; then
///   FWD="$("$ADB" forward --list 2>/dev/null | awk '{print $2}')"
///   if print -r -- "$FWD" | grep -qx 'tcp:9943' || print -r -- "$FWD" | grep -qx 'tcp:9944'; then
///     chk warn net.adb-forwards …
///   else tap net.adb-forwards ok; fi
/// else tap net.adb-forwards skipped; fi
/// ```
fn net_adb_forwards(ctx: &CheckCtx) -> CheckOutcome {
    if !ctx.opts.allow_adb_probes {
        return CheckOutcome::skipped("net.adb-forwards", SkipReason::new(PROBES_DISABLED));
    }
    let Some(adb) = ctx.paths.adb.as_deref() else {
        return CheckOutcome::skipped("net.adb-forwards", SkipReason::new("adb not found"));
    };
    let specs = match adb_forward_local_specs(adb) {
        Ok(specs) => specs,
        // A4-4 / A3b packet: a failed probe is not "no stale forwards" —
        // stale tcp:9943/tcp:9944 forwards may still be present and would
        // silently break WiFi discovery, so this must not resolve to Pass.
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
        // Silent `tap … ok` in the shell.
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

    // ── net.ports ────────────────────────────────────────────────────────────

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

    // ── net.adb-forwards ─────────────────────────────────────────────────────

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

    // ── A4-4 / A3b packet: a failed adb probe must not read as "clean" ────────

    #[test]
    fn adb_forward_local_specs_reports_spawn_failure_as_err() {
        let bogus = Path::new("/nonexistent/sabrage-network-probe/not-a-real-adb-binary");
        let err = adb_forward_local_specs(bogus).expect_err("spawn must fail for a missing binary");
        assert!(
            err.contains("failed to run"),
            "unexpected error text: {err}"
        );
    }

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
        assert_ne!(o.message, "no stale adb port forwards");
    }

    #[test]
    fn adb_forward_local_specs_reports_nonzero_exit_as_err() {
        // `false` always exits 1 without touching stdout — exercises the
        // exit-status branch distinctly from the spawn-failure branch above.
        let false_bin = Path::new("/usr/bin/false");
        if !false_bin.is_file() {
            return; // not present on this machine; the spawn-failure test covers the Result plumbing
        }
        let err = adb_forward_local_specs(false_bin).expect_err("non-zero exit must be Err");
        assert!(err.contains("exited with"), "unexpected error text: {err}");
    }

    #[test]
    fn defs_binds_both_slugs_in_contract_order() {
        let slugs: Vec<&str> = defs().into_iter().map(|(s, _)| s).collect();
        assert_eq!(slugs, vec!["net.ports", "net.adb-forwards"]);
    }
}
