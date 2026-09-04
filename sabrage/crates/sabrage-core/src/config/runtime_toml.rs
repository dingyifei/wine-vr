//! `oxrsys-runtime.toml` — typed read, format-preserving patch, backed-up write.
//!
//! Sabrage edits the six streaming keys of a file `scripts/demo/setup.sh` writes
//! once and never touches again; design-core §4.1 makes that a narrow override:
//! create an absent file from [`crate::util::toml_template`] byte-for-byte, then
//! edit values in place with a rolling backup, never regenerate, never migrate.
//!
//! The consumer is not a TOML library. `ext/oxrsys/runtime/src/Config.cpp` reads
//! lines: `"`-aware `#` comments, `[table]` headers ignored so a key counts
//! wherever it sits, split on the first `=`, one pair of double quotes off a
//! string value, last *accepted* assignment wins. Values therefore always come
//! from [`read_lines_like_the_runtime`] and never from `toml_edit`, which answers
//! a different question — see PARITY.md § Declared by the 2026-08-30
//! adversarial review (round 1 fixes), "Config readers: doctor emulates `awk`"
//! and tests::{the_comment_stripper_matches_config_cpp,
//! effective_accepted_agrees_with_the_line_reader_on_the_shadowed_fixtures}.
//!
//! `toml_edit` decides whether the file can be rewritten safely and performs the
//! rewrite. Every other byte is preserved — the file is hand-maintained — and a
//! same-line `#` comment on a rewritten line is relocated above the key because
//! runtime builds before the 2026-08 parser fix mis-read trailing comments; see
//! tests::{a_key_inside_a_multiline_string_reads_live_and_refuses_the_write,
//! a_same_line_comment_moves_to_its_own_line_above_the_key}.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use toml_edit::{DocumentMut, Formatted, Item, Table, Value};

use crate::error::{Result, SabrageError};
use crate::events::{StageEvent, StepId};
use crate::executor::Executor;
use crate::fixes::{FixAction, FixReport};
use crate::session::SessionBlock;
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

    view.fill_from(&text);
    view
}

impl RuntimeConfigView {
    /// Everything a view derives from the file's bytes: what the runtime would
    /// use (always the line reader) and whether Sabrage may rewrite the file.
    fn fill_from(&mut self, text: &str) {
        let (values, invalid, shadowed) = read_lines_like_the_runtime(text);
        self.values = values;
        self.invalid = invalid;
        self.shadowed = shadowed;
        self.parse_error = round_trip_error(text);
    }
}

/// Why this file must not be rewritten, if it must not be — the value
/// [`RuntimeConfigView::parse_error`] carries and [`write`] refuses on.
///
/// `Some` when `toml_edit` cannot parse the text at all, or when the physical
/// lines and the parsed document disagree about an editable key
/// (`line_document_mismatch`); either way an edit would not land where the
/// runtime looks.
fn round_trip_error(text: &str) -> Option<String> {
    let body = strip_bom(text);
    let doc = match body.parse::<DocumentMut>() {
        Ok(doc) => doc,
        Err(e) => return Some(e.to_string()),
    };
    line_document_mismatch(&doc, text)
}

/// The second half of `round_trip_error`, over an already-parsed document, so
/// [`apply_patch`] can consult it without parsing the file twice.
///
/// `Some` when an editable key's physical assignment count differs from the
/// parsed one, in either direction: a key inside a `"""…"""` block is live to
/// the runtime and invisible to `toml_edit`, and a key behind a byte-order mark
/// is the reverse. Both are refused rather than repaired — the BOM is a byte
/// Sabrage does not own. See
/// tests::{a_key_inside_a_multiline_string_reads_live_and_refuses_the_write,
/// a_bom_on_a_root_key_is_not_round_trippable}.
fn line_document_mismatch(doc: &DocumentMut, text: &str) -> Option<String> {
    let seen: Vec<(&str, &str)> = raw_assignments(text).collect();
    for key in EDITABLE_KEYS {
        let physical = seen.iter().filter(|(k, _)| *k == key).count();
        let parsed = occurrences_of(doc.as_table(), key).len();
        if physical > parsed {
            return Some(format!(
                "'{key}' is assigned on a physical line that TOML reads as string content (a \
                 multiline string): the runtime honours that line and an edit here could not \
                 reach it — edit this file by hand"
            ));
        }
        if parsed > physical {
            return Some(format!(
                "'{key}' is assigned on a line the runtime's reader does not see (a byte-order \
                 mark or another stray byte in front of the key): the runtime ignores that line \
                 and an edit here could not change what it uses — edit this file by hand"
            ));
        }
    }
    None
}

fn strip_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

/// Millisecond mtime, for "the file changed under us" checks in the UI.
fn modified_unix_ms(path: &Path) -> Option<u64> {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
}

/// One accepted assignment of one editable key.
///
/// Parsing into this instead of validating a `&str` in one place and
/// re-deriving it in another is what keeps [`read_lines_like_the_runtime`],
/// [`validate`] and [`apply_patch`] from drifting: a value that produced a
/// `Setting` is by construction one the runtime accepts, and each key's
/// accepted set is spelled out exactly once, in [`accepted`].
#[derive(Debug, Clone, Copy, PartialEq)]
enum Setting {
    Protocol(Protocol),
    VideoCodec(VideoCodec),
    EncoderProcess(EncoderProcess),
    Bitrate(u32),
    Refresh(u32),
    Scale(f64),
}

/// What the runtime accepts for one key, phrased for the UI. One string per
/// key, in one place — every [`InvalidValue::reason`] comes from here.
fn accepted(key: &str) -> String {
    match key {
        "protocol" => "expected \"alvr\" or \"oxrsys\"".to_string(),
        "video_codec" => "expected \"auto\", \"h265\" or \"h264\"".to_string(),
        "encoder_process" => "expected \"auto\", \"native\" or \"inproc\"".to_string(),
        "bitrate_mbps" => format!(
            "expected an integer {}..={}",
            BITRATE_RANGE.0, BITRATE_RANGE.1
        ),
        "refresh_rate_hz" => format!(
            "expected one of {}",
            REFRESH_RATES
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        "resolution_scale" => format!(
            "expected a number {}..={}",
            RESOLUTION_SCALE_RANGE.0, RESOLUTION_SCALE_RANGE.1
        ),
        _ => String::new(),
    }
}

impl Setting {
    /// Which key this is an assignment of.
    fn key(self) -> &'static str {
        match self {
            Setting::Protocol(_) => "protocol",
            Setting::VideoCodec(_) => "video_codec",
            Setting::EncoderProcess(_) => "encoder_process",
            Setting::Bitrate(_) => "bitrate_mbps",
            Setting::Refresh(_) => "refresh_rate_hz",
            Setting::Scale(_) => "resolution_scale",
        }
    }

    /// Read one raw line value the way `ParseConfigToml` does: one pair of
    /// **double** quotes comes off, `std::stoi`/`std::stof` become Rust's
    /// numeric parses, and the accepted set is checked.
    ///
    /// `Ok(None)` means "not one of the six" — the runtime knows plenty of
    /// other keys and Sabrage passes them through untouched.
    fn read_raw(key: &str, raw: &str) -> std::result::Result<Option<Setting>, InvalidValue> {
        let s = unquote(raw);
        let parsed = match key {
            "protocol" => Protocol::parse(s).map(Setting::Protocol),
            "video_codec" => VideoCodec::parse(s).map(Setting::VideoCodec),
            "encoder_process" => EncoderProcess::parse(s).map(Setting::EncoderProcess),
            "bitrate_mbps" => stoi(s)
                .filter(|n| in_bitrate_range(*n))
                .map(|n| Setting::Bitrate(n as u32)),
            "refresh_rate_hz" => stoi(s)
                .filter(|n| REFRESH_RATES.iter().any(|r| i64::from(*r) == *n))
                .map(|n| Setting::Refresh(n as u32)),
            "resolution_scale" => stof(s).filter(|f| in_scale_range(*f)).map(Setting::Scale),
            _ => return Ok(None),
        };
        parsed
            .map(Some)
            .ok_or_else(|| InvalidValue::new(key, raw, accepted(key)))
    }

    /// This key's value in a patch, if the patch sets it. Unvalidated: the
    /// enums cannot be wrong, the three numbers can, and [`validate`] is where
    /// that is decided.
    fn from_patch(key: &str, patch: &RuntimeConfigPatch) -> Option<Setting> {
        match key {
            "protocol" => patch.protocol.map(Setting::Protocol),
            "video_codec" => patch.video_codec.map(Setting::VideoCodec),
            "encoder_process" => patch.encoder_process.map(Setting::EncoderProcess),
            "bitrate_mbps" => patch.bitrate_mbps.map(Setting::Bitrate),
            "refresh_rate_hz" => patch.refresh_rate_hz.map(Setting::Refresh),
            "resolution_scale" => patch.resolution_scale.map(Setting::Scale),
            _ => None,
        }
    }

    /// The complaint to show when a patch carries a value the runtime would
    /// ignore, or `None` when it would not.
    fn out_of_range(self) -> Option<InvalidValue> {
        let bad = |raw: String| Some(InvalidValue::new(self.key(), raw, accepted(self.key())));
        match self {
            Setting::Bitrate(n) if !in_bitrate_range(i64::from(n)) => bad(n.to_string()),
            Setting::Refresh(hz) if !REFRESH_RATES.contains(&hz) => bad(hz.to_string()),
            Setting::Scale(f) if !in_scale_range(f) => bad(format!("{f}")),
            _ => None,
        }
    }

    /// An accepted value as the plain string the runtime holds — no quotes,
    /// the enum's own spelling for the three string keys. What
    /// [`effective_accepted`] hands the launch preflight, which compares
    /// against `"alvr"`/`"inproc"` the way `run.sh` compares its `awk` capture.
    fn runtime_string(self) -> String {
        match self {
            Setting::Protocol(p) => p.as_str().to_string(),
            Setting::VideoCodec(c) => c.as_str().to_string(),
            Setting::EncoderProcess(e) => e.as_str().to_string(),
            Setting::Bitrate(n) | Setting::Refresh(n) => n.to_string(),
            Setting::Scale(f) => format!("{f}"),
        }
    }

    /// Store an accepted assignment. Later calls win, which is the runtime's
    /// last-valid-wins rule expressed as a fold.
    fn apply(self, values: &mut RuntimeConfigValues) {
        match self {
            Setting::Protocol(p) => values.protocol = Some(p),
            Setting::VideoCodec(c) => values.video_codec = Some(c),
            Setting::EncoderProcess(e) => values.encoder_process = Some(e),
            Setting::Bitrate(n) => values.bitrate_mbps = Some(n),
            Setting::Refresh(hz) => values.refresh_rate_hz = Some(hz),
            Setting::Scale(f) => values.resolution_scale = Some(f),
        }
    }

    /// The canonical spelling Sabrage writes: integers bare (`80`), floats
    /// always with a fractional part (`1.0`, never `1` — `resolution_scale` is
    /// a float key and a bare `1` reads as an integer to a stricter parser),
    /// strings as basic (double-quoted) strings, which is the only string form
    /// the runtime unquotes.
    fn to_value(self) -> Value {
        match self {
            Setting::Protocol(p) => string_value(p.as_str()),
            Setting::VideoCodec(c) => string_value(c.as_str()),
            Setting::EncoderProcess(e) => string_value(e.as_str()),
            Setting::Bitrate(n) => Value::Integer(Formatted::new(i64::from(n))),
            Setting::Refresh(hz) => Value::Integer(Formatted::new(i64::from(hz))),
            Setting::Scale(f) => Value::Float(Formatted::new(f)),
        }
    }
}

/// `std::stoi`, whose prefix rule Rust's `parse` does not share: an optional
/// sign and the run of decimal digits after it, with everything past that
/// ignored, and a throw (→ `None`) when there are no leading digits at all.
///
/// So `80 # old` never reaches here (the comment is already stripped), `80abc`
/// is 80, TOML's `1_0` is **1**, and `0x50` is `0` — base 10 stops at the `x`,
/// which is why a hex-spelled bitrate reads as out of range rather than as 80.
/// Being stricter than this looks safer and is not: it makes Sabrage report a
/// value the runtime is not using.
fn stoi(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    let mut i = usize::from(matches!(b.first(), Some(b'+' | b'-')));
    let start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i == start {
        return None;
    }
    s[..i].parse::<i64>().ok()
}

/// `std::stof`, same prefix rule: optional sign, digits, optional fraction,
/// optional exponent — `0.9 (was 0.75)` is 0.9, `abc` throws.
///
/// The exotic spellings `strtof` also takes (`0x1p3`, `inf`, `nan`) are left
/// out: `in_scale_range` rejects every one of them anyway.
fn stof(s: &str) -> Option<f64> {
    let b = s.as_bytes();
    let mut i = usize::from(matches!(b.first(), Some(b'+' | b'-')));
    let int_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let mut digits = i - int_start;
    if b.get(i) == Some(&b'.') {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            digits += 1;
            i += 1;
        }
    }
    if digits == 0 {
        return None;
    }
    if b.get(i).is_some_and(|c| c.eq_ignore_ascii_case(&b'e')) {
        let mut j = i + 1;
        if matches!(b.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        let exp_start = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > exp_start {
            i = j;
        }
    }
    s[..i].parse::<f64>().ok()
}

fn in_bitrate_range(n: i64) -> bool {
    n >= i64::from(BITRATE_RANGE.0) && n <= i64::from(BITRATE_RANGE.1)
}

/// `Config.cpp`'s bounds check at `Config.cpp`'s precision: `std::stof` yields a
/// C++ `float`, so `1.00000001` *is* `1.0f` to the runtime and is accepted.
///
/// Comparing the `f64` Rust parsed would reject it, and the Settings screen would
/// then show the default for a file the runtime reads as `1.0`. The bounds are
/// exactly representable, so the two comparisons agree everywhere except within
/// one `f32` ulp of an endpoint. See
/// tests::the_scale_bounds_are_checked_at_the_runtimes_float_precision.
fn in_scale_range(f: f64) -> bool {
    let narrowed = f as f32;
    narrowed.is_finite()
        && f64::from(narrowed) >= RESOLUTION_SCALE_RANGE.0
        && f64::from(narrowed) <= RESOLUTION_SCALE_RANGE.1
}

/// Every `key = value` the runtime's line reader sees, in physical order.
///
/// `ext/oxrsys/runtime/src/Config.cpp`'s `ParseConfigToml` loop, ported. Tables
/// are not tracked because the runtime does not track them — a key counts
/// wherever it sits (tests::the_line_reader_agrees_with_parse_config_toml).
fn raw_assignments(text: &str) -> impl Iterator<Item = (&str, &str)> {
    text.lines().filter_map(|line| {
        let line = strip_comment(line).trim();
        if line.is_empty() || line.starts_with('[') {
            return None;
        }
        let (k, v) = line.split_once('=')?;
        Some((k.trim(), v.trim()))
    })
}

/// What the runtime would use, read the way the runtime reads it — the
/// **primary** reader (see the module header), used whether or not `toml_edit`
/// can also parse the file.
///
/// Returns the last *accepted* assignment of each editable key, so a junk
/// assignment after a valid one leaves the valid value in force, plus every
/// rejected occurrence (`invalid`) and every key assigned more than once
/// (`shadowed`). See
/// tests::a_later_invalid_assignment_does_not_erase_an_earlier_valid_one.
pub fn read_lines_like_the_runtime(
    text: &str,
) -> (RuntimeConfigValues, Vec<InvalidValue>, Vec<String>) {
    let seen: Vec<(&str, &str)> = raw_assignments(text).collect();

    let mut values = RuntimeConfigValues::default();
    let mut invalid = Vec::new();
    let mut shadowed = Vec::new();
    for key in EDITABLE_KEYS {
        let hits = seen.iter().filter(|(k, _)| *k == key).map(|(_, v)| *v);
        let mut count = 0usize;
        for raw in hits {
            count += 1;
            match Setting::read_raw(key, raw) {
                Ok(Some(setting)) => setting.apply(&mut values),
                Ok(None) => {}
                Err(bad) => {
                    if !invalid.contains(&bad) {
                        invalid.push(bad);
                    }
                }
            }
        }
        if count > 1 {
            shadowed.push(key.to_string());
        }
    }
    (values, invalid, shadowed)
}

/// The value the runtime would end up with for **any** key, as a plain string,
/// with no accepted-set filtering: the last assignment wins, whatever table it
/// sits in, with one pair of double quotes removed. `None` means the key is never
/// assigned, so the caller's own default applies (`${…:-auto}` in the shell).
///
/// Deliberately free of `toml_edit` — it answers what the runtime does, not what
/// TOML says — so the run preflight reads the file the way `Config.cpp` does
/// instead of carrying its own quote-blind, first-match split. The doctor checks
/// do *not* share it: PARITY.md § Declared by the 2026-08-30 adversarial
/// review (round 1 fixes), "Config readers: doctor emulates `awk`". See
/// tests::effective_string_is_last_wins_table_blind_and_double_quote_only.
pub fn effective_string(text: &str, key: &str) -> Option<String> {
    raw_assignments(text)
        .filter(|(k, _)| *k == key)
        .map(|(_, v)| unquote(v).to_string())
        .last()
}

/// The value the runtime would end up with for one of the **six modeled** keys:
/// the last assignment it would *accept*, in the key's canonical spelling.
///
/// [`effective_string`] answers "what does the last line say"; this answers
/// "what is the runtime holding", and the two differ where `Config.cpp` throws a
/// value away — `protocol = "alvr"` followed by `protocol = "banana"` leaves
/// ALVR in force, and reading that file with the raw helper would block a launch
/// the runtime would have run. `None` means no occurrence was accepted (including
/// an absent key and a key outside [`EDITABLE_KEYS`]), so the caller's own default
/// applies. See tests::{effective_accepted_keeps_the_last_value_the_runtime_would_accept,
/// effective_accepted_agrees_with_the_line_reader_on_the_shadowed_fixtures}.
pub fn effective_accepted(text: &str, key: &str) -> Option<String> {
    raw_assignments(text)
        .filter(|(k, _)| *k == key)
        .filter_map(|(_, raw)| Setting::read_raw(key, raw).ok().flatten())
        .map(Setting::runtime_string)
        .last()
}

/// The runtime's comment stripper (`StripTomlComment`): `#` ends the line
/// unless a `"` opened a string first.
///
/// Only `"` toggles, and there is no escape handling — both deliberately, to
/// match Config.cpp byte for byte. A `'` is an ordinary character there, so
/// `x = 'a # b'` really does lose everything from the `#`, and a `\"` inside a
/// basic string really does close it.
fn strip_comment(line: &str) -> &str {
    let mut in_string = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_string = !in_string,
            '#' if !in_string => return &line[..i],
            _ => {}
        }
    }
    line
}

/// Strip one layer of **double** quotes, the way the runtime's `ParseString`
/// does before comparing a string value.
///
/// Double quotes only, and no escape processing: `'alvr'` keeps its quotes and
/// therefore matches nothing in the runtime's whitelist, which is why Sabrage
/// reports that spelling invalid and rewrites it as `"alvr"`.
fn unquote(raw: &str) -> &str {
    let b = raw.as_bytes();
    if b.len() >= 2 && b[0] == b'"' && b[b.len() - 1] == b'"' {
        &raw[1..raw.len() - 1]
    } else {
        raw
    }
}

/// The runtime's own bounds, applied to a patch before anything touches disk.
///
/// The enums are unrepresentable-when-wrong; only the three numeric keys can
/// carry a value the runtime would ignore, and writing one is worse than
/// refusing — the file would look edited and the runtime would keep its
/// default.
pub fn validate(patch: &RuntimeConfigPatch) -> Vec<InvalidValue> {
    EDITABLE_KEYS
        .iter()
        .filter_map(|key| Setting::from_patch(key, patch))
        .filter_map(Setting::out_of_range)
        .collect()
}

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
/// **Dotted tables are skipped on purpose**: `streaming.protocol = "alvr"` is one
/// physical line whose key text is `streaming.protocol`, which the runtime's line
/// reader does not see as `protocol` (tests::a_dotted_key_is_not_an_occurrence).
fn occurrences_of(root: &Table, key: &str) -> Vec<Occurrence> {
    let mut out = Vec::new();
    walk(root, -1, &mut Vec::new(), key, &mut out);
    out.sort_by_key(|o| o.order);
    out
}

/// Whether the document's spelling of `key` is the bare one the runtime's reader
/// recognises. A key Sabrage synthesised has no repr yet; it renders bare.
///
/// The runtime splits the line on its first `=` and trims, so the key text of
/// `"protocol" = "oxrsys"` is literally `"protocol"` and matches nothing, while
/// `toml_edit` decodes it to `protocol` — without this check Sabrage would edit a
/// line the runtime ignores. See tests::{a_quoted_key_is_not_an_occurrence,
/// a_key_that_exists_only_as_a_quoted_key_is_refused}.
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

    let shape = ByteShape::of(text);
    let mut doc: DocumentMut = strip_bom(text).parse().map_err(|e| {
        SabrageError::InvalidInput(format!(
            "oxrsys-runtime.toml is not valid TOML, refusing to rewrite it: {e}"
        ))
    })?;

    // Enforced here and not only in [`RuntimeConfigView::parse_error`]: `write`,
    // `edit_protocol` and the golden tests all come through this function, and
    // reporting success for an edit the runtime never reads is worse than refusing.
    if let Some(why) = line_document_mismatch(&doc, text) {
        return Err(SabrageError::InvalidInput(format!(
            "refusing to rewrite oxrsys-runtime.toml: {why}"
        )));
    }

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

    // "Nothing changed" means no key changed, never "the re-render happens to match":
    // `doc.to_string()` normalises CRLF, drops a BOM and adds a final newline, which
    // `ByteShape` undoes for a real edit (tests::an_empty_patch_is_the_identity_on_every_input_shape).
    let text = if changed_keys.is_empty() {
        text.to_string()
    } else {
        shape.restore(doc.to_string())
    };

    Ok(Patched {
        text,
        changed_keys,
        shadowed,
    })
}

/// The three byte-level properties `toml_edit`'s renderer does not preserve: a
/// CRLF file, a leading BOM, and the absence of a final newline. Captured before
/// parsing and restored after rendering, because Sabrage owns six values and not
/// the file's shape.
///
/// Mixed line endings are the one shape not preserved — there is no "the file's"
/// ending to restore — so such a file is rendered LF.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ByteShape {
    bom: bool,
    crlf: bool,
    final_newline: bool,
}

impl ByteShape {
    fn of(text: &str) -> ByteShape {
        let body = strip_bom(text);
        let breaks = body.matches('\n').count();
        ByteShape {
            bom: body.len() != text.len(),
            crlf: breaks > 0 && body.matches("\r\n").count() == breaks,
            // An empty document has no line ending to preserve; whatever the
            // renderer emits for it is the whole file.
            final_newline: body.is_empty() || body.ends_with('\n'),
        }
    }

    fn restore(self, rendered: String) -> String {
        let mut out = if self.crlf {
            rendered.replace("\r\n", "\n").replace('\n', "\r\n")
        } else {
            rendered
        };
        // Only ever *remove* a line ending the input did not have; if the
        // renderer did not add one, there is nothing to undo.
        if !self.final_newline {
            if out.ends_with("\r\n") {
                out.truncate(out.len() - 2);
            } else if out.ends_with('\n') {
                out.truncate(out.len() - 1);
            }
        }
        if self.bom {
            out.insert(0, '\u{feff}');
        }
        out
    }
}

/// The patch's value for one key as a `toml_edit` value in Sabrage's canonical
/// spelling — see [`Setting::to_value`].
fn patch_value(key: &str, patch: &RuntimeConfigPatch) -> Option<Value> {
    Setting::from_patch(key, patch).map(Setting::to_value)
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
/// A key already carrying the wanted value is left **completely** untouched,
/// including a same-line comment: design-core §4.1 rule 2 relocates a comment on a
/// line this function rewrites, and reformatting a line the patch did not need to
/// touch would break "an unchanged patch writes nothing"
/// (tests::setting_every_key_to_its_current_value_changes_nothing).
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

    // design-core §4.1 rule 2: a same-line comment on a line Sabrage rewrites moves above
    // the key, at the key's own indentation — runtime builds before the 2026-08 parser fix
    // mis-read trailing comments (tests::a_same_line_comment_moves_to_its_own_line_above_the_key).
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
/// Textual equality of the value's own bytes, and nothing looser: it is the
/// runtime, not `toml_edit`, that has to agree. `0x50` and `80` are one integer to
/// `toml_edit` and two different values to the runtime, and a `'alvr'` literal
/// string is valid TOML the runtime does not unquote. Every spelling the runtime
/// would misread therefore counts as a change, which is what makes saving one fix
/// it (tests::a_literal_quoted_string_is_invalid_and_gets_rewritten).
fn same_to_the_runtime(old: &Value, new: &Value) -> bool {
    value_repr(old) == value_repr(new)
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

/// Patch the file on disk, creating it from the shared template first when it is
/// absent and backing up the previous contents when it is not.
///
/// The write-once override of design-core §4.1: an absent file is created with
/// [`crate::util::toml_template`] byte-for-byte, never a rendering of the patch,
/// so both front-ends still agree on first-write bytes — PARITY.md § Invariants
/// that must NOT change (byte/behavior parity), "Write-once `oxrsys-runtime.toml`
/// creation". Every mutation goes through `executor`, so a
/// [`crate::DryRunExecutor`] plans the create, the backup, the prune and the write
/// without touching disk.
///
/// # Errors
///
/// Refuses a patch the runtime would ignore, a file that cannot be round-tripped,
/// a live session ([`blocking_session`]: the runtime re-reads this file every
/// 250 ms and rebuilds the encoder, so a save mid-stream is a live
/// reconfiguration) and a concurrent edit (`still_safe_to_replace`: the bytes on
/// disk must still be the ones the patch was computed against). See
/// tests::{write_creates_from_the_template_byte_identically_then_patches,
/// write_refuses_while_a_session_is_live_and_touches_nothing,
/// the_replacement_refuses_when_the_file_changed_underneath}.
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

    let session_state = session_state_beside(backups_dir);
    if let Some(block) = blocking_session(&session_state, &runtime_status_beside(toml_path)) {
        return Err(live_session_refusal(toml_path, &block));
    }

    // A10-1: serializes read-patch-backup-write across processes at the documented lock path
    // (tests::write_takes_the_cross_process_lock_at_the_documented_path); best-effort, since
    // `still_safe_to_replace`'s compare-and-swap is the net. A dry run has nothing to lock.
    let _lock = if executor.is_dry_run() {
        None
    } else {
        lock_toml(toml_path).await
    };

    // A10-1: the probe and the create are two syscalls apart, so this branch uses
    // [`Executor::create_new`] (`O_EXCL`) and never `write_atomic`'s unconditional rename — a
    // file created in that window survives (tests::write_never_clobbers_a_file_created_in_the_toctou_window).
    let mut exists = toml_path.is_file();
    if !exists {
        if let Some(parent) = toml_path.parent() {
            executor.create_dir_all(parent).await?;
        }
        exists = toml_path.is_file();
    }
    let base = if exists {
        std::fs::read_to_string(toml_path).map_err(|e| SabrageError::io(toml_path, e))?
    } else {
        let template = crate::util::toml_template();
        if executor.create_new(toml_path, template.as_bytes()).await? {
            template.to_string()
        } else {
            // Lost the create race: read what the other writer put there rather
            // than overwriting it, and continue on the "file already existed"
            // path, whose compare-and-swap still guards against a third writer.
            exists = true;
            std::fs::read_to_string(toml_path).map_err(|e| SabrageError::io(toml_path, e))?
        }
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
        // Nothing to write, so nothing to back up: an existing file is not even
        // opened, and the test is "no key changed", never `patched.text == base`
        // (tests::a_no_op_write_leaves_the_file_and_backups_untouched).
        return Ok(report);
    }

    still_safe_to_replace(executor, toml_path, &session_state, &base)?;

    // The prune list is computed BEFORE the new backup is written: a real run
    // would otherwise see it in the listing and a dry run would not, and the
    // two modes must plan the same set of removals. It is *executed* after the
    // commit — see below.
    let mut stale: Vec<BackupInfo> = Vec::new();
    let mut reserved: Option<PathBuf> = None;
    if exists {
        stale = list_backups(backups_dir)
            .into_iter()
            .skip(BACKUP_KEEP.saturating_sub(1))
            .collect();
        executor.create_dir_all(backups_dir).await?;
        let backup =
            reserve_backup_path(executor, backups_dir, unix_secs(), base.as_bytes()).await?;
        report.backup_path = Some(backup.display().to_string());
        reserved = Some(backup);
    }

    // A10-2: everything from here to the rename is undone on failure — the reservation is
    // unlinked, nothing older is dropped before the commit — and the compare-and-swap runs again
    // because the backup write is the widest window (tests::a_failed_write_prunes_nothing_and_leaves_no_reservation).
    let committed = match still_safe_to_replace(executor, toml_path, &session_state, &base) {
        Ok(()) => {
            executor
                .write_atomic(toml_path, patched.text.as_bytes())
                .await
        }
        Err(e) => Err(e),
    };
    if let Err(e) = committed {
        if let Some(backup) = &reserved {
            // Best-effort: a backup we cannot remove is a stray copy of bytes
            // that are still on disk, which is harmless next to reporting the
            // original failure.
            let _ = executor.remove_file(backup).await;
        }
        return Err(e);
    }

    // A10-2: best-effort and deliberately not `?`. The commit already happened, so an `Err` here
    // would report a failed save over a file that holds the new bytes; a stray backup is
    // recoverable, a lie about the write is not (tests::an_unprunable_stale_backup_still_reports_a_committed_save).
    for old in stale {
        let _ = executor.remove_file(Path::new(&old.path)).await;
    }
    Ok(report)
}

/// The session that must be stopped before this file may be rewritten, or `None`
/// when nothing is streaming.
///
/// Delegates to [`crate::session::session_block_at`] so that every "not while the
/// game is running" door asks the same question — PARITY.md § Declared by the
/// 2026-08-30 adversarial review (round 1 fixes), "External sessions".
///
/// The door exists because the runtime does not read `oxrsys-runtime.toml` once at
/// game start: `Config::GetValues()` refreshes it whenever the mtime moved, at
/// most every 250 ms, and `AlvrStreamingBackend::EnsureEncoder` retires the
/// encoder when `encoder_process`/`video_codec` drift — so a save mid-stream
/// rebuilds the encoder, and selecting `native` with no staged helper drops frames
/// for the rest of the session.
pub fn blocking_session(
    session_state_path: &Path,
    runtime_status_path: &Path,
) -> Option<SessionBlock> {
    crate::session::session_block_at(session_state_path, runtime_status_path)
}

/// Where `runtime_status.json` sits, given the config file.
///
/// Both live in `<oxr_appsup>` ([`crate::paths::Paths`]), so deriving it keeps
/// the guard hermetic for the same reason [`session_state_beside`] does: a test
/// that points `toml_path` at a temp dir gets a temp status path with it.
fn runtime_status_beside(toml_path: &Path) -> PathBuf {
    toml_path.with_file_name("runtime_status.json")
}

fn live_session_refusal(toml_path: &Path, block: &SessionBlock) -> SabrageError {
    let reason = &block.reason;
    let bottle = block.bottle.as_deref().unwrap_or("<name>");
    SabrageError::InvalidInput(format!(
        "refusing to edit {} while a session is live — {reason}; the runtime re-reads this \
         file every 250 ms and rebuilds the encoder when encoder_process or video_codec \
         changes, so saving mid-stream drops frames; stop the session first: ./demo.sh stop \
         --bottle {bottle}",
        toml_path.display()
    ))
}

/// Where `session-state.json` sits, given the backups directory.
///
/// `backups_dir` is always `<sabrage_appsup>/backups` ([`crate::paths::Paths`]), so
/// its parent is the directory that record lives in. Deriving it keeps the guard
/// hermetic under test: a temp `backups_dir` yields a temp session path, never the
/// developer's real running session.
fn session_state_beside(backups_dir: &Path) -> PathBuf {
    backups_dir
        .parent()
        .unwrap_or(backups_dir)
        .join("session-state.json")
}

/// How long [`lock_toml`] waits for another writer's read-modify-write before
/// proceeding without the lock — the same shape and budget as
/// [`crate::session::state`]'s record lock.
const TOML_LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// Poll interval inside [`TOML_LOCK_WAIT`].
const TOML_LOCK_POLL: std::time::Duration = std::time::Duration::from_millis(50);

/// Takes the advisory lock at [`Paths::toml_lock_path`], derived from
/// `toml_path`'s parent because [`Paths::toml_path`] and [`Paths::toml_lock_path`]
/// always share one directory. Held by the returned `File`; dropping it releases
/// the `flock`.
///
/// A separate dotfile, not a lock on the config itself, so it survives
/// [`Executor::write_atomic`]'s rename. `None` on any failure, including a holder
/// that will not let go: this narrows the window `still_safe_to_replace` guards,
/// it does not replace it.
///
/// [`Paths`]: crate::paths::Paths
async fn lock_toml(toml_path: &Path) -> Option<std::fs::File> {
    let path = toml_path.with_file_name(".oxrsys-runtime.toml.lock");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok()?;
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .ok()?;
    let deadline = tokio::time::Instant::now() + TOML_LOCK_WAIT;
    loop {
        match file.try_lock() {
            Ok(()) => return Some(file),
            Err(std::fs::TryLockError::WouldBlock) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(TOML_LOCK_POLL).await;
            }
            _ => return None,
        }
    }
}

/// Compare-and-swap: refuse unless the file is still the bytes `base` was read
/// from, and still nothing is streaming.
///
/// A dry run is exempt from the byte check: it planned the create instead of
/// performing it, so on the absent path there is deliberately nothing on disk
/// to compare against.
fn still_safe_to_replace(
    executor: &dyn Executor,
    toml_path: &Path,
    session_state_path: &Path,
    base: &str,
) -> Result<()> {
    if let Some(block) = blocking_session(session_state_path, &runtime_status_beside(toml_path)) {
        return Err(live_session_refusal(toml_path, &block));
    }
    if executor.is_dry_run() {
        return Ok(());
    }
    let now = std::fs::read_to_string(toml_path).map_err(|e| SabrageError::io(toml_path, e))?;
    if now != base {
        return Err(SabrageError::InvalidInput(format!(
            "{} changed on disk while Sabrage was editing it — nothing was written; reload \
             Settings and try again",
            toml_path.display()
        )));
    }
    Ok(())
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

/// A10-1: reserves `<backups_dir>/oxrsys-runtime.toml.<secs>[-n]` and writes
/// `bytes` into it via [`Executor::create_new`] (`O_EXCL`).
///
/// `next_backup_path`'s `!exists()` probe is itself a check-then-create race
/// between two Sabrage processes backing up in the same second. A lost race
/// retries the whole probe rather than bumping the suffix locally, because the
/// directory listing has changed underneath. See
/// tests::{concurrent_backups_in_the_same_second_each_keep_their_own_bytes,
/// a_same_second_backup_collision_gets_a_numeric_suffix}.
async fn reserve_backup_path(
    executor: &dyn Executor,
    backups_dir: &Path,
    secs: u64,
    bytes: &[u8],
) -> Result<PathBuf> {
    loop {
        let candidate = next_backup_path(backups_dir, secs);
        if executor.create_new(&candidate, bytes).await? {
            return Ok(candidate);
        }
    }
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
    // [`write`] refuses on its own too; this runs first so the refusal arrives as
    // a fix's `fatal` with its remedy attached rather than a bare InvalidInput.
    if let Some(block) = crate::session::live_session_block(&ctx.paths) {
        let reason = &block.reason;
        return Err(ctx.fatal(
            format!(
                "refusing to edit {} while a session is live — {reason}; the runtime re-reads \
                 this file while it streams",
                ctx.paths.toml_path.display(),
            ),
            Some(format!(
                "./demo.sh stop --bottle {}",
                block
                    .bottle
                    .as_deref()
                    .or(ctx.opts.bottle_name.as_deref())
                    .unwrap_or("<name>")
            )),
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
mod tests;
