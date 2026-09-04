use super::*;
use crate::checks::CheckStatus;
use crate::contract::{CONTRACT_FILES, CONTRACT_GEN_REL_PATH};
use crate::events::StageEvent;
use crate::paths::{Bottle, Paths};
use crate::stages::{StageCtx, StageOptions};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use tokio_util::sync::CancellationToken;

#[test]
fn the_slug_list_is_unique_gating_only_and_includes_the_run_only_slugs() {
    let slugs = preflight_slugs();
    assert!(!slugs.is_empty());

    // No duplicates, and nothing gated `none` sneaks in.
    let unique: std::collections::BTreeSet<_> = slugs.iter().collect();
    assert_eq!(unique.len(), slugs.len());
    for slug in &slugs {
        let spec = contract().check(slug).expect("slug is in the contract");
        assert_ne!(spec.native_gate, Gate::None, "{slug} is doctor-only");
    }

    // The run-only slugs exist only here — they have no doctor row at all.
    assert!(slugs.contains(&"run.wine-exec"));
    assert!(slugs.contains(&"run.bridge-built"));
    assert!(slugs.contains(&"run.wired-adb"));
}

/// Every `autofix`-gated slug must have an arm in [`apply_fix`], and every
/// `block`-gated slug an entry in [`block_die`]'s table. The `_ =>` arms
/// make both total at compile time, so this asserts the *set* instead.
#[test]
fn every_gated_slug_is_accounted_for() {
    for slug in preflight_slugs() {
        let spec = contract().check(slug).unwrap();
        match spec.native_gate {
            Gate::Autofix => assert!(
                slug == "bottle.gfx-dxmt" || HELPER_SLUGS.contains(&slug),
                "{slug} is gated autofix but apply_fix has no arm"
            ),
            Gate::Warn => assert_eq!(
                slug, "game.version",
                "a second warn-gated slug needs its run.sh text in `gate`"
            ),
            Gate::Block | Gate::None => {}
        }
    }
}

/// The same file, read by this module and by the Settings view, must name
/// the same backend — the bug was preflight validating `[streaming]`'s
/// `alvr` while the runtime obeyed a later `oxrsys`.
#[test]
fn the_preflight_facts_agree_with_the_settings_view_on_a_shadowed_key() {
    let dir = scratch("shadowed-agree");
    let toml = dir.join("oxrsys-runtime.toml");
    std::fs::write(
            &toml,
            b"[streaming]\nprotocol = \"alvr\"\nencoder_process = \"inproc\"\n\n              [tweaks]\nprotocol = \"oxrsys\"\nencoder_process = \"native\"\n",
        )
        .unwrap();

    let facts = read_toml_facts(&toml);
    let view = crate::config::runtime_toml::read(&toml);
    assert_eq!(
        facts.protocol,
        view.values.protocol.unwrap().as_str(),
        "preflight and the Settings view must read one value"
    );
    assert_eq!(
        facts.encoder_process,
        view.values.encoder_process.unwrap().as_str()
    );

    // A3b-1/A7-1: a *valid* assignment shadowed by a later INVALID one.
    // Config.cpp assigns only inside its whitelist, so the runtime keeps
    // alvr/inproc rather than the raw last value.
    std::fs::write(
            &toml,
            b"protocol = \"alvr\"\nencoder_process = \"inproc\"\n\n[tweaks]\nprotocol = \"banana\"\nencoder_process = \"garbage\"\n",
        )
        .unwrap();
    let facts = read_toml_facts(&toml);
    let view = crate::config::runtime_toml::read(&toml);
    assert_eq!(facts.protocol, "alvr");
    assert_eq!(facts.encoder_process, "inproc");
    assert_eq!(facts.protocol, view.values.protocol.unwrap().as_str());
    assert_eq!(
        facts.encoder_process,
        view.values.encoder_process.unwrap().as_str()
    );

    // …but when NO occurrence is one the runtime would accept, the raw
    // last assignment is still what the die text and the unrecognized-
    // encoder warn quote back.
    std::fs::write(
        &toml,
        b"protocol = \"banana\"\nencoder_process = \"garbage\"\n",
    )
    .unwrap();
    let facts = read_toml_facts(&toml);
    assert_eq!(facts.protocol, "banana");
    assert_eq!(facts.encoder_process, "garbage");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn encoder_process_defaults_to_auto_exactly_like_the_shell() {
    let dir = scratch("facts");
    let toml = dir.join("oxrsys-runtime.toml");

    // Missing file: `awk` on a nonexistent path captures nothing.
    let facts = read_toml_facts(&toml);
    assert!(!facts.present);
    assert_eq!(facts.protocol, "");
    assert_eq!(facts.encoder_process, "auto", "${{ENCODER_PROC:-auto}}");

    std::fs::write(&toml, "protocol = \"alvr\"\n").unwrap();
    let facts = read_toml_facts(&toml);
    assert!(facts.present);
    assert_eq!(facts.protocol, "alvr");
    assert_eq!(facts.encoder_process, "auto");

    std::fs::write(&toml, "protocol = \"alvr\"\nencoder_process = \"inproc\"\n").unwrap();
    assert_eq!(read_toml_facts(&toml).encoder_process, "inproc");

    std::fs::remove_dir_all(&dir).ok();
}

fn scratch(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "sabrage-preflight-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::remove_dir_all(&p).ok();
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn write(p: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, bytes).unwrap();
}

/// The checkout this test binary was compiled from — three levels above
/// the crate manifest (`sabrage/crates/sabrage-core`), the same recipe
/// `checks::meta`'s and `util`'s own tests use.
fn checkout_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("repo root resolves")
}

/// Copy the checkout's `contract/` and its generated shell mirror into a
/// scratch root, so the walk's **first** slug — `meta.contract-sync`,
/// `block`-gated on this side — passes there and every test below reaches
/// the row it is actually about instead of dying on row zero.
///
/// The live files, not synthesised ones: the evaluator also compares the
/// contract compiled into THIS binary, which came from the same checkout.
fn seed_contract(root: &Path) {
    let src = checkout_root();
    for rel in CONTRACT_FILES
        .iter()
        .copied()
        .chain([CONTRACT_GEN_REL_PATH])
    {
        let dst = root.join(rel);
        std::fs::create_dir_all(dst.parent().unwrap()).unwrap();
        std::fs::copy(src.join(rel), &dst)
            .unwrap_or_else(|e| panic!("seeding {rel} into the scratch root: {e}"));
    }
}

fn write_exec(p: &Path, bytes: &[u8]) {
    write(p, bytes);
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
}

struct Fixture {
    root: PathBuf,
    ctx: StageCtx,
    events: Arc<StdMutex<Vec<StageEvent>>>,
}

impl Fixture {
    fn events(&self) -> Vec<StageEvent> {
        self.events.lock().unwrap().clone()
    }

    fn checks(&self) -> Vec<(String, CheckStatus)> {
        self.events()
            .into_iter()
            .filter_map(|e| match e {
                StageEvent::Check { outcome, .. } => Some((outcome.slug, outcome.status)),
                _ => None,
            })
            .collect()
    }

    fn check(&self, slug: &str) -> Option<CheckOutcome> {
        self.events().into_iter().find_map(|e| match e {
            StageEvent::Check { outcome, .. } if outcome.slug == slug => Some(outcome),
            _ => None,
        })
    }

    fn lines(&self) -> Vec<String> {
        self.events()
            .into_iter()
            .filter_map(|e| match e {
                StageEvent::Line { text, .. } => Some(text),
                _ => None,
            })
            .collect()
    }

    fn auto_fixed(&self) -> Vec<FixAction> {
        self.events()
            .into_iter()
            .filter_map(|e| match e {
                StageEvent::AutoFixed { fix, .. } => Some(fix),
                _ => None,
            })
            .collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).ok();
    }
}

/// A fully synthetic pipeline root: fixture bottle, fixture OXRSys support
/// dir, fixture CrossOver tree. Nothing under the real `$HOME`,
/// `/usr/local`, or `CrossOver.app` is read or written, and the executor
/// is [`crate::executor::DryRunExecutor`] unless `dry_run` is false.
fn fixture(tag: &str, dry_run: bool) -> Fixture {
    let root = scratch(tag);
    // Row zero of the walk checks the checkout itself, so the scratch root
    // has to look like a real (self-consistent) checkout.
    seed_contract(&root);
    let prefix = root.join("bottle");
    let cx = root.join("CrossOver");

    let mut paths = Paths::new(&root);
    paths.oxr_appsup = root.join("OXRSys");
    paths.toml_path = paths.oxr_appsup.join("oxrsys-runtime.toml");
    paths.sabrage_appsup = root.join("Sabrage");
    paths.host_xr_json = root.join("host/active_runtime.x86_64.json");
    paths.cx_app = Some(cx.join("CrossOver.app"));
    paths.cx = Some(cx.clone());
    paths.wine = Some(cx.join("bin/wine"));
    paths.wineserver = Some(cx.join("bin/wineserver"));
    paths.adb = None;

    let opts = StageOptions {
        bottle_name: Some("FixtureBottle".to_string()),
        // Never let `BS_DIR` fall back to the real bottles root: the
        // default derives from `$HOME`, and these tests write a fake
        // `Beat Saber.exe` into it.
        bs_dir_override: Some(root.join("BeatSaber")),
        dry_run,
        ..StageOptions::default()
    };

    let events: Arc<StdMutex<Vec<StageEvent>>> = Arc::new(StdMutex::new(Vec::new()));
    let seen = events.clone();
    let sink: crate::stages::EventSink = Arc::new(move |e| seen.lock().unwrap().push(e));

    let mut ctx = StageCtx::new(paths, opts, sink, CancellationToken::new());
    ctx.bottle = Some(Bottle {
        name: "FixtureBottle".to_string(),
        sys32: prefix.join("drive_c/windows/system32"),
        prefix: prefix.clone(),
    });

    // Belt and braces: every path this preflight can write through must
    // stay inside the scratch root. `BS_DIR` in particular defaults to a
    // path under the machine's REAL bottles directory.
    assert!(
        ctx.bs_dir.starts_with(&root),
        "fixture BS_DIR escaped the scratch root: {}",
        ctx.bs_dir.display()
    );
    assert!(ctx
        .bottle
        .as_ref()
        .is_some_and(|b| b.prefix.starts_with(&root)));

    Fixture { root, ctx, events }
}

/// Make every `block`-gated slug before `cfg.protocol.*` pass, so a test
/// can drive the preflight all the way down to the row it cares about.
fn make_everything_pass(f: &Fixture) {
    let p = &f.ctx.paths;
    let b = f.ctx.bottle.clone().unwrap();

    // bottle.gfx-dxmt (autofix) — already current, so no fix runs.
    write(
        &b.conf_path(),
        b"\"Template\" = \"win11_64\"\n\"CX_GRAPHICS_BACKEND\" = \"dxmt\"\n",
    );
    // dep.goldberg — present (hash will not match the pin; run tolerates).
    write(&p.gbe_dll, b"not the pinned build");
    // game.present / game.version
    write(&f.ctx.bs_dir.join("Beat Saber.exe"), b"MZ");
    write(&f.ctx.bs_dir.join("BeatSaberVersion.txt"), b"1.29.4\n");
    // build.helper-* — staged copy is this test binary (thin arm64 here).
    let helper = std::env::current_exe().unwrap();
    std::fs::create_dir_all(p.oxr_helper_staged.parent().unwrap()).unwrap();
    std::fs::copy(&helper, &p.oxr_helper_staged).ok();
    // overlay.* — src == dst for all four.
    for (src, dst) in [
        (
            p.dxmt_art.join("x86_64-windows/d3d11.dll"),
            p.cx_dxmt("x86_64-windows/d3d11.dll").unwrap(),
        ),
        (
            p.dxmt_art.join("x86_64-unix/winemetal.so"),
            p.cx_dxmt("x86_64-unix/winemetal.so").unwrap(),
        ),
        (
            p.woxr_dll.clone(),
            p.cx_wine_lib("x86_64-windows/wineopenxr.dll").unwrap(),
        ),
        (
            p.woxr_so.clone(),
            p.cx_wine_lib("x86_64-unix/wineopenxr.so").unwrap(),
        ),
    ] {
        write(&src, b"overlay-bytes");
        write(&dst, b"overlay-bytes");
    }
    // bottle-bridge
    write(&b.sys32.join("wineopenxr.dll"), b"overlay-bytes");
    write(&b.openxr_manifest(), b"{}");
    write(
        &b.system_reg(),
        b"\"ActiveRuntime\"=\"C:\\\\openxr\\\\wineopenxr64.json\"\n",
    );
    // host.manifest — must parse and point at an existing dylib.
    write(&p.oxr_dylib, b"dylib");
    write(
        &p.host_xr_json,
        format!(
            "{{\"file_format_version\":\"1.0.0\",\"runtime\":{{\"name\":\"oxrsys\",\
                 \"library_path\":\"{}\"}}}}\n",
            p.oxr_dylib.display()
        )
        .as_bytes(),
    );
    // run.wine-exec / run.bridge-built
    write_exec(p.wine.as_ref().unwrap(), b"#!/bin/sh\n");
    write(&p.woxr_dll, b"overlay-bytes");
    // cfg.protocol.*
    write(&p.toml_path, b"protocol = \"alvr\"\n");
}

#[test]
fn applicability_table() {
    let mut f = fixture("applicability", true);

    assert_eq!(
        not_applicable_reason(&f.ctx, "run.wired-adb", EncoderMode::HelperRequired),
        Some("not --wired")
    );
    f.ctx.opts.wired = true;
    assert_eq!(
        not_applicable_reason(&f.ctx, "run.wired-adb", EncoderMode::HelperRequired),
        None
    );

    // inproc: both helper slugs are not applicable; everything else is.
    for slug in HELPER_SLUGS {
        assert_eq!(
            not_applicable_reason(&f.ctx, slug, EncoderMode::Inproc),
            Some("encoder_process=inproc — the native helper is disabled")
        );
        assert_eq!(
            not_applicable_reason(&f.ctx, slug, EncoderMode::HelperRequired),
            None
        );
        assert_eq!(
            not_applicable_reason(&f.ctx, slug, EncoderMode::UnrecognizedTreatedAsAuto),
            None,
            "an unrecognized value still requires the helper"
        );
    }
    assert_eq!(
        not_applicable_reason(&f.ctx, "game.present", EncoderMode::Inproc),
        None
    );
}

/// Stop during the preflight aborts it without a die row — the walk is
/// read-only, so there is nothing to unwind.
#[tokio::test]
async fn a_cancelled_token_stops_the_walk() {
    let f = fixture("cancelled", true);
    make_everything_pass(&f);
    f.ctx.cancel.cancel();

    let err = run(&f.ctx).await.unwrap_err();
    assert!(matches!(err, SabrageError::Cancelled), "{err:?}");
    assert!(f.checks().is_empty(), "aborted before the first row");
}

#[tokio::test]
async fn require_bottle_dies_before_any_check_row() {
    let mut f = fixture("no-bottle", true);
    f.ctx.opts.bottle_name = None;
    f.ctx.bottle = None;

    let err = run(&f.ctx).await.unwrap_err();
    assert!(
        err.to_string()
            .starts_with("CrossOver bottle name required"),
        "{err}"
    );
    assert!(f.checks().is_empty(), "no check row before require_bottle");
}

#[tokio::test]
async fn a_clean_machine_walks_every_slug_in_contract_order() {
    let f = fixture("clean", true);
    make_everything_pass(&f);

    let facts = run(&f.ctx).await.expect("preflight passes");
    assert_eq!(facts.protocol, "alvr");
    assert_eq!(facts.encoder_process, "auto");

    let seen: Vec<String> = f.checks().into_iter().map(|(s, _)| s).collect();
    let want: Vec<String> = preflight_slugs().iter().map(|s| s.to_string()).collect();
    assert_eq!(seen, want, "one Check per slug, in contract order");
    // Asserted separately: this gate forces the fixture to seed a whole scratch checkout.
    assert_eq!(
        seen.first().map(String::as_str),
        Some("meta.contract-sync"),
        "the contract tripwire is row zero of the walk: {seen:?}"
    );

    // run.wired-adb is the only skipped row on a non-wired clean machine.
    for (slug, status) in f.checks() {
        if slug == "run.wired-adb" {
            assert_eq!(status, CheckStatus::Skipped);
        } else {
            assert!(
                matches!(status, CheckStatus::Pass | CheckStatus::Warn),
                "{slug} = {status:?}"
            );
        }
    }
    assert!(f.auto_fixed().is_empty(), "nothing needed fixing");
    assert!(
        !f.lines().iter().any(|l| l.contains("unrecognized")),
        "encoder_process=auto is a recognized value — no unrecognized-encoder warn: {:?}",
        f.lines()
    );
}

/// `meta.contract-sync` is `native_gate = "block"`: a checkout whose
/// `contract.gen.sh` header does not match its `contract/` refuses to launch
/// on the contract's very first slug, before the preflight has probed — or
/// auto-fixed — anything else.
///
/// The slug has no arm in [`block_die`], so the die text is the evaluator's
/// own message and remedy, through the fallback arm.
#[tokio::test]
async fn a_stale_contract_gen_header_blocks_the_launch_on_the_first_slug() {
    let f = fixture("contract-stale", true);
    make_everything_pass(&f);
    // The shape of a real header, a hash of nothing: the seeded checkout
    // is now internally inconsistent, exactly as it would be after a
    // `contract/` edit without `scripts/dev/parity.sh --regen`.
    write(
        &f.root.join(CONTRACT_GEN_REL_PATH),
        b"# contract-sha256: \
              0000000000000000000000000000000000000000000000000000000000000000\n",
    );

    let err = run(&f.ctx).await.unwrap_err();
    let SabrageError::Fatal { message, remedy } = &err else {
        panic!("a block gate must be Fatal, got {err:?}")
    };
    assert_eq!(
        message,
        "contract/ and scripts/demo/contract.gen.sh out of sync (contract edited without \
             regen, or the generated file was hand-edited)"
    );
    assert_eq!(remedy.as_deref(), Some("scripts/dev/parity.sh --regen"));

    // Row zero, and nothing after it: the walk stops on the first block.
    assert_eq!(
        f.checks(),
        vec![("meta.contract-sync".to_string(), CheckStatus::Fail)]
    );
    assert!(
        f.auto_fixed().is_empty(),
        "nothing runs behind a stale contract"
    );
    // The Fatal carries the same remedy the check row does, so the GUI
    // offers `--regen` rather than a bare slug.
    let fatals: Vec<Option<String>> = f
        .events()
        .into_iter()
        .filter_map(|e| match e {
            StageEvent::Fatal { remedy, .. } => Some(remedy),
            _ => None,
        })
        .collect();
    assert_eq!(
        fatals,
        vec![Some("scripts/dev/parity.sh --regen".to_string())]
    );
}

#[tokio::test]
async fn goldberg_hash_mismatch_does_not_block_the_launch() {
    let f = fixture("gbe-warn", true);
    make_everything_pass(&f);
    // gbe_dll is deliberately not the pinned build.
    run(&f.ctx).await.expect("a hash mismatch must not block");

    let o = f.check("dep.goldberg").expect("row emitted");
    assert_eq!(o.status, CheckStatus::Warn);
    assert!(
        o.detail.unwrap().contains("run tolerates a hash mismatch"),
        "the tolerance must be explained on the row"
    );
}

#[tokio::test]
async fn a_missing_goldberg_dll_dies_with_run_shs_text() {
    let f = fixture("gbe-missing", true);
    make_everything_pass(&f);
    std::fs::remove_file(&f.ctx.paths.gbe_dll).unwrap();

    let err = run(&f.ctx).await.unwrap_err();
    assert_eq!(err.to_string(), "Goldberg dll missing — ./demo.sh setup");
    assert_eq!(f.check("dep.goldberg").unwrap().status, CheckStatus::Fail);
}

#[tokio::test]
async fn a_wrong_game_version_warns_with_run_shs_text_and_continues() {
    let f = fixture("gamever", true);
    make_everything_pass(&f);
    write(&f.ctx.bs_dir.join("BeatSaberVersion.txt"), b"1.34.2\n");

    run(&f.ctx).await.expect("a version warn never blocks");
    assert!(
            f.lines()
                .iter()
                .any(|l| l
                    == "Beat Saber version '1.34.2' != 1.29.4 — the Meta gate may block startup"),
            "{:?}",
            f.lines()
        );
}

#[tokio::test]
async fn a_missing_game_dies_with_the_three_line_die() {
    let f = fixture("game-missing", true);
    make_everything_pass(&f);
    std::fs::remove_file(f.ctx.bs_dir.join("Beat Saber.exe")).unwrap();

    let err = run(&f.ctx).await.unwrap_err();
    let want = format!(
        "Beat Saber not found at {}\n       download 1.29.4: {}\n       \
             (or pass --bs-dir / set WINEVR_BS_DIR)",
        f.ctx.bs_dir.display(),
        contract().depot_command(&f.ctx.bs_dir)
    );
    assert_eq!(err.to_string(), want);
}

#[tokio::test]
async fn a_missing_toml_dies_with_the_setup_remedy() {
    let f = fixture("toml-missing", true);
    make_everything_pass(&f);
    std::fs::remove_file(&f.ctx.paths.toml_path).unwrap();

    let err = run(&f.ctx).await.unwrap_err();
    assert_eq!(
        err.to_string(),
        format!(
            "{} missing — ./demo.sh setup",
            f.ctx.paths.toml_path.display()
        )
    );
}

// Both spellings reach the same gate: the runtime obeys the last
// `protocol` assignment it would accept, whatever table it sits under.
// The shadowed row is finding A7-2 of the round-1 review; its label carries
// the round because round 2 reuses the id for an unrelated finding.
#[tokio::test]
async fn an_oxrsys_protocol_blocks_the_launch_with_both_lines() {
    let rows: &[(&str, &[u8])] = &[
        ("direct oxrsys", b"protocol = \"oxrsys\"\n"),
        (
            "r1:A7-2 shadowed alvr then oxrsys",
            b"[streaming]\nprotocol = \"alvr\"\n\n[tweaks]\nprotocol = \"oxrsys\"\n",
        ),
    ];
    for &(label, toml) in rows {
        let tag = format!(
            "proto-{}",
            label.replace(|c: char| !c.is_ascii_alphanumeric(), "-")
        );
        let f = fixture(&tag, true);
        make_everything_pass(&f);
        write(&f.ctx.paths.toml_path, toml);

        let err = run(&f.ctx).await.expect_err(label);
        assert_eq!(
                err.to_string(),
                "protocol=oxrsys (legacy USB path) — the demo path is alvr\n       \
                 Sabrage does not launch the legacy protocol — use ./demo.sh run --bottle FixtureBottle",
                "{label}"
            );
        // The supported-set row passed; the legacy row is the one that blocked.
        assert_eq!(
            f.check("cfg.protocol.supported").unwrap().status,
            CheckStatus::Pass,
            "{label}"
        );
        assert_eq!(
            f.check("cfg.protocol.legacy-oxrsys").unwrap().status,
            CheckStatus::Fail,
            "{label}"
        );
    }
}

#[tokio::test]
async fn an_unknown_protocol_dies_with_run_shs_two_line_text() {
    let f = fixture("proto-garbage", true);
    make_everything_pass(&f);
    write(&f.ctx.paths.toml_path, b"protocol = \"banana\"\n");

    let err = run(&f.ctx).await.unwrap_err();
    assert_eq!(
        err.to_string(),
        format!(
            "oxrsys-runtime.toml protocol='banana' is not valid for the demo\n       \
                 set protocol = \"alvr\" in {} (or delete the file and re-run ./demo.sh setup)",
            f.ctx.paths.toml_path.display()
        )
    );
    // The legacy row is never reached (the supported-set die aborts first).
    assert!(f.check("cfg.protocol.legacy-oxrsys").is_none());
}

#[tokio::test]
async fn inproc_prints_the_info_row_once_and_skips_both_helper_slugs() {
    let f = fixture("inproc", true);
    make_everything_pass(&f);
    std::fs::remove_file(&f.ctx.paths.oxr_helper_staged).ok();
    write(
        &f.ctx.paths.toml_path,
        b"protocol = \"alvr\"\nencoder_process = \"inproc\"\n",
    );

    let facts = run(&f.ctx).await.expect("inproc needs no helper");
    assert_eq!(facts.encoder_process, "inproc");

    let notice = "encoder_process=inproc — in-process x86_64 encode (native helper disabled)";
    assert_eq!(
        f.lines().iter().filter(|l| *l == notice).count(),
        1,
        "printed exactly once"
    );
    for slug in HELPER_SLUGS {
        let o = f.check(slug).expect("row still emitted");
        assert_eq!(o.status, CheckStatus::Skipped);
        assert_eq!(
            o.message,
            "encoder_process=inproc — the native helper is disabled"
        );
    }
    assert!(f.auto_fixed().is_empty(), "no restage under inproc");
}

#[tokio::test]
async fn an_unrecognized_encoder_process_warns_once_and_still_requires_the_helper() {
    let f = fixture("enc-unknown", true);
    make_everything_pass(&f);
    write(
        &f.ctx.paths.toml_path,
        b"protocol = \"alvr\"\nencoder_process = \"banana\"\n",
    );

    let facts = run(&f.ctx).await.expect("the staged helper is fine");
    assert_eq!(facts.encoder_process, "banana");

    let notice = "oxrsys-runtime.toml encoder_process='banana' unrecognized — the runtime \
                      treats unknown values as auto";
    assert_eq!(f.lines().iter().filter(|l| *l == notice).count(), 1);
    assert_eq!(
        f.check("build.helper-staged").unwrap().status,
        CheckStatus::Pass,
        "the helper pair still applies"
    );
}

#[tokio::test]
async fn an_unverifiable_applicable_check_is_fatal_not_a_pass() {
    let f = fixture("unverifiable", true);
    make_everything_pass(&f);
    // No CrossOver.app at all: every `overlay.*` row reports Skipped
    // ("CrossOver.app not found"), which is applicable-but-unverifiable.
    let mut ctx = f.ctx.clone();
    ctx.paths.cx = None;
    ctx.paths.cx_app = None;
    ctx.paths.wine = None;
    ctx.paths.wineserver = None;

    let err = run(&ctx).await.unwrap_err();
    assert_eq!(
        err.to_string(),
        "cannot verify overlay.dxmt-d3d11: CrossOver.app not found"
    );
    assert_eq!(
        f.check("overlay.dxmt-d3d11").unwrap().status,
        CheckStatus::Skipped,
        "the row is still reported before the die"
    );
}

#[tokio::test]
async fn wired_without_adb_dies_with_run_shs_text() {
    let f = fixture("wired-noadb", true);
    make_everything_pass(&f);
    let mut ctx = f.ctx.clone();
    ctx.opts.wired = true;
    ctx.paths.adb = None;

    let err = run(&ctx).await.unwrap_err();
    assert_eq!(
        err.to_string(),
        "--wired needs adb (Android platform-tools) on PATH or under ~/Library/Android/sdk"
    );
}

/// The real thing: a `cxbottle.conf` that says `auto`, a real (non-dry)
/// executor over a fixture bottle. The fix must run, the re-check must
/// pass, and exactly one `AutoFixed` must be emitted — followed by the
/// final (passing) `Check`.
#[tokio::test]
async fn a_failing_backend_row_is_fixed_rechecked_and_reported_once() {
    let f = fixture("autofix-backend", false);
    make_everything_pass(&f);
    let conf = f.ctx.bottle.clone().unwrap().conf_path();
    write(
        &conf,
        b"\"Template\" = \"win11_64\"\n\"CX_GRAPHICS_BACKEND\" = \"auto\"\n",
    );

    run(&f.ctx).await.expect("the auto-fix resolves it");

    assert_eq!(
        std::fs::read_to_string(&conf).unwrap(),
        "\"Template\" = \"win11_64\"\n\"CX_GRAPHICS_BACKEND\" = \"dxmt\"\n"
    );
    assert_eq!(
        f.auto_fixed(),
        vec![FixAction::SetGraphicsBackend],
        "exactly one AutoFixed, for the one row that needed it"
    );
    let o = f.check("bottle.gfx-dxmt").unwrap();
    assert_eq!(o.status, CheckStatus::Pass, "the row reports the RE-check");
    assert_eq!(
        f.checks()
            .iter()
            .filter(|(s, _)| s == "bottle.gfx-dxmt")
            .count(),
        1,
        "one Check per slug, never the pre-fix one as well"
    );
}

/// A dry run plans the write and never performs it, so the re-check
/// necessarily still fails. Reporting that as "the fix failed" would be a
/// lie about a run that deliberately touched nothing.
#[tokio::test]
async fn a_dry_run_reports_the_planned_fix_instead_of_a_failed_recheck() {
    let f = fixture("autofix-dry", true);
    make_everything_pass(&f);
    let conf = f.ctx.bottle.clone().unwrap().conf_path();
    write(&conf, b"\"CX_GRAPHICS_BACKEND\" = \"auto\"\n");

    run(&f.ctx)
        .await
        .expect("a dry run must not die on its own plan");

    assert_eq!(
        std::fs::read_to_string(&conf).unwrap(),
        "\"CX_GRAPHICS_BACKEND\" = \"auto\"\n",
        "a dry run never writes"
    );
    assert_eq!(f.auto_fixed(), vec![FixAction::SetGraphicsBackend]);
    let o = f.check("bottle.gfx-dxmt").unwrap();
    assert_eq!(o.status, CheckStatus::Info);
    assert!(
        o.message.ends_with("— auto-fix planned (dry run)"),
        "{}",
        o.message
    );
}

/// Neither a staged nor a built arm64 helper: `restage_helper` itself dies
/// with run.sh's two-line `ensure_helper_staged` text, raising exactly one
/// `Fatal`, and the slug's Check row is emitted alongside it.
#[tokio::test]
async fn an_unfixable_helper_dies_once_with_run_shs_ensure_helper_text_and_its_check_row() {
    let f = fixture("helper-unfixable", false);
    make_everything_pass(&f);
    std::fs::remove_file(&f.ctx.paths.oxr_helper_staged).unwrap();

    let err = run(&f.ctx).await.unwrap_err();
    let want = format!(
        "encoder_process=auto needs the arm64 helper, but neither the staged copy\n       \
             ({}) nor the build output ({}) is an arm64 executable — ./demo.sh build",
        f.ctx.paths.oxr_helper_staged.display(),
        f.ctx.paths.oxr_helper_built.display()
    );
    assert_eq!(err.to_string(), want);
    let rows = f.checks();
    assert_eq!(
        rows.iter()
            .filter(|(s, _)| s == "build.helper-staged")
            .count(),
        1,
        "the slug's row must still be emitted: {rows:?}"
    );
    assert_eq!(
        f.events()
            .iter()
            .filter(|e| matches!(e, StageEvent::Fatal { .. }))
            .count(),
        1,
        "the fix's own Fatal, not a second one"
    );
}

/// The repository's own "every key assigned twice, the second one junk"
/// fixture, driven through the whole preflight: the launch and the Settings
/// view must read the same `protocol` and `encoder_process`, and neither of
/// them the junk. (`oxrsys-runtime.shadowed-invalid-last.toml` is the file
/// `config::runtime_toml`'s tests pin the reader against.)
#[tokio::test]
async fn the_shadowed_invalid_last_fixture_launches_on_its_valid_values() {
    let f = fixture("proto-fixture-invalid-tail", true);
    make_everything_pass(&f);
    let fixture_toml = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/phase4/oxrsys-runtime.shadowed-invalid-last.toml");
    write(
        &f.ctx.paths.toml_path,
        &std::fs::read(&fixture_toml).expect("the phase4 fixture reads"),
    );

    let facts = run(&f.ctx).await.expect("the runtime accepts this file");
    let view = crate::config::runtime_toml::read(&f.ctx.paths.toml_path);
    assert_eq!(facts.protocol, "alvr");
    assert_eq!(facts.encoder_process, "native");
    assert_eq!(facts.protocol, view.values.protocol.unwrap().as_str());
    assert_eq!(
        facts.encoder_process,
        view.values.encoder_process.unwrap().as_str()
    );
    // `native` still needs the staged arm64 helper, and it is there.
    for slug in HELPER_SLUGS {
        assert_eq!(f.check(slug).unwrap().status, CheckStatus::Pass, "{slug}");
    }
    assert!(
        !f.lines().iter().any(|l| l.contains("unrecognized")),
        "encoder_process=native is a recognized value — no unrecognized-encoder warn: {:?}",
        f.lines()
    );
}

/// A7-1/A3b-1: a valid assignment shadowed by a later INVALID one — the
/// mirror of `the_shadowed_invalid_last_fixture_launches_on_its_valid_values`.
/// The runtime keeps the *valid* value, so the launch must too; the raw-last
/// reading died on `protocol='banana'` and demanded the arm64 helper for a
/// runtime that encodes in-process.
#[tokio::test]
async fn a_trailing_invalid_assignment_leaves_the_previous_valid_one_in_force() {
    let f = fixture("proto-invalid-tail", true);
    make_everything_pass(&f);
    // No staged helper at all: if the encoder fact regressed to `garbage`
    // (unrecognized ⇒ treated as auto) this would die on the helper.
    std::fs::remove_file(&f.ctx.paths.oxr_helper_staged).unwrap();
    write(
            &f.ctx.paths.toml_path,
            b"protocol = \"alvr\"\nencoder_process = \"inproc\"\n\n[tweaks]\nprotocol = \"banana\"\nencoder_process = \"garbage\"\n",
        );

    let facts = run(&f.ctx).await.expect("the runtime accepts this file");
    assert_eq!(facts.protocol, "alvr");
    assert_eq!(facts.encoder_process, "inproc");

    for slug in HELPER_SLUGS {
        assert_eq!(
            f.check(slug).unwrap().status,
            CheckStatus::Skipped,
            "{slug} must be skipped for an inproc runtime"
        );
    }
    assert!(
        !f.lines().iter().any(|l| l.contains("unrecognized")),
        "no unrecognized-encoder warn for a value the runtime ignores: {:?}",
        f.lines()
    );
    // The doctor evaluator still reports its own `awk` verdict (PARITY.md §
    // Declared by the 2026-08-30 adversarial review (round 1 fixes),
    // "Config readers: doctor emulates `awk`") — the row says why launch went on.
    let row = f.check("cfg.protocol.supported").unwrap();
    assert_eq!(row.status, CheckStatus::Fail);
    assert!(
        row.detail
            .unwrap()
            .contains("the last value it would accept is protocol='alvr'"),
        "a red row the launch ignores must explain itself"
    );
}

/// A7-4: `run.wired-adb` shells out to `adb devices`. A wedged adb used to
/// block inside the synchronous evaluator, so Stop could not land until it
/// returned — with the operation lock held throughout.
#[tokio::test]
async fn a_cancel_during_the_wired_adb_probe_stops_the_walk_promptly() {
    let f = fixture("wired-probe-cancel", true);
    make_everything_pass(&f);
    let mut ctx = f.ctx.clone();
    ctx.opts.wired = true;
    // An `adb` that marks that it started and then never answers.
    let adb = f.root.join("platform-tools/adb");
    // `sleep 5`, not a longer one: the abandoned blocking thread outlives
    // the assertion and the test runtime waits for it on shutdown.
    write_exec(
        &adb,
        b"#!/bin/sh\n: > \"$(dirname \"$0\")/probing\"\nsleep 5\n",
    );
    ctx.paths.adb = Some(adb);

    let cancel = ctx.cancel.clone();
    let marker = f.root.join("platform-tools/probing");
    tokio::spawn(async move {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !marker.exists() && std::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        cancel.cancel();
    });

    let started = std::time::Instant::now();
    let err = run(&ctx).await.unwrap_err();
    let waited = started.elapsed();
    assert!(matches!(err, SabrageError::Cancelled), "{err:?}");
    assert!(
        waited < std::time::Duration::from_secs(3),
        "Stop must not wait out the probe, waited {waited:?}"
    );
}

/// The same shadowing for `encoder_process`: a first `inproc` must not skip
/// the helper pair when a later `native` is what the runtime will use.
#[tokio::test]
async fn a_shadowed_encoder_process_still_requires_the_helper() {
    let f = fixture("enc-shadowed", true);
    make_everything_pass(&f);
    std::fs::remove_file(&f.ctx.paths.oxr_helper_staged).unwrap();
    write(
            &f.ctx.paths.toml_path,
            b"protocol = \"alvr\"\nencoder_process = \"inproc\"\n\n              [tweaks]\nencoder_process = \"native\"\n",
        );

    // Dies on the missing helper: the later `native` is the value the runtime
    // uses, so both helper rows stay applicable.
    let err = run(&f.ctx).await.unwrap_err();
    assert!(err.to_string().contains("needs the arm64 helper"), "{err}");
    assert!(
        !f.lines()
            .iter()
            .any(|l| l.contains("encoder_process=inproc")),
        "the shadowed inproc must not print the in-process notice: {:?}",
        f.lines()
    );
}

/// A7-6: an `Err` out of the fix used to return through `?` before the
/// slug's `Check` was emitted — an event-only consumer saw a failed stage
/// with the row left hanging. One Check, one Fatal, and the io cause.
#[tokio::test]
async fn a_backend_autofix_that_cannot_write_still_emits_its_check_and_dies_run_shs_way() {
    let f = fixture("autofix-unwritable", false);
    make_everything_pass(&f);
    let b = f.ctx.bottle.clone().unwrap();
    let conf = b.conf_path();
    write(
        &conf,
        b"\"Template\" = \"win11_64\"\n\"CX_GRAPHICS_BACKEND\" = \"auto\"\n",
    );
    // Read-only directory: the atomic write cannot create its temp file.
    let dir = conf.parent().unwrap().to_path_buf();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();

    let err = run(&f.ctx).await.unwrap_err();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(
        err.to_string(),
        format!(
            "could not force graphics backend to dxmt in {}",
            conf.display()
        ),
        "run.sh's post-fix die text, not a raw io error"
    );
    // Exactly one Check for the slug, carrying the failure and its cause.
    let rows: Vec<CheckOutcome> = f
        .events()
        .into_iter()
        .filter_map(|e| match e {
            StageEvent::Check { outcome, .. } if outcome.slug == "bottle.gfx-dxmt" => Some(outcome),
            _ => None,
        })
        .collect();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].status, CheckStatus::Fail);
    assert!(rows[0].detail.is_some(), "the io cause belongs on the row");
    // One Fatal, and the cause is visible as a stderr-shaped Output line.
    let fatals: Vec<String> = f
        .events()
        .into_iter()
        .filter_map(|e| match e {
            StageEvent::Fatal { message, .. } => Some(message),
            _ => None,
        })
        .collect();
    assert_eq!(fatals.len(), 1, "{fatals:?}");
    assert!(
        f.events().iter().any(|e| matches!(
            e,
            StageEvent::Output {
                stream: crate::events::Stream::Stderr,
                ..
            }
        )),
        "the io cause must reach the user"
    );
    assert!(f.auto_fixed().is_empty(), "nothing was fixed");
    // The conf is untouched.
    assert_eq!(
        std::fs::read_to_string(&conf).unwrap(),
        "\"Template\" = \"win11_64\"\n\"CX_GRAPHICS_BACKEND\" = \"auto\"\n"
    );
}
