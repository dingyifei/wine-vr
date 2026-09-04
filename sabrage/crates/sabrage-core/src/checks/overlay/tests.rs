use super::*;
use crate::checks::{CheckOptions, CheckStatus};
use crate::paths::Paths;
use std::fs;

fn scratch(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("sabrage-overlay-test-{}-{tag}", std::process::id()))
}

#[test]
fn without_crossover_all_four_are_skipped() {
    // Force cx/cx_app to None explicitly rather than trusting Paths::new()'s
    // machine probe — this test must hold even on a dev machine that has a
    // real CrossOver.app installed under ~/Applications or /Applications.
    let mut paths = Paths::new("/nonexistent/sabrage-overlay-probe");
    paths.cx_app = None;
    paths.cx = None;
    paths.wine = None;
    paths.wineserver = None;
    let ctx = CheckCtx::new(paths, CheckOptions::new());
    for eval in [
        dxmt_d3d11 as Evaluator,
        dxmt_winemetal as Evaluator,
        woxr_dll as Evaluator,
        woxr_so as Evaluator,
    ] {
        let o = eval(&ctx);
        assert_eq!(o.status, CheckStatus::Skipped, "{}", o.slug);
    }
}

/// Builds a `Paths` whose `cx_app` points at a fake CrossOver.app under
/// `tmp`, without depending on the real machine having CrossOver
/// installed.
fn ctx_with_fake_crossover(tmp: &Path) -> CheckCtx {
    let cx_app = tmp.join("CrossOver.app");
    fs::create_dir_all(cx_app.join("Contents/SharedSupport/CrossOver/lib/dxmt")).unwrap();
    fs::create_dir_all(cx_app.join("Contents/SharedSupport/CrossOver/lib/wine")).unwrap();
    // Paths::new() only probes ~/Applications and /Applications, so build
    // the struct by hand for the CrossOver-present branch.
    let mut paths = Paths::new(tmp);
    let cx = cx_app.join("Contents/SharedSupport/CrossOver");
    paths.wine = Some(cx.join("bin/wine"));
    paths.wineserver = Some(cx.join("bin/wineserver"));
    paths.cx_app = Some(cx_app);
    paths.cx = Some(cx);
    CheckCtx::new(paths, CheckOptions::new())
}

#[test]
fn matching_overlay_pair_passes_with_basename() {
    let tmp = scratch("match");
    let ctx = ctx_with_fake_crossover(&tmp);

    let src = &ctx.paths.dxmt_art;
    fs::create_dir_all(src.join("x86_64-windows")).unwrap();
    fs::write(src.join("x86_64-windows/d3d11.dll"), b"same-bytes").unwrap();
    let dst = ctx.paths.cx_dxmt("x86_64-windows/d3d11.dll").unwrap();
    fs::create_dir_all(dst.parent().unwrap()).unwrap();
    fs::write(&dst, b"same-bytes").unwrap();

    let o = dxmt_d3d11(&ctx);
    assert_eq!(o.status, CheckStatus::Pass);
    assert_eq!(o.message, "global overlay current: d3d11.dll");
    fs::remove_dir_all(&tmp).ok();
}

#[test]
fn stale_overlay_fails_with_install_remedy() {
    let tmp = scratch("stale");
    let opts = CheckOptions {
        bottle_name: Some("Steam".to_string()),
        ..CheckOptions::new()
    };
    let mut ctx = ctx_with_fake_crossover(&tmp);
    ctx.opts = opts;
    // src present, dst absent -> cmp -s fails.
    fs::create_dir_all(ctx.paths.dxmt_art.join("x86_64-unix")).unwrap();
    fs::write(
        ctx.paths.dxmt_art.join("x86_64-unix/winemetal.so"),
        b"content",
    )
    .unwrap();

    let o = dxmt_winemetal(&ctx);
    assert_eq!(o.status, CheckStatus::Fail);
    assert_eq!(o.message, "global overlay stale/missing: winemetal.so");
    assert_eq!(
        o.remedy.as_deref(),
        Some("./demo.sh install --bottle Steam")
    );
    fs::remove_dir_all(&tmp).ok();
}

#[test]
fn stale_overlay_remedy_uses_the_name_placeholder_without_a_bottle() {
    let tmp = scratch("placeholder");
    let ctx = ctx_with_fake_crossover(&tmp);
    let dst = ctx
        .paths
        .cx_wine_lib("x86_64-windows/wineopenxr.dll")
        .unwrap();
    fs::create_dir_all(dst.parent().unwrap()).unwrap();
    // No src file at all: still a Fail (missing), never a Skip.
    let o = woxr_dll(&ctx);
    assert_eq!(
        o.remedy.as_deref(),
        Some("./demo.sh install --bottle <name>")
    );
    fs::remove_dir_all(&tmp).ok();
}
