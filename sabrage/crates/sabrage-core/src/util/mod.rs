//! Primitives shared by checks, fixes, and stages: byte-level ports of the
//! shell pipeline's idioms.

pub mod hash;
pub mod winpath;

pub use hash::{file_sha256_matches, sha256_bytes, sha256_file};
pub use winpath::win_path;

/// Re-exported from [`crate::checks::build`] so the fix and stage layers share the
/// one implementation: the `arm64e`-must-not-satisfy rule is a parity invariant
/// (PARITY.md § Doctor / checks, "`helper_is_arm64` currently shells out to `lipo`").
pub use crate::checks::build::{helper_is_arm64, lipo_archs_stdout};

use std::io::Read;
use std::path::Path;

use crate::contract::{contract, CONTRACT_FILES, CONTRACT_GEN_REL_PATH, HOST_MANIFEST_PLACEHOLDER};
use crate::paths::Paths;

/// `cmp -s "$1" "$2"`: true iff both files exist, are readable, and are
/// byte-identical. Any error (missing, permission, directory) returns `false`,
/// so it never reports "equal" for a file it could not read.
/// See tests::cmp_files_matches_cmp_s_semantics.
pub fn cmp_files(a: &Path, b: &Path) -> bool {
    fn open(p: &Path) -> Option<(std::fs::File, u64)> {
        let f = std::fs::File::open(p).ok()?;
        let len = f.metadata().ok()?.len();
        Some((f, len))
    }
    let (mut fa, la) = match open(a) {
        Some(v) => v,
        None => return false,
    };
    let (mut fb, lb) = match open(b) {
        Some(v) => v,
        None => return false,
    };
    if la != lb {
        return false;
    }
    let mut ba = vec![0u8; 64 * 1024];
    let mut bb = vec![0u8; 64 * 1024];
    loop {
        let na = match read_full(&mut fa, &mut ba) {
            Ok(n) => n,
            Err(_) => return false,
        };
        let nb = match read_full(&mut fb, &mut bb) {
            Ok(n) => n,
            Err(_) => return false,
        };
        if na != nb {
            return false;
        }
        if na == 0 {
            return true;
        }
        if ba[..na] != bb[..nb] {
            return false;
        }
    }
}

fn read_full(f: &mut std::fs::File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match f.read(&mut buf[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    Ok(filled)
}

/// Strip trailing newlines the way command substitution does (`$(<file)`,
/// `$(cat file)` — zsh drops *all* trailing `\n`, and nothing else).
pub fn strip_trailing_newlines(s: &str) -> &str {
    s.trim_end_matches('\n')
}

/// Best-effort Beat Saber version for `bs_dir`, reproducing lib.sh's
/// `bs_version()` quirks: the marker file wins even when empty; otherwise every
/// stamp on the first matching line of `globalgamemanagers`, newline-joined, else
/// `?`. Trailing newlines are stripped (doctor captures via `$(bs_version)`).
///
/// See tests::bs_version_falls_back_to_question_mark and
/// tests::version_stamp_scan_matches_grep.
pub fn bs_version(bs_dir: &Path) -> String {
    if let Ok(bytes) = std::fs::read(bs_dir.join("BeatSaberVersion.txt")) {
        let text = String::from_utf8_lossy(&bytes).into_owned();
        return strip_trailing_newlines(&text).to_string();
    }
    if let Ok(bytes) = std::fs::read(bs_dir.join("Beat Saber_Data/globalgamemanagers")) {
        if let Some(found) = first_matching_line_stamps(&bytes) {
            return found;
        }
    }
    "?".to_string()
}

/// All version stamps on the first line of `haystack` that contains one,
/// newline-joined (`grep -a -o -E -m1` output).
fn first_matching_line_stamps(haystack: &[u8]) -> Option<String> {
    for line in haystack.split(|&c| c == b'\n') {
        let mut out: Vec<String> = Vec::new();
        let mut i = 0usize;
        while i < line.len() {
            match version_stamp_at(line, i) {
                Some(end) => {
                    out.push(String::from_utf8_lossy(&line[i..end]).into_owned());
                    i = end; // grep -o resumes after a match (no overlaps)
                }
                None => i += 1,
            }
        }
        if !out.is_empty() {
            return Some(out.join("\n"));
        }
    }
    None
}

fn digit_run(b: &[u8], i: usize) -> usize {
    let mut j = i;
    while j < b.len() && b[j].is_ascii_digit() {
        j += 1;
    }
    j - i
}

/// Leftmost-longest ERE match of `[0-9]{1,2}\.[0-9]{1,3}\.[0-9]{1,3}_[0-9]{6,}`
/// anchored at `i`; returns the exclusive end index.
///
/// A digit run longer than the bound can never match, because the character
/// after the (necessarily shorter) capture would be a digit where the pattern
/// demands `.` or `_` — so "too long" collapses to a plain rejection.
fn version_stamp_at(b: &[u8], i: usize) -> Option<usize> {
    let n1 = digit_run(b, i);
    if n1 == 0 || n1 > 2 {
        return None;
    }
    let mut j = i + n1;
    if b.get(j) != Some(&b'.') {
        return None;
    }
    j += 1;

    let n2 = digit_run(b, j);
    if n2 == 0 || n2 > 3 {
        return None;
    }
    j += n2;
    if b.get(j) != Some(&b'.') {
        return None;
    }
    j += 1;

    let n3 = digit_run(b, j);
    if n3 == 0 || n3 > 3 {
        return None;
    }
    j += n3;
    if b.get(j) != Some(&b'_') {
        return None;
    }
    j += 1;

    let n4 = digit_run(b, j);
    if n4 < 6 {
        return None;
    }
    Some(j + n4)
}

/// Render the host OpenXR manifest for `dylib_path` in its *comparison* form:
/// the template with the placeholder replaced by the JSON-escaped path, minus the
/// template's trailing newline (install.sh reads with `$(<file)`, which strips it).
///
/// Use [`host_manifest_file_bytes`] for what lands on disk — one extra byte and the
/// two front-ends thrash each other with sudo prompts. See
/// sabrage-parity tests::artifact_goldens::render_host_manifest_matches_the_on_disk_template,
/// sabrage-parity tests::artifact_goldens::render_host_manifest_json_escapes_the_dylib_path.
pub fn render_host_manifest(dylib_path: &Path) -> String {
    strip_trailing_newlines(crate::contract::HOST_MANIFEST_TEMPLATE).replace(
        HOST_MANIFEST_PLACEHOLDER,
        &json_escape_string(&dylib_path.to_string_lossy()),
    )
}

/// Escape `s` for embedding in a JSON string literal, byte-for-byte the way
/// install.sh does: backslash first (so introduced escapes are not re-escaped),
/// then double quote, nothing else. The escaped path lands in the root-owned host
/// manifest (PARITY.md § Declared by the 2026-08-30 adversarial review (round 1 fixes),
/// "Control characters in the checkout path.").
///
/// Not a full JSON encoder: control characters stay unescaped, as on the zsh side,
/// so widening this must land on both sides in the same commit. See
/// tests::json_escape_string_is_install_shs_two_substitutions.
pub fn json_escape_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// The bytes written to `/usr/local/share/openxr/1/active_runtime.x86_64.json`:
/// [`render_host_manifest`] plus the single trailing newline `print -- "$WANT"`
/// appends.
pub fn host_manifest_file_bytes(dylib_path: &Path) -> String {
    let mut s = render_host_manifest(dylib_path);
    s.push('\n');
    s
}

/// install.sh's currency test, `[ -f "$f" ] && [ "$(cat "$f")" = "$WANT" ]`.
///
/// Trailing newlines on disk are irrelevant (command substitution eats them), so
/// a file written by either front-end reads as current to the other.
pub fn host_manifest_is_current(path: &Path, want: &str) -> bool {
    match std::fs::read_to_string(path) {
        Ok(text) => strip_trailing_newlines(&text) == want,
        Err(_) => false,
    }
}

/// The `oxrsys-runtime.toml` first-write template, byte-for-byte: exactly the
/// bytes setup writes, including the trailing newline and every comment (comments
/// are load-bearing — the pre-2026-08 runtime parser choked on a same-line `#`).
///
/// **Write-once**: never regenerate, never migrate. See
/// sabrage-parity tests::artifact_goldens::toml_template_matches_the_on_disk_contract_file.
pub fn toml_template() -> &'static str {
    crate::contract::RUNTIME_TOML_TEMPLATE
}

/// True when every `[dxmt] files` entry is present under `ext/dxmt-artifacts/` —
/// ALL of them, never a subset. A partial overlay black-windows the game with no
/// error of its own, so `install` refuses to half-apply it.
/// See tests::dxmt_helpers_need_every_file_and_a_current_marker.
pub fn dxmt_files_ok(paths: &Paths) -> bool {
    contract()
        .dxmt
        .files
        .iter()
        .all(|f| paths.dxmt_art.join(f).is_file())
}

/// True when the `.sha256` provenance marker matches the contract pin **and**
/// every `[dxmt] files` entry is present. Trailing newlines are irrelevant
/// (command-substitution semantics), so a marker written by either front-end
/// reads as current to the other. See [`contract_marker_bytes`] for the write
/// side and tests::dxmt_helpers_need_every_file_and_a_current_marker.
pub fn dxmt_ok(paths: &Paths) -> bool {
    let marker = std::fs::read_to_string(paths.dxmt_art.join(".sha256")).unwrap_or_default();
    strip_trailing_newlines(&marker) == contract().deps.dxmt_tgz_sha256 && dxmt_files_ok(paths)
}

/// The exact bytes of the `.sha256` provenance marker `setup` writes: the pin
/// plus **one** trailing newline.
///
/// Zero or two would still *read* as current (command substitution eats them) but
/// would make the two front-ends write different bytes for the same state; see
/// tests::marker_bytes_are_the_pin_plus_exactly_one_newline.
pub fn contract_marker_bytes(sha: &str) -> String {
    format!("{sha}\n")
}

/// The `meta.contract-sync` hash over contract bytes already in memory:
/// `cat <parts…> | shasum -a 256`, in the order given.
///
/// Same recipe as [`contract_hash`], so the compiled-in identity
/// ([`crate::contract::COMPILED_CONTRACT_SHA256`]) and the on-disk recompute
/// can differ only in *what* they hash, which is the skew they exist to expose.
pub fn contract_sha256_from(parts: &[&str]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for part in parts {
        h.update(part.as_bytes());
    }
    hex::encode(h.finalize())
}

/// The `meta.contract-sync` hash, recomputed from the contract files **on disk**
/// under `repo_root`, concatenated in `CONTRACT_FILES` order.
///
/// # Errors
/// Any contract file that cannot be opened or read.
///
/// Pinned by doctor.sh section 0 and the `# contract-sha256:` header of the
/// generated shell file. Runtime reads, not [`include_str!`]: this compares
/// the *checkout* against its own generated file, so a stale compiled-in copy
/// would defeat the tripwire. See tests::contract_hash_matches_the_generated_header.
pub fn contract_hash(repo_root: &Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for rel in CONTRACT_FILES {
        let mut f = std::fs::File::open(repo_root.join(rel))?;
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
        }
    }
    Ok(hex::encode(h.finalize()))
}

/// The hash recorded in `scripts/demo/contract.gen.sh`'s header, i.e.
/// `sed -n 's/^# contract-sha256: //p' … | head -1`. `None` when the file is
/// missing or carries no header.
pub fn contract_gen_recorded_hash(repo_root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(repo_root.join(CONTRACT_GEN_REL_PATH)).ok()?;
    text.lines()
        .find_map(|l| l.strip_prefix("# contract-sha256: "))
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
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
}
