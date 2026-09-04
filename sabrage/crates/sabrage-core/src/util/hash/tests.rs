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
