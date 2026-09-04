//! Group `config` — the oxrsys runtime config and ALVR session state.
//!
//! Binds `cfg.protocol.supported`, `cfg.protocol.legacy-oxrsys` and
//! `cfg.session-pins`, in contract order, to read-only
//! `fn(&CheckCtx) -> CheckOutcome` probes whose message and remedy strings
//! match the shell verbatim except where noted below.
//! Reference: scripts/demo/doctor.sh sections 13 and 13b.
//!
//! Multi-pin WARN entries keep session.json file order because this crate
//! enables `serde_json/preserve_order`; see PARITY.md § Doctor / checks,
//! "`host.manifest` / `cfg.session-pins` parse JSON natively (serde)".
//!
//! TODO(A3b-3): an unreadable or malformed `session.json` Warns here where
//! doctor.sh's `try/except: sys.exit(0)` reports the clean Pass; the
//! divergence still owes a `scripts/demo/doctor.sh` change or a
//! `sabrage/PARITY.md` row. Pinned by tests::malformed_json_warns and
//! tests::unreadable_session_json_warns.

use std::path::Path;

use super::Evaluator;
#[allow(unused_imports)]
use super::{CheckCtx, CheckOutcome, CheckStatus, SkipReason};

/// One `protocol = "…"` line as doctor.sh's parser would resolve it.
enum ProtocolState {
    /// No regular file at the configured `oxrsys-runtime.toml` path.
    Missing,
    Alvr,
    Oxrsys,
    /// Anything else, including an unset/unquoted value (empty string).
    Other(String),
}

/// The `protocol` value doctor.sh's last-match `awk` recipe would resolve,
/// or the empty string when no line assigns a quoted one.
///
/// Table-blind and last-assignment-wins, matching the runtime's line-oriented
/// reader and the `awk` form doctor.sh and run.sh share; `#`-commented lines
/// and keys like `protocol_foo` never match; an unquoted assignment resolves
/// to the empty string. Pinned by tests::parse_protocol_matches_the_awk_recipe.
fn parse_protocol(toml_text: &str) -> String {
    let mut value = String::new();
    for line in toml_text.lines() {
        let after_leading_ws = line.trim_start();
        let Some(rest) = after_leading_ws.strip_prefix("protocol") else {
            continue;
        };
        if !rest.trim_start().starts_with('=') {
            continue;
        }
        let mut fields = line.split('"');
        let _before_first_quote = fields.next();
        value = fields.next().unwrap_or("").to_string();
    }
    value
}

/// The [`ProtocolState`] of the configured `oxrsys-runtime.toml`.
///
/// A read error after the existence check (a permission race, say)
/// degrades to an empty `protocol`, like the shell's unredirected `awk`
/// failing silently into an empty capture.
fn read_protocol_state(ctx: &CheckCtx) -> ProtocolState {
    let toml_path = &ctx.paths.toml_path;
    if !toml_path.is_file() {
        return ProtocolState::Missing;
    }
    let text = std::fs::read_to_string(toml_path).unwrap_or_default();
    match parse_protocol(&text).as_str() {
        "alvr" => ProtocolState::Alvr,
        "oxrsys" => ProtocolState::Oxrsys,
        other => ProtocolState::Other(other.to_string()),
    }
}

/// `oxrsys-runtime.toml protocol='<proto>' — the demo streams via ALVR` — the
/// message text shared, verbatim, by both the `cfg.protocol.supported` "any
/// other value" branch and the `cfg.protocol.legacy-oxrsys` FAIL branch.
fn protocol_mismatch_message(proto: &str) -> String {
    format!("oxrsys-runtime.toml protocol='{proto}' — the demo streams via ALVR")
}

/// `set protocol = "alvr" in <TOML>` — likewise shared by both FAIL branches.
fn protocol_mismatch_remedy(toml_path: &Path) -> String {
    format!("set protocol = \"alvr\" in {}", toml_path.display())
}

/// `cfg.protocol.supported`: Pass for `protocol = "alvr"`, silent Pass for
/// `oxrsys` (the shell prints that row from `cfg.protocol.legacy-oxrsys`
/// instead), Fail when the toml is missing or names any other value.
/// Reference: scripts/demo/doctor.sh section 13.
fn cfg_protocol_supported(ctx: &CheckCtx) -> CheckOutcome {
    match read_protocol_state(ctx) {
        ProtocolState::Missing => CheckOutcome::fail(
            "cfg.protocol.supported",
            format!("{} missing", ctx.paths.toml_path.display()),
            "./demo.sh setup",
        ),
        ProtocolState::Alvr => CheckOutcome::pass(
            "cfg.protocol.supported",
            "oxrsys-runtime.toml: protocol=alvr",
        ),
        // The shell prints this row from cfg.protocol.legacy-oxrsys instead (PARITY.md § Doctor / checks).
        ProtocolState::Oxrsys => CheckOutcome::silent_pass(
            "cfg.protocol.supported",
            "oxrsys-runtime.toml: protocol=oxrsys (supported; see cfg.protocol.legacy-oxrsys)",
        ),
        ProtocolState::Other(proto) => CheckOutcome::fail(
            "cfg.protocol.supported",
            protocol_mismatch_message(&proto),
            protocol_mismatch_remedy(&ctx.paths.toml_path),
        ),
    }
}

/// `cfg.protocol.legacy-oxrsys`: Fail on `protocol = "oxrsys"` (the legacy
/// USB/adb-reverse path), silent Pass on `alvr`, Skipped when the toml is
/// missing or names any other value.
/// Reference: scripts/demo/doctor.sh section 13.
fn cfg_protocol_legacy_oxrsys(ctx: &CheckCtx) -> CheckOutcome {
    match read_protocol_state(ctx) {
        ProtocolState::Missing => CheckOutcome::skipped(
            "cfg.protocol.legacy-oxrsys",
            SkipReason::new(format!("{} missing", ctx.paths.toml_path.display())),
        ),
        ProtocolState::Alvr => CheckOutcome::silent_pass(
            "cfg.protocol.legacy-oxrsys",
            "oxrsys-runtime.toml: protocol=alvr (not the legacy oxrsys path)",
        ),
        ProtocolState::Oxrsys => CheckOutcome::fail(
            "cfg.protocol.legacy-oxrsys",
            protocol_mismatch_message("oxrsys"),
            protocol_mismatch_remedy(&ctx.paths.toml_path),
        ),
        ProtocolState::Other(_) => CheckOutcome::skipped(
            "cfg.protocol.legacy-oxrsys",
            SkipReason::new("protocol is neither alvr nor oxrsys — see cfg.protocol.supported"),
        ),
    }
}

/// What [`inspect_session_pins`] found.
enum SessionPinState {
    /// `fs::read_to_string` failed (missing between the `is_file()` gate and
    /// the read, permissions, …). Warned rather than collapsed into `Clean`
    /// the way doctor.sh's `try/except: sys.exit(0)` would (A3b-3): "could
    /// not tell" is the degraded state this check exists to surface.
    Unreadable(std::io::Error),
    /// `serde_json::from_str` failed (malformed JSON). Warned for the same
    /// reason, and with the same deliberate doctor.sh divergence, as
    /// `Unreadable` (A3b-3).
    Malformed(serde_json::Error),
    /// The JSON parsed, but its shape breaks the walk over
    /// `client_connections` (the top level, `client_connections` itself, an
    /// entry under it, or a non-empty `manual_ips` with the wrong type).
    /// doctor.sh's Python raises outside its `try/except` here too, so its
    /// "broken python3?" WARN is mirrored.
    Corrupt,
    /// Parsed and well-shaped; no client has a non-empty `manual_ips`.
    Clean,
    /// Space-joined `"name=ip,ip "` entries carrying doctor.sh's trailing
    /// space, so concatenating directly before `"— fine while …"` reproduces
    /// the single space the shell gets. Pinned by
    /// tests::one_pinned_client_warns_with_the_trailing_space_quirk.
    Pinned(String),
}

/// Python truthiness for the `x or default` idiom the inspector script uses
/// twice (`client_connections`, `manual_ips`).
fn json_falsy(v: &serde_json::Value) -> bool {
    match v {
        serde_json::Value::Null => true,
        serde_json::Value::Bool(b) => !b,
        serde_json::Value::Number(n) => n.as_f64().map(|f| f == 0.0).unwrap_or(false),
        serde_json::Value::String(s) => s.is_empty(),
        serde_json::Value::Array(a) => a.is_empty(),
        serde_json::Value::Object(o) => o.is_empty(),
    }
}

/// The [`SessionPinState`] of an ALVR `session.json`, mirroring doctor.sh's
/// inline python inspector: every `client_connections` entry with a
/// non-empty `manual_ips` contributes one `name=ip,ip` entry.
/// Reference: scripts/demo/doctor.sh section 13b.
fn inspect_session_pins(path: &Path) -> SessionPinState {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => return SessionPinState::Unreadable(e),
    };
    let value: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => return SessionPinState::Malformed(e),
    };
    let Some(top) = value.as_object() else {
        return SessionPinState::Corrupt;
    };
    let cc = match top.get("client_connections") {
        None => return SessionPinState::Clean,
        Some(v) if json_falsy(v) => return SessionPinState::Clean,
        Some(v) => v,
    };
    let Some(cc_obj) = cc.as_object() else {
        return SessionPinState::Corrupt;
    };

    let mut pinned = String::new();
    for (name, conn) in cc_obj {
        let Some(conn_obj) = conn.as_object() else {
            return SessionPinState::Corrupt;
        };
        let ips = match conn_obj.get("manual_ips") {
            None => continue,
            Some(v) if json_falsy(v) => continue,
            Some(v) => v,
        };
        let Some(ips_arr) = ips.as_array() else {
            return SessionPinState::Corrupt;
        };
        let mut parts = Vec::with_capacity(ips_arr.len());
        for el in ips_arr {
            match el.as_str() {
                Some(s) => parts.push(s.to_string()),
                None => return SessionPinState::Corrupt,
            }
        }
        pinned.push_str(name);
        pinned.push('=');
        pinned.push_str(&parts.join(","));
        pinned.push(' ');
    }

    if pinned.is_empty() {
        SessionPinState::Clean
    } else {
        SessionPinState::Pinned(pinned)
    }
}

/// `cfg.session-pins`: Skipped when `session.json` is absent, Pass when no
/// client carries a manual IP pin, Warn otherwise — including the A3b-3
/// read/parse failures doctor.sh reports as clean.
/// Reference: scripts/demo/doctor.sh section 13b.
fn cfg_session_pins(ctx: &CheckCtx) -> CheckOutcome {
    let sessjson = ctx.paths.alvr_session_json();
    if !sessjson.is_file() {
        return CheckOutcome::skipped(
            "cfg.session-pins",
            SkipReason::new(format!("{} not present", sessjson.display())),
        );
    }
    match inspect_session_pins(&sessjson) {
        SessionPinState::Clean => CheckOutcome::pass(
            "cfg.session-pins",
            "ALVR session state has no stale manual-IP pins",
        ),
        // A3b-3: a read or parse failure is a degraded state, not a clean one,
        // and no python3 is in this process to blame for it.
        // Pinned by tests::malformed_json_warns, tests::unreadable_session_json_warns.
        SessionPinState::Unreadable(e) => CheckOutcome::warn(
            "cfg.session-pins",
            format!("could not inspect {}: {e}", sessjson.display()),
        )
        .with_detail(format!("read error: {e}")),
        SessionPinState::Malformed(e) => CheckOutcome::warn(
            "cfg.session-pins",
            format!(
                "could not inspect {}: invalid JSON ({e})",
                sessjson.display()
            ),
        )
        .with_detail(format!("JSON parse error: {e}")),
        // `Corrupt` mirrors doctor.sh: these shape violations raise outside the
        // shell probe's try/except, so "broken python3?" is accurate here.
        SessionPinState::Corrupt => CheckOutcome::warn(
            "cfg.session-pins",
            format!("could not inspect {} (broken python3?)", sessjson.display()),
        ),
        SessionPinState::Pinned(pinned) => CheckOutcome::warn(
            "cfg.session-pins",
            format!(
                "session.json pins client IP(s): {pinned}— fine while the Quest keeps that IP; \
                 if streaming stops after a DHCP change, edit the pinned IP in '{}' in place (do \
                 not delete the file: a recreated session.json streams a black 800x900 screen)",
                sessjson.display()
            ),
        ),
    }
}

/// Evaluators this module binds, keyed by contract slug.
pub fn defs() -> Vec<(&'static str, Evaluator)> {
    vec![
        (
            "cfg.protocol.supported",
            cfg_protocol_supported as Evaluator,
        ),
        (
            "cfg.protocol.legacy-oxrsys",
            cfg_protocol_legacy_oxrsys as Evaluator,
        ),
        ("cfg.session-pins", cfg_session_pins as Evaluator),
    ]
}

#[cfg(test)]
mod tests {
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
                (CheckStatus::Pass, "oxrsys-runtime.toml: protocol=alvr", None),
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
                (CheckStatus::Pass, "oxrsys-runtime.toml: protocol=alvr", None),
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
}
