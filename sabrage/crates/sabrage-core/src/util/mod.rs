//! Primitives shared by checks, fixes, and stages. Every function here is a
//! deliberate port of one shell idiom, and each documents the idiom it mirrors —
//! that is what keeps the two front-ends from drifting.

pub mod hash;
pub mod winpath;

pub use hash::{file_sha256_matches, sha256_bytes, sha256_file};
pub use winpath::win_path;

use std::io::Read;
use std::path::Path;

use crate::contract::{CONTRACT_FILES, CONTRACT_GEN_REL_PATH, HOST_MANIFEST_PLACEHOLDER};

/// `cmp -s "$1" "$2"`: true iff both files exist, are readable, and are
/// byte-identical. Any error (missing, permission, directory) is `false`,
/// exactly like the shell's non-zero exit.
///
/// Used everywhere the pipeline asks "is this overlay current?" — install's
/// `install_if_changed` and doctor sections 10/11 both hinge on it, so it must
/// never report "equal" for a file it could not read.
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

// ── Beat Saber version ────────────────────────────────────────────────────────

/// lib.sh's `bs_version()`: best-effort Beat Saber version for `bs_dir`.
///
/// ```zsh
/// bs_version() {
///   cat "$BS_DIR/BeatSaberVersion.txt" 2>/dev/null && return
///   grep -a -o -E -m1 '[0-9]{1,2}\.[0-9]{1,3}\.[0-9]{1,3}_[0-9]{6,}' \
///     "$BS_DIR/Beat Saber_Data/globalgamemanagers" 2>/dev/null || echo '?'
/// }
/// ```
///
/// Reproduced quirks:
/// * the marker file wins even when it is **empty** (`cat` succeeds, so the
///   shell returns an empty string rather than falling through to the scan);
/// * `grep -o -m1` emits *every* match on the *first matching line*, so a line
///   with two stamps yields both, newline-joined;
/// * the caller-visible value has trailing newlines stripped, because doctor
///   captures it with `$(bs_version)`.
///
/// The scan is hand-rolled rather than `regex`-backed on purpose: sabrage-core
/// stays dependency-light, and the pattern is a fixed digit/separator shape.
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

// ── byte-shared artifacts ─────────────────────────────────────────────────────

/// Render the host OpenXR manifest for `dylib_path`.
///
/// install.sh:
/// ```zsh
/// WANT="${$(<"$ROOT/contract/active_runtime.x86_64.json.template")//@OXR_DYLIB@/$OXR_DYLIB}"
/// ```
/// `$(<file)` strips the template's trailing newline, so the returned string has
/// **no** trailing newline — it is the *comparison* form, the exact bytes
/// install.sh's `[ "$(cat "$HOST_XR_JSON")" = "$WANT" ]` test compares against.
/// Use [`host_manifest_file_bytes`] for what actually lands on disk.
///
/// This is the single most drift-sensitive artifact in the pipeline: one extra
/// byte and the two front-ends thrash each other with sudo prompts.
pub fn render_host_manifest(dylib_path: &Path) -> String {
    strip_trailing_newlines(crate::contract::HOST_MANIFEST_TEMPLATE)
        .replace(HOST_MANIFEST_PLACEHOLDER, &dylib_path.to_string_lossy())
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

/// The `oxrsys-runtime.toml` first-write template, byte-for-byte.
///
/// setup.sh writes it with `cat template > "$TOML"`, so these are exactly the
/// bytes that must land on disk — including the trailing newline and every
/// comment (comments are load-bearing; the pre-2026-08 runtime parser choked on
/// same-line `#`). **Write-once**: never regenerate, never migrate.
pub fn toml_template() -> &'static str {
    crate::contract::RUNTIME_TOML_TEMPLATE
}

// ── contract sync ─────────────────────────────────────────────────────────────

/// The `meta.contract-sync` hash, recomputed from the contract files **on disk**
/// under `repo_root`.
///
/// Recipe pinned by doctor.sh section 0 and by the `# contract-sha256:` header of
/// the generated shell file:
///
/// ```sh
/// cat contract/pipeline.toml \
///     contract/oxrsys-runtime.toml.template \
///     contract/active_runtime.x86_64.json.template | shasum -a 256
/// ```
///
/// Runtime reads, not [`include_str!`], on purpose: this compares the *checkout*
/// against its own generated file, so a stale compiled-in copy would defeat the
/// entire point of the tripwire.
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

    /// The repo root, four levels above this crate's manifest.
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
    fn host_manifest_is_template_minus_trailing_newline() {
        let want = render_host_manifest(Path::new("/repo/ext/oxrsys/build-x64/runtime/lib.dylib"));
        assert!(!want.ends_with('\n'));
        assert!(!want.contains(HOST_MANIFEST_PLACEHOLDER));
        assert!(want.contains("/repo/ext/oxrsys/build-x64/runtime/lib.dylib"));
        assert_eq!(
            host_manifest_file_bytes(Path::new("/repo/ext/oxrsys/build-x64/runtime/lib.dylib")),
            format!("{want}\n")
        );
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
