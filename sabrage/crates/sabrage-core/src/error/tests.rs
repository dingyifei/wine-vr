use super::*;

#[test]
fn already_reported_covers_the_variants_that_emit_their_own_row() {
    for e in [
        SabrageError::fatal("no bottle", "create it"),
        SabrageError::TccDenied {
            path: PathBuf::from("/Applications/CrossOver.app"),
        },
        SabrageError::AdminDeclined,
        SabrageError::Cancelled,
    ] {
        assert!(e.already_reported(), "{e:?}");
    }
    for e in [
        SabrageError::InvalidInput("--nope".into()),
        SabrageError::io(
            "/x",
            std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        ),
        SabrageError::ChildFailed {
            argv0: "cmake".into(),
            status: 2,
            tail: Vec::new(),
        },
        SabrageError::Download {
            url: "https://h/x".into(),
            detail: None,
        },
        SabrageError::HashMismatch {
            label: "DXMT".into(),
            got: "abc".into(),
        },
    ] {
        assert!(!e.already_reported(), "{e:?}");
    }
}

/// `Display` for `Download` and `HashMismatch` is byte-identical to
/// lib.sh fetch_pinned's die text (report row B0712).
#[test]
fn display_matches_lib_sh_die_text() {
    let cases: &[(&str, SabrageError, &str)] = &[
        (
            "Download",
            SabrageError::Download {
                url: "https://example.invalid/x.dylib".into(),
                detail: None,
            },
            "download failed: https://example.invalid/x.dylib",
        ),
        (
            "HashMismatch",
            SabrageError::HashMismatch {
                label: "dxmt".into(),
                got: "deadbeef".into(),
            },
            "sha256 mismatch for dxmt (got deadbeef)",
        ),
    ];
    for (tag, err, expected) in cases {
        assert_eq!(err.to_string(), *expected, "{tag}");
    }
}
