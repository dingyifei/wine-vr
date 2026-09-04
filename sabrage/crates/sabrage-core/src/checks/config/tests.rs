use super::*;
use crate::checks::CheckOptions;
use crate::paths::Paths;
use std::fs;
use std::path::PathBuf;

fn scratch(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("sabrage-config-test-{}-{tag}", std::process::id()))
}

fn ctx_with_toml(tmp: &Path, toml_path: PathBuf) -> CheckCtx {
    let mut paths = Paths::new(tmp);
    paths.toml_path = toml_path;
    CheckCtx::new(paths, CheckOptions::new())
}

#[test]
fn parse_protocol_matches_the_awk_recipe() {
    assert_eq!(parse_protocol("protocol = \"alvr\"\n"), "alvr");
    assert_eq!(parse_protocol("  protocol=\"oxrsys\"\n"), "oxrsys");
    assert_eq!(parse_protocol("# protocol = \"alvr\"\n"), "");
    assert_eq!(parse_protocol("protocol_extra = \"alvr\"\n"), "");
    assert_eq!(parse_protocol("protocol=alvr\n"), "");
    assert_eq!(
        parse_protocol("video_codec = \"h264\"\nprotocol = \"alvr\"\n"),
        "alvr"
    );
    // Last match wins, like the runtime's line reader and doctor.sh's
    // `{v=$2} END{print v}` awk (not `exit` after the first match).
    assert_eq!(
        parse_protocol("protocol = \"first\"\nprotocol = \"second\"\n"),
        "second"
    );
    // Table-blind: a later occurrence under `[streaming]` still wins over
    // an earlier root-level one.
    assert_eq!(
        parse_protocol("protocol = \"alvr\"\n\n[streaming]\nprotocol = \"oxrsys\"\n"),
        "oxrsys"
    );
    assert_eq!(parse_protocol(""), "");
}

#[test]
fn missing_toml_fails_supported_and_skips_legacy() {
    let tmp = scratch("missing-toml");
    let toml_path = tmp.join("OXRSys/oxrsys-runtime.toml");
    let ctx = ctx_with_toml(&tmp, toml_path.clone());

    let supported = cfg_protocol_supported(&ctx);
    assert_eq!(supported.status, CheckStatus::Fail);
    assert_eq!(
        supported.message,
        format!("{} missing", toml_path.display())
    );
    assert_eq!(supported.remedy.as_deref(), Some("./demo.sh setup"));

    let legacy = cfg_protocol_legacy_oxrsys(&ctx);
    assert_eq!(legacy.status, CheckStatus::Skipped);
}

/// A3b-1 regression: `protocol = "alvr"` shadowed by a later
/// `[streaming] protocol = "oxrsys"` must resolve like the runtime (last
/// assignment wins, table-blind) — Fail on `cfg.protocol.legacy-oxrsys`,
/// not the false-green Pass a first-match reader would give.
#[test]
fn shadowed_protocol_alvr_then_oxrsys_resolves_to_the_last_assignment() {
    let tmp = scratch("shadowed-alvr-then-oxrsys");
    let toml_path = tmp.join("OXRSys/oxrsys-runtime.toml");
    fs::create_dir_all(toml_path.parent().unwrap()).unwrap();
    fs::write(
        &toml_path,
        "protocol = \"alvr\"\n\n[streaming]\nprotocol = \"oxrsys\"\n",
    )
    .unwrap();
    let ctx = ctx_with_toml(&tmp, toml_path.clone());

    assert_eq!(cfg_protocol_supported(&ctx).status, CheckStatus::Pass);

    let legacy = cfg_protocol_legacy_oxrsys(&ctx);
    assert_eq!(legacy.status, CheckStatus::Fail);
    assert_eq!(
        legacy.message,
        "oxrsys-runtime.toml protocol='oxrsys' — the demo streams via ALVR"
    );
    assert_eq!(
        legacy.remedy.as_deref(),
        Some(format!("set protocol = \"alvr\" in {}", toml_path.display()).as_str())
    );
    fs::remove_dir_all(&tmp).ok();
}

/// The `cfg.protocol.supported` / `cfg.protocol.legacy-oxrsys` pair for the
/// `oxrsys-runtime.toml` bodies that differ only in the protocol value
/// doctor.sh's last-match `awk` resolves to. `{toml}` in an expected remedy
/// is the row's own scratch toml path. The absent file and the A3b-1
/// shadowing regression keep their own functions.
#[test]
fn config_protocol_state_matrix() {
    // (status, message, remedy) expected from one evaluator.
    type Expected = (CheckStatus, &'static str, Option<&'static str>);
    let cases: &[(&str, &str, Expected, Expected)] = &[
        (
            "alvr",
            "protocol = \"alvr\"\n",
            (
                CheckStatus::Pass,
                "oxrsys-runtime.toml: protocol=alvr",
                None,
            ),
            (
                CheckStatus::Pass,
                "oxrsys-runtime.toml: protocol=alvr (not the legacy oxrsys path)",
                None,
            ),
        ),
        (
            "oxrsys",
            "protocol = \"oxrsys\"\n",
            (
                CheckStatus::Pass,
                "oxrsys-runtime.toml: protocol=oxrsys (supported; see cfg.protocol.legacy-oxrsys)",
                None,
            ),
            (
                CheckStatus::Fail,
                "oxrsys-runtime.toml protocol='oxrsys' — the demo streams via ALVR",
                Some("set protocol = \"alvr\" in {toml}"),
            ),
        ),
        (
            "legacy_usb-unsupported",
            "protocol = \"legacy_usb\"\n",
            (
                CheckStatus::Fail,
                "oxrsys-runtime.toml protocol='legacy_usb' — the demo streams via ALVR",
                Some("set protocol = \"alvr\" in {toml}"),
            ),
            (
                CheckStatus::Skipped,
                "protocol is neither alvr nor oxrsys — see cfg.protocol.supported",
                None,
            ),
        ),
        (
            "shadowed-oxrsys-then-alvr",
            "[streaming]\nprotocol = \"oxrsys\"\nprotocol = \"alvr\"\n",
            (
                CheckStatus::Pass,
                "oxrsys-runtime.toml: protocol=alvr",
                None,
            ),
            (
                CheckStatus::Pass,
                "oxrsys-runtime.toml: protocol=alvr (not the legacy oxrsys path)",
                None,
            ),
        ),
    ];

    for (label, toml_text, supported, legacy) in cases {
        let tmp = scratch(label);
        // Removed at entry as well as exit (3.8): a row that panicked on an
        // earlier run leaves its directory behind.
        fs::remove_dir_all(&tmp).ok();
        let toml_path = tmp.join("OXRSys/oxrsys-runtime.toml");
        fs::create_dir_all(toml_path.parent().unwrap()).unwrap();
        fs::write(&toml_path, toml_text.as_bytes()).unwrap();
        let ctx = ctx_with_toml(&tmp, toml_path.clone());
        let shown = toml_path.display().to_string();
        let remedy = |r: Option<&str>| r.map(|s| s.replace("{toml}", &shown));

        let got = cfg_protocol_supported(&ctx);
        assert_eq!(
            (got.status, got.message, got.remedy),
            (supported.0, supported.1.to_string(), remedy(supported.2)),
            "row {label}: cfg.protocol.supported"
        );

        let got = cfg_protocol_legacy_oxrsys(&ctx);
        assert_eq!(
            (got.status, got.message, got.remedy),
            (legacy.0, legacy.1.to_string(), remedy(legacy.2)),
            "row {label}: cfg.protocol.legacy-oxrsys"
        );
        fs::remove_dir_all(&tmp).ok();
    }
}

fn ctx_with_session(tmp: &Path, sessjson: PathBuf) -> CheckCtx {
    let mut paths = Paths::new(tmp);
    // sessjson lives at <oxr_appsup>/alvr/session.json; point oxr_appsup
    // at sessjson's grandparent so `alvr_session_json()` resolves to it.
    paths.oxr_appsup = sessjson
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .to_path_buf();
    CheckCtx::new(paths, CheckOptions::new())
}

#[test]
fn missing_session_json_is_skipped() {
    let tmp = scratch("session-missing");
    let sessjson = tmp.join("OXRSys/alvr/session.json");
    let ctx = ctx_with_session(&tmp, sessjson);
    assert_eq!(cfg_session_pins(&ctx).status, CheckStatus::Skipped);
}

/// A3b-3 regression: malformed JSON is a degraded state, not a clean one.
#[test]
fn malformed_json_warns() {
    let tmp = scratch("session-malformed");
    let sessjson = tmp.join("OXRSys/alvr/session.json");
    fs::create_dir_all(sessjson.parent().unwrap()).unwrap();
    fs::write(&sessjson, b"{not json").unwrap();
    let ctx = ctx_with_session(&tmp, sessjson.clone());
    let o = cfg_session_pins(&ctx);
    assert_eq!(o.status, CheckStatus::Warn);
    let parse_err = serde_json::from_str::<serde_json::Value>("{not json").unwrap_err();
    assert_eq!(
        o.message,
        format!(
            "could not inspect {}: invalid JSON ({parse_err})",
            sessjson.display()
        )
    );
    // A3b-3 round 2: this evaluator's own serde_json code hit the
    // failure — no python3 in the process, so the message must not
    // claim one is to blame.
    assert!(!o.message.contains("python3"), "message: {}", o.message);
    assert!(o
        .detail
        .as_deref()
        .is_some_and(|d| d.contains("JSON parse error")));
    fs::remove_dir_all(&tmp).ok();
}

/// A3b-3 regression: an unreadable session.json (here, permissions
/// stripped so `is_file()` still gates it in but the read fails) also
/// Warns rather than reporting the false-clean Pass. Skipped when running
/// as root (permissions are unenforceable), matching the `chmod 000`
/// caveat other RealExecutor-style tests in this crate use.
#[test]
fn unreadable_session_json_warns() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if unsafe { libc_geteuid() } == 0 {
            eprintln!("skipping unreadable_session_json_warns: running as root");
            return;
        }
        let tmp = scratch("session-unreadable");
        let sessjson = tmp.join("OXRSys/alvr/session.json");
        fs::create_dir_all(sessjson.parent().unwrap()).unwrap();
        fs::write(&sessjson, b"{}").unwrap();
        fs::set_permissions(&sessjson, fs::Permissions::from_mode(0o000)).unwrap();
        let ctx = ctx_with_session(&tmp, sessjson.clone());
        let o = cfg_session_pins(&ctx);
        // Restore permissions before any panic-driven early return so
        // cleanup below can actually remove the directory.
        fs::set_permissions(&sessjson, fs::Permissions::from_mode(0o644)).ok();
        assert_eq!(o.status, CheckStatus::Warn);
        assert!(
            o.message
                .starts_with(&format!("could not inspect {}: ", sessjson.display())),
            "message: {}",
            o.message
        );
        // A3b-3 round 2: this evaluator's own std::fs code hit the
        // failure — no python3 in the process, so the message must not
        // claim one is to blame.
        assert!(!o.message.contains("python3"), "message: {}", o.message);
        assert!(o
            .detail
            .as_deref()
            .is_some_and(|d| d.contains("read error")));
        fs::remove_dir_all(&tmp).ok();
    }
}

#[cfg(unix)]
unsafe fn libc_geteuid() -> u32 {
    extern "C" {
        fn geteuid() -> u32;
    }
    geteuid()
}

#[test]
fn one_pinned_client_warns_with_the_trailing_space_quirk() {
    let tmp = scratch("session-one-pin");
    let sessjson = tmp.join("OXRSys/alvr/session.json");
    fs::create_dir_all(sessjson.parent().unwrap()).unwrap();
    fs::write(
        &sessjson,
        br#"{"client_connections":{"Quest 3":{"manual_ips":["192.168.1.42"]}}}"#,
    )
    .unwrap();
    let ctx = ctx_with_session(&tmp, sessjson.clone());
    let o = cfg_session_pins(&ctx);
    assert_eq!(o.status, CheckStatus::Warn);
    assert_eq!(
        o.message,
        format!(
            "session.json pins client IP(s): Quest 3=192.168.1.42 — fine while the Quest \
                 keeps that IP; if streaming stops after a DHCP change, edit the pinned IP in \
                 '{}' in place (do not delete the file: a recreated session.json streams a black \
                 800x900 screen)",
            sessjson.display()
        )
    );
    fs::remove_dir_all(&tmp).ok();
}

#[test]
fn multiple_ips_on_one_client_are_comma_joined() {
    let tmp = scratch("session-multi-ip");
    let sessjson = tmp.join("OXRSys/alvr/session.json");
    fs::create_dir_all(sessjson.parent().unwrap()).unwrap();
    fs::write(
        &sessjson,
        br#"{"client_connections":{"Quest 3":{"manual_ips":["10.0.0.5","10.0.0.6"]}}}"#,
    )
    .unwrap();
    let ctx = ctx_with_session(&tmp, sessjson);
    let o = cfg_session_pins(&ctx);
    assert_eq!(o.status, CheckStatus::Warn);
    assert!(o.message.contains("Quest 3=10.0.0.5,10.0.0.6"));
    fs::remove_dir_all(&tmp).ok();
}

/// `cfg.session-pins` for the session.json bodies whose shape alone decides
/// the verdict. `{session}` in an expected message is the row's own scratch
/// session.json path.
#[test]
fn session_json_shape_matrix() {
    let cases: &[(&str, &str, CheckStatus, &str)] = &[
        (
            "missing-client-connections",
            "{}",
            CheckStatus::Pass,
            "ALVR session state has no stale manual-IP pins",
        ),
        // The only fixture whose `manual_ips` is present but falsy: it is
        // the sole killer of the `json_falsy` guard on the per-entry
        // `manual_ips` match, so its bytes stay exactly as written.
        (
            "empty-manual-ips",
            r#"{"client_connections":{"Quest 3":{"manual_ips":[]}}}"#,
            CheckStatus::Pass,
            "ALVR session state has no stale manual-IP pins",
        ),
        (
            "non-object-top-level",
            "[1,2,3]",
            CheckStatus::Warn,
            "could not inspect {session} (broken python3?)",
        ),
        (
            "non-object-client-entry",
            r#"{"client_connections":{"Quest 3":"not-an-object"}}"#,
            CheckStatus::Warn,
            "could not inspect {session} (broken python3?)",
        ),
    ];

    for (label, json, status, message) in cases {
        let tmp = scratch(label);
        // Removed at entry as well as exit (3.8): a row that panicked on an
        // earlier run leaves its directory behind.
        fs::remove_dir_all(&tmp).ok();
        let sessjson = tmp.join("OXRSys/alvr/session.json");
        fs::create_dir_all(sessjson.parent().unwrap()).unwrap();
        fs::write(&sessjson, json.as_bytes()).unwrap();
        let ctx = ctx_with_session(&tmp, sessjson.clone());

        let o = cfg_session_pins(&ctx);
        assert_eq!(
            (o.status, o.message),
            (
                *status,
                message.replace("{session}", &sessjson.display().to_string())
            ),
            "row {label}"
        );
        fs::remove_dir_all(&tmp).ok();
    }
}
