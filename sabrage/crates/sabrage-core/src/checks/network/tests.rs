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
