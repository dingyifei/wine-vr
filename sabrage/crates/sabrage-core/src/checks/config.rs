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
mod tests;
