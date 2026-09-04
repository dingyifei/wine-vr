use super::*;
use crate::checks::CheckOptions;
use crate::paths::Paths;
use std::fs;
use std::path::PathBuf;

fn scratch(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("sabrage-host-test-{}-{tag}", std::process::id()))
}

fn ctx_with(tmp: &Path, host_json: PathBuf, oxr_dylib: PathBuf) -> CheckCtx {
    let mut paths = Paths::new(tmp);
    paths.host_xr_json = host_json;
    paths.oxr_dylib = oxr_dylib;
    CheckCtx::new(paths, CheckOptions::new())
}

fn write_manifest(path: &Path, library_path: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path,
        format!(r#"{{"runtime":{{"name":"oxrsys","library_path":"{library_path}"}}}}"#),
    )
    .unwrap();
}

#[test]
fn missing_manifest_fails_with_the_install_remedy() {
    let tmp = scratch("missing");
    let host_json = tmp.join("host/active_runtime.x86_64.json");
    let ctx = ctx_with(&tmp, host_json.clone(), tmp.join("dylib"));
    let o = host_manifest(&ctx);
    assert_eq!(o.status, CheckStatus::Fail);
    assert_eq!(o.message, format!("{} missing", host_json.display()));
    assert_eq!(
        o.remedy.as_deref(),
        Some("./demo.sh install --bottle <name> (sudo writes it)")
    );
}

#[test]
fn invalid_manifest_bytes_fail_with_the_parse_remedy() {
    // Both rows reach the same None arm by different hops: bytes serde
    // cannot parse, and valid JSON with no "runtime" key.
    const CASES: &[(&str, &[u8])] = &[
        ("malformed json", b"{not json"),
        ("missing runtime key", b"{}"),
    ];
    for &(label, bytes) in CASES {
        let tmp = scratch(&label.replace(' ', "-"));
        let host_json = tmp.join("host/active_runtime.x86_64.json");
        fs::create_dir_all(host_json.parent().unwrap()).unwrap();
        fs::write(&host_json, bytes).unwrap();
        let ctx = ctx_with(&tmp, host_json.clone(), tmp.join("dylib"));
        let o = host_manifest(&ctx);
        assert_eq!(o.status, CheckStatus::Fail, "{label}");
        assert_eq!(
            o.message,
            format!(
                "cannot parse {} (broken python3 or malformed JSON)",
                host_json.display()
            ),
            "{label}"
        );
        assert_eq!(
            o.remedy.as_deref(),
            Some("check 'python3 -V' works (xcode-select --install), then inspect the file"),
            "{label}"
        );
        fs::remove_dir_all(&tmp).ok();
    }
}

#[test]
fn matching_and_present_dylib_passes() {
    let tmp = scratch("match");
    let host_json = tmp.join("host/active_runtime.x86_64.json");
    let dylib = tmp.join("ext/oxrsys/build-x64/runtime/liboxrsys-runtime.dylib");
    fs::create_dir_all(dylib.parent().unwrap()).unwrap();
    fs::write(&dylib, b"stub").unwrap();
    write_manifest(&host_json, &dylib.to_string_lossy());
    let ctx = ctx_with(&tmp, host_json, dylib.clone());
    let o = host_manifest(&ctx);
    assert_eq!(o.status, CheckStatus::Pass);
    assert_eq!(
        o.message,
        format!("host OpenXR registration -> {}", dylib.display())
    );
    assert!(o.remedy.is_none());
    fs::remove_dir_all(&tmp).ok();
}

#[test]
fn different_but_existing_dylib_warns() {
    let tmp = scratch("different");
    let host_json = tmp.join("host/active_runtime.x86_64.json");
    let actual = tmp.join("elsewhere/lib.dylib");
    let expected = tmp.join("ext/oxrsys/build-x64/runtime/liboxrsys-runtime.dylib");
    fs::create_dir_all(actual.parent().unwrap()).unwrap();
    fs::write(&actual, b"stub").unwrap();
    write_manifest(&host_json, &actual.to_string_lossy());
    let ctx = ctx_with(&tmp, host_json, expected.clone());
    let o = host_manifest(&ctx);
    assert_eq!(o.status, CheckStatus::Warn);
    assert_eq!(
        o.message,
        format!(
            "host registration points at {} (expected {})",
            actual.display(),
            expected.display()
        )
    );
    fs::remove_dir_all(&tmp).ok();
}

#[test]
fn pointing_at_a_missing_dylib_fails() {
    let tmp = scratch("dangling");
    let host_json = tmp.join("host/active_runtime.x86_64.json");
    let expected = tmp.join("ext/oxrsys/build-x64/runtime/liboxrsys-runtime.dylib");
    write_manifest(&host_json, "/nowhere/liboxrsys-runtime.dylib");
    let ctx = ctx_with(&tmp, host_json.clone(), expected);
    let o = host_manifest(&ctx);
    assert_eq!(o.status, CheckStatus::Fail);
    assert_eq!(o.message, "host registration points at a missing dylib");
    assert_eq!(
        o.remedy.as_deref(),
        Some(
            format!(
                "./demo.sh install --bottle <name> (sudo rewrites {})",
                host_json.display()
            )
            .as_str()
        )
    );
    fs::remove_dir_all(&tmp).ok();
}
