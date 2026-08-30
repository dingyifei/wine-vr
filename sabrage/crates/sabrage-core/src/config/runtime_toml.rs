//! `oxrsys-runtime.toml` — typed read, format-preserving patch, backed-up write.
//!
//! This is the one place Sabrage edits a file `demo.sh` treats as **write-once**
//! (`scripts/demo/setup.sh` writes the shared template when the file is absent
//! and never touches it again; its "protocol is not alvr" branch tells the user
//! to *edit it themselves*). design-core §4.1 makes that a deliberate, narrow
//! override rather than a divergence in the pipeline: create-if-absent from
//! [`crate::util::toml_template`] byte-for-byte, then in-place value edits with
//! a rolling backup, and never a regeneration or a migration.
//!
//! # Why the edits are so fussy
//!
//! The consumer is not a TOML library. `ext/oxrsys/runtime/src/Config.cpp` is a
//! line-oriented reader (verified 2026-08-30):
//!
//! * `#` starts a comment (quote-aware) and the rest of the line is discarded;
//! * blank lines and `[table]` headers are skipped — **tables are ignored
//!   entirely**, so a key counts wherever it sits;
//! * the line is split on its first `=`, both halves trimmed;
//! * a later assignment overwrites an earlier one — **last wins**;
//! * a value outside the accepted set is *silently ignored* and the compiled-in
//!   default stays.
//!
//! Three consequences shape this module:
//!
//! 1. the key the runtime honors is the **last** occurrence in the file, in
//!    physical order, whatever table it is in — that is the one
//!    [`apply_patch`] edits, and the others are reported in `shadowed`;
//! 2. a same-line `# comment` is moved onto its own line above the key. Runtime
//!    builds before the 2026-08 parser fix mis-parsed same-line comments
//!    (`docs/troubleshooting.md`, Config rows), and a user can still be running
//!    one. Sabrage never authors a trailing comment and never leaves one on a
//!    line it rewrote;
//! 3. every other byte is preserved. The file is hand-maintained — the deployed
//!    copy carries a four-line provenance header about a Catch2 run that once
//!    clobbered it — so a reformat would destroy real user notes.
//!
//! # Two readers
//!
//! [`read`] parses with `toml_edit` and, when that fails, falls back to
//! [`read_lines_like_the_runtime`] — the runtime's own semantics — so the GUI
//! can still show what the runtime *would* use in a file neither the GUI nor
//! `toml_edit` can round-trip. In that state [`RuntimeConfigView::parse_error`]
//! is set and writes are refused: never rewrite a file you cannot round-trip.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use toml_edit::{DocumentMut, Formatted, Item, Table, Value};

use crate::error::{Result, SabrageError};
use crate::events::{StageEvent, StepId};
use crate::executor::Executor;
use crate::fixes::{FixAction, FixReport};
use crate::stages::{EventSink, StageCtx};

/// The six keys Sabrage is allowed to write, in the order the Settings screen
/// shows them. Every other key the runtime knows (`render_device`,
/// `encoder_10bit`, `client_sharpening`, `abr_mode`, `fov_degrees`, …) is
/// preserved byte-for-byte and never touched.
pub const EDITABLE_KEYS: [&str; 6] = [
    "protocol",
    "bitrate_mbps",
    "encoder_process",
    "video_codec",
    "resolution_scale",
    "refresh_rate_hz",
];

/// Where a key that is absent from the whole document gets inserted.
///
/// The runtime ignores tables, so this choice is cosmetic to it — but the
/// template puts everything under `[streaming]` and so does the deployed file,
/// and a key that lands somewhere else would read as noise to the human who
/// maintains this file by hand.
pub const TABLE: &str = "streaming";

/// How many backups [`write`] keeps.
pub const BACKUP_KEEP: usize = 10;

/// Backup filename prefix: `oxrsys-runtime.toml.<unix-secs>[-<n>]`.
pub const BACKUP_PREFIX: &str = "oxrsys-runtime.toml.";

/// The accepted `refresh_rate_hz` values (the runtime rejects everything else).
pub const REFRESH_RATES: [u32; 5] = [60, 72, 80, 90, 120];

/// Inclusive `bitrate_mbps` bounds.
pub const BITRATE_RANGE: (u32, u32) = (1, 200);

/// Inclusive `resolution_scale` bounds.
pub const RESOLUTION_SCALE_RANGE: (f64, f64) = (0.25, 1.0);

// ── typed values ─────────────────────────────────────────────────────────────

/// `protocol` — which streaming backend the runtime instantiates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    /// The demo path: embedded `alvr_server_core`, stock Quest client.
    Alvr,
    /// The legacy USB/adb-reverse protocol. doctor FAILs on it.
    Oxrsys,
}

/// `video_codec` — what the runtime offers the client during negotiation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VideoCodec {
    Auto,
    H265,
    H264,
}

/// `encoder_process` — in-process (Rosetta, H.264 only) vs. the native-arm64
/// helper (hardware HEVC).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EncoderProcess {
    Auto,
    Native,
    Inproc,
}

macro_rules! str_enum {
    ($t:ty, $($s:literal => $v:path),+ $(,)?) => {
        impl $t {
            /// The spelling in the toml (also the serde spelling).
            pub fn as_str(self) -> &'static str {
                match self { $($v => $s),+ }
            }
            /// Parse the toml spelling. `None` for a value the runtime would
            /// silently ignore.
            pub fn parse(s: &str) -> Option<Self> {
                match s { $($s => Some($v),)+ _ => None }
            }
            /// Every accepted value, in the order the runtime documents them.
            pub const EVERY: &'static [$t] = &[$($v),+];
        }
        impl std::fmt::Display for $t {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

str_enum!(Protocol, "alvr" => Protocol::Alvr, "oxrsys" => Protocol::Oxrsys);
str_enum!(
    VideoCodec,
    "auto" => VideoCodec::Auto,
    "h265" => VideoCodec::H265,
    "h264" => VideoCodec::H264,
);
str_enum!(
    EncoderProcess,
    "auto" => EncoderProcess::Auto,
    "native" => EncoderProcess::Native,
    "inproc" => EncoderProcess::Inproc,
);

/// The six editable keys as typed values.
///
/// `None` means two different things depending on which side of the API you are
/// on: in a [`RuntimeConfigView`] it means *the file does not set this key (or
/// sets it to something the runtime ignores), so the runtime default applies*;
/// in a [`RuntimeConfigPatch`] it means *leave this key exactly as it is*.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RuntimeConfigValues {
    pub protocol: Option<Protocol>,
    pub bitrate_mbps: Option<u32>,
    pub encoder_process: Option<EncoderProcess>,
    pub video_codec: Option<VideoCodec>,
    pub resolution_scale: Option<f64>,
    pub refresh_rate_hz: Option<u32>,
}

/// `Some` = set this key, `None` = leave it untouched. Same shape as the view's
/// `values`, so the UI can diff one against the other and send the difference.
pub type RuntimeConfigPatch = RuntimeConfigValues;

/// The runtime's compiled-in defaults — what applies when a key is absent or
/// its value is one the runtime ignores.
///
/// Note `protocol`: the runtime defaults to `oxrsys`, while the shared template
/// writes `alvr`. A file that lost its `protocol` line therefore streams over
/// the legacy path, which is exactly what `cfg.protocol.supported` FAILs on.
pub fn runtime_defaults() -> RuntimeConfigValues {
    RuntimeConfigValues {
        protocol: Some(Protocol::Oxrsys),
        bitrate_mbps: Some(50),
        encoder_process: Some(EncoderProcess::Auto),
        video_codec: Some(VideoCodec::H265),
        resolution_scale: Some(0.75),
        refresh_rate_hz: Some(72),
    }
}

/// A key whose value on disk (or in a patch) is one the runtime would ignore.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvalidValue {
    pub key: String,
    /// The value as written, for the UI to quote back.
    pub raw: String,
    /// What the runtime accepts instead.
    pub reason: String,
}

impl InvalidValue {
    fn new(key: &str, raw: impl Into<String>, reason: impl Into<String>) -> InvalidValue {
        InvalidValue {
            key: key.to_string(),
            raw: raw.into(),
            reason: reason.into(),
        }
    }
}

/// Everything the Settings screen needs about the file, in one read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeConfigView {
    pub path: String,
    pub exists: bool,
    /// What the runtime would honor — the *last* occurrence of each key.
    pub values: RuntimeConfigValues,
    /// [`runtime_defaults`], so the UI can say "runtime default: 72" without
    /// hard-coding it in TypeScript.
    pub defaults: RuntimeConfigValues,
    /// Keys present but set to something the runtime ignores.
    pub invalid: Vec<InvalidValue>,
    /// Keys assigned more than once across the document. The last assignment
    /// wins; the earlier ones are dead lines the user probably thinks are live.
    pub shadowed: Vec<String>,
    pub modified_unix_ms: Option<u64>,
    /// Set when `toml_edit` could not parse the file: `values` then come from
    /// the line-oriented fallback reader (what the runtime would use) and
    /// [`write`] refuses.
    pub parse_error: Option<String>,
}

/// What [`apply_patch`] produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Patched {
    pub text: String,
    /// Keys whose bytes actually changed — a patch that re-sets a key to the
    /// value already on disk leaves it out.
    pub changed_keys: Vec<String>,
    /// Patched keys that had more than one assignment in the document.
    pub shadowed: Vec<String>,
}

/// What [`write`] did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteReport {
    /// The file was absent and was created from [`crate::util::toml_template`]
    /// before the patch was applied (the write-once invariant: those exact
    /// bytes, never a rendering of the patch).
    pub created_from_template: bool,
    pub backup_path: Option<String>,
    pub changed_keys: Vec<String>,
    pub shadowed: Vec<String>,
    pub path: String,
}

/// One file in the backups directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupInfo {
    pub path: String,
    pub created_unix_secs: u64,
    pub size: u64,
}

// ── read ─────────────────────────────────────────────────────────────────────

/// Read the file. Never fails: an absent file, an unreadable one and one
/// `toml_edit` cannot parse are all *states of the view*, not errors — the
/// Settings screen must be able to render every one of them.
pub fn read(path: &Path) -> RuntimeConfigView {
    let mut view = RuntimeConfigView {
        path: path.display().to_string(),
        exists: path.is_file(),
        values: RuntimeConfigValues::default(),
        defaults: runtime_defaults(),
        invalid: Vec::new(),
        shadowed: Vec::new(),
        modified_unix_ms: modified_unix_ms(path),
        parse_error: None,
    };
    if !view.exists {
        return view;
    }
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            view.parse_error = Some(format!("cannot read {}: {e}", path.display()));
            return view;
        }
    };

    match text.parse::<DocumentMut>() {
        Ok(doc) => {
            let (values, invalid, shadowed) = harvest(&doc);
            view.values = values;
            view.invalid = invalid;
            view.shadowed = shadowed;
        }
        Err(e) => {
            // The runtime does not care that the file is invalid TOML — it
            // reads lines. Show what it would use, and refuse to write.
            view.parse_error = Some(e.to_string());
            let (values, invalid, shadowed) = read_lines_like_the_runtime(&text);
            view.values = values;
            view.invalid = invalid;
            view.shadowed = shadowed;
        }
    }
    view
}

/// Millisecond mtime, for "the file changed under us" checks in the UI.
fn modified_unix_ms(path: &Path) -> Option<u64> {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
}

/// Pull the six keys out of a parsed document with the runtime's resolution
/// rules: every table counts, the last assignment wins.
fn harvest(doc: &DocumentMut) -> (RuntimeConfigValues, Vec<InvalidValue>, Vec<String>) {
    let mut values = RuntimeConfigValues::default();
    let mut invalid = Vec::new();
    let mut shadowed = Vec::new();

    for key in EDITABLE_KEYS {
        let occurrences = occurrences_of(doc.as_table(), key);
        if occurrences.len() > 1 {
            shadowed.push(key.to_string());
        }
        let Some(last) = occurrences.last() else {
            continue;
        };
        let Some(value) = value_at(doc.as_table(), &last.path, key) else {
            continue;
        };
        match interpret(key, value) {
            Ok(()) => {}
            Err(bad) => {
                invalid.push(bad);
                continue;
            }
        }
        assign(&mut values, key, value);
    }
    (values, invalid, shadowed)
}

/// Type-check one `toml_edit` value against the runtime's accepted set,
/// producing the [`InvalidValue`] the UI shows when it does not fit.
fn interpret(key: &str, value: &Value) -> std::result::Result<(), InvalidValue> {
    let raw = value_repr(value);
    match key {
        "protocol" => match value.as_str().and_then(Protocol::parse) {
            Some(_) => Ok(()),
            None => Err(InvalidValue::new(
                key,
                raw,
                "expected \"alvr\" or \"oxrsys\"",
            )),
        },
        "video_codec" => match value.as_str().and_then(VideoCodec::parse) {
            Some(_) => Ok(()),
            None => Err(InvalidValue::new(
                key,
                raw,
                "expected \"auto\", \"h265\" or \"h264\"",
            )),
        },
        "encoder_process" => match value.as_str().and_then(EncoderProcess::parse) {
            Some(_) => Ok(()),
            None => Err(InvalidValue::new(
                key,
                raw,
                "expected \"auto\", \"native\" or \"inproc\"",
            )),
        },
        "bitrate_mbps" => match value.as_integer() {
            Some(n) if in_bitrate_range(n) => Ok(()),
            _ => Err(InvalidValue::new(key, raw, bitrate_reason())),
        },
        "refresh_rate_hz" => match value.as_integer() {
            Some(n) if REFRESH_RATES.iter().any(|r| i64::from(*r) == n) => Ok(()),
            _ => Err(InvalidValue::new(key, raw, refresh_reason())),
        },
        "resolution_scale" => match as_scale(value) {
            Some(f) if in_scale_range(f) => Ok(()),
            _ => Err(InvalidValue::new(key, raw, scale_reason())),
        },
        _ => Ok(()),
    }
}

/// Store an already-validated value.
fn assign(values: &mut RuntimeConfigValues, key: &str, value: &Value) {
    match key {
        "protocol" => values.protocol = value.as_str().and_then(Protocol::parse),
        "video_codec" => values.video_codec = value.as_str().and_then(VideoCodec::parse),
        "encoder_process" => {
            values.encoder_process = value.as_str().and_then(EncoderProcess::parse)
        }
        "bitrate_mbps" => values.bitrate_mbps = value.as_integer().map(|n| n as u32),
        "refresh_rate_hz" => values.refresh_rate_hz = value.as_integer().map(|n| n as u32),
        "resolution_scale" => values.resolution_scale = as_scale(value),
        _ => {}
    }
}

/// `resolution_scale` is a float key, but `1` parses as a TOML integer and the
/// runtime's `std::stod` accepts it, so accept both here too.
fn as_scale(value: &Value) -> Option<f64> {
    value
        .as_float()
        .or_else(|| value.as_integer().map(|n| n as f64))
}

fn in_bitrate_range(n: i64) -> bool {
    n >= i64::from(BITRATE_RANGE.0) && n <= i64::from(BITRATE_RANGE.1)
}

fn in_scale_range(f: f64) -> bool {
    f.is_finite() && f >= RESOLUTION_SCALE_RANGE.0 && f <= RESOLUTION_SCALE_RANGE.1
}

fn bitrate_reason() -> String {
    format!(
        "expected an integer {}..={}",
        BITRATE_RANGE.0, BITRATE_RANGE.1
    )
}

fn refresh_reason() -> String {
    format!(
        "expected one of {}",
        REFRESH_RATES
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn scale_reason() -> String {
    format!(
        "expected a number {}..={}",
        RESOLUTION_SCALE_RANGE.0, RESOLUTION_SCALE_RANGE.1
    )
}

// ── the fallback reader (the runtime's own semantics) ────────────────────────

/// `ext/oxrsys/runtime/src/Config.cpp`, ported: strip the quote-aware `#`
/// comment, skip blanks and `[table]` headers, split on the first `=`, trim,
/// last assignment wins.
///
/// Used only when `toml_edit` cannot parse the file. It is deliberately *not*
/// the primary reader: it cannot round-trip, so nothing built on it may write.
pub fn read_lines_like_the_runtime(
    text: &str,
) -> (RuntimeConfigValues, Vec<InvalidValue>, Vec<String>) {
    let mut seen: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        let line = strip_comment(line);
        let line = line.trim();
        if line.is_empty() || line.starts_with('[') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        seen.push((k.trim().to_string(), v.trim().to_string()));
    }

    let mut values = RuntimeConfigValues::default();
    let mut invalid = Vec::new();
    let mut shadowed = Vec::new();
    for key in EDITABLE_KEYS {
        let hits: Vec<&String> = seen
            .iter()
            .filter(|(k, _)| k == key)
            .map(|(_, v)| v)
            .collect();
        if hits.len() > 1 {
            shadowed.push(key.to_string());
        }
        let Some(raw) = hits.last() else { continue };
        match interpret_raw(key, raw) {
            Ok(()) => assign_raw(&mut values, key, raw),
            Err(bad) => invalid.push(bad),
        }
    }
    (values, invalid, shadowed)
}

/// The runtime's comment stripper: `#` ends the line unless it is inside a
/// quoted string. Both quote flavours, with backslash escapes in basic strings.
fn strip_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_basic = false;
    let mut in_literal = false;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if in_basic => i += 1,
            b'"' if !in_literal => in_basic = !in_basic,
            b'\'' if !in_basic => in_literal = !in_literal,
            b'#' if !in_basic && !in_literal => return &line[..i],
            _ => {}
        }
        i += 1;
    }
    line
}

/// Strip one layer of matching quotes, the way the runtime does before
/// comparing a string value.
fn unquote(raw: &str) -> &str {
    let b = raw.as_bytes();
    if b.len() >= 2
        && ((b[0] == b'"' && b[b.len() - 1] == b'"') || (b[0] == b'\'' && b[b.len() - 1] == b'\''))
    {
        &raw[1..raw.len() - 1]
    } else {
        raw
    }
}

fn interpret_raw(key: &str, raw: &str) -> std::result::Result<(), InvalidValue> {
    let s = unquote(raw);
    match key {
        "protocol" => Protocol::parse(s)
            .map(|_| ())
            .ok_or_else(|| InvalidValue::new(key, raw, "expected \"alvr\" or \"oxrsys\"")),
        "video_codec" => VideoCodec::parse(s)
            .map(|_| ())
            .ok_or_else(|| InvalidValue::new(key, raw, "expected \"auto\", \"h265\" or \"h264\"")),
        "encoder_process" => EncoderProcess::parse(s).map(|_| ()).ok_or_else(|| {
            InvalidValue::new(key, raw, "expected \"auto\", \"native\" or \"inproc\"")
        }),
        "bitrate_mbps" => match s.parse::<i64>() {
            Ok(n) if in_bitrate_range(n) => Ok(()),
            _ => Err(InvalidValue::new(key, raw, bitrate_reason())),
        },
        "refresh_rate_hz" => match s.parse::<i64>() {
            Ok(n) if REFRESH_RATES.iter().any(|r| i64::from(*r) == n) => Ok(()),
            _ => Err(InvalidValue::new(key, raw, refresh_reason())),
        },
        "resolution_scale" => match s.parse::<f64>() {
            Ok(f) if in_scale_range(f) => Ok(()),
            _ => Err(InvalidValue::new(key, raw, scale_reason())),
        },
        _ => Ok(()),
    }
}

fn assign_raw(values: &mut RuntimeConfigValues, key: &str, raw: &str) {
    let s = unquote(raw);
    match key {
        "protocol" => values.protocol = Protocol::parse(s),
        "video_codec" => values.video_codec = VideoCodec::parse(s),
        "encoder_process" => values.encoder_process = EncoderProcess::parse(s),
        "bitrate_mbps" => values.bitrate_mbps = s.parse::<u32>().ok(),
        "refresh_rate_hz" => values.refresh_rate_hz = s.parse::<u32>().ok(),
        "resolution_scale" => values.resolution_scale = s.parse::<f64>().ok(),
        _ => {}
    }
}

// ── validate ─────────────────────────────────────────────────────────────────

/// The runtime's own bounds, applied to a patch before anything touches disk.
///
/// The enums are unrepresentable-when-wrong; only the three numeric keys can
/// carry a value the runtime would ignore, and writing one is worse than
/// refusing — the file would look edited and the runtime would keep its
/// default.
pub fn validate(patch: &RuntimeConfigPatch) -> Vec<InvalidValue> {
    let mut out = Vec::new();
    if let Some(n) = patch.bitrate_mbps {
        if !in_bitrate_range(i64::from(n)) {
            out.push(InvalidValue::new(
                "bitrate_mbps",
                n.to_string(),
                bitrate_reason(),
            ));
        }
    }
    if let Some(hz) = patch.refresh_rate_hz {
        if !REFRESH_RATES.contains(&hz) {
            out.push(InvalidValue::new(
                "refresh_rate_hz",
                hz.to_string(),
                refresh_reason(),
            ));
        }
    }
    if let Some(f) = patch.resolution_scale {
        if !in_scale_range(f) {
            out.push(InvalidValue::new(
                "resolution_scale",
                format!("{f}"),
                scale_reason(),
            ));
        }
    }
    out
}

// ── occurrence walk ──────────────────────────────────────────────────────────

/// One step of the path from the document root to a table.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Seg {
    Table(String),
    ArrayOfTables(String, usize),
}

/// One assignment of a key, with the physical order of the table it lives in.
#[derive(Debug, Clone)]
struct Occurrence {
    /// `toml_edit` numbers tables in document order; the root's key-values are
    /// always physically first (TOML forbids a bare key after a header), which
    /// `-1` encodes.
    order: isize,
    path: Vec<Seg>,
}

/// Every assignment of `key` in the document, in physical order.
///
/// **Dotted tables are skipped on purpose.** `streaming.protocol = "alvr"` is
/// one physical line whose key text is `streaming.protocol`, so the runtime's
/// line reader does *not* see it as `protocol`. Descending into it would make
/// Sabrage edit a line the runtime ignores.
fn occurrences_of(root: &Table, key: &str) -> Vec<Occurrence> {
    let mut out = Vec::new();
    walk(root, -1, &mut Vec::new(), key, &mut out);
    out.sort_by_key(|o| o.order);
    out
}

/// Whether the document's spelling of `key` is the bare one the runtime's
/// reader recognises.
///
/// Same reasoning as the dotted-table skip above: the runtime splits the line
/// on its first `=` and trims, so the key text of `"protocol" = "oxrsys"` is
/// literally `"protocol"` — quotes included — and matches nothing. `toml_edit`
/// decodes it to `protocol`, so without this check Sabrage would read, edit and
/// report success on a line the runtime ignores.
///
/// A key we synthesised ourselves has no repr yet; it renders bare.
fn key_is_bare(table: &Table, key: &str) -> bool {
    match table
        .key(key)
        .and_then(|k| k.as_repr())
        .and_then(|r| r.as_raw().as_str())
    {
        Some(raw) => raw.trim() == key,
        None => true,
    }
}

fn walk(table: &Table, order: isize, path: &mut Vec<Seg>, key: &str, out: &mut Vec<Occurrence>) {
    if matches!(table.get(key), Some(Item::Value(_))) && key_is_bare(table, key) {
        out.push(Occurrence {
            order,
            path: path.clone(),
        });
    }
    for (name, item) in table.iter() {
        match item {
            Item::Table(sub) if !sub.is_dotted() => {
                path.push(Seg::Table(name.to_string()));
                walk(sub, sub.position().unwrap_or(-1), path, key, out);
                path.pop();
            }
            Item::ArrayOfTables(aot) => {
                for (i, sub) in aot.iter().enumerate() {
                    path.push(Seg::ArrayOfTables(name.to_string(), i));
                    walk(sub, sub.position().unwrap_or(-1), path, key, out);
                    path.pop();
                }
            }
            _ => {}
        }
    }
}

fn table_at<'a>(root: &'a Table, path: &[Seg]) -> Option<&'a Table> {
    let mut cur = root;
    for seg in path {
        cur = match seg {
            Seg::Table(name) => cur.get(name)?.as_table()?,
            Seg::ArrayOfTables(name, i) => cur.get(name)?.as_array_of_tables()?.get(*i)?,
        };
    }
    Some(cur)
}

fn table_at_mut<'a>(root: &'a mut Table, path: &[Seg]) -> Option<&'a mut Table> {
    let mut cur = root;
    for seg in path {
        cur = match seg {
            Seg::Table(name) => cur.get_mut(name)?.as_table_mut()?,
            Seg::ArrayOfTables(name, i) => {
                cur.get_mut(name)?.as_array_of_tables_mut()?.get_mut(*i)?
            }
        };
    }
    Some(cur)
}

fn value_at<'a>(root: &'a Table, path: &[Seg], key: &str) -> Option<&'a Value> {
    table_at(root, path)?.get(key)?.as_value()
}

// ── apply_patch ──────────────────────────────────────────────────────────────

/// Apply a patch to the document text, preserving every byte it does not have
/// to change.
///
/// Pure: no I/O, no clock, no executor. [`write`] is the only caller that
/// touches disk, and the golden tests drive this function directly.
pub fn apply_patch(text: &str, patch: &RuntimeConfigPatch) -> Result<Patched> {
    let bad = validate(patch);
    if !bad.is_empty() {
        let detail = bad
            .iter()
            .map(|b| format!("{} = {} ({})", b.key, b.raw, b.reason))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(SabrageError::InvalidInput(format!(
            "refusing to write values the runtime would ignore: {detail}"
        )));
    }

    let mut doc: DocumentMut = text.parse().map_err(|e| {
        SabrageError::InvalidInput(format!(
            "oxrsys-runtime.toml is not valid TOML, refusing to rewrite it: {e}"
        ))
    })?;

    let mut changed_keys = Vec::new();
    let mut shadowed = Vec::new();

    for key in EDITABLE_KEYS {
        let Some(new) = patch_value(key, patch) else {
            continue;
        };
        let occurrences = occurrences_of(doc.as_table(), key);
        if occurrences.len() > 1 {
            shadowed.push(key.to_string());
        }
        let changed = match occurrences.last() {
            // The runtime honors the LAST assignment; that is the one to edit.
            Some(last) => {
                let path = last.path.clone();
                let table = table_at_mut(doc.as_table_mut(), &path).ok_or_else(|| {
                    SabrageError::InvalidInput(format!(
                        "internal: lost the table holding '{key}' between passes"
                    ))
                })?;
                edit_in_place(table, key, new)
            }
            None => insert_into_streaming(&mut doc, key, new)?,
        };
        if changed {
            changed_keys.push(key.to_string());
        }
    }

    // Rule 6 taken literally: when no key was edited, hand back the input
    // bytes. `doc.to_string()` is a re-render, and `toml_edit` normalises CRLF
    // to LF, drops a leading BOM and appends a missing trailing newline — so on
    // a file that is not already LF + newline-terminated an *empty* patch would
    // come back different from its input, and [`write`] would back the file up
    // and rewrite it whole while reporting `changed_keys: []`. "Nothing
    // changed" means no key changed, never "the re-render happens to match".
    let text = if changed_keys.is_empty() {
        text.to_string()
    } else {
        doc.to_string()
    };

    Ok(Patched {
        text,
        changed_keys,
        shadowed,
    })
}

/// The patch's value for one key, rendered as a `toml_edit` value with the
/// canonical spelling: integers bare (`80`), floats always with a fractional
/// part (`1.0`, never `1` — `resolution_scale` is a float key and a bare `1`
/// reads as an integer to a stricter parser), strings as basic strings.
fn patch_value(key: &str, patch: &RuntimeConfigPatch) -> Option<Value> {
    match key {
        "protocol" => patch.protocol.map(|p| string_value(p.as_str())),
        "bitrate_mbps" => patch
            .bitrate_mbps
            .map(|n| Value::Integer(Formatted::new(i64::from(n)))),
        "encoder_process" => patch.encoder_process.map(|e| string_value(e.as_str())),
        "video_codec" => patch.video_codec.map(|c| string_value(c.as_str())),
        "resolution_scale" => patch
            .resolution_scale
            .map(|f| Value::Float(Formatted::new(f))),
        "refresh_rate_hz" => patch
            .refresh_rate_hz
            .map(|n| Value::Integer(Formatted::new(i64::from(n)))),
        _ => None,
    }
}

/// A value's own text, with its decor (the `=` spacing and any trailing
/// comment) stripped — `toml_edit`'s `Display` for a `Value` includes both.
fn value_repr(value: &Value) -> String {
    let mut bare = value.clone();
    bare.decor_mut().set_prefix("");
    bare.decor_mut().set_suffix("");
    bare.to_string()
}

fn string_value(s: &str) -> Value {
    Value::String(Formatted::new(s.to_string()))
}

/// Replace the value of an existing key, keeping the key, its own decor and the
/// `=` spacing. Returns whether anything changed.
///
/// A key already carrying the wanted value is left **completely** untouched —
/// including a same-line comment. Rule 5 relocates the comment on a line this
/// function rewrites; it is not a licence to reformat a line the patch did not
/// need to touch, which is what makes "an unchanged patch writes nothing" hold.
fn edit_in_place(table: &mut Table, key: &str, new: Value) -> bool {
    let Some(old) = table.get(key).and_then(Item::as_value).cloned() else {
        return false;
    };
    let old_decor = old.decor().clone();
    let prefix = decor_str(old_decor.prefix());
    let suffix = decor_str(old_decor.suffix());
    if same_to_the_runtime(&old, &new) {
        return false;
    }

    // Rule 5: a same-line comment on the line we are about to rewrite moves to
    // its own line above the key, at the key's own indentation.
    let trailing_comment = suffix.contains('#').then(|| suffix.trim().to_string());
    if let Some(comment) = &trailing_comment {
        if let Some(mut k) = table.key_mut(key) {
            let key_prefix = decor_str(k.leaf_decor().prefix()).into_owned();
            let indent = key_prefix
                .rsplit_once('\n')
                .map(|(_, tail)| tail)
                .unwrap_or(&key_prefix)
                .to_string();
            k.leaf_decor_mut()
                .set_prefix(format!("{key_prefix}{comment}\n{indent}"));
        }
    }

    let mut value = new;
    value
        .decor_mut()
        .set_prefix(prefix.clone().into_owned().to_string());
    value.decor_mut().set_suffix(if trailing_comment.is_some() {
        String::new()
    } else {
        suffix.into_owned()
    });
    if let Some(item) = table.get_mut(key) {
        *item = Item::Value(value);
    }
    true
}

/// Whether the runtime would read the same value out of both, i.e. whether
/// rewriting the line would be a pure reformat.
///
/// Textual equality alone is too strict for strings: the runtime strips one
/// matching pair of quotes of either flavour ([`unquote`]), so `'alvr'` and
/// `"alvr"` are one value to it, while [`patch_value`] always builds a basic
/// string. Without this, re-saving a value a hand-maintained file spells with
/// literal quotes would count as a change and burn a backup on a rewrite the
/// runtime cannot observe.
///
/// Numbers stay on textual comparison on purpose: `0x50` and `80` are the same
/// integer to `toml_edit` and *not* to the runtime, which parses the raw line
/// text — a number spelled in a form it would misread must still be rewritten.
/// So must a multi-line or escaped string, whose one-layer unquote does not
/// yield the wanted text.
fn same_to_the_runtime(old: &Value, new: &Value) -> bool {
    let old_repr = value_repr(old);
    if old_repr == value_repr(new) {
        return true;
    }
    matches!(
        (old.as_str(), new.as_str()),
        (Some(_), Some(wanted)) if unquote(&old_repr) == wanted
    )
}

fn decor_str(raw: Option<&toml_edit::RawString>) -> std::borrow::Cow<'_, str> {
    raw.and_then(|r| r.as_str())
        .map(std::borrow::Cow::Borrowed)
        .unwrap_or(std::borrow::Cow::Borrowed(""))
}

/// Insert a key that is nowhere in the document into `[streaming]`, after that
/// table's last key. The table is created at the end of the document, with one
/// blank line before its header, when it is absent.
fn insert_into_streaming(doc: &mut DocumentMut, key: &str, new: Value) -> Result<bool> {
    let root_empty = doc.as_table().is_empty();
    match doc.as_table().get(TABLE) {
        None => {
            let mut table = Table::new();
            table.set_implicit(false);
            // Explicit rather than relying on toml_edit's default, so an empty
            // document does not gain a leading blank line.
            table
                .decor_mut()
                .set_prefix(if root_empty { "" } else { "\n" });
            doc.as_table_mut().insert(TABLE, Item::Table(table));
        }
        Some(Item::Table(t)) if t.is_dotted() => {
            return Err(SabrageError::InvalidInput(format!(
                "'{TABLE}' is a dotted key group in this file, so '{key}' cannot be added under a \
                 [{TABLE}] header without changing what the runtime reads — add the line by hand"
            )));
        }
        Some(Item::Table(_)) => {}
        Some(other) => {
            return Err(SabrageError::InvalidInput(format!(
                "'{TABLE}' is a {} in this file, not a table — cannot add '{key}' under it",
                other.type_name()
            )));
        }
    }

    let table = doc
        .as_table_mut()
        .get_mut(TABLE)
        .and_then(Item::as_table_mut)
        .expect("just ensured [streaming] is a table");
    if table.is_implicit() {
        // Only `[streaming.x]` headers exist; give the key a real home.
        table.set_implicit(false);
    }
    if let Some(existing) = table.get(key) {
        if !existing.is_value() {
            return Err(SabrageError::InvalidInput(format!(
                "'{TABLE}.{key}' is a {} in this file, not a value",
                existing.type_name()
            )));
        }
        if !key_is_bare(table, key) {
            // `Table::insert` would keep the existing quoted key and swap its
            // value — editing the one line the runtime does not read while
            // leaving the document with no line that it does.
            return Err(SabrageError::InvalidInput(format!(
                "'{key}' is written as a quoted key in [{TABLE}], which the runtime's line reader \
                 does not recognise — rewrite it as a bare '{key} = …' by hand"
            )));
        }
    }
    table.insert(key, Item::Value(new));
    Ok(true)
}

// ── write ────────────────────────────────────────────────────────────────────

/// Patch the file on disk, creating it from the shared template first when it
/// is absent and backing up the previous contents when it is not.
///
/// DIVERGENCE (PARITY.md, "Config"): `setup.sh` writes `oxrsys-runtime.toml`
/// once and never again — its non-alvr branch prints "edit it yourself".
/// Sabrage edits the six streaming keys in place instead, which is the whole
/// point of the Settings screen. The write-once invariant is preserved where it
/// is load-bearing: a file that does not exist is created with
/// [`crate::util::toml_template`] byte-for-byte (never a rendering of the
/// patch), so the two front-ends still agree on first-write bytes, and the
/// patch is applied to those bytes afterwards.
///
/// Every mutation goes through `executor`, so a [`crate::DryRunExecutor`] plans
/// the create, the backup, the prune and the write without touching disk.
pub async fn write(
    executor: &dyn Executor,
    toml_path: &Path,
    backups_dir: &Path,
    patch: &RuntimeConfigPatch,
) -> Result<WriteReport> {
    let bad = validate(patch);
    if !bad.is_empty() {
        let detail = bad
            .iter()
            .map(|b| format!("{} = {} ({})", b.key, b.raw, b.reason))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(SabrageError::InvalidInput(format!(
            "refusing to write values the runtime would ignore: {detail}"
        )));
    }

    let exists = toml_path.is_file();
    let base = if exists {
        std::fs::read_to_string(toml_path).map_err(|e| SabrageError::io(toml_path, e))?
    } else {
        if let Some(parent) = toml_path.parent() {
            executor.create_dir_all(parent).await?;
        }
        let template = crate::util::toml_template();
        executor
            .write_atomic(toml_path, template.as_bytes())
            .await?;
        template.to_string()
    };

    let patched = apply_patch(&base, patch)?;
    let mut report = WriteReport {
        created_from_template: !exists,
        backup_path: None,
        changed_keys: patched.changed_keys.clone(),
        shadowed: patched.shadowed.clone(),
        path: toml_path.display().to_string(),
    };

    if patched.changed_keys.is_empty() {
        // Nothing to write, so nothing to back up either. An existing file is
        // not even opened for writing — no mtime bump, no backup churn from a
        // Settings screen that saved without changing anything.
        //
        // The test is "no key changed", NOT `patched.text == base`: `apply_patch`
        // guarantees the identity in that case anyway, and comparing rendered
        // text used to rewrite (and back up) any file whose bytes `toml_edit`
        // would normalise — CRLF, a BOM, a missing final newline — on a patch
        // that changed nothing.
        return Ok(report);
    }

    if exists {
        let backup = next_backup_path(backups_dir, unix_secs());
        // The prune list is computed BEFORE the new backup is written: a real
        // run would otherwise see it in the listing and a dry run would not,
        // and the two modes must plan the same set of removals.
        let stale: Vec<BackupInfo> = list_backups(backups_dir)
            .into_iter()
            .skip(BACKUP_KEEP.saturating_sub(1))
            .collect();
        executor.create_dir_all(backups_dir).await?;
        executor.write_atomic(&backup, base.as_bytes()).await?;
        for old in stale {
            executor.remove_file(Path::new(&old.path)).await?;
        }
        report.backup_path = Some(backup.display().to_string());
    }

    executor
        .write_atomic(toml_path, patched.text.as_bytes())
        .await?;
    Ok(report)
}

fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `<backups_dir>/oxrsys-runtime.toml.<secs>`, then `-2`, `-3`, … until the name
/// is free. Two saves inside one second are ordinary (a slider release plus a
/// keyboard nudge), and overwriting the earlier backup would throw away the
/// state the user is most likely to want back.
fn next_backup_path(backups_dir: &Path, secs: u64) -> PathBuf {
    let base = format!("{BACKUP_PREFIX}{secs}");
    let first = backups_dir.join(&base);
    if !first.exists() {
        return first;
    }
    for n in 2u32.. {
        let candidate = backups_dir.join(format!("{base}-{n}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!("u32 exhausted")
}

/// The backups in `backups_dir`, newest first.
///
/// The timestamp comes from the filename, not the mtime: the name is what the
/// pruning order is defined on, and a restore-from-Time-Machine would otherwise
/// reshuffle the list.
pub fn list_backups(backups_dir: &Path) -> Vec<BackupInfo> {
    let Ok(entries) = std::fs::read_dir(backups_dir) else {
        return Vec::new();
    };
    let mut out: Vec<(u64, u32, BackupInfo)> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(suffix) = name.strip_prefix(BACKUP_PREFIX) else {
            continue;
        };
        let (secs_text, index) = match suffix.split_once('-') {
            Some((s, n)) => (s, n.parse::<u32>().unwrap_or(0)),
            None => (suffix, 1),
        };
        let Ok(secs) = secs_text.parse::<u64>() else {
            continue;
        };
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        out.push((
            secs,
            index,
            BackupInfo {
                path: entry.path().display().to_string(),
                created_unix_secs: secs,
                size,
            },
        ));
    }
    out.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    out.into_iter().map(|(_, _, info)| info).collect()
}

// ── fix.edit-protocol ────────────────────────────────────────────────────────

const EDIT_PROTOCOL_STEP: StepId = "fix.edit-protocol";

/// [`FixAction::EditProtocol`] — set `protocol = "alvr"`, the remedy the
/// contract binds to `cfg.protocol.supported` and `cfg.protocol.legacy-oxrsys`.
///
/// doctor's remedy string is `set protocol = "alvr" in <TOML>`
/// ([`crate::checks::config`]); this is that sentence, mechanised. It is the
/// only fix that writes the runtime config, and it goes through [`write`], so
/// it inherits the backup, the create-from-template branch and the dry-run
/// plan.
pub async fn edit_protocol(ctx: &StageCtx, sink: &EventSink) -> Result<FixReport> {
    if let Some(live) = crate::session::live_session() {
        return Err(ctx.fatal(
            format!(
                "refusing to edit {} while a session is live (bottle '{}') — the runtime reads \
                 this file once at game start",
                ctx.paths.toml_path.display(),
                live.bottle
            ),
            Some("./demo.sh stop --bottle <name>".to_string()),
        ));
    }

    let patch = RuntimeConfigPatch {
        protocol: Some(Protocol::Alvr),
        ..RuntimeConfigPatch::default()
    };
    let executor = ctx.executor_for(EDIT_PROTOCOL_STEP);
    let report = write(
        &*executor,
        &ctx.paths.toml_path,
        &ctx.paths.sabrage_appsup.join("backups"),
        &patch,
    )
    .await?;

    let changed = report.created_from_template || !report.changed_keys.is_empty();
    if !changed {
        let description = format!(
            "{} already has protocol = \"alvr\"",
            ctx.paths.toml_path.display()
        );
        sink(StageEvent::info(
            ctx.run_id,
            Some(EDIT_PROTOCOL_STEP),
            description.clone(),
        ));
        return Ok(FixReport::unchanged(FixAction::EditProtocol, description));
    }

    let verb = if executor.is_dry_run() {
        "would set"
    } else {
        "set"
    };
    let mut description = format!(
        "{verb} protocol = \"alvr\" in {}",
        ctx.paths.toml_path.display()
    );
    if report.created_from_template {
        description.push_str(" (file was absent — created from the shared template first)");
    }
    if let Some(backup) = &report.backup_path {
        description.push_str(&format!(" (previous contents backed up to {backup})"));
    }
    if !report.shadowed.is_empty() {
        description.push_str(&format!(
            " — note: 'protocol' is assigned {} times in this file; the last one wins",
            report.shadowed.len() + 1
        ));
    }
    if executor.is_dry_run() {
        sink(StageEvent::info(
            ctx.run_id,
            Some(EDIT_PROTOCOL_STEP),
            description.clone(),
        ));
    } else {
        sink(StageEvent::ok(
            ctx.run_id,
            Some(EDIT_PROTOCOL_STEP),
            description.clone(),
        ));
    }
    Ok(FixReport::changed(FixAction::EditProtocol, description))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::{DryRunExecutor, PlannedKind, RealExecutor};
    use crate::paths::Paths;
    use crate::stages::{null_sink, StageCtx};
    use std::path::PathBuf;
    use tokio_util::sync::CancellationToken;

    fn real() -> RealExecutor {
        RealExecutor::new(uuid::Uuid::new_v4(), null_sink(), CancellationToken::new())
    }

    fn dry() -> DryRunExecutor {
        DryRunExecutor::new(uuid::Uuid::new_v4(), null_sink(), CancellationToken::new())
    }

    // ── fixtures ─────────────────────────────────────────────────────────────

    fn fixture(name: &str) -> String {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/phase4")
            .join(name);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    fn deployed() -> String {
        fixture("oxrsys-runtime.deployed.toml")
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sabrage-runtime-toml-{tag}-{}-{}",
            std::process::id(),
            unix_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn unix_nanos() -> u128 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    }

    fn patch_bitrate(n: u32) -> RuntimeConfigPatch {
        RuntimeConfigPatch {
            bitrate_mbps: Some(n),
            ..RuntimeConfigPatch::default()
        }
    }

    /// The lines that differ between two texts, as (before, after) pairs.
    fn diff_lines(before: &str, after: &str) -> Vec<(String, String)> {
        let b: Vec<&str> = before.lines().collect();
        let a: Vec<&str> = after.lines().collect();
        assert_eq!(
            b.len(),
            a.len(),
            "line count changed\n--- {before}\n+++ {after}"
        );
        b.iter()
            .zip(a.iter())
            .filter(|(x, y)| x != y)
            .map(|(x, y)| (x.to_string(), y.to_string()))
            .collect()
    }

    // ── rule 6: identity ─────────────────────────────────────────────────────

    #[test]
    fn an_empty_patch_is_the_identity_on_the_deployed_file() {
        let text = deployed();
        let out = apply_patch(&text, &RuntimeConfigPatch::default()).unwrap();
        assert_eq!(out.text, text, "empty patch must be byte-identical");
        assert!(out.changed_keys.is_empty());
        assert!(out.shadowed.is_empty());
    }

    #[test]
    fn the_shared_template_round_trips_unchanged() {
        let text = crate::util::toml_template();
        let out = apply_patch(text, &RuntimeConfigPatch::default()).unwrap();
        assert_eq!(out.text, text);
    }

    /// Re-setting every key to what the file already says must not move a byte —
    /// this is what makes "Save with nothing dirty writes nothing" true even
    /// when the UI sends the whole value set.
    #[test]
    fn setting_every_key_to_its_current_value_changes_nothing() {
        let text = deployed();
        let view = read_text(&text);
        let out = apply_patch(&text, &view.values).unwrap();
        assert_eq!(out.text, text);
        assert!(out.changed_keys.is_empty(), "{:?}", out.changed_keys);
    }

    /// Regression: `toml_edit` normalises CRLF to LF, drops a BOM and appends a
    /// missing final newline, so a re-render is NOT the identity on a file that
    /// is not already LF + newline-terminated. An empty patch must never go
    /// through the renderer at all.
    #[test]
    fn an_empty_patch_is_the_identity_on_text_toml_edit_would_normalise() {
        for text in [
            // no trailing newline
            "[streaming]\nbitrate_mbps = 42",
            // CRLF throughout
            "# hdr\r\n[streaming]\r\nprotocol = \"alvr\"\r\nbitrate_mbps = 42\r\n",
            // leading BOM
            "\u{feff}[streaming]\nbitrate_mbps = 42\n",
        ] {
            let out = apply_patch(text, &RuntimeConfigPatch::default()).unwrap();
            assert_eq!(out.text, text, "empty patch rewrote {text:?}");
            assert!(out.changed_keys.is_empty());
        }
    }

    /// The same guarantee for a patch that names keys the file already carries:
    /// nothing changed means nothing rendered.
    #[test]
    fn a_no_op_patch_is_the_identity_without_a_trailing_newline() {
        let text = "# my notes\n[streaming]\nprotocol = \"alvr\"\nbitrate_mbps = 80";
        let out = apply_patch(
            text,
            &RuntimeConfigPatch {
                protocol: Some(Protocol::Alvr),
                bitrate_mbps: Some(80),
                ..RuntimeConfigPatch::default()
            },
        )
        .unwrap();
        assert_eq!(out.text, text);
        assert!(out.changed_keys.is_empty(), "{:?}", out.changed_keys);
    }

    /// A value the file spells with literal quotes is already the value the
    /// runtime reads (its unquote takes either flavour), so re-saving it is a
    /// pure reformat and must not count as a change.
    #[test]
    fn a_literal_quoted_string_is_not_a_change() {
        let text = "[streaming]\nprotocol = 'alvr'\n";
        let out = apply_patch(
            text,
            &RuntimeConfigPatch {
                protocol: Some(Protocol::Alvr),
                ..RuntimeConfigPatch::default()
            },
        )
        .unwrap();
        assert_eq!(out.text, text);
        assert!(out.changed_keys.is_empty(), "{:?}", out.changed_keys);
        // A different value still rewrites, and in the canonical spelling.
        let out = apply_patch(
            text,
            &RuntimeConfigPatch {
                protocol: Some(Protocol::Oxrsys),
                ..RuntimeConfigPatch::default()
            },
        )
        .unwrap();
        assert_eq!(out.text, "[streaming]\nprotocol = \"oxrsys\"\n");
        assert_eq!(out.changed_keys, vec!["protocol".to_string()]);
    }

    /// A quoted key is a different key to the runtime's line reader (its key
    /// text keeps the quotes), so `toml_edit`'s decoded view must not be
    /// allowed to disagree with it — same rule as the dotted key above. Here
    /// the quoted spelling is the LAST one in the document, so before the fix
    /// it was both the value `read` reported and the line `apply_patch` edited,
    /// while the runtime only ever saw the bare one in `[streaming]`.
    #[test]
    fn a_quoted_key_is_not_an_occurrence() {
        let text = "[streaming]\nprotocol = \"alvr\"\n\n[extra]\n\"protocol\" = \"oxrsys\"\n";
        let view = read_text(text);
        let (line_values, _, line_shadowed) = read_lines_like_the_runtime(text);
        assert_eq!(view.values.protocol, Some(Protocol::Alvr));
        assert_eq!(view.values.protocol, line_values.protocol);
        assert!(view.shadowed.is_empty(), "{:?}", view.shadowed);
        assert_eq!(view.shadowed, line_shadowed);

        // ...and the edit lands on the live line, not the quoted one.
        let out = apply_patch(
            text,
            &RuntimeConfigPatch {
                protocol: Some(Protocol::Oxrsys),
                ..RuntimeConfigPatch::default()
            },
        )
        .unwrap();
        assert_eq!(
            out.text,
            "[streaming]\nprotocol = \"oxrsys\"\n\n[extra]\n\"protocol\" = \"oxrsys\"\n"
        );
        assert_eq!(out.changed_keys, vec!["protocol".to_string()]);
    }

    /// When the quoted key is the ONLY spelling in `[streaming]`, `insert`
    /// would swap its value and leave the document with no line the runtime
    /// reads. Refuse and say so, the way a dotted `[streaming]` is refused.
    #[test]
    fn a_key_that_exists_only_as_a_quoted_key_is_refused() {
        let err = apply_patch("[streaming]\n\"bitrate_mbps\" = 42\n", &patch_bitrate(60))
            .unwrap_err()
            .to_string();
        assert!(err.contains("bitrate_mbps"), "{err}");
        assert!(err.contains("quoted key"), "{err}");
    }

    fn read_text(text: &str) -> RuntimeConfigView {
        let doc: DocumentMut = text.parse().unwrap();
        let (values, invalid, shadowed) = harvest(&doc);
        RuntimeConfigView {
            path: String::new(),
            exists: true,
            values,
            defaults: runtime_defaults(),
            invalid,
            shadowed,
            modified_unix_ms: None,
            parse_error: None,
        }
    }

    // ── rule 4/6: the golden one-line edit ───────────────────────────────────

    #[test]
    fn editing_the_bitrate_changes_exactly_one_line() {
        let text = deployed();
        let out = apply_patch(&text, &patch_bitrate(60)).unwrap();
        assert_ne!(out.text, text);
        let diff = diff_lines(&text, &out.text);
        assert_eq!(diff.len(), 1, "{diff:?}");
        assert_eq!(diff[0].0, "bitrate_mbps = 80");
        assert_eq!(diff[0].1, "bitrate_mbps = 60");
        assert_eq!(out.changed_keys, vec!["bitrate_mbps".to_string()]);
        // Every comment — including the four-line provenance header — survives.
        assert!(out.text.starts_with("# Restored 2026-07-04"));
        assert!(out
            .text
            .contains("# helper negotiates HEVC; falls back H.264 in-process"));
    }

    #[test]
    fn a_float_keeps_its_fractional_part() {
        let text = deployed();
        let patch = RuntimeConfigPatch {
            resolution_scale: Some(1.0),
            ..RuntimeConfigPatch::default()
        };
        // 1.0 is already on disk: nothing changes, and nothing is reformatted.
        let same = apply_patch(&text, &patch).unwrap();
        assert_eq!(same.text, text);
        assert!(same.changed_keys.is_empty());

        // 0.75 → back to 1.0 renders "1.0", never "1".
        let down = apply_patch(
            &text,
            &RuntimeConfigPatch {
                resolution_scale: Some(0.75),
                ..RuntimeConfigPatch::default()
            },
        )
        .unwrap();
        assert!(down.text.contains("resolution_scale = 0.75"));
        let up = apply_patch(&down.text, &patch).unwrap();
        assert!(up.text.contains("resolution_scale = 1.0"), "{}", up.text);
        assert_eq!(up.text, text);
    }

    #[test]
    fn the_equals_spacing_and_key_decor_survive_an_edit() {
        let text = "[streaming]\n  bitrate_mbps   =    80\n";
        let out = apply_patch(text, &patch_bitrate(60)).unwrap();
        assert_eq!(out.text, "[streaming]\n  bitrate_mbps   =    60\n");
    }

    // ── rule 3: insertion ────────────────────────────────────────────────────

    #[test]
    fn an_absent_key_is_inserted_under_streaming_after_its_last_key() {
        let text = deployed();
        let out = apply_patch(
            &text,
            &RuntimeConfigPatch {
                video_codec: None,
                bitrate_mbps: None,
                ..RuntimeConfigPatch::default()
            },
        )
        .unwrap();
        assert_eq!(out.text, text, "guard: that patch is empty");

        // `abr_mode` is not editable; use a key the deployed file lacks.
        let stripped = text.replace("refresh_rate_hz = 90\n", "");
        assert!(!stripped.contains("refresh_rate_hz"));
        let out = apply_patch(
            &stripped,
            &RuntimeConfigPatch {
                refresh_rate_hz: Some(90),
                ..RuntimeConfigPatch::default()
            },
        )
        .unwrap();
        assert_eq!(out.changed_keys, vec!["refresh_rate_hz".to_string()]);
        assert!(
            out.text.starts_with(&stripped),
            "prefix preserved:\n{}",
            out.text
        );
        assert_eq!(out.text, format!("{stripped}refresh_rate_hz = 90\n"));
    }

    #[test]
    fn a_missing_streaming_table_is_created_at_the_end_with_one_blank_line() {
        let text = "# a hand-written file\nabr_mode = \"off\"\n";
        let out = apply_patch(&text, &patch_bitrate(60)).unwrap();
        assert_eq!(
            out.text,
            "# a hand-written file\nabr_mode = \"off\"\n\n[streaming]\nbitrate_mbps = 60\n"
        );
    }

    #[test]
    fn an_empty_document_gains_no_leading_blank_line() {
        let out = apply_patch("", &patch_bitrate(60)).unwrap();
        assert_eq!(out.text, "[streaming]\nbitrate_mbps = 60\n");
    }

    // ── rule 5: same-line comment relocation ─────────────────────────────────

    #[test]
    fn a_same_line_comment_moves_to_its_own_line_above_the_key() {
        let text = "[streaming]\nbitrate_mbps = 80 # was 42\n";
        let out = apply_patch(text, &patch_bitrate(60)).unwrap();
        assert_eq!(out.text, "[streaming]\n# was 42\nbitrate_mbps = 60\n");
        assert!(!out.text.contains("60 #"), "never leave a trailing comment");
    }

    #[test]
    fn comment_relocation_keeps_the_keys_indentation_and_existing_prefix() {
        let text = "[streaming]\n# note\n    bitrate_mbps = 80  # inline\n";
        let out = apply_patch(text, &patch_bitrate(60)).unwrap();
        assert_eq!(
            out.text,
            "[streaming]\n# note\n    # inline\n    bitrate_mbps = 60\n"
        );
    }

    #[test]
    fn the_inline_comment_fixture_relocates_and_leaves_everything_else_alone() {
        let text = fixture("oxrsys-runtime.inline-comment.toml");
        let out = apply_patch(&text, &patch_bitrate(60)).unwrap();
        assert!(out
            .text
            .contains("# measured 2026-08-10, was 42\nbitrate_mbps = 60\n"));
        // The other same-line comment, on a key we did not touch, stays put.
        assert!(out.text.contains("refresh_rate_hz = 90 # Quest 3 panel\n"));
    }

    // ── rule 2: shadowing ────────────────────────────────────────────────────

    #[test]
    fn a_duplicate_key_across_tables_edits_the_last_and_reports_it() {
        let text = fixture("oxrsys-runtime.shadowed.toml");
        let out = apply_patch(
            &text,
            &RuntimeConfigPatch {
                protocol: Some(Protocol::Alvr),
                ..RuntimeConfigPatch::default()
            },
        )
        .unwrap();
        assert_eq!(out.shadowed, vec!["protocol".to_string()]);
        let diff = diff_lines(&text, &out.text);
        assert_eq!(diff.len(), 1, "{diff:?}");
        assert_eq!(diff[0].0, "protocol = \"oxrsys\"");
        assert_eq!(diff[0].1, "protocol = \"alvr\"");
        // The first (dead) assignment is untouched: it is what the user must
        // see to understand why their edit "did nothing" before.
        assert!(
            out.text.contains("\nprotocol = \"alvr\"\n\n[streaming]\n"),
            "{}",
            out.text
        );
        // ...and the surviving line is the LAST one in the file.
        let last = out.text.rfind("protocol = ").unwrap();
        let first = out.text.find("protocol = ").unwrap();
        assert!(last > first);
    }

    #[test]
    fn reading_the_shadowed_fixture_reports_the_last_value_and_the_shadow() {
        let view = read_text(&fixture("oxrsys-runtime.shadowed.toml"));
        assert_eq!(view.values.protocol, Some(Protocol::Oxrsys));
        assert_eq!(view.shadowed, vec!["protocol".to_string()]);
    }

    /// A dotted key is a different key to the runtime's line reader
    /// (`streaming.protocol`), so it is neither read nor edited.
    #[test]
    fn a_dotted_key_is_not_an_occurrence() {
        let text = "streaming.protocol = \"oxrsys\"\n";
        let view = read_text(text);
        assert_eq!(view.values.protocol, None);
        let err = apply_patch(
            text,
            &RuntimeConfigPatch {
                protocol: Some(Protocol::Alvr),
                ..RuntimeConfigPatch::default()
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("dotted key group"), "{err}");
    }

    // ── rules 1 and 7: refusals ──────────────────────────────────────────────

    #[test]
    fn a_parse_failure_refuses_to_rewrite() {
        let text = fixture("oxrsys-runtime.broken.toml");
        assert!(
            text.parse::<DocumentMut>().is_err(),
            "fixture must be invalid TOML"
        );
        let err = apply_patch(&text, &patch_bitrate(60)).unwrap_err();
        assert!(err.to_string().contains("refusing to rewrite"), "{err}");
    }

    #[test]
    fn out_of_range_values_refuse_and_name_the_key() {
        for (patch, key) in [
            (patch_bitrate(0), "bitrate_mbps"),
            (patch_bitrate(201), "bitrate_mbps"),
            (
                RuntimeConfigPatch {
                    refresh_rate_hz: Some(75),
                    ..RuntimeConfigPatch::default()
                },
                "refresh_rate_hz",
            ),
            (
                RuntimeConfigPatch {
                    resolution_scale: Some(1.5),
                    ..RuntimeConfigPatch::default()
                },
                "resolution_scale",
            ),
            (
                RuntimeConfigPatch {
                    resolution_scale: Some(f64::NAN),
                    ..RuntimeConfigPatch::default()
                },
                "resolution_scale",
            ),
        ] {
            assert_eq!(validate(&patch).len(), 1, "{key}");
            let err = apply_patch(&deployed(), &patch).unwrap_err();
            assert!(err.to_string().contains(key), "{err}");
        }
        // The bounds themselves are inclusive.
        assert!(validate(&patch_bitrate(1)).is_empty());
        assert!(validate(&patch_bitrate(200)).is_empty());
        assert!(validate(&RuntimeConfigPatch {
            resolution_scale: Some(0.25),
            ..RuntimeConfigPatch::default()
        })
        .is_empty());
    }

    // ── read ─────────────────────────────────────────────────────────────────

    #[test]
    fn read_of_the_deployed_file_matches_what_the_runtime_would_use() {
        let dir = scratch("read");
        let path = dir.join("oxrsys-runtime.toml");
        std::fs::write(&path, deployed()).unwrap();
        let view = read(&path);
        assert!(view.exists);
        assert!(view.parse_error.is_none());
        assert_eq!(view.values.protocol, Some(Protocol::Alvr));
        assert_eq!(view.values.bitrate_mbps, Some(80));
        assert_eq!(view.values.resolution_scale, Some(1.0));
        assert_eq!(view.values.refresh_rate_hz, Some(90));
        assert_eq!(view.values.encoder_process, Some(EncoderProcess::Auto));
        assert_eq!(view.values.video_codec, Some(VideoCodec::Auto));
        assert!(view.invalid.is_empty());
        assert!(view.shadowed.is_empty());
        assert!(view.modified_unix_ms.is_some());
        assert_eq!(view.defaults, runtime_defaults());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_of_an_absent_file_is_a_state_not_an_error() {
        let dir = scratch("absent");
        let view = read(&dir.join("nope.toml"));
        assert!(!view.exists);
        assert_eq!(view.values, RuntimeConfigValues::default());
        assert!(view.parse_error.is_none());
        assert!(view.modified_unix_ms.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_value_the_runtime_would_ignore_reads_as_invalid_and_absent() {
        let view = read_text("[streaming]\nbitrate_mbps = 500\nprotocol = \"nope\"\n");
        assert_eq!(
            view.values.bitrate_mbps, None,
            "the runtime keeps its default"
        );
        assert_eq!(view.values.protocol, None);
        assert_eq!(view.invalid.len(), 2);
        assert!(view
            .invalid
            .iter()
            .any(|i| i.key == "bitrate_mbps" && i.raw == "500"));
    }

    #[test]
    fn an_unparseable_file_falls_back_to_the_runtimes_own_reader() {
        let dir = scratch("broken");
        let path = dir.join("oxrsys-runtime.toml");
        std::fs::write(&path, fixture("oxrsys-runtime.broken.toml")).unwrap();
        let view = read(&path);
        assert!(view.exists);
        assert!(view.parse_error.is_some(), "toml_edit must have refused it");
        // The runtime would still read these lines.
        assert_eq!(view.values.protocol, Some(Protocol::Alvr));
        assert_eq!(view.values.bitrate_mbps, Some(80));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_fallback_reader_matches_the_runtimes_line_semantics() {
        // Tables ignored, last wins, quote-aware comment stripping.
        let text = concat!(
            "protocol = \"oxrsys\"   # not this one\n",
            "[whatever]\n",
            "bitrate_mbps = 33\n",
            "[streaming]\n",
            "protocol = \"alvr\"\n",
            "bitrate_mbps = 80\n",
            "# encoder_process = \"inproc\"\n",
        );
        let (values, invalid, shadowed) = read_lines_like_the_runtime(text);
        assert_eq!(values.protocol, Some(Protocol::Alvr));
        assert_eq!(values.bitrate_mbps, Some(80));
        assert_eq!(
            values.encoder_process, None,
            "a commented line is not an assignment"
        );
        assert!(invalid.is_empty());
        assert_eq!(
            shadowed,
            vec!["protocol".to_string(), "bitrate_mbps".to_string()]
        );
    }

    #[test]
    fn the_comment_stripper_is_quote_aware() {
        assert_eq!(strip_comment("a = \"x # y\" # tail"), "a = \"x # y\" ");
        assert_eq!(strip_comment("a = 'x # y'"), "a = 'x # y'");
        assert_eq!(strip_comment("# whole line"), "");
        assert_eq!(strip_comment("a = 1"), "a = 1");
    }

    // ── write ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn write_creates_from_the_template_byte_identically_then_patches() {
        let dir = scratch("create");
        let path = dir.join("OXRSys/oxrsys-runtime.toml");
        let backups = dir.join("backups");
        let ex = real();

        let report = write(&ex, &path, &backups, &patch_bitrate(60))
            .await
            .unwrap();
        assert!(report.created_from_template);
        assert_eq!(report.backup_path, None, "nothing existed to back up");
        assert_eq!(report.changed_keys, vec!["bitrate_mbps".to_string()]);

        let on_disk = std::fs::read_to_string(&path).unwrap();
        // The create wrote the template verbatim; the patch then moved exactly
        // the one line.
        let diff = diff_lines(crate::util::toml_template(), &on_disk);
        assert_eq!(diff.len(), 1, "{diff:?}");
        assert_eq!(diff[0].0, "bitrate_mbps = 42");
        assert_eq!(diff[0].1, "bitrate_mbps = 60");
        assert!(!backups.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn write_creates_the_template_verbatim_for_an_empty_patch() {
        let dir = scratch("create-empty");
        let path = dir.join("oxrsys-runtime.toml");
        let ex = real();
        let report = write(
            &ex,
            &path,
            &dir.join("backups"),
            &RuntimeConfigPatch::default(),
        )
        .await
        .unwrap();
        assert!(report.created_from_template);
        assert!(report.changed_keys.is_empty());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            crate::util::toml_template(),
            "the write-once bytes are the shared template, byte-for-byte"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn write_backs_up_before_overwriting_and_leaves_the_backup_identical() {
        let dir = scratch("backup");
        let path = dir.join("oxrsys-runtime.toml");
        let backups = dir.join("backups");
        std::fs::write(&path, deployed()).unwrap();
        let ex = real();

        let report = write(&ex, &path, &backups, &patch_bitrate(60))
            .await
            .unwrap();
        assert!(!report.created_from_template);
        let backup = report.backup_path.clone().unwrap();
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), deployed());
        assert!(
            Path::new(&backup)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(BACKUP_PREFIX),
            "{backup}"
        );
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("bitrate_mbps = 60"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn an_unchanged_patch_writes_nothing_and_takes_no_backup() {
        let dir = scratch("noop");
        let path = dir.join("oxrsys-runtime.toml");
        let backups = dir.join("backups");
        std::fs::write(&path, deployed()).unwrap();
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();
        let ex = real();

        let report = write(&ex, &path, &backups, &patch_bitrate(80))
            .await
            .unwrap();
        assert!(report.changed_keys.is_empty());
        assert_eq!(report.backup_path, None);
        assert!(!report.created_from_template);
        assert!(!backups.exists(), "no backup directory is even created");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), deployed());
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            before,
            "the file must not be reopened for writing"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Regression: a hand-maintained file that is not LF + newline-terminated
    /// used to be backed up and rewritten whole by an EMPTY patch, because the
    /// no-op test compared `toml_edit`'s re-render against the file. The report
    /// said `changedKeys: []` while the bytes on disk had moved — exactly the
    /// "Sabrage touched my file and told me it didn't" failure this module
    /// exists to prevent.
    #[tokio::test]
    async fn a_no_op_write_leaves_an_unnormalised_file_alone() {
        let dir = scratch("noop-unnormalised");
        let path = dir.join("oxrsys-runtime.toml");
        let backups = dir.join("backups");
        let original = "# my notes\n[streaming]\nprotocol = \"alvr\"\nbitrate_mbps = 80";
        std::fs::write(&path, original).unwrap();
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();
        let ex = real();

        for patch in [
            RuntimeConfigPatch::default(),
            RuntimeConfigPatch {
                protocol: Some(Protocol::Alvr),
                ..RuntimeConfigPatch::default()
            },
        ] {
            let report = write(&ex, &path, &backups, &patch).await.unwrap();
            assert!(report.changed_keys.is_empty(), "{:?}", report.changed_keys);
            assert_eq!(report.backup_path, None);
            assert!(!backups.exists(), "no backup slot may be burned");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
            assert_eq!(
                std::fs::metadata(&path).unwrap().modified().unwrap(),
                before
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_same_second_backup_collision_gets_a_numeric_suffix() {
        let dir = scratch("collision");
        let backups = dir.join("backups");
        std::fs::create_dir_all(&backups).unwrap();
        let secs = 1_756_500_000u64;
        std::fs::write(backups.join(format!("{BACKUP_PREFIX}{secs}")), "a").unwrap();
        assert_eq!(
            next_backup_path(&backups, secs),
            backups.join(format!("{BACKUP_PREFIX}{secs}-2"))
        );
        std::fs::write(backups.join(format!("{BACKUP_PREFIX}{secs}-2")), "b").unwrap();
        assert_eq!(
            next_backup_path(&backups, secs),
            backups.join(format!("{BACKUP_PREFIX}{secs}-3"))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn backups_are_pruned_to_the_newest_ten() {
        let dir = scratch("prune");
        let path = dir.join("oxrsys-runtime.toml");
        let backups = dir.join("backups");
        std::fs::create_dir_all(&backups).unwrap();
        std::fs::write(&path, deployed()).unwrap();
        // 12 older backups; after the write there must be exactly BACKUP_KEEP.
        for i in 0..12u64 {
            std::fs::write(backups.join(format!("{BACKUP_PREFIX}{}", 1_000 + i)), "old").unwrap();
        }
        let ex = real();
        write(&ex, &path, &backups, &patch_bitrate(60))
            .await
            .unwrap();

        let kept = list_backups(&backups);
        assert_eq!(kept.len(), BACKUP_KEEP, "{kept:#?}");
        // The three oldest went; the new one is newest.
        assert!(!backups.join(format!("{BACKUP_PREFIX}1000")).exists());
        assert!(!backups.join(format!("{BACKUP_PREFIX}1001")).exists());
        assert!(!backups.join(format!("{BACKUP_PREFIX}1002")).exists());
        assert!(backups.join(format!("{BACKUP_PREFIX}1011")).exists());
        assert!(
            kept[0].created_unix_secs >= kept[1].created_unix_secs,
            "newest first"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_backups_sorts_newest_first_and_ignores_strangers() {
        let dir = scratch("list");
        std::fs::write(dir.join(format!("{BACKUP_PREFIX}100")), "aa").unwrap();
        std::fs::write(dir.join(format!("{BACKUP_PREFIX}100-2")), "bbb").unwrap();
        std::fs::write(dir.join(format!("{BACKUP_PREFIX}99")), "c").unwrap();
        std::fs::write(dir.join("session.json.100"), "x").unwrap();
        std::fs::write(dir.join(format!("{BACKUP_PREFIX}notanumber")), "x").unwrap();

        let got = list_backups(&dir);
        assert_eq!(got.len(), 3);
        assert!(got[0].path.ends_with("100-2"));
        assert_eq!(got[0].size, 3);
        assert_eq!(got[0].created_unix_secs, 100);
        assert!(got[1].path.ends_with(".100"));
        assert!(got[2].path.ends_with(".99"));
        assert!(list_backups(&dir.join("nope")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_dry_run_plans_the_write_and_touches_nothing() {
        let dir = scratch("dryrun");
        let path = dir.join("oxrsys-runtime.toml");
        let backups = dir.join("backups");
        std::fs::write(&path, deployed()).unwrap();
        let ex = dry();

        let report = write(&ex, &path, &backups, &patch_bitrate(60))
            .await
            .unwrap();
        assert_eq!(report.changed_keys, vec!["bitrate_mbps".to_string()]);
        assert!(report.backup_path.is_some());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            deployed(),
            "a dry run must not write"
        );
        assert!(!backups.exists());

        let plan = ex.planned();
        let kinds: Vec<PlannedKind> = plan.iter().map(|p| p.kind).collect();
        assert!(kinds.contains(&PlannedKind::CreateDir));
        assert_eq!(
            kinds.iter().filter(|k| **k == PlannedKind::Write).count(),
            2,
            "backup + config: {plan:#?}"
        );
        assert!(plan.last().unwrap().dst.as_deref() == Some(path.as_path()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_dry_run_plans_the_create_from_template() {
        let dir = scratch("dryrun-create");
        let path = dir.join("OXRSys/oxrsys-runtime.toml");
        let ex = dry();
        let report = write(&ex, &path, &dir.join("backups"), &patch_bitrate(60))
            .await
            .unwrap();
        assert!(report.created_from_template);
        assert!(!path.exists());
        // The patch was computed against the template, not against nothing.
        assert_eq!(report.changed_keys, vec!["bitrate_mbps".to_string()]);
        let plan = ex.planned();
        assert_eq!(
            plan.iter().filter(|p| p.kind == PlannedKind::Write).count(),
            2,
            "template create + patched write: {plan:#?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn write_refuses_a_file_it_cannot_round_trip() {
        let dir = scratch("write-broken");
        let path = dir.join("oxrsys-runtime.toml");
        std::fs::write(&path, fixture("oxrsys-runtime.broken.toml")).unwrap();
        let ex = real();
        let err = write(&ex, &path, &dir.join("backups"), &patch_bitrate(60))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("refusing to rewrite"), "{err}");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            fixture("oxrsys-runtime.broken.toml")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn write_refuses_an_out_of_range_patch_before_touching_disk() {
        let dir = scratch("write-range");
        let path = dir.join("oxrsys-runtime.toml");
        let ex = real();
        let err = write(&ex, &path, &dir.join("backups"), &patch_bitrate(9_999))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("bitrate_mbps"), "{err}");
        assert!(!path.exists(), "not even the template create happens");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── fix.edit-protocol ────────────────────────────────────────────────────

    /// A context whose `toml_path` and `sabrage_appsup` both live under a
    /// scratch directory — the real ones are never touched by a test (a Catch2
    /// suite once clobbered the user's live config; see the deployed fixture's
    /// own header).
    fn fix_ctx(dir: &Path, dry_run: bool) -> StageCtx {
        let mut paths = Paths::new(dir);
        paths.oxr_appsup = dir.join("OXRSys");
        paths.toml_path = paths.oxr_appsup.join("oxrsys-runtime.toml");
        paths.sabrage_appsup = dir.join("Sabrage");
        StageCtx::new(
            paths,
            crate::stages::StageOptions {
                dry_run,
                ..crate::stages::StageOptions::default()
            },
            null_sink(),
            CancellationToken::new(),
        )
    }

    #[tokio::test]
    async fn edit_protocol_rewrites_only_the_protocol_line_and_backs_up() {
        let dir = scratch("fix-edit");
        let ctx = fix_ctx(&dir, false);
        std::fs::create_dir_all(ctx.paths.toml_path.parent().unwrap()).unwrap();
        let before = deployed().replace("protocol = \"alvr\"", "protocol = \"oxrsys\"");
        std::fs::write(&ctx.paths.toml_path, &before).unwrap();

        let report = edit_protocol(&ctx, &null_sink()).await.unwrap();
        assert_eq!(report.action, FixAction::EditProtocol);
        assert!(report.changed);
        assert!(
            report
                .description
                .starts_with("set protocol = \"alvr\" in "),
            "{report:?}"
        );

        let after = std::fs::read_to_string(&ctx.paths.toml_path).unwrap();
        assert_eq!(after, deployed(), "only the protocol line moves");
        // The backup lives under Sabrage's own support dir, not OXRSys's.
        let backups = list_backups(&ctx.paths.sabrage_appsup.join("backups"));
        assert_eq!(backups.len(), 1);
        assert_eq!(std::fs::read_to_string(&backups[0].path).unwrap(), before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn edit_protocol_is_an_unchanged_noop_when_the_file_already_says_alvr() {
        let dir = scratch("fix-noop");
        let ctx = fix_ctx(&dir, false);
        std::fs::create_dir_all(ctx.paths.toml_path.parent().unwrap()).unwrap();
        std::fs::write(&ctx.paths.toml_path, deployed()).unwrap();

        let report = edit_protocol(&ctx, &null_sink()).await.unwrap();
        assert!(!report.changed);
        assert!(
            report.description.contains("already has protocol"),
            "{report:?}"
        );
        assert!(!ctx.paths.sabrage_appsup.join("backups").exists());
        assert_eq!(
            std::fs::read_to_string(&ctx.paths.toml_path).unwrap(),
            deployed()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn edit_protocol_creates_the_file_from_the_template_when_it_is_absent() {
        let dir = scratch("fix-create");
        let ctx = fix_ctx(&dir, false);
        let report = edit_protocol(&ctx, &null_sink()).await.unwrap();
        assert!(report.changed, "creating the file IS a change");
        assert!(report
            .description
            .contains("created from the shared template"));
        assert_eq!(
            std::fs::read_to_string(&ctx.paths.toml_path).unwrap(),
            crate::util::toml_template(),
            "the template already says alvr, so nothing is patched on top"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn edit_protocol_under_dry_run_plans_and_writes_nothing() {
        let dir = scratch("fix-dry");
        let ctx = fix_ctx(&dir, true);
        std::fs::create_dir_all(ctx.paths.toml_path.parent().unwrap()).unwrap();
        let before = deployed().replace("protocol = \"alvr\"", "protocol = \"oxrsys\"");
        std::fs::write(&ctx.paths.toml_path, &before).unwrap();

        let report = edit_protocol(&ctx, &null_sink()).await.unwrap();
        assert!(
            report.changed,
            "a dry run reports what a real apply would achieve"
        );
        assert!(report.description.starts_with("would set "), "{report:?}");
        assert_eq!(
            std::fs::read_to_string(&ctx.paths.toml_path).unwrap(),
            before
        );
        assert!(!ctx.paths.sabrage_appsup.join("backups").exists());
        assert!(!ctx.executor.planned().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── the enums ────────────────────────────────────────────────────────────

    #[test]
    fn the_enums_serialize_as_the_toml_spellings() {
        assert_eq!(serde_json::to_string(&Protocol::Alvr).unwrap(), "\"alvr\"");
        assert_eq!(
            serde_json::to_string(&VideoCodec::H265).unwrap(),
            "\"h265\""
        );
        assert_eq!(
            serde_json::to_string(&EncoderProcess::Inproc).unwrap(),
            "\"inproc\""
        );
        for p in Protocol::EVERY {
            assert_eq!(Protocol::parse(p.as_str()), Some(*p));
        }
        for c in VideoCodec::EVERY {
            assert_eq!(VideoCodec::parse(c.as_str()), Some(*c));
        }
        for e in EncoderProcess::EVERY {
            assert_eq!(EncoderProcess::parse(e.as_str()), Some(*e));
        }
        assert_eq!(Protocol::parse("ALVR"), None);
    }

    #[test]
    fn the_view_serializes_camel_case_for_the_ui() {
        let json = serde_json::to_value(read_text(&deployed())).unwrap();
        assert!(json.get("parseError").is_some());
        assert!(json["values"].get("bitrateMbps").is_some());
        assert!(json["defaults"].get("refreshRateHz").is_some());
    }

    #[test]
    fn editable_keys_and_the_values_struct_agree() {
        // Every editable key must be reachable through patch_value, or an
        // apply_patch would silently ignore it.
        let all = RuntimeConfigValues {
            protocol: Some(Protocol::Alvr),
            bitrate_mbps: Some(80),
            encoder_process: Some(EncoderProcess::Auto),
            video_codec: Some(VideoCodec::Auto),
            resolution_scale: Some(1.0),
            refresh_rate_hz: Some(90),
        };
        for key in EDITABLE_KEYS {
            assert!(patch_value(key, &all).is_some(), "{key}");
            assert!(
                patch_value(key, &RuntimeConfigValues::default()).is_none(),
                "{key}"
            );
        }
    }
}
