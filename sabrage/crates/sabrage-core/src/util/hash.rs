//! sha256 helpers, in the two shapes the pipeline actually uses:
//! `shasum -a 256 <file> | awk '{print $1}'` and lib.sh's `sha256_ok`.

use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};

/// Lowercase hex sha256 of `bytes`.
pub fn sha256_bytes(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

/// Lowercase hex sha256 of a file, streamed (dxmt tarballs and dlls are large).
///
/// Equivalent to `shasum -a 256 "$1" | awk '{print $1}'`.
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

/// lib.sh's `sha256_ok file expected-hash`: false when the file is missing or
/// unreadable, true only on an exact digest match.
///
/// The comparison is ASCII-case-insensitive; every pin in the contract is
/// lowercase, but a hand-pasted uppercase digest should not read as tampering.
pub fn file_sha256_matches(path: &Path, expected_hex: &str) -> bool {
    match sha256_file(path) {
        Ok(got) => got.eq_ignore_ascii_case(expected_hex),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vector() {
        // sha256("") — the standard empty-input digest.
        assert_eq!(
            sha256_bytes(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn missing_file_never_matches() {
        assert!(!file_sha256_matches(
            Path::new("/nonexistent/sabrage/probe"),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        ));
    }
}
