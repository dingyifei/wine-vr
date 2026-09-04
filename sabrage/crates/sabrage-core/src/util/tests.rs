use super::*;
use std::path::PathBuf;

/// The repo root, three directories above this crate's manifest directory.
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("repo root resolves")
}

#[test]
fn cmp_files_matches_cmp_s_semantics() {
    let root = repo_root();
    let a = root.join("contract/pipeline.toml");
    assert!(cmp_files(&a, &a));
    assert!(!cmp_files(
        &a,
        &root.join("contract/oxrsys-runtime.toml.template")
    ));
    assert!(!cmp_files(&a, Path::new("/nonexistent/sabrage/probe")));
    assert!(!cmp_files(Path::new("/nonexistent/sabrage/probe"), &a));
}

#[test]
fn json_escape_string_is_install_shs_two_substitutions() {
    assert_eq!(json_escape_string("/plain/path"), "/plain/path");
    assert_eq!(json_escape_string(r#"a"b"#), r#"a\"b"#);
    assert_eq!(json_escape_string(r"a\b"), r"a\\b");
    // Backslash first: the escape it introduces is not re-escaped.
    assert_eq!(json_escape_string(r#"\""#), r#"\\\""#);
    // Deliberately NOT a full JSON encoder — the zsh side escapes exactly
    // these two characters and nothing else.
    assert_eq!(json_escape_string("a\nb"), "a\nb");
}

#[test]
fn contract_hash_matches_the_generated_header() {
    let root = repo_root();
    let want = contract_hash(&root).expect("contract files readable");
    let have = contract_gen_recorded_hash(&root).expect("generated header present");
    assert_eq!(
        want, have,
        "contract/ and scripts/demo/contract.gen.sh out of sync"
    );
}

#[test]
fn dxmt_helpers_need_every_file_and_a_current_marker() {
    let root = std::env::temp_dir().join(format!("sabrage-util-dxmt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let paths = Paths::new(&root);
    let pin = &contract().deps.dxmt_tgz_sha256;

    assert!(!dxmt_files_ok(&paths));
    assert!(!dxmt_ok(&paths));

    // All five files, no marker: files ok, dxmt_ok still false.
    for f in &contract().dxmt.files {
        let p = paths.dxmt_art.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, b"x").unwrap();
    }
    assert!(dxmt_files_ok(&paths));
    assert!(!dxmt_ok(&paths));

    // Marker written the way setup does it.
    let marker = paths.dxmt_art.join(".sha256");
    std::fs::write(&marker, contract_marker_bytes(pin)).unwrap();
    assert!(dxmt_ok(&paths));
    // A marker with no trailing newline still reads as current.
    std::fs::write(&marker, pin).unwrap();
    assert!(dxmt_ok(&paths));
    // A stale marker does not.
    std::fs::write(&marker, contract_marker_bytes("deadbeef")).unwrap();
    assert!(!dxmt_ok(&paths));

    // One missing file sinks both.
    std::fs::remove_file(paths.dxmt_art.join(&contract().dxmt.files[0])).unwrap();
    assert!(!dxmt_files_ok(&paths));
    std::fs::write(&marker, contract_marker_bytes(pin)).unwrap();
    assert!(!dxmt_ok(&paths));

    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn marker_bytes_are_the_pin_plus_exactly_one_newline() {
    let pin = &contract().deps.dxmt_tgz_sha256;
    let bytes = contract_marker_bytes(pin);
    assert_eq!(bytes, format!("{pin}\n"));
    assert!(bytes.ends_with('\n'));
    assert!(!bytes.ends_with("\n\n"));
    assert_eq!(strip_trailing_newlines(&bytes), pin);
}

#[test]
fn bs_version_falls_back_to_question_mark() {
    assert_eq!(bs_version(Path::new("/nonexistent/sabrage/bs")), "?");
}

#[test]
fn version_stamp_scan_matches_grep() {
    // Single stamp, embedded in binary noise.
    assert_eq!(
        first_matching_line_stamps(b"\x00\x01Unity 1.29.4_4575554838\x00").as_deref(),
        Some("1.29.4_4575554838")
    );
    // Leading digits that overflow {1,2} shift the match right, as ERE does.
    assert_eq!(
        first_matching_line_stamps(b"12345.1.1_123456").as_deref(),
        Some("45.1.1_123456")
    );
    // Fewer than six trailing digits is not a stamp.
    assert_eq!(first_matching_line_stamps(b"1.29.4_12345"), None);
    // -m1 selects the first matching LINE; all matches on it are emitted.
    assert_eq!(
        first_matching_line_stamps(b"nope\n1.29.4_100000 and 1.30.0_200000\n1.31.0_300000")
            .as_deref(),
        Some("1.29.4_100000\n1.30.0_200000")
    );
    // A four-digit middle field exceeds {1,3}.
    assert_eq!(first_matching_line_stamps(b"1.1234.5_123456"), None);
}
