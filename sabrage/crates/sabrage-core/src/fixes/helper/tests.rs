use super::*;
use crate::paths::Paths;
use crate::stages::{StageCtx, StageOptions};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use tokio_util::sync::CancellationToken;

#[test]
fn parse_encoder_process_follows_the_runtime_semantics() {
    assert_eq!(
        parse_encoder_process("encoder_process = \"native\"\n"),
        "native"
    );
    assert_eq!(
        parse_encoder_process("  encoder_process=\"auto\"\n"),
        "auto"
    );
    assert_eq!(
        parse_encoder_process("# encoder_process = \"native\"\n"),
        ""
    );
    assert_eq!(parse_encoder_process("encoder_process_extra = \"x\"\n"), "");
    assert_eq!(
        parse_encoder_process("protocol = \"alvr\"\nencoder_process = \"inproc\"\n"),
        "inproc"
    );
    assert_eq!(parse_encoder_process(""), "");

    // The runtime is table-blind and last-assignment-wins, so a shadowed
    // earlier line is NOT the value the launched runtime uses.
    assert_eq!(
        parse_encoder_process(
            "encoder_process = \"inproc\"\n[streaming]\nencoder_process = \"native\"\n"
        ),
        "native"
    );
    // The runtime strips one layer of quotes and accepts the bare word…
    assert_eq!(parse_encoder_process("encoder_process=native\n"), "native");
    // …and silently ignores a value outside the accepted set, keeping its
    // compiled-in default — which `encoder_process_or_default` then reports.
    assert_eq!(parse_encoder_process("encoder_process = \"bogus\"\n"), "");
}

#[test]
fn encoder_process_or_default_falls_back_to_auto() {
    assert_eq!(encoder_process_or_default(""), "auto");
    assert_eq!(encoder_process_or_default("protocol = \"alvr\"\n"), "auto");
    assert_eq!(
        encoder_process_or_default("encoder_process = \"\"\n"),
        "auto"
    );
    assert_eq!(
        encoder_process_or_default("encoder_process = \"native\"\n"),
        "native"
    );
}

fn scratch(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("sabrage-helper-fix-{tag}-{}", std::process::id()))
}

fn write_thin_arm64_stub(path: &Path) {
    // Use the CURRENT test binary's own bytes as a real, verifiable
    // thin-arm64 (or whatever this machine's build actually is) Mach-O —
    // exactly the trick `checks::build`'s own tests use. Skipped by
    // callers on a non-arm64 build machine (see `arm64_available` below).
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::copy(std::env::current_exe().unwrap(), path).unwrap();
    let mut perms = std::fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).unwrap();
}

/// Whether this machine's test binary is itself arm64 — the same
/// precondition `checks::build`'s `helper_is_arm64` tests gate on. Skip
/// (rather than fail) elsewhere, matching that module's convention.
fn arm64_available() -> bool {
    helper_is_arm64(&std::env::current_exe().unwrap())
}

fn ctx_for(root: &Path, dry_run: bool) -> (StageCtx, Arc<StdMutex<Vec<StageEvent>>>) {
    let seen = Arc::new(StdMutex::new(Vec::new()));
    let s = seen.clone();
    let sink: EventSink = Arc::new(move |ev| s.lock().unwrap().push(ev));
    let opts = StageOptions {
        dry_run,
        ..StageOptions::default()
    };
    let ctx = StageCtx::new(Paths::new(root), opts, sink, CancellationToken::new());
    (ctx, seen)
}

#[tokio::test]
async fn already_staged_arm64_is_a_silent_noop() {
    if !arm64_available() {
        return;
    }
    let root = scratch("noop");
    let (ctx, seen) = ctx_for(&root, false);
    write_thin_arm64_stub(&ctx.paths.oxr_helper_staged);

    let report = restage_helper(&ctx, &ctx.sink.clone()).await.unwrap();
    assert!(!report.changed);
    assert!(
        seen.lock().unwrap().is_empty(),
        "run.sh prints nothing here"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn restages_from_the_build_tree_when_staged_is_missing() {
    if !arm64_available() {
        return;
    }
    let root = scratch("restage");
    let (ctx, seen) = ctx_for(&root, false);
    write_thin_arm64_stub(&ctx.paths.oxr_helper_built);
    // The staged FILE is intentionally absent, but its directory (as it
    // would be after `./demo.sh build` populated `build-x64/runtime/`)
    // already exists — `copy_if_changed` does not `mkdir -p` for you, any
    // more than `cp` does.
    std::fs::create_dir_all(ctx.paths.oxr_helper_staged.parent().unwrap()).unwrap();

    let report = restage_helper(&ctx, &ctx.sink.clone()).await.unwrap();
    assert!(report.changed);
    assert_eq!(report.description, "encoder helper restaged (arm64)");
    assert!(ctx.paths.oxr_helper_staged.is_file());
    assert!(helper_is_arm64(&ctx.paths.oxr_helper_staged));

    let evs = seen.lock().unwrap();
    let texts: Vec<&str> = evs
        .iter()
        .filter_map(|e| match e {
            StageEvent::Line { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(texts[0].starts_with("encoder helper missing/not arm64 at"));
    assert!(texts[0].ends_with("restaging from the helper build tree"));
    assert!(texts.iter().any(|t| t.starts_with("installed: ")));
    assert_eq!(*texts.last().unwrap(), "encoder helper restaged (arm64)");

    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn dry_run_reports_would_restage_without_writing_or_re_validating() {
    if !arm64_available() {
        return;
    }
    let root = scratch("dry");
    let (ctx, _seen) = ctx_for(&root, true);
    write_thin_arm64_stub(&ctx.paths.oxr_helper_built);

    let report = restage_helper(&ctx, &ctx.sink.clone()).await.unwrap();
    assert!(report.changed);
    assert_eq!(
        report.description,
        "encoder helper would be restaged (arm64)"
    );
    assert!(
        !ctx.paths.oxr_helper_staged.exists(),
        "dry run must never write"
    );

    std::fs::remove_dir_all(&root).ok();
}

/// A byte-identical staged copy that lost its execute bit is repaired:
/// `copy_if_changed`'s byte compare cannot see the mode, so without the
/// repair re-validation fails on every retry and neither this fix nor
/// `./demo.sh build` can recover.
#[tokio::test]
async fn a_byte_identical_but_non_executable_staged_helper_is_repaired() {
    if !arm64_available() {
        return;
    }
    let root = scratch("mode-repair");
    let (ctx, seen) = ctx_for(&root, false);
    write_thin_arm64_stub(&ctx.paths.oxr_helper_built);
    std::fs::create_dir_all(ctx.paths.oxr_helper_staged.parent().unwrap()).unwrap();
    std::fs::copy(&ctx.paths.oxr_helper_built, &ctx.paths.oxr_helper_staged).unwrap();
    let mut perms = std::fs::metadata(&ctx.paths.oxr_helper_staged)
        .unwrap()
        .permissions();
    perms.set_mode(0o644);
    std::fs::set_permissions(&ctx.paths.oxr_helper_staged, perms).unwrap();
    assert!(!helper_is_arm64(&ctx.paths.oxr_helper_staged));

    let report = restage_helper(&ctx, &ctx.sink.clone()).await.unwrap();
    assert!(report.changed);
    assert_eq!(report.description, "encoder helper restaged (arm64)");
    assert!(
        helper_is_arm64(&ctx.paths.oxr_helper_staged),
        "the execute bit must be repaired, not compared away"
    );
    assert_eq!(
        std::fs::metadata(&ctx.paths.oxr_helper_staged)
            .unwrap()
            .permissions()
            .mode()
            & 0o111,
        std::fs::metadata(&ctx.paths.oxr_helper_built)
            .unwrap()
            .permissions()
            .mode()
            & 0o111
    );

    let evs = seen.lock().unwrap();
    let texts: Vec<&str> = evs
        .iter()
        .filter_map(|e| match e {
            StageEvent::Line { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        texts.iter().any(|t| t.starts_with("installed: ")),
        "a repair is an install, not an `unchanged:` row: {texts:?}"
    );
    assert!(!texts.iter().any(|t| t.starts_with("unchanged: ")));

    drop(evs);
    std::fs::remove_dir_all(&root).ok();
}

/// The same state under a dry run: the staged file is untouched, the mode
/// repair appears in the plan as a `Copy` rather than a skip, and the report
/// says a restage *would* happen.
#[tokio::test]
async fn a_dry_run_plans_the_mode_repair_without_performing_it() {
    if !arm64_available() {
        return;
    }
    let root = scratch("mode-repair-dry");
    let (ctx, _seen) = ctx_for(&root, true);
    write_thin_arm64_stub(&ctx.paths.oxr_helper_built);
    std::fs::create_dir_all(ctx.paths.oxr_helper_staged.parent().unwrap()).unwrap();
    std::fs::copy(&ctx.paths.oxr_helper_built, &ctx.paths.oxr_helper_staged).unwrap();
    let mut perms = std::fs::metadata(&ctx.paths.oxr_helper_staged)
        .unwrap()
        .permissions();
    perms.set_mode(0o644);
    std::fs::set_permissions(&ctx.paths.oxr_helper_staged, perms).unwrap();

    let report = restage_helper(&ctx, &ctx.sink.clone()).await.unwrap();
    assert!(report.changed);
    assert_eq!(
        report.description,
        "encoder helper would be restaged (arm64)"
    );
    assert_eq!(
        std::fs::metadata(&ctx.paths.oxr_helper_staged)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o644,
        "a dry run must not repair anything"
    );
    let planned = ctx.executor.planned();
    assert!(
        planned
            .iter()
            .any(|p| p.kind == crate::executor::PlannedKind::Copy
                && p.dst.as_deref() == Some(ctx.paths.oxr_helper_staged.as_path())),
        "the mode repair must appear in the plan as work, not as a skip: {planned:?}"
    );
    assert!(!planned
        .iter()
        .any(|p| p.kind == crate::executor::PlannedKind::Skip));

    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn neither_staged_nor_built_is_arm64_dies_with_the_verbatim_message() {
    let root = scratch("fatal");
    let toml_dir = root.join("OXRSys");
    std::fs::create_dir_all(&toml_dir).unwrap();
    let mut paths = Paths::new(&root);
    paths.toml_path = toml_dir.join("oxrsys-runtime.toml");
    std::fs::write(&paths.toml_path, "encoder_process = \"native\"\n").unwrap();
    // Neither oxr_helper_staged nor oxr_helper_built exists at all.

    let opts = StageOptions::default();
    let sink: EventSink = Arc::new(|_| {});
    let ctx = StageCtx::new(paths, opts, sink.clone(), CancellationToken::new());
    let err = restage_helper(&ctx, &sink).await.unwrap_err();
    let msg = err.to_string();
    assert!(msg
        .starts_with("encoder_process=native needs the arm64 helper, but neither the staged copy"));
    assert!(msg.contains(&format!(
        "\n       ({}) nor the build output ({}) is an arm64 executable — ./demo.sh build",
        ctx.paths.oxr_helper_staged.display(),
        ctx.paths.oxr_helper_built.display()
    )));

    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn missing_toml_defaults_encoder_process_to_auto_in_the_die_text() {
    let root = scratch("fatal-no-toml");
    let opts = StageOptions::default();
    let sink: EventSink = Arc::new(|_| {});
    let ctx = StageCtx::new(
        Paths::new(&root),
        opts,
        sink.clone(),
        CancellationToken::new(),
    );
    let err = restage_helper(&ctx, &sink).await.unwrap_err();
    assert!(err
        .to_string()
        .starts_with("encoder_process=auto needs the arm64 helper"));
    std::fs::remove_dir_all(&root).ok();
}
