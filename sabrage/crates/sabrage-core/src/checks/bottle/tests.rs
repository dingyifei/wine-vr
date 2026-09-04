use super::*;
use crate::checks::CheckOptions;
use crate::paths::Paths;
use std::fs;
use std::path::PathBuf;

fn ctx_with(bottle: Option<Bottle>, bottle_requested: bool, bs_dir: PathBuf) -> CheckCtx {
    CheckCtx {
        paths: Paths::new("/nonexistent/repo"),
        bottle,
        bs_dir,
        bottle_requested,
        opts: CheckOptions {
            bottle_name: if bottle_requested {
                Some("TestBottle".into())
            } else {
                None
            },
            ..CheckOptions::new()
        },
    }
}

#[test]
fn bottle_named_fail_without_a_name_skips_the_rest_of_the_section() {
    let ctx = ctx_with(None, false, PathBuf::from("/bs"));
    let out = bottle_named(&ctx);
    assert_eq!(out.status, CheckStatus::Fail);
    assert_eq!(out.message, "no bottle name given (--bottle/WINEVR_BOTTLE)");
    assert!(out
        .remedy
        .as_deref()
        .unwrap()
        .starts_with("create a win11_64 bottle in CrossOver; existing: "));

    assert_eq!(bottle_exists(&ctx).status, CheckStatus::Skipped);
    assert_eq!(bottle_template(&ctx).status, CheckStatus::Skipped);
    assert_eq!(bottle_gfx_dxmt(&ctx).status, CheckStatus::Skipped);
    assert_eq!(bottle_zdrive(&ctx).status, CheckStatus::Skipped);
}

#[test]
fn bottle_exists_fail_reports_prefix_and_skips_downstream_rows() {
    let ctx = ctx_with(None, true, PathBuf::from("/bs"));
    let out = bottle_exists(&ctx);
    let want_prefix = Bottle::unvalidated("TestBottle").prefix;
    assert_eq!(out.status, CheckStatus::Fail);
    assert_eq!(
        out.message,
        format!("bottle 'TestBottle' not found at {}", want_prefix.display())
    );
    assert_eq!(
        out.remedy.as_deref(),
        Some("create it in the CrossOver UI (win11_64)")
    );
    assert_eq!(bottle_template(&ctx).status, CheckStatus::Skipped);
    assert_eq!(bottle_gfx_dxmt(&ctx).status, CheckStatus::Skipped);
    assert_eq!(bottle_zdrive(&ctx).status, CheckStatus::Skipped);
}

fn temp_bottle(label: &str) -> Bottle {
    let dir = std::env::temp_dir().join(format!(
        "sabrage-bottle-test-{label}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp bottle dir");
    Bottle {
        name: "TestBottle".to_string(),
        sys32: dir.join("drive_c/windows/system32"),
        prefix: dir,
    }
}

#[test]
fn bottle_template_and_gfx_dxmt_pass_on_a_matching_conf() {
    let b = temp_bottle("conf-ok");
    fs::write(
        b.conf_path(),
        "\"Template\" = \"win11_64\"\n\"CX_GRAPHICS_BACKEND\" = \"dxmt\"\n",
    )
    .unwrap();
    let ctx = ctx_with(Some(b.clone()), true, b.prefix.join("drive_c/bs"));
    assert_eq!(bottle_template(&ctx).status, CheckStatus::Pass);
    assert_eq!(bottle_gfx_dxmt(&ctx).status, CheckStatus::Pass);
    fs::remove_dir_all(&b.prefix).ok();
}

#[test]
fn bottle_template_warns_and_gfx_dxmt_fails_on_mismatch() {
    let b = temp_bottle("conf-bad");
    fs::write(
        b.conf_path(),
        "\"Template\" = \"win10_64\"\n\"CX_GRAPHICS_BACKEND\" = \"auto\"\n",
    )
    .unwrap();
    let ctx = ctx_with(Some(b.clone()), true, b.prefix.join("drive_c/bs"));

    let t = bottle_template(&ctx);
    assert_eq!(t.status, CheckStatus::Warn);
    assert_eq!(
        t.message,
        "bottle template is not win11_64 (\"Template\" = \"win10_64\") — only win11_64 is verified"
    );
    assert!(t.remedy.is_none());

    let g = bottle_gfx_dxmt(&ctx);
    assert_eq!(g.status, CheckStatus::Fail);
    assert_eq!(
        g.remedy.as_deref(),
        Some(
            "./demo.sh run auto-fixes this, or set Graphics Backend to DXMT in the \
                 CrossOver bottle settings"
        )
    );
    fs::remove_dir_all(&b.prefix).ok();
}

#[test]
fn bottle_template_warns_with_an_empty_parenthetical_when_the_key_is_absent() {
    let b = temp_bottle("conf-no-template");
    fs::write(b.conf_path(), "\"CX_GRAPHICS_BACKEND\" = \"dxmt\"\n").unwrap();
    let ctx = ctx_with(Some(b.clone()), true, b.prefix.join("drive_c/bs"));
    let t = bottle_template(&ctx);
    assert_eq!(
        t.message,
        "bottle template is not win11_64 () — only win11_64 is verified"
    );
    fs::remove_dir_all(&b.prefix).ok();
}

#[test]
fn bottle_zdrive_gates_on_outside_drive_c_and_z_drive_presence() {
    let b = temp_bottle("zdrive");
    fs::create_dir_all(b.prefix.join("drive_c")).unwrap();

    // Inside drive_c: skipped regardless of z: presence.
    let ctx_inside = ctx_with(Some(b.clone()), true, b.prefix.join("drive_c/bs"));
    assert_eq!(bottle_zdrive(&ctx_inside).status, CheckStatus::Skipped);

    // Outside drive_c, no z: -> fail.
    let ctx_outside = ctx_with(Some(b.clone()), true, PathBuf::from("/elsewhere/bs"));
    let out = bottle_zdrive(&ctx_outside);
    assert_eq!(out.status, CheckStatus::Fail);
    assert_eq!(
        out.remedy.as_deref(),
        Some("add dosdevices/z: -> / or move the install under drive_c")
    );

    // Outside drive_c, z: present -> pass.
    fs::create_dir_all(b.prefix.join("dosdevices")).unwrap();
    fs::write(b.prefix.join("dosdevices/z:"), b"").unwrap();
    let out = bottle_zdrive(&ctx_outside);
    assert_eq!(
        out.status,
        CheckStatus::Pass,
        "z: drive now present, outside drive_c"
    );
    assert_eq!(
        out.message,
        "bottle z: drive maps / (Beat Saber lives outside drive_c)"
    );

    fs::remove_dir_all(&b.prefix).ok();
}
