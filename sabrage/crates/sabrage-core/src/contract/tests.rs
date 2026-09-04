use super::*;

#[test]
fn contract_parses_and_include_path_resolves() {
    // The include path is proven by compiling; these assertions pin the
    // values the rest of the crate hard-codes.
    let c = contract();
    assert_eq!(c.game.appid, 620980);
    assert_eq!(c.game.depot, 620981);
    assert_eq!(c.game.manifest, "6291266771922375922");
    assert_eq!(c.game.bs_dir_leaf, "Beat Saber 1294");
    assert_eq!(
        c.paths.host_xr_json,
        "/usr/local/share/openxr/1/active_runtime.x86_64.json"
    );
    assert_eq!(c.ports.stream, vec![9943, 9944]);
    assert_eq!(c.ports.dashboard_addr, "127.0.0.1:8082");
    assert_eq!(c.dxmt.files.len(), 5);
    assert!(!c.checks.is_empty());
}

#[test]
fn meta_contract_sync_is_the_first_compiled_check() {
    let c = contract();
    assert_eq!(c.checks[0].slug, "meta.contract-sync");
}

#[test]
fn gates_round_trip() {
    let c = contract();
    let helper = c.check("build.helper-arm64").expect("slug present");
    assert_eq!(helper.shell_gate, Gate::Autofix);
    assert_eq!(helper.native_gate, Gate::Autofix);
    assert_eq!(helper.fix.as_deref(), Some("fix.restage-helper"));
    assert!(!helper.volatile);

    let legacy = c.check("cfg.protocol.legacy-oxrsys").expect("slug present");
    assert_eq!(legacy.shell_gate, Gate::Warn);
    assert_eq!(legacy.native_gate, Gate::Block);

    let ports = c.check("net.ports").expect("slug present");
    assert!(ports.volatile);
    assert_eq!(ports.native_gate, Gate::None);
    assert!(!ports.native_gate.is_gating());
}

#[test]
fn depot_command_matches_lib_sh() {
    let c = contract();
    assert_eq!(
        c.depot_command(Path::new("/tmp/Beat Saber 1294")),
        "DepotDownloader -app 620980 -depot 620981 -manifest 6291266771922375922 \
             -username <steam-user> -dir \"/tmp/Beat Saber 1294\""
    );
}

/// The repo root — the checkout this binary was compiled from.
fn repo_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("repo root resolves")
}

#[test]
fn compiled_contract_sha256_matches_the_checkout_it_was_built_from() {
    let on_disk = crate::util::contract_hash(&repo_root()).expect("contract files readable");
    assert_eq!(*COMPILED_CONTRACT_SHA256, on_disk);
    assert_eq!(COMPILED_CONTRACT_SHA256.len(), 64);
}

#[test]
fn templates_are_the_bytes_the_shell_writes() {
    assert!(RUNTIME_TOML_TEMPLATE.contains("protocol = \"alvr\""));
    assert!(HOST_MANIFEST_TEMPLATE.contains(HOST_MANIFEST_PLACEHOLDER));
    assert!(HOST_MANIFEST_TEMPLATE.ends_with('\n'));
}
