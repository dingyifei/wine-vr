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
//! Message and remedy strings must match `scripts/demo/doctor.sh` verbatim,
//! with one recorded exception (below).
//!
//! ## `cfg.session-pins` unreadable/malformed divergence (A3b-3)
//!
//! doctor.sh's python inspector does `try: json.load(...) except Exception:
//! sys.exit(0)`, so a read failure or malformed JSON exits 0 and doctor
//! reports the same "no stale pins" Pass as a genuinely clean file. This
//! module does not mirror that: [`inspect_session_pins`] distinguishes
//! `Unreadable`/`Malformed` from `Clean` and [`cfg_session_pins`] Warns on
//! the former, because collapsing "could not tell" into "clean" hides the
//! exact degraded state this check exists to surface. `Corrupt` (a shape
//! failure *after* a successful parse) genuinely mirrors doctor.sh — that
//! shape check runs *outside* the shell probe's try/except, so real
//! doctor.sh hits an uncaught Python exception there too, and "broken
//! python3?" is an accurate diagnosis on that arm. `Unreadable`/`Malformed`
//! are different: they are native-only Warns with no shell counterpart at
//! all (round 2 / A3b-3: their message no longer borrows the "broken
//! python3?" wording, since this evaluator's own std::fs/serde_json code hit
//! the failure and there is no python3 to blame). Needs either a matching
//! `scripts/demo/doctor.sh` change or a `sabrage/PARITY.md` row declaring
//! the divergence (cross-area — this module cannot make either edit).
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

/// `awk -F'"' '/^[[:space:]]*protocol[[:space:]]*=/{v=$2} END{print v}' "$TOML"`.
///
/// * The regex requires the line to start (after leading whitespace) with the
///   literal `protocol`, then optional whitespace, then `=` — a key like
///   `protocol_foo` or a `#`-commented line does not match. It does not care
///   which (or whether a) `[table]` the line sits under: this is table-blind,
///   matching `ext/oxrsys/runtime/src/Config.cpp`'s own line-oriented reader.
/// * `-F'"'` splits on double quotes; `$2` is the text between the first and
///   second quote on a matching line, or empty if that line has no quote at
///   all (an unquoted assignment).
/// * Every matching line overwrites `v`, so the **last** matching line in the
///   file wins — not `exit`-after-first-match. This mirrors both the runtime
///   (a later assignment overwrites an earlier one, regardless of table) and
///   doctor.sh's own `awk` recipe (`scripts/demo/doctor.sh`,
///   `scripts/demo/run.sh`), which use this exact `{v=$2} END{print v}` form.
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
    /// `fs::read_to_string` failed (missing between the `is_file()` gate and
    /// the read, permissions, …). doctor.sh's inspector script's
    /// `try/except: sys.exit(0)` swallows the equivalent Python failure
    /// (`open()` raising) and reports the *clean* state, not an error —
    /// intentionally NOT mirrored here (A3b-3): collapsing a read failure
    /// into "no stale pins" hides exactly the degraded state this check
    /// exists to expose. See the module doc for the resulting divergence.
    Unreadable(std::io::Error),
    /// `serde_json::from_str` failed (malformed JSON). Same doctor.sh
    /// swallow-and-report-clean behavior as `Unreadable`, and the same
    /// intentional divergence here.
    Malformed(serde_json::Error),
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
        SessionPinState::Clean => CheckOutcome::pass(
            "cfg.session-pins",
            "ALVR session state has no stale manual-IP pins",
        ),
        // A3b-3 (round 2): a read or parse failure is a degraded state, not
        // a clean one — Warn instead of collapsing into the same Pass as
        // `Clean`. Unlike `Corrupt` below, this evaluator's own std::fs /
        // serde_json code hit the failure — there is no python3 here to
        // blame, so the message says so accurately; `.detail` carries the
        // underlying error for anything that renders it.
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
        // Unlike the two arms above, `Corrupt` genuinely mirrors doctor.sh:
        // the shape violations it covers happen *outside* the shell probe's
        // try/except (see the enum doc), so real doctor.sh hits an
        // uncaught Python exception here too — "broken python3?" is an
        // accurate diagnosis on this arm, not a borrowed one.
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
    /// is the row's own scratch toml path. Two cases keep their own functions:
    /// the absent file (`missing_toml_fails_supported_and_skips_legacy`, which
    /// writes no toml at all) and the A3b-1 regression
    /// (`shadowed_protocol_alvr_then_oxrsys_resolves_to_the_last_assignment`).
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

    /// The only fixture whose `manual_ips` is present but falsy: it is the sole
    /// killer of the `json_falsy` guard on the per-entry `manual_ips` match, so
    /// it stays out of `session_json_shape_matrix`.
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
    /// session.json path. The missing, unreadable and malformed files, the
    /// bodies that do carry pins, and the present-but-falsy `manual_ips` body
    /// (`empty_manual_ips_is_clean`) keep their own functions: their setup,
    /// their assertions or the mutants they alone kill differ.
    #[test]
    fn session_json_shape_matrix() {
        let cases: &[(&str, &str, CheckStatus, &str)] = &[
            (
                "missing-client-connections",
                "{}",
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
