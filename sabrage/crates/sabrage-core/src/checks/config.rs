//! Group `config` — doctor.sh section 13, 13b: the oxrsys runtime config and ALVR session state.
//!
//! Slugs owned here, in contract order:
//!
//! * `cfg.protocol.supported` — `oxrsys-runtime.toml` exists and its
//!   `protocol` is a value the demo supports
//! * `cfg.protocol.legacy-oxrsys` — `protocol = "oxrsys"` (the legacy
//!   USB/adb-reverse path) — doctor FAILs, run WARNs, and the native run
//!   preflight BLOCKS (recorded divergence)
//! * `cfg.session-pins` — `<oxr_appsup>/alvr/session.json` has no
//!   `manual_ips` entries; pins are fine while the Quest keeps that IP but
//!   break streaming after a DHCP change. Volatile
//!
//! Every evaluator is `fn(&CheckCtx) -> CheckOutcome`: a **read-only probe**.
//! Message and remedy strings must match `scripts/demo/doctor.sh` verbatim.
//!
//! ## `cfg.session-pins` ordering caveat
//!
//! doctor.sh's python inspector iterates `client_connections` in JSON
//! insertion order (CPython dict order). `serde_json::Value` here is backed
//! by a plain `BTreeMap` (this crate does not enable the `preserve_order`
//! feature — that is a dependency change and out of this module's remit), so
//! when *more than one* client has a stale IP pin simultaneously, the pinned
//! entries in the WARN message are sorted by client name instead of insertion
//! order. Single-pin sessions — overwhelmingly the common case — are
//! unaffected.

use std::path::Path;

use super::Evaluator;
#[allow(unused_imports)]
use super::{CheckCtx, CheckOutcome, CheckStatus, SkipReason};

// ── section 13: protocol ─────────────────────────────────────────────────────

/// One `protocol = "…"` line as doctor.sh's parser would resolve it.
enum ProtocolState {
    /// `[ -f "$TOML" ]` was false.
    Missing,
    Alvr,
    Oxrsys,
    /// Anything else, including an unset/unquoted value (empty string).
    Other(String),
}

/// `awk -F'"' '/^[[:space:]]*protocol[[:space:]]*=/{print $2; exit}' "$TOML"`.
///
/// * The regex requires the line to start (after leading whitespace) with the
///   literal `protocol`, then optional whitespace, then `=` — a key like
///   `protocol_foo` or a `#`-commented line does not match.
/// * `-F'"'` splits on double quotes; `$2` is the text between the first and
///   second quote on the *matching* line, or empty if that line has no quote
///   at all (an unquoted assignment, or none).
/// * `exit` after the first match: later matching lines are ignored.
fn parse_protocol(toml_text: &str) -> String {
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
        return fields.next().unwrap_or("").to_string();
    }
    String::new()
}

/// `[ -f "$TOML" ]`, then [`parse_protocol`] on its contents. A read error
/// after the existence check (e.g. a permission race) degrades to an empty
/// `PROTO`, exactly like the shell's unredirected `awk` failing silently into
/// an empty capture.
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

/// doctor.sh section 13, the `cfg.protocol.supported` half:
/// ```sh
/// if [ -f "$TOML" ]; then
///   PROTO="$(awk … "$TOML")"
///   if [ "$PROTO" = "alvr" ]; then chk ok cfg.protocol.supported …
///   elif [ "$PROTO" = "oxrsys" ]; then tap cfg.protocol.supported ok
///   else chk fail cfg.protocol.supported … ; fi
/// else chk fail cfg.protocol.supported "$TOML missing" "./demo.sh setup"; fi
/// ```
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
        // Silent `tap … ok` in the shell (the row is printed by
        // cfg.protocol.legacy-oxrsys instead) — still Pass, but the CLI
        // console suppresses it.
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

/// doctor.sh section 13, the `cfg.protocol.legacy-oxrsys` half:
/// ```sh
/// if [ "$PROTO" = "alvr" ]; then tap cfg.protocol.legacy-oxrsys ok
/// elif [ "$PROTO" = "oxrsys" ]; then chk fail cfg.protocol.legacy-oxrsys …
/// else tap cfg.protocol.legacy-oxrsys skipped; fi
/// # ($TOML missing) -> tap cfg.protocol.legacy-oxrsys skipped
/// ```
fn cfg_protocol_legacy_oxrsys(ctx: &CheckCtx) -> CheckOutcome {
    match read_protocol_state(ctx) {
        ProtocolState::Missing => CheckOutcome::skipped(
            "cfg.protocol.legacy-oxrsys",
            SkipReason::new(format!("{} missing", ctx.paths.toml_path.display())),
        ),
        // Silent `tap … ok` in the shell — the CLI console suppresses it.
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

// ── section 13b: stale client IP pins ────────────────────────────────────────

/// What [`inspect_session_pins`] found.
enum SessionPinState {
    /// `json.load()` itself raised (bad syntax, unreadable file) — the
    /// inspector script's `try/except: sys.exit(0)` swallows exactly this,
    /// so `PINNED` comes out empty and doctor reports the *clean* state, not
    /// an error.
    UnreadableOrMalformed,
    /// The JSON parsed, but its shape breaks an assumption the *un-tried*
    /// walk over `client_connections` makes (top level not an object,
    /// `client_connections` present as a non-empty non-object, an entry
    /// under it not an object, or `manual_ips` present as a non-empty
    /// non-array/non-string-element value). In real Python this walk is
    /// outside the `try/except`, so it is an uncaught exception -> non-zero
    /// exit -> doctor's "broken python3?" WARN branch.
    Corrupt,
    /// Parsed and well-shaped; no client has a non-empty `manual_ips`.
    Clean,
    /// Parsed and well-shaped; the space-joined, trailing-space
    /// `"name=ip,ip "` entries doctor.sh's `tr '\n' ' '` step produces —
    /// concatenating this directly before `"— fine while …"` reproduces the
    /// exact single space the shell gets from that trailing space.
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

/// ```py
/// import json,sys
/// try: s = json.load(open(sys.argv[1]))
/// except Exception: sys.exit(0)
/// for n, c in (s.get("client_connections") or {}).items():
///     ips = c.get("manual_ips") or []
///     if ips: print(n + "=" + ",".join(ips))
/// ```
fn inspect_session_pins(path: &Path) -> SessionPinState {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return SessionPinState::UnreadableOrMalformed,
    };
    let value: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => return SessionPinState::UnreadableOrMalformed,
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

/// doctor.sh section 13b: `[ -f "$SESSJSON" ]` gates the whole check —
/// `tap cfg.session-pins skipped` when it is absent.
fn cfg_session_pins(ctx: &CheckCtx) -> CheckOutcome {
    let sessjson = ctx.paths.alvr_session_json();
    if !sessjson.is_file() {
        return CheckOutcome::skipped(
            "cfg.session-pins",
            SkipReason::new(format!("{} not present", sessjson.display())),
        );
    }
    match inspect_session_pins(&sessjson) {
        SessionPinState::UnreadableOrMalformed | SessionPinState::Clean => CheckOutcome::pass(
            "cfg.session-pins",
            "ALVR session state has no stale manual-IP pins",
        ),
        SessionPinState::Corrupt => CheckOutcome::warn(
            "cfg.session-pins",
            format!("could not inspect {} (broken python3?)", sessjson.display()),
        ),
        SessionPinState::Pinned(pinned) => CheckOutcome::warn(
            "cfg.session-pins",
            format!(
                "session.json pins client IP(s): {pinned}— fine while the Quest keeps that IP; \
                 if streaming stops after a DHCP change, delete '{}' (recreated with \
                 discovery+auto-trust)",
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

    // ── parse_protocol ──────────────────────────────────────────────────────

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
        // First match wins even if a later line also matches.
        assert_eq!(
            parse_protocol("protocol = \"first\"\nprotocol = \"second\"\n"),
            "first"
        );
        assert_eq!(parse_protocol(""), "");
    }

    // ── cfg.protocol.* ───────────────────────────────────────────────────────

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

    #[test]
    fn alvr_protocol_passes_both_slugs() {
        let tmp = scratch("alvr");
        let toml_path = tmp.join("OXRSys/oxrsys-runtime.toml");
        fs::create_dir_all(toml_path.parent().unwrap()).unwrap();
        fs::write(&toml_path, "protocol = \"alvr\"\n").unwrap();
        let ctx = ctx_with_toml(&tmp, toml_path);

        assert_eq!(cfg_protocol_supported(&ctx).status, CheckStatus::Pass);
        assert_eq!(cfg_protocol_legacy_oxrsys(&ctx).status, CheckStatus::Pass);
        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn oxrsys_protocol_passes_supported_and_fails_legacy() {
        let tmp = scratch("oxrsys");
        let toml_path = tmp.join("OXRSys/oxrsys-runtime.toml");
        fs::create_dir_all(toml_path.parent().unwrap()).unwrap();
        fs::write(&toml_path, "protocol = \"oxrsys\"\n").unwrap();
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

    #[test]
    fn garbage_protocol_fails_supported_and_skips_legacy() {
        let tmp = scratch("garbage");
        let toml_path = tmp.join("OXRSys/oxrsys-runtime.toml");
        fs::create_dir_all(toml_path.parent().unwrap()).unwrap();
        fs::write(&toml_path, "protocol = \"legacy_usb\"\n").unwrap();
        let ctx = ctx_with_toml(&tmp, toml_path.clone());

        let supported = cfg_protocol_supported(&ctx);
        assert_eq!(supported.status, CheckStatus::Fail);
        assert_eq!(
            supported.message,
            "oxrsys-runtime.toml protocol='legacy_usb' — the demo streams via ALVR"
        );
        assert_eq!(
            supported.remedy.as_deref(),
            Some(format!("set protocol = \"alvr\" in {}", toml_path.display()).as_str())
        );

        assert_eq!(
            cfg_protocol_legacy_oxrsys(&ctx).status,
            CheckStatus::Skipped
        );
        fs::remove_dir_all(&tmp).ok();
    }

    // ── cfg.session-pins ─────────────────────────────────────────────────────

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

    #[test]
    fn malformed_json_is_silently_clean() {
        let tmp = scratch("session-malformed");
        let sessjson = tmp.join("OXRSys/alvr/session.json");
        fs::create_dir_all(sessjson.parent().unwrap()).unwrap();
        fs::write(&sessjson, b"{not json").unwrap();
        let ctx = ctx_with_session(&tmp, sessjson);
        let o = cfg_session_pins(&ctx);
        assert_eq!(o.status, CheckStatus::Pass);
        assert_eq!(o.message, "ALVR session state has no stale manual-IP pins");
        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn no_client_connections_key_is_clean() {
        let tmp = scratch("session-no-cc");
        let sessjson = tmp.join("OXRSys/alvr/session.json");
        fs::create_dir_all(sessjson.parent().unwrap()).unwrap();
        fs::write(&sessjson, b"{}").unwrap();
        let ctx = ctx_with_session(&tmp, sessjson);
        assert_eq!(cfg_session_pins(&ctx).status, CheckStatus::Pass);
        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn empty_manual_ips_is_clean() {
        let tmp = scratch("session-empty-ips");
        let sessjson = tmp.join("OXRSys/alvr/session.json");
        fs::create_dir_all(sessjson.parent().unwrap()).unwrap();
        fs::write(
            &sessjson,
            br#"{"client_connections":{"Quest 3":{"manual_ips":[]}}}"#,
        )
        .unwrap();
        let ctx = ctx_with_session(&tmp, sessjson);
        assert_eq!(cfg_session_pins(&ctx).status, CheckStatus::Pass);
        fs::remove_dir_all(&tmp).ok();
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
                 keeps that IP; if streaming stops after a DHCP change, delete '{}' (recreated \
                 with discovery+auto-trust)",
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

    #[test]
    fn non_object_top_level_is_corrupt() {
        let tmp = scratch("session-corrupt-top");
        let sessjson = tmp.join("OXRSys/alvr/session.json");
        fs::create_dir_all(sessjson.parent().unwrap()).unwrap();
        fs::write(&sessjson, b"[1,2,3]").unwrap();
        let ctx = ctx_with_session(&tmp, sessjson.clone());
        let o = cfg_session_pins(&ctx);
        assert_eq!(o.status, CheckStatus::Warn);
        assert_eq!(
            o.message,
            format!("could not inspect {} (broken python3?)", sessjson.display())
        );
        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn non_dict_client_connections_entry_is_corrupt() {
        let tmp = scratch("session-corrupt-entry");
        let sessjson = tmp.join("OXRSys/alvr/session.json");
        fs::create_dir_all(sessjson.parent().unwrap()).unwrap();
        fs::write(
            &sessjson,
            br#"{"client_connections":{"Quest 3":"not-an-object"}}"#,
        )
        .unwrap();
        let ctx = ctx_with_session(&tmp, sessjson.clone());
        let o = cfg_session_pins(&ctx);
        assert_eq!(o.status, CheckStatus::Warn);
        assert!(o.message.starts_with("could not inspect "));
        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn defs_binds_all_three_slugs_in_contract_order() {
        let slugs: Vec<&str> = defs().into_iter().map(|(s, _)| s).collect();
        assert_eq!(
            slugs,
            vec![
                "cfg.protocol.supported",
                "cfg.protocol.legacy-oxrsys",
                "cfg.session-pins",
            ]
        );
    }
}
