use super::*;

/// The generated file minus its `# contract-sha256:` header line.
///
/// The header changes for *any* contract edit, so comparing whole files
/// cannot tell "the shell actually got the new value" from "only the
/// tripwire hash moved". Every assertion about a field reaching the shell
/// has to look at the body.
fn body(generated: &str) -> String {
    generated
        .lines()
        .filter(|l| !l.starts_with("# contract-sha256: "))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every contract scalar the zsh side consumes must be *emitted*, not
/// hard-coded in lib.sh/setup.sh: mutate it in `pipeline.toml` and a body
/// line has to change. A field that only moves the header hash never
/// reaches the shell, yet `--regen`, `--check` and `meta.contract-sync`
/// all report "in sync" while the two front-ends use different values.
#[test]
fn every_shell_consumed_contract_field_changes_a_body_line() {
    let base = generate();
    for (field, from, to) in [
        (
            "deps.url",
            r#"url = "https://github.com/dingyifei/wine-vr/releases/download/deps-v1""#,
            r#"url = "https://github.com/dingyifei/wine-vr/releases/download/deps-v9""#,
        ),
        (
            "deps.gbe_dll_asset",
            r#"gbe_dll_asset = "gbe-steam_api64-regular-x64.dll""#,
            r#"gbe_dll_asset = "gbe-steam_api64-other-x64.dll""#,
        ),
        (
            "deps.gbe_dll_sha256",
            r#"gbe_dll_sha256 = "cc5a2c9cb93fdbde7dadb825138ab7f694e3f8c310cdd675f733eaa784cbcc3e""#,
            r#"gbe_dll_sha256 = "0000000000000000000000000000000000000000000000000000000000000000""#,
        ),
        (
            "deps.dxmt_tgz_asset",
            r#"dxmt_tgz_asset = "dxmt-artifacts-monofunc.tar.gz""#,
            r#"dxmt_tgz_asset = "dxmt-artifacts-other.tar.gz""#,
        ),
        (
            "deps.dxmt_tgz_sha256",
            r#"dxmt_tgz_sha256 = "487e57e86e9866c922f8d8e42a50cb0818697b927739b6741fae8f4447e2df96""#,
            r#"dxmt_tgz_sha256 = "1111111111111111111111111111111111111111111111111111111111111111""#,
        ),
        ("game.appid", "appid = 620980", "appid = 620999"),
        ("game.depot", "depot = 620981", "depot = 620999"),
        (
            "game.manifest",
            r#"manifest = "6291266771922375922""#,
            r#"manifest = "1234567890123456789""#,
        ),
        (
            "game.bs_dir_leaf",
            r#"bs_dir_leaf = "Beat Saber 1294""#,
            r#"bs_dir_leaf = "Beat Saber 1295""#,
        ),
        (
            "paths.host_xr_json",
            r#"host_xr_json = "/usr/local/share/openxr/1/active_runtime.x86_64.json""#,
            r#"host_xr_json = "/opt/openxr/1/active_runtime.x86_64.json""#,
        ),
        (
            "ports.stream",
            "stream = [9943, 9944]",
            "stream = [9955, 9956]",
        ),
        (
            "ports.legacy_reverse",
            "legacy_reverse = [9944, 9945, 9946, 9948]",
            "legacy_reverse = [9944, 9945]",
        ),
        (
            "dxmt.files",
            r#""x86_64-windows/dxgi.dll","#,
            r#""x86_64-windows/dxgi2.dll","#,
        ),
    ] {
        let mutated = PIPELINE_TOML.replace(from, to);
        assert_ne!(
            mutated, PIPELINE_TOML,
            "{field}: the test's own mutation no longer matches pipeline.toml"
        );
        let generated = generate_from(&mutated, RUNTIME_TOML_TEMPLATE, HOST_MANIFEST_TEMPLATE)
            .expect("mutated contract still parses");
        assert_ne!(
            body(&generated),
            body(&base),
            "{field} changed but no line of contract.gen.sh's body did — the shell \
                 hard-codes this value instead of sourcing it"
        );
    }
}

#[test]
fn zsh_encoders_are_minimal_for_ordinary_contract_values() {
    // Everything the real contract holds today keeps its historical
    // spelling — that is what makes the committed file byte-identical.
    assert_eq!(
        zsh_scalar("https://github.com/dingyifei/wine-vr/releases/download/deps-v1"),
        "\"https://github.com/dingyifei/wine-vr/releases/download/deps-v1\""
    );
    assert_eq!(zsh_scalar("Beat Saber 1294"), "\"Beat Saber 1294\"");
    assert_eq!(zsh_word("6291266771922375922"), "6291266771922375922");
    assert_eq!(
        zsh_word("x86_64-windows/d3d10core.dll"),
        "x86_64-windows/d3d10core.dll"
    );

    // Anything that would be *shell code* becomes a single-quoted literal.
    assert_eq!(zsh_scalar("a$(id)b"), "'a$(id)b'");
    assert_eq!(zsh_scalar("a`id`b"), "'a`id`b'");
    assert_eq!(zsh_scalar(r"a\b"), r"'a\b'");
    assert_eq!(zsh_scalar("a\"b"), "'a\"b'");
    assert_eq!(zsh_scalar("a\nb"), "'a\nb'");
    assert_eq!(zsh_word("two words"), "'two words'");
    assert_eq!(zsh_word("glob*.dll"), "'glob*.dll'");
    assert_eq!(zsh_word("~/x"), "'~/x'");
    assert_eq!(zsh_word("=ls"), "'=ls'");
    assert_eq!(zsh_word(""), "''");
    // `'` is literal inside double quotes, so the scalar form keeps it;
    // the word form single-quotes, closing and reopening around it.
    assert_eq!(zsh_scalar("it's"), "\"it's\"");
    assert_eq!(zsh_word("it's"), r#"'it'\''s'"#);
    assert_eq!(zsh_word("a'b$(id)"), r#"'a'\''b$(id)'"#);
}

/// TOML basic-string escaping, so the fixture below can hold `"` and `\`.
fn toml_esc(s: &str) -> String {
    s.replace('\\', r"\\").replace('"', "\\\"")
}

/// A `pipeline.toml` whose scalars are valid TOML but hostile shell.
fn hostile_pipeline(url: &str) -> (String, [(&'static str, String); 4]) {
    let leaf = r#"Beat 'Saber' "1294" \ $USER"#.to_string();
    let manifest = "6291266771922375922; touch /tmp/sabrage-pwned".to_string();
    let file0 = "x86_64-windows/two words/*.dll".to_string();
    let mutated = PIPELINE_TOML
        .replace(
            r#"url = "https://github.com/dingyifei/wine-vr/releases/download/deps-v1""#,
            &format!("url = \"{}\"", toml_esc(url)),
        )
        .replace(
            r#"bs_dir_leaf = "Beat Saber 1294""#,
            &format!("bs_dir_leaf = \"{}\"", toml_esc(&leaf)),
        )
        .replace(
            r#"manifest = "6291266771922375922""#,
            &format!("manifest = \"{}\"", toml_esc(&manifest)),
        )
        .replace(
            r#""x86_64-windows/d3d10core.dll","#,
            &format!("\"{}\",", toml_esc(&file0)),
        );
    assert_ne!(mutated, PIPELINE_TOML, "fixture mutations still apply");
    (
        mutated,
        [
            ("DEPS_URL", url.to_string()),
            ("BS_DIR_LEAF", leaf),
            ("BS_MANIFEST", manifest),
            ("DXMT_FILES[1]", file0),
        ],
    )
}

/// contract.gen.sh is `source`d by lib.sh, so every emitted value is shell
/// code unless it is quoted. A contract value containing `$(…)`, a
/// backtick, whitespace or a glob must survive as its literal self.
#[test]
fn hostile_contract_values_are_emitted_as_zsh_literals() {
    let (pipeline, expected) = hostile_pipeline("https://ex.com/$(id)/`id`/${USER}/v1");
    let generated = generate_from(&pipeline, RUNTIME_TOML_TEMPLATE, HOST_MANIFEST_TEMPLATE)
        .expect("hostile contract still parses");

    for (var, value) in &expected[..3] {
        let want = if *var == "BS_MANIFEST" {
            format!("{var}={}\n", zsh_word(value))
        } else {
            format!("{var}={}\n", zsh_scalar(value))
        };
        assert!(
            generated.contains(&want),
            "{var} was not emitted as a literal:\n{generated}"
        );
    }
    // The array element with a space and a glob stays exactly one word.
    assert!(
        generated.contains("DXMT_FILES=('x86_64-windows/two words/*.dll' "),
        "hostile array element was not quoted per element:\n{generated}"
    );
    // Independently of the encoders: a hostile scalar must not land inside
    // double quotes (where `$`/backtick still expand) or bare.
    for var in ["DEPS_URL", "BS_DIR_LEAF", "BS_MANIFEST"] {
        let line = generated
            .lines()
            .find(|l| l.starts_with(&format!("{var}=")))
            .unwrap_or_else(|| panic!("{var} not emitted:\n{generated}"));
        assert!(
            line[var.len() + 1..].starts_with('\''),
            "hostile value is still shell-expandable: {line}"
        );
    }
}

/// The same fixture, proven by the only authority that matters: zsh.
///
/// Skipped where zsh is absent (the tier-1 CI runner is ubuntu); the
/// structural test above always runs.
#[test]
fn zsh_sources_hostile_contract_values_verbatim() {
    use std::process::Command;
    if Command::new("zsh")
        .arg("-c")
        .arg("exit 0")
        .status()
        .is_err()
    {
        eprintln!("skipping: no zsh on PATH");
        return;
    }

    let dir = std::env::temp_dir().join(format!(
        "sabrage-contract-gen-{}-{:?}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let canary = dir.join("pwned");
    let url = format!(
        "https://ex.com/$(touch {})/`id`/${{USER}}/v1",
        canary.display()
    );
    let (pipeline, expected) = hostile_pipeline(&url);
    let generated = generate_from(&pipeline, RUNTIME_TOML_TEMPLATE, HOST_MANIFEST_TEMPLATE)
        .expect("hostile contract still parses");
    let gen_path = dir.join("contract.gen.sh");
    std::fs::write(&gen_path, &generated).expect("write generated file");

    let out = Command::new("zsh")
        .arg("-c")
        .arg(format!(
            "source {}; print -rl -- \"$DEPS_URL\" \"$BS_DIR_LEAF\" \"$BS_MANIFEST\" \
                 ${{#DXMT_FILES[@]}} \"${{DXMT_FILES[1]}}\"",
            gen_path.display()
        ))
        .output()
        .expect("zsh runs");
    assert!(
        out.status.success(),
        "sourcing the generated file failed: {}\n{generated}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    let got: Vec<&str> = stdout.lines().collect();
    assert_eq!(got.len(), 5, "unexpected zsh output: {stdout:?}");
    assert_eq!(got[0], expected[0].1, "DEPS_URL");
    assert_eq!(got[1], expected[1].1, "BS_DIR_LEAF");
    assert_eq!(got[2], expected[2].1, "BS_MANIFEST");
    assert_eq!(
        got[3], "5",
        "DXMT_FILES lost/gained elements to word splitting"
    );
    assert_eq!(got[4], expected[3].1, "DXMT_FILES[1]");
    assert!(
        !canary.exists(),
        "command substitution in a contract value executed at source time"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
