use super::*;
use crate::executor::{DryRunExecutor, PlannedKind, RealExecutor};
use crate::paths::Paths;
use crate::stages::{null_sink, StageCtx};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;

fn real() -> RealExecutor {
    RealExecutor::new(uuid::Uuid::new_v4(), null_sink(), CancellationToken::new())
}

fn dry() -> DryRunExecutor {
    DryRunExecutor::new(uuid::Uuid::new_v4(), null_sink(), CancellationToken::new())
}

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
/// through the renderer at all — not for those shapes, and not for the
/// deployed file or the shared template, which must come back byte-for-byte
/// too.
#[test]
fn an_empty_patch_is_the_identity_on_every_input_shape() {
    let deployed_text = deployed();
    for (label, text) in [
        ("no trailing newline", "[streaming]\nbitrate_mbps = 42"),
        (
            "crlf throughout",
            "# hdr\r\n[streaming]\r\nprotocol = \"alvr\"\r\nbitrate_mbps = 42\r\n",
        ),
        ("leading bom", "\u{feff}[streaming]\nbitrate_mbps = 42\n"),
        ("deployed", deployed_text.as_str()),
        ("shared-template", crate::util::toml_template()),
    ] {
        let out = apply_patch(text, &RuntimeConfigPatch::default()).unwrap();
        assert_eq!(out.text, text, "{label}: empty patch rewrote {text:?}");
        assert!(
            out.changed_keys.is_empty(),
            "{label}: {:?}",
            out.changed_keys
        );
        assert!(out.shadowed.is_empty(), "{label}: {:?}", out.shadowed);
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

/// A value spelled with **literal** quotes is one the runtime throws
/// away: `ParseString` strips double quotes only, so `'alvr'` fails the
/// whitelist and `streamingProtocol` keeps its `oxrsys` default.
/// Re-saving rewrites the line in the spelling the runtime does read.
///
/// The defect this pins: accepting either quote flavour in `unquote` and
/// `same_to_the_runtime` shows ALVR with no warning and reports Save as
/// a success while the dead line stays on disk.
#[test]
fn a_literal_quoted_string_is_invalid_and_gets_rewritten() {
    let text = "[streaming]\nprotocol = 'alvr'\n";
    let view = read_text(text);
    assert_eq!(view.values.protocol, None, "the runtime keeps its default");
    assert_eq!(view.invalid.len(), 1, "{:?}", view.invalid);
    assert_eq!(view.invalid[0].key, "protocol");
    assert_eq!(view.invalid[0].raw, "'alvr'");
    assert!(view.parse_error.is_none(), "it is still valid TOML");

    let out = apply_patch(
        text,
        &RuntimeConfigPatch {
            protocol: Some(Protocol::Alvr),
            ..RuntimeConfigPatch::default()
        },
    )
    .unwrap();
    assert_eq!(out.text, "[streaming]\nprotocol = \"alvr\"\n");
    assert_eq!(out.changed_keys, vec!["protocol".to_string()]);
    // A different value likewise rewrites, in the canonical spelling.
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
/// text keeps the quotes), so `toml_edit`'s decoded view must not
/// disagree — same rule as the dotted key above. The quoted spelling is
/// the LAST in the document: the defect this pins reports its value from
/// `read` and edits its line from `apply_patch`, while the runtime only
/// ever sees the bare one in `[streaming]`.
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

/// `read`, minus the file. Goes through the same `fill_from` the real
/// reader does, so these tests exercise the shipped resolution rules and
/// not a second copy of them.
fn read_text(text: &str) -> RuntimeConfigView {
    let mut view = RuntimeConfigView {
        path: String::new(),
        exists: true,
        values: RuntimeConfigValues::default(),
        defaults: runtime_defaults(),
        invalid: Vec::new(),
        shadowed: Vec::new(),
        modified_unix_ms: None,
        parse_error: None,
    };
    view.fill_from(text);
    view
}

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

#[test]
fn an_absent_key_is_inserted_under_streaming_after_its_last_key() {
    let text = deployed();
    // `abr_mode` is not editable; use a key the deployed file lacks.
    let stripped = text.replace("refresh_rate_hz = 90\n", "");
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
fn a_missing_streaming_table_is_created_at_the_end() {
    let cases: &[(&str, &str, &str)] = &[
        (
            "blank line before an appended [streaming] table",
            "# a hand-written file\nabr_mode = \"off\"\n",
            "# a hand-written file\nabr_mode = \"off\"\n\n[streaming]\nbitrate_mbps = 60\n",
        ),
        (
            "no leading blank line on an empty document",
            "",
            "[streaming]\nbitrate_mbps = 60\n",
        ),
    ];
    for &(label, input, expected) in cases {
        let out = apply_patch(input, &patch_bitrate(60)).unwrap();
        assert_eq!(out.text, expected, "{label}");
    }
}

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

#[test]
fn a_parse_failure_refuses_to_rewrite() {
    let text = fixture("oxrsys-runtime.broken.toml");
    let err = apply_patch(&text, &patch_bitrate(60)).unwrap_err();
    assert!(err.to_string().contains("is not valid TOML"), "{err}");
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

/// `StripTomlComment` verbatim: only `"` toggles string context, and there
/// is no escape handling — so a `'` protects nothing and a `\\"` closes the
/// string it appears in. Being *more* clever than Config.cpp here is how
/// Sabrage ends up reading a different file than the runtime does.
#[test]
fn the_comment_stripper_matches_config_cpp() {
    assert_eq!(strip_comment("a = \"x # y\" # tail"), "a = \"x # y\" ");
    assert_eq!(strip_comment("# whole line"), "");
    assert_eq!(strip_comment("a = 1"), "a = 1");
    // A literal-quoted string does NOT protect its '#'.
    assert_eq!(strip_comment("a = 'x # y'"), "a = 'x ");
    // No escape handling: the backslash is an ordinary byte, so the '\\"'
    // closes the string and the '#' after it starts a comment.
    assert_eq!(strip_comment("a = \"x\\\" # y\""), "a = \"x\\\" ");
}

/// The runtime's `catch` block keeps "the last valid/default setting", so a
/// good assignment followed by a junk one leaves the good value in force.
/// Reading only the last occurrence reported the key as absent and fell
/// back to the compiled-in default — for `protocol` that is `oxrsys`, i.e.
/// a false legacy-backend warning on a file that streams over ALVR.
#[test]
fn a_later_invalid_assignment_does_not_erase_an_earlier_valid_one() {
    let text = fixture("oxrsys-runtime.shadowed-invalid-last.toml");
    let view = read_text(&text);

    assert_eq!(view.values.protocol, Some(Protocol::Alvr));
    assert_eq!(view.values.bitrate_mbps, Some(80));
    assert_eq!(view.values.encoder_process, Some(EncoderProcess::Native));
    assert_eq!(view.values.video_codec, Some(VideoCodec::H265));
    assert_eq!(view.values.resolution_scale, Some(0.9));
    assert_eq!(view.values.refresh_rate_hz, Some(90));

    // Every rejected line is still reported — the user must see the dead
    // assignment, just not have it override the live one.
    let mut named: Vec<&str> = view.invalid.iter().map(|i| i.key.as_str()).collect();
    named.sort_unstable();
    let mut expected = EDITABLE_KEYS.to_vec();
    expected.sort_unstable();
    assert_eq!(named, expected, "{:?}", view.invalid);
    assert!(view
        .invalid
        .iter()
        .any(|i| i.key == "protocol" && i.raw == "\"bogus\""));
    assert!(
        view.invalid
            .iter()
            .any(|i| i.key == "encoder_process" && i.raw == "'native'"),
        "a literal-quoted value is junk to the runtime too: {:?}",
        view.invalid
    );

    let mut shadowed = view.shadowed.clone();
    shadowed.sort_unstable();
    assert_eq!(shadowed, expected);

    // The edit still lands on the LAST physical occurrence: that is the one
    // the runtime reaches last, so it has to be the one that wins.
    let out = apply_patch(&text, &patch_bitrate(60)).unwrap();
    assert_eq!(out.changed_keys, vec!["bitrate_mbps".to_string()]);
    let diff = diff_lines(&text, &out.text);
    assert_eq!(diff.len(), 1, "{diff:?}");
    assert_eq!(diff[0].0, "bitrate_mbps = 9001");
    assert_eq!(diff[0].1, "bitrate_mbps = 60");
}

/// A table-driven differential against `ParseConfigToml`'s documented
/// semantics: single quotes are not quotes, `#` is only protected by `"`,
/// `std::stoi`-unfriendly spellings are junk, and tables never matter.
#[test]
fn the_line_reader_agrees_with_parse_config_toml() {
    // (source, effective protocol, effective bitrate)
    let cases: &[(&str, Option<Protocol>, Option<u32>)] = &[
        // ParseString takes double quotes only.
        ("protocol = 'alvr'\n", None, None),
        ("protocol = \"alvr\"\n", Some(Protocol::Alvr), None),
        // Bare (unquoted) values pass straight through the whitelist.
        ("protocol = alvr\n", Some(Protocol::Alvr), None),
        // No escape handling: the value keeps its inner backslash and quote.
        ("protocol = \"al\\\"vr\"\n", None, None),
        // Tables are ignored entirely; last accepted wins.
        (
            "[a]\nprotocol = \"oxrsys\"\n[b]\nprotocol = \"alvr\"\n",
            Some(Protocol::Alvr),
            None,
        ),
        // A '#' inside a basic string survives; one after it does not.
        (
            "protocol = \"alvr\" # was oxrsys\n",
            Some(Protocol::Alvr),
            None,
        ),
        // std::stoi is base 10: it reads "0x50" as 0, which is below the
        // minimum bitrate, so the runtime keeps its default.
        ("bitrate_mbps = 0x50\n", None, None),
        ("bitrate_mbps = 80\n", None, Some(80)),
        // TOML's numeric underscores mean nothing to std::stoi: it takes
        // the leading digits and stops, so this really is 1, not 10.
        ("bitrate_mbps = 1_0\n", None, Some(1)),
        // …and trailing junk after a valid number is likewise ignored.
        ("bitrate_mbps = 80 mbps\n", None, Some(80)),
        // Arrays and inline tables are values no editable key accepts.
        ("bitrate_mbps = [80]\n", None, None),
        ("protocol = { a = \"alvr\" }\n", None, None),
        // A dotted key is a different key text after the split.
        ("streaming.protocol = \"alvr\"\n", None, None),
        // So is a quoted one.
        ("\"protocol\" = \"alvr\"\n", None, None),
    ];
    for (text, protocol, bitrate) in cases {
        let (values, _, _) = read_lines_like_the_runtime(text);
        assert_eq!(values.protocol, *protocol, "protocol of {text:?}");
        assert_eq!(values.bitrate_mbps, *bitrate, "bitrate of {text:?}");
    }
}

/// A multiline string is one value to TOML and a run of live assignments to
/// the runtime. `read` must show the runtime's answer, and `write` must
/// refuse: there is no line `apply_patch` could edit that would beat the
/// one inside the string.
#[test]
fn a_key_inside_a_multiline_string_reads_live_and_refuses_the_write() {
    let text = "[streaming]\nprotocol = \"alvr\"\nnote = \"\"\"\nprotocol = \"oxrsys\"\n\"\"\"\n";
    let view = read_text(text);
    assert_eq!(
        view.values.protocol,
        Some(Protocol::Oxrsys),
        "the runtime reads the physical line inside the string"
    );
    let err = view.parse_error.expect("must refuse to rewrite this file");
    assert!(err.contains("protocol"), "{err}");
    assert!(err.contains("multiline string"), "{err}");
}

/// A10-3/A10-4: the refusal belongs to [`apply_patch`], which every caller
/// goes through, not to the *view* alone — a caller that does not render
/// Settings first (`write`, `edit_protocol`, the CLI) would otherwise
/// rewrite the outer line, report success, and leave the runtime obeying
/// the line inside the string.
#[tokio::test]
async fn the_multiline_shadow_is_refused_by_apply_patch_write_and_edit_protocol() {
    let _g = crate::session::lock_session_globals();
    let text = fixture("oxrsys-runtime.multiline-shadow.toml");
    assert_eq!(
        read_lines_like_the_runtime(&text).0.protocol,
        Some(Protocol::Oxrsys),
        "the runtime honours the line inside the string"
    );

    let err = apply_patch(
        &text,
        &RuntimeConfigPatch {
            protocol: Some(Protocol::Alvr),
            ..RuntimeConfigPatch::default()
        },
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("refusing to rewrite"), "{err}");
    assert!(err.contains("multiline string"), "{err}");

    // `write` inherits it, and leaves the file — and the backups — alone.
    let dir = scratch("multiline-shadow");
    let path = dir.join("oxrsys-runtime.toml");
    let backups = dir.join("backups");
    std::fs::write(&path, &text).unwrap();
    let err = write(&real(), &path, &backups, &patch_bitrate(60))
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("multiline string"), "{err}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    assert!(!backups.exists(), "no backup churn from a refused write");
    let _ = std::fs::remove_dir_all(&dir);

    // …and so does the fix, which must not claim it set protocol.
    let dir = scratch("multiline-shadow-fix");
    let ctx = fix_ctx(&dir, false);
    std::fs::create_dir_all(&ctx.paths.oxr_appsup).unwrap();
    std::fs::write(&ctx.paths.toml_path, &text).unwrap();
    let err = edit_protocol(&ctx, &null_sink())
        .await
        .expect_err("the fix cannot honestly claim it set protocol")
        .to_string();
    assert!(err.contains("multiline string"), "{err}");
    assert_eq!(std::fs::read_to_string(&ctx.paths.toml_path).unwrap(), text);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A10-5: a BOM on a root key's own line is the mirror image of the
/// multiline case — `toml_edit` (handed the stripped body) sees an editable
/// `protocol`, while the runtime's line reader sees `\u{feff}protocol` and
/// ignores it. Editing that line changes nothing the runtime reads, so it is
/// refused instead of silently reported as saved.
#[test]
fn a_bom_on_a_root_key_is_not_round_trippable() {
    let text = "\u{feff}protocol = \"alvr\"\n[streaming]\nbitrate_mbps = 42\n";
    let (values, invalid, _) = read_lines_like_the_runtime(text);
    assert_eq!(
        values.protocol, None,
        "the runtime does not see a key behind a BOM, so its default applies"
    );
    assert!(invalid.is_empty(), "{invalid:#?}");

    let err = read_text(text)
        .parse_error
        .expect("the view must refuse this file");
    assert!(err.contains("protocol"), "{err}");
    assert!(err.contains("byte-order mark"), "{err}");

    for (label, patch) in [
        (
            "the shadowed key itself",
            RuntimeConfigPatch {
                protocol: Some(Protocol::Alvr),
                ..RuntimeConfigPatch::default()
            },
        ),
        ("an unrelated key", patch_bitrate(60)),
    ] {
        let err = apply_patch(text, &patch).unwrap_err().to_string();
        assert!(err.contains("refusing to rewrite"), "{label}: {err}");
    }
}

/// The BOM shape that is *not* a mismatch: the byte sits on a `[table]`
/// header, which neither reader treats as an assignment. Refusing that one
/// would make every BOM'd file unsavable.
#[test]
fn a_bom_in_front_of_a_table_header_stays_writable() {
    let text = "\u{feff}[streaming]\nprotocol = \"alvr\"\nbitrate_mbps = 42\n";
    assert_eq!(
        read_lines_like_the_runtime(text).0.protocol,
        Some(Protocol::Alvr)
    );
    assert!(read_text(text).parse_error.is_none());
    let out = apply_patch(text, &patch_bitrate(60)).unwrap();
    assert_eq!(out.text, text.replace("= 42", "= 60"));
}

/// A10-7: `Config.cpp:367-372` narrows to a C++ `float` and *then* checks
/// the bounds (`float val = std::stof(value); if (val >= 0.25f && val <=
/// 1.0f)`), so a value inside one `f32` ulp of an endpoint is accepted
/// there. Checking the `f64` instead reported the key invalid and the
/// Settings screen showed the `0.75` default for a file the runtime reads
/// as `1.0`.
#[test]
fn the_scale_bounds_are_checked_at_the_runtimes_float_precision() {
    for (raw, accepted) in [
        ("1.0", true),
        ("0.25", true),
        ("0.75", true),
        // Round to exactly 1.0f / 0.25f — the runtime takes both.
        ("1.00000001", true),
        ("0.2499999999", true),
        // Still out of range after narrowing.
        ("1.01", false),
        ("0.24", false),
        ("2.0", false),
        ("-1.0", false),
        ("1e40", false),
    ] {
        let text = format!("[streaming]\nresolution_scale = {raw}\n");
        let (values, invalid, _) = read_lines_like_the_runtime(&text);
        assert_eq!(
            values.resolution_scale.is_some(),
            accepted,
            "{raw}: {invalid:?}"
        );
        assert_eq!(invalid.is_empty(), accepted, "{raw}: {invalid:?}");
    }
}

/// A10-9: two rejected occurrences of one key both reach the UI, which keys
/// its list by the pair below — so the pair has to stay unique. It is, by
/// construction (the fold drops an occurrence identical to one already
/// collected), and this pins it: a dedupe loosened to the key alone, or
/// dropped, would hand Svelte a duplicate key and throw the screen away.
#[test]
fn two_invalid_occurrences_of_one_key_stay_distinguishable() {
    let text = "[streaming]\nprotocol = 'alvr'\nbitrate_mbps = 80\n\n[legacy]\nprotocol = \
                    \"oxrsys-usb\"\n";
    let view = read_text(text);
    assert!(view.parse_error.is_none(), "{:?}", view.parse_error);
    assert_eq!(view.invalid.len(), 2, "{:#?}", view.invalid);
    assert!(view.invalid.iter().all(|iv| iv.key == "protocol"));

    let mut identities: Vec<(&str, &str)> = view
        .invalid
        .iter()
        .map(|iv| (iv.key.as_str(), iv.raw.as_str()))
        .collect();
    let before = identities.len();
    identities.sort_unstable();
    identities.dedup();
    assert_eq!(
        identities.len(),
        before,
        "(key, raw) is the UI's list identity and must be unique: {:#?}",
        view.invalid
    );
}

#[test]
fn effective_string_is_last_wins_table_blind_and_double_quote_only() {
    assert_eq!(
        effective_string(
            "[a]\nencoder_process = \"inproc\"\n[b]\nencoder_process = \"native\"\n",
            "encoder_process"
        ),
        Some("native".to_string()),
        "last assignment wins, whatever table it sits in"
    );
    assert_eq!(
        effective_string("[streaming]\nprotocol = \"alvr\" # keep\n", "protocol"),
        Some("alvr".to_string()),
        "a same-line comment is not part of the value"
    );
    assert_eq!(
        effective_string("protocol = 'alvr'\n", "protocol"),
        Some("'alvr'".to_string()),
        "literal quotes stay on, so the caller's whitelist rejects them"
    );
    assert_eq!(
        effective_string(
            "  protocol = \"alvr\"\nencoder_process=\"native\"\n",
            "protocol"
        ),
        Some("alvr".to_string()),
        "an indented assignment is still an assignment"
    );
    assert_eq!(
        effective_string(
            "  protocol = \"alvr\"\nencoder_process=\"native\"\n",
            "encoder_process"
        ),
        Some("native".to_string()),
        "no spaces around `=`, and each key answers only for itself"
    );
    assert_eq!(
        effective_string(
            "protocol_foo = \"x\"\n# protocol = \"alvr\"\nprotocol = \"oxrsys\"\n",
            "protocol"
        ),
        Some("oxrsys".to_string()),
        "`protocol_foo` is a different key and a commented line is no assignment"
    );
    assert_eq!(
        effective_string("protocol = \"al#vr\"\n", "protocol"),
        Some("al#vr".to_string()),
        "a `#` inside the quoted value is part of the value"
    );
    assert_eq!(
        effective_string("protocol = alvr\n", "protocol"),
        Some("alvr".to_string()),
        "an unquoted value is read as-is, where run.sh's `awk -F'\"'` captures nothing"
    );
    assert_eq!(effective_string("[streaming]\n", "protocol"), None);
    assert_eq!(
        effective_string("", "protocol"),
        None,
        "an empty file assigns nothing"
    );
    assert_eq!(
        effective_string("# protocol = \"alvr\"\n", "protocol"),
        None,
        "a commented line is not an assignment"
    );
}

/// A3b-1/A7-1: last-**valid**-wins, the rule `Config.cpp`'s `catch` block
/// implements, for the keys Sabrage models. `effective_string` above stops
/// at last-*raw*, which reports `banana` for a file the runtime is running
/// as ALVR — enough to block a launch that would have worked.
#[test]
fn effective_accepted_keeps_the_last_value_the_runtime_would_accept() {
    assert_eq!(
        effective_accepted("protocol = \"alvr\"\nprotocol = \"banana\"\n", "protocol"),
        Some("alvr".to_string()),
        "a trailing junk value is ignored and the previous valid one stays"
    );
    assert_eq!(
        effective_string("protocol = \"alvr\"\nprotocol = \"banana\"\n", "protocol"),
        Some("banana".to_string()),
        "which is exactly where the raw reader differs"
    );
    assert_eq!(
        effective_accepted(
            "[a]\nencoder_process = \"inproc\"\n[b]\nencoder_process = \"native\"\n",
            "encoder_process"
        ),
        Some("native".to_string()),
        "still table-blind and still last-wins between two accepted values"
    );
    assert_eq!(
        effective_accepted("protocol = alvr\n", "protocol"),
        Some("alvr".to_string()),
        "an unquoted value is one the runtime accepts"
    );
    assert_eq!(
        effective_accepted("protocol = 'alvr'\n", "protocol"),
        None,
        "a literal-quoted value keeps its quotes and matches nothing"
    );
    assert_eq!(effective_accepted("[streaming]\n", "protocol"), None);
    assert_eq!(
        effective_accepted("render_device = \"gpu\"\n", "render_device"),
        None,
        "a key Sabrage does not model has no accepted set — use effective_string"
    );
    assert_eq!(
        effective_accepted("bitrate_mbps = 80 # was 50\n", "bitrate_mbps"),
        Some("80".to_string()),
        "numbers come back in the runtime's own spelling"
    );
    assert_eq!(
        effective_accepted("bitrate_mbps = 80\nbitrate_mbps = 900\n", "bitrate_mbps"),
        Some("80".to_string()),
        "out of range is 'malformed' too"
    );
}

/// The shipped fixture the launch gate has to agree with: a valid `protocol`
/// followed by an invalid one. Whatever reads it must land on ALVR.
#[test]
fn effective_accepted_agrees_with_the_line_reader_on_the_shadowed_fixtures() {
    for name in [
        "oxrsys-runtime.shadowed.toml",
        "oxrsys-runtime.shadowed-invalid-last.toml",
        "oxrsys-runtime.deployed.toml",
    ] {
        let text = fixture(name);
        let (values, _, _) = read_lines_like_the_runtime(&text);
        assert_eq!(
            effective_accepted(&text, "protocol"),
            values.protocol.map(|p| p.as_str().to_string()),
            "{name}: one reader, one answer"
        );
        assert_eq!(
            effective_accepted(&text, "encoder_process"),
            values.encoder_process.map(|e| e.as_str().to_string()),
            "{name}: one reader, one answer"
        );
    }
}

/// `toml_edit`'s renderer normalises CRLF to LF, drops a BOM and appends a
/// final newline. A no-op patch already avoided that by returning the input
/// bytes; a *real* edit went through the renderer and rewrote every line of
/// a CRLF file to change one value.
#[test]
fn a_real_edit_preserves_crlf_a_bom_and_a_missing_final_newline() {
    let lf = "[streaming]\nprotocol = \"alvr\"\nbitrate_mbps = 42\n# note\nrefresh_rate_hz = 72\n";
    for (label, text) in [
        ("lf", lf.to_string()),
        ("crlf", lf.replace('\n', "\r\n")),
        ("bom", format!("\u{feff}{lf}")),
        ("no-final-newline", lf.trim_end_matches('\n').to_string()),
        ("bom+crlf", format!("\u{feff}{}", lf.replace('\n', "\r\n"))),
    ] {
        let out = apply_patch(&text, &patch_bitrate(60)).unwrap();
        assert_eq!(
            out.changed_keys,
            vec!["bitrate_mbps".to_string()],
            "{label}"
        );
        assert_eq!(
            out.text,
            text.replace("bitrate_mbps = 42", "bitrate_mbps = 60"),
            "{label}: only the edited value may differ"
        );
    }
}

#[tokio::test]
async fn write_creates_from_the_template_byte_identically_then_patches() {
    let _g = crate::session::lock_session_globals();
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

/// A10-1: the absent-file branch goes through [`Executor::create_new`]
/// (`O_EXCL`), never an unconditional rename, so a file that appears
/// between the probe and the create is read back instead of clobbered.
#[tokio::test]
async fn write_never_clobbers_a_file_created_in_the_toctou_window() {
    let _g = crate::session::lock_session_globals();
    let dir = scratch("create-race");
    let path = dir.join("oxrsys-runtime.toml");
    let backups = dir.join("backups");
    let ex = real();

    // Simulate the race directly: the file is absent when `write` probes
    // for it (the scratch dir starts empty), so drive the same primitive
    // `write` would use and confirm the loser reads the winner's bytes.
    let winner = "[streaming]\nprotocol = \"alvr\"\nbitrate_mbps = 7\n";
    assert!(ex.create_new(&path, winner.as_bytes()).await.unwrap());
    assert!(!ex.create_new(&path, b"loser").await.unwrap());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), winner);

    // `write` itself, called against that now-existing file, must treat it
    // as pre-existing (never `created_from_template`) and patch the
    // winner's bytes, not silently discard them.
    let report = write(&ex, &path, &backups, &patch_bitrate(60))
        .await
        .unwrap();
    assert!(!report.created_from_template);
    assert!(std::fs::read_to_string(&path)
        .unwrap()
        .contains("bitrate_mbps = 60"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A10-1: two writers backing up in the same second must not collide —
/// [`reserve_backup_path`] retries the whole probe on a lost `create_new`
/// race rather than trusting a stale `!exists()` read, so the loser lands
/// on the next free suffix instead of overwriting the winner's backup.
#[tokio::test]
async fn concurrent_backups_in_the_same_second_each_keep_their_own_bytes() {
    let dir = scratch("backup-race");
    let backups = dir.join("backups");
    std::fs::create_dir_all(&backups).unwrap();
    let ex = real();
    let secs = 1_756_500_000u64;

    let first = reserve_backup_path(&ex, &backups, secs, b"first")
        .await
        .unwrap();
    let second = reserve_backup_path(&ex, &backups, secs, b"second")
        .await
        .unwrap();
    assert_ne!(
        first, second,
        "the second writer must not overwrite the first"
    );
    assert_eq!(std::fs::read_to_string(&first).unwrap(), "first");
    assert_eq!(std::fs::read_to_string(&second).unwrap(), "second");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A10-1: [`lock_toml`] is [`Paths::toml_lock_path`]'s doc-promised call
/// site, held across a `write` from acquire to release — an in-process
/// contender opened as a fresh `File` (a distinct open file description,
/// so its own `try_lock` genuinely contends with the held one) must wait
/// until the first write finishes rather than interleaving with it.
///
/// [`Paths::toml_lock_path`]: crate::paths::Paths::toml_lock_path
#[tokio::test]
async fn write_takes_the_cross_process_lock_at_the_documented_path() {
    let _g = crate::session::lock_session_globals();
    let dir = scratch("toml-lock");
    let path = dir.join("oxrsys-runtime.toml");
    let backups = dir.join("backups");
    let ex = real();
    write(&ex, &path, &backups, &patch_bitrate(60))
        .await
        .unwrap();

    let lock_path = path.with_file_name(".oxrsys-runtime.toml.lock");
    assert!(
        lock_path.is_file(),
        "write must leave the documented lock file behind: {lock_path:?}"
    );

    // Hold the lock from a second, independent open — exactly the shape a
    // concurrent CLI process's `flock` would take.
    let contender = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock_path)
        .unwrap();
    contender.lock().unwrap();

    let started = tokio::time::Instant::now();
    write(&ex, &path, &backups, &patch_bitrate(72))
        .await
        .unwrap();
    // Best-effort: proceeds without the lock once `TOML_LOCK_WAIT` elapses,
    // rather than blocking forever — this asserts it actually waited out
    // (most of) the budget instead of slipping past the held lock.
    assert!(
        started.elapsed() >= std::time::Duration::from_millis(1800),
        "a held lock must be waited out, not bypassed: {:?}",
        started.elapsed()
    );
    drop(contender);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn write_creates_the_template_verbatim_for_an_empty_patch() {
    let _g = crate::session::lock_session_globals();
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
    let _g = crate::session::lock_session_globals();
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

/// Regression: a hand-maintained file that is not LF + newline-terminated
/// used to be backed up and rewritten whole by an EMPTY patch, because the
/// no-op test compared `toml_edit`'s re-render against the file. The report
/// said `changedKeys: []` while the bytes on disk had moved — exactly the
/// "Sabrage touched my file and told me it didn't" failure this module
/// exists to prevent.
#[tokio::test]
async fn a_no_op_write_leaves_the_file_and_backups_untouched() {
    let _g = crate::session::lock_session_globals();
    let dep = deployed();
    let unnormalised = "# my notes\n[streaming]\nprotocol = \"alvr\"\nbitrate_mbps = 80";
    let cases: &[(&str, &str, RuntimeConfigPatch)] = &[
        (
            "deployed file, same-bitrate patch",
            dep.as_str(),
            patch_bitrate(80),
        ),
        (
            "unnormalised file, empty patch",
            unnormalised,
            RuntimeConfigPatch::default(),
        ),
        (
            "unnormalised file, same protocol",
            unnormalised,
            RuntimeConfigPatch {
                protocol: Some(Protocol::Alvr),
                ..RuntimeConfigPatch::default()
            },
        ),
    ];
    let ex = real();
    for (i, &(label, original, patch)) in cases.iter().enumerate() {
        let dir = scratch(&format!("write-noop-{i}"));
        let path = dir.join("oxrsys-runtime.toml");
        let backups = dir.join("backups");
        std::fs::write(&path, original).unwrap();
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();

        let report = write(&ex, &path, &backups, &patch).await.unwrap();
        assert!(
            report.changed_keys.is_empty(),
            "{label}: {:?}",
            report.changed_keys
        );
        assert_eq!(report.backup_path, None, "{label}");
        assert!(!report.created_from_template, "{label}");
        assert!(!backups.exists(), "{label}: no backup slot may be burned");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original, "{label}");
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            before,
            "{label}: the file must not be reopened for writing"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
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
    let _g = crate::session::lock_session_globals();
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
    // The three oldest went; the newest old one stayed.
    assert!(!backups.join(format!("{BACKUP_PREFIX}1000")).exists());
    assert!(!backups.join(format!("{BACKUP_PREFIX}1001")).exists());
    assert!(!backups.join(format!("{BACKUP_PREFIX}1002")).exists());
    assert!(backups.join(format!("{BACKUP_PREFIX}1011")).exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A10-2: pruning is recovery history, and a save that never committed has
/// no right to it. The prune used to run between the reservation and the
/// replacement, so a failed `write_atomic` returned an error with the config
/// untouched and the three oldest backups already gone — and the
/// compare-and-swap's "nothing was written" was false for the same reason.
#[tokio::test]
async fn a_failed_write_prunes_nothing_and_leaves_no_reservation() {
    let _g = crate::session::lock_session_globals();
    let dir = scratch("failed-write-keeps-backups");
    let path = dir.join("oxrsys-runtime.toml");
    let backups = dir.join("backups");
    std::fs::create_dir_all(&backups).unwrap();
    std::fs::write(&path, deployed()).unwrap();
    for i in 0..12u64 {
        std::fs::write(backups.join(format!("{BACKUP_PREFIX}{}", 1_000 + i)), "old").unwrap();
    }
    let before = list_backups(&backups);
    assert_eq!(before.len(), 12);

    // `write_atomic` stages its temp file in the config's own directory, so
    // a read-only parent fails the commit — and only the commit: `backups/`
    // is an existing directory of its own and stays writable.
    let mode = std::fs::metadata(&dir).unwrap().permissions();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    if std::fs::File::create(dir.join(".writable-probe")).is_ok() {
        // Running as root: the mode says nothing. Nothing to assert.
        std::fs::set_permissions(&dir, mode).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    let err = write(&real(), &path, &backups, &patch_bitrate(60))
        .await
        .expect_err("the commit cannot succeed against a read-only directory");
    std::fs::set_permissions(&dir, mode).unwrap();

    assert!(
        err.to_string().contains(&dir.display().to_string()),
        "{err}: the failure must name the path it could not write"
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        deployed(),
        "the config is unchanged"
    );
    assert_eq!(
        list_backups(&backups),
        before,
        "an aborted save is a complete no-op: no prune, no new backup"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The other half of the no-op rule above: once `write_atomic` has
/// committed, nothing that happens afterwards may turn the save into an
/// `Err`. The prune runs *after* the commit, and its unlink can fail for
/// reasons that say nothing about the config file — here the oldest
/// backup is a directory, so `remove_file` gets EPERM/EISDIR rather than
/// `NotFound`. Reporting that as a failed save left the Settings screen
/// showing the old values over a file the runtime had already re-read.
#[tokio::test]
async fn an_unprunable_stale_backup_still_reports_a_committed_save() {
    let _g = crate::session::lock_session_globals();
    let dir = scratch("unprunable-stale-backup");
    let path = dir.join("oxrsys-runtime.toml");
    let backups = dir.join("backups");
    std::fs::create_dir_all(&backups).unwrap();
    std::fs::write(&path, deployed()).unwrap();
    // 12 > BACKUP_KEEP, so the three oldest are stale and get pruned.
    for i in 0..12u64 {
        std::fs::write(backups.join(format!("{BACKUP_PREFIX}{}", 1_000 + i)), "old").unwrap();
    }
    let unprunable = backups.join(format!("{BACKUP_PREFIX}1000"));
    std::fs::remove_file(&unprunable).unwrap();
    std::fs::create_dir(&unprunable).unwrap();
    assert!(
        std::fs::remove_file(&unprunable).is_err(),
        "the fixture only means anything if this unlink really fails"
    );

    let report = write(&real(), &path, &backups, &patch_bitrate(60))
        .await
        .expect("a committed write is a success even when the prune cannot finish");

    assert_eq!(report.changed_keys, vec!["bitrate_mbps".to_string()]);
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("bitrate_mbps = 60"),
        "the new bytes are on disk — which is why this must not report failure"
    );
    assert!(unprunable.is_dir(), "the unprunable entry is left alone");
    assert!(
        !backups.join(format!("{BACKUP_PREFIX}1001")).exists()
            && !backups.join(format!("{BACKUP_PREFIX}1002")).exists(),
        "the prunable stale backups are still pruned"
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
    let _g = crate::session::lock_session_globals();
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
    let _g = crate::session::lock_session_globals();
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

/// A `session-state.json` naming a wine process that is still alive. The
/// pid is this test process, which is exactly what makes `is_same_process`
/// true without launching anything.
fn write_live_session(dir: &Path, bottle: &str, owner_pid: u32) {
    let mut state = crate::session::state::SessionState::new(
        uuid::Uuid::new_v4(),
        bottle,
        dir.join("bs"),
        dir.join("run.log"),
        0,
    );
    state.set_owner(owner_pid);
    state.wine = crate::process::ProcInfo::observe(std::process::id());
    assert!(state.wine.is_some(), "the test process must be observable");
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(
        dir.join("session-state.json"),
        serde_json::to_vec(&state).unwrap(),
    )
    .unwrap();
}

/// The runtime polls this file every 250 ms and rebuilds the encoder when
/// `encoder_process`/`video_codec` drift, so a Settings save mid-stream is
/// not "next launch" — it is a live reconfiguration. `write` refuses, and
/// the file is left untouched down to its mtime.
#[tokio::test]
async fn write_refuses_while_a_session_is_live_and_touches_nothing() {
    let _g = crate::session::lock_session_globals();
    let dir = scratch("live-guard");
    let path = dir.join("oxrsys-runtime.toml");
    let backups = dir.join("backups");
    std::fs::write(&path, deployed()).unwrap();
    let before = std::fs::metadata(&path).unwrap().modified().unwrap();
    // `backups_dir` is `<sabrage_appsup>/backups`, so the record goes beside it.
    write_live_session(&dir, "beatsaber", std::process::id());

    let err = write(&real(), &path, &backups, &patch_bitrate(60))
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("beatsaber"), "{err}");
    // The Settings screen renders this string verbatim: one wrapped literal
    // with the bottle spliced into the remedy, no 10-space craters in it.
    assert!(err.contains("./demo.sh stop --bottle beatsaber"), "{err}");
    assert!(!err.contains("  "), "double space in: {err}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), deployed());
    assert_eq!(
        std::fs::metadata(&path).unwrap().modified().unwrap(),
        before
    );
    assert!(!backups.exists(), "no backup churn from a refused write");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A `./demo.sh run` session writes no `session-state.json` and publishes
/// no in-process handle: the fresh `runtime_status.json` beside this very
/// file is the only trace it leaves, and the old local predicate could not
/// see it — so Settings would happily rebuild the encoder mid-stream.
#[tokio::test]
async fn write_refuses_while_only_the_runtime_reports_a_live_session() {
    let _g = crate::session::lock_session_globals();
    let dir = scratch("live-guard-runtime-status");
    let path = dir.join("oxrsys-runtime.toml");
    std::fs::write(&path, deployed()).unwrap();
    assert!(
        !dir.join("session-state.json").exists(),
        "the shell pipeline writes no session record"
    );
    let now = crate::session::now_unix_ms();
    // A live `process_id` and a fresh stamp together: that is
    // `watcher::runtime_status_live`, the single predicate both this door and
    // the phase the Session screen renders go through (A10-8).
    let pid = std::process::id();
    std::fs::write(
        dir.join("runtime_status.json"),
        format!(r#"{{"state":"streaming","process_id":{pid},"updated_at_unix_ms":{now}}}"#),
    )
    .unwrap();

    let err = write(&real(), &path, &dir.join("backups"), &patch_bitrate(60))
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("streaming"), "{err}");
    // No signal names a bottle here, so the remedy keeps demo.sh's placeholder.
    assert!(err.contains("./demo.sh stop --bottle <name>"), "{err}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), deployed());
    let _ = std::fs::remove_dir_all(&dir);
}

/// The guard has to be about the *machine*, not this process: a session the
/// `sabrage` CLI (or a detached Sabrage) owns has no in-process handle here,
/// and its encoder is just as rebuildable.
#[tokio::test]
async fn the_live_guard_fires_for_a_session_owned_by_another_process() {
    let _g = crate::session::lock_session_globals();
    let dir = scratch("live-guard-other");
    let path = dir.join("oxrsys-runtime.toml");
    std::fs::write(&path, deployed()).unwrap();
    assert!(
        crate::session::live_session().is_none(),
        "no in-process handle in this test"
    );
    write_live_session(&dir, "otherbottle", std::process::id().wrapping_add(1));

    let err = write(&real(), &path, &dir.join("backups"), &patch_bitrate(60))
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("otherbottle"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A crashed session leaves the record behind. It names a dead pid, so it
/// must not wedge the Settings screen forever.
#[tokio::test]
async fn a_stale_session_record_does_not_block_a_write() {
    let _g = crate::session::lock_session_globals();
    let dir = scratch("live-guard-stale");
    let path = dir.join("oxrsys-runtime.toml");
    std::fs::write(&path, deployed()).unwrap();
    let mut state = crate::session::state::SessionState::new(
        uuid::Uuid::new_v4(),
        "beatsaber",
        dir.join("bs"),
        dir.join("run.log"),
        0,
    );
    // A pid that cannot be running, with a start time nothing can match.
    state.wine = Some(crate::process::ProcInfo {
        pid: 0xffff_fffe,
        start_time: 1,
        exe: PathBuf::from("/nope"),
    });
    std::fs::write(
        dir.join("session-state.json"),
        serde_json::to_vec(&state).unwrap(),
    )
    .unwrap();

    let report = write(&real(), &path, &dir.join("backups"), &patch_bitrate(60))
        .await
        .unwrap();
    assert_eq!(report.changed_keys, vec!["bitrate_mbps".to_string()]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// `write` reads, patches in memory, backs up and only then replaces. If
/// anything moved the file in between — the `sabrage` CLI, `setup.sh`, a
/// human with an editor — the replacement would drop their work and the
/// backup would describe bytes that no longer existed.
#[test]
fn the_replacement_refuses_when_the_file_changed_underneath() {
    let _g = crate::session::lock_session_globals();
    let dir = scratch("cas");
    let path = dir.join("oxrsys-runtime.toml");
    let session = dir.join("session-state.json");
    std::fs::write(&path, deployed()).unwrap();

    still_safe_to_replace(&real(), &path, &session, &deployed()).expect("unchanged file must pass");

    std::fs::write(&path, "protocol = \"alvr\"\n").unwrap();
    let err = still_safe_to_replace(&real(), &path, &session, &deployed())
        .unwrap_err()
        .to_string();
    assert!(err.contains("changed on disk"), "{err}");
    assert!(err.contains("nothing was written"), "{err}");
    assert!(
        err.contains(&path.display().to_string()),
        "the message must name the file: {err}"
    );

    // A dry run planned the write instead of performing it, so there is
    // deliberately nothing on disk for it to compare against.
    still_safe_to_replace(&dry(), &path, &session, &deployed())
        .expect("a dry run is exempt from the byte check");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A context whose `toml_path` and `sabrage_appsup` both live under a
/// scratch directory: no test may touch the real ones, which a
/// config-writing suite would otherwise overwrite (see the deployed
/// fixture's own header).
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
