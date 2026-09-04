use super::*;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sabrage-build-test-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_fake_tool(dir: &Path, name: &str, script: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    std::fs::write(&path, script).unwrap();
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).unwrap();
    path
}

/// Row of the ninja-progress table: (label, input line, expected parse).
type NinjaCase = (&'static str, &'static str, Option<(u64, u64)>);

#[test]
fn parse_ninja_progress_maps_status_lines_and_rejects_other_shapes() {
    let cases: &[NinjaCase] = &[
        (
            "default status prefix",
            "[12/340] Building CXX object foo.cpp.o",
            Some((12, 340)),
        ),
        (
            "leading whitespace tolerated — a chunk boundary can land mid-line padding",
            "  [1/2] Linking CXX foo",
            Some((1, 2)),
        ),
        (
            "a cmake configure line is not ninja's shape",
            "-- Configuring done",
            None,
        ),
        (
            "a compiler warning is not ninja's shape",
            "foo.cpp:12:3: warning: unused variable",
            None,
        ),
        (
            "wineopenxr's Makefiles-style `[ 50%]` is not ninja's shape",
            "[ 50%] Building CXX object foo",
            None,
        ),
        ("empty line", "", None),
        ("missing denominator", "[1/]", None),
        ("missing numerator", "[/2]", None),
        ("non-numeric numerator", "[abc/2]", None),
        ("no brackets at all", "no brackets at all", None),
        (
            "ninja's final summary line",
            "[340/340] Linking CXX executable oxrsys-runtime",
            Some((340, 340)),
        ),
    ];
    for (label, line, expected) in cases {
        assert_eq!(parse_ninja_progress(line), *expected, "{label}");
    }
}

#[test]
fn resolve_tool_finds_an_executable_and_rejects_absent_or_non_executable_ones() {
    let dir = scratch("resolve-tool");
    let real = write_fake_tool(&dir, "cmake", "#!/bin/sh\necho hi\n");

    assert_eq!(
        resolve_tool("cmake", &dir.display().to_string()),
        Some(real)
    );
    assert_eq!(resolve_tool("ninja", &dir.display().to_string()), None);

    // Present but not executable: must not satisfy the gate.
    let not_exec = dir.join("ninja");
    std::fs::write(&not_exec, b"not executable").unwrap();
    assert_eq!(resolve_tool("ninja", &dir.display().to_string()), None);

    // Search order: an earlier directory without the tool falls through
    // to a later one that has it.
    let dir2 = scratch("resolve-tool-2");
    let real2 = write_fake_tool(&dir2, "rustup", "#!/bin/sh\n");
    let search_path = format!("{}:{}", dir.display(), dir2.display());
    assert_eq!(resolve_tool("rustup", &search_path), Some(real2));

    std::fs::remove_dir_all(&dir).ok();
    std::fs::remove_dir_all(&dir2).ok();
}

#[test]
fn tool_gate_reports_the_first_missing_tool_in_shell_order() {
    let dir = scratch("tool-gate");
    // Nothing present: cmake is first.
    let empty_path = dir.display().to_string();
    assert_eq!(
        tool_gate_message(&empty_path),
        Some(missing_tool_message("cmake"))
    );

    // cmake present, ninja absent: ninja is reported next.
    write_fake_tool(&dir, "cmake", "#!/bin/sh\n");
    assert_eq!(
        tool_gate_message(&empty_path),
        Some(missing_tool_message("ninja"))
    );

    // cmake + ninja present, mingw absent.
    write_fake_tool(&dir, "ninja", "#!/bin/sh\n");
    assert_eq!(
        tool_gate_message(&empty_path),
        Some(missing_tool_message("x86_64-w64-mingw32-gcc"))
    );

    // All three present: no die text.
    write_fake_tool(&dir, "x86_64-w64-mingw32-gcc", "#!/bin/sh\n");
    assert_eq!(tool_gate_message(&empty_path), None);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn missing_tool_message_is_verbatim() {
    let cases: &[(&str, &str, &str)] = &[
        (
            "cmake",
            "cmake",
            "cmake missing — brew install cmake ninja mingw-w64",
        ),
        (
            "ninja",
            "ninja",
            "ninja missing — brew install cmake ninja mingw-w64",
        ),
        (
            "mingw gcc",
            "x86_64-w64-mingw32-gcc",
            "x86_64-w64-mingw32-gcc missing — brew install cmake ninja mingw-w64",
        ),
    ];
    for (label, tool, expected) in cases {
        assert_eq!(missing_tool_message(tool), *expected, "{label}");
    }
}

#[test]
fn fixed_die_texts_are_verbatim() {
    let cases: &[(&str, &str, &str)] = &[
        (
            "rustup target missing",
            RUSTUP_TARGET_MISSING_MESSAGE,
            "rustup x86_64-apple-darwin target missing — install rustup via https://rustup.rs \
                 and source ~/.cargo/env, then: rustup toolchain install stable && rustup target \
                 add x86_64-apple-darwin",
        ),
        (
            "submodules not initialized",
            SUBMODULES_NOT_INITIALIZED_MESSAGE,
            "submodules not initialized — ./demo.sh setup",
        ),
        (
            "alvr_dashboard build failed",
            DASHBOARD_BUILD_FAILED_MESSAGE,
            "alvr_dashboard build failed — retry with: (cd ext/ALVR && cargo build -p \
                 alvr_dashboard --release)",
        ),
    ];
    for (label, actual, expected) in cases {
        assert_eq!(actual, expected, "{label}");
    }
}

#[tokio::test]
async fn rustup_gate_dies_unless_the_x86_64_target_is_installed() {
    // (label, scratch tag, fake tool filename, script, expected). Row 1's
    // tool is deliberately misnamed so `resolve_tool("rustup", ..)` misses
    // it: that is the absent-binary path, reached without a branch.
    let cases: &[(&str, &str, &str, &str, Option<&str>)] = &[
        (
            "rustup binary absent",
            "rustup-absent",
            "not-rustup",
            "#!/bin/sh\n",
            Some(RUSTUP_TARGET_MISSING_MESSAGE),
        ),
        (
            "target not installed",
            "rustup-no-target",
            "rustup",
            "#!/bin/sh\necho aarch64-apple-darwin\n",
            Some(RUSTUP_TARGET_MISSING_MESSAGE),
        ),
        (
            "target installed, not on the first line",
            "rustup-present",
            "rustup",
            "#!/bin/sh\necho aarch64-apple-darwin\necho x86_64-apple-darwin\n",
            None,
        ),
    ];
    for (label, tag, tool, script, expected) in cases {
        let dir = scratch(tag);
        write_fake_tool(&dir, tool, script);
        let search_path = dir.display().to_string();
        assert_eq!(
            rustup_gate_message(&search_path, &CancellationToken::new())
                .await
                .unwrap(),
            *expected,
            "{label}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}

#[tokio::test]
async fn rustup_gate_is_cancel_aware_and_kills_the_child() {
    // A `rustup` that would otherwise block for 5s — long enough that this
    // test would time out if cancellation weren't observed.
    let dir = scratch("rustup-cancel");
    write_fake_tool(
        &dir,
        "rustup",
        "#!/bin/sh\nsleep 5\necho x86_64-apple-darwin\n",
    );
    let search_path = dir.display().to_string();
    let cancel = CancellationToken::new();
    cancel.cancel();

    let err = rustup_gate_message(&search_path, &cancel)
        .await
        .unwrap_err();
    assert!(matches!(err, SabrageError::Cancelled));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn helper_missing_binary_message_is_verbatim() {
    let path = Path::new("/repo/ext/oxrsys/build-helper-arm64/runtime/oxrsys-encoder-helper");
    assert_eq!(
        helper_missing_binary_message(path),
        "encoder helper build produced no binary at \
             /repo/ext/oxrsys/build-helper-arm64/runtime/oxrsys-encoder-helper"
    );
}

#[test]
fn helper_wrong_arch_message_embeds_the_lipo_output_and_the_build_dir() {
    let dir = scratch("wrong-arch");
    let bin = dir.join("oxrsys-encoder-helper");
    // Not a Mach-O at all: lipo fails, stdout capture is empty — exactly
    // like the shell's `$(lipo -archs … 2>/dev/null)` on a bad file
    // (checks/build.rs's own test for this fixture shape agrees).
    std::fs::write(&bin, b"not a mach-o").unwrap();
    let build_dir = dir.join("ext/oxrsys/build-helper-arm64");
    assert_eq!(
        helper_wrong_arch_message(&bin, &build_dir),
        format!(
            "encoder helper is not an arm64 executable () — delete {} and re-run ./demo.sh build",
            build_dir.display()
        )
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn missing_output_message_is_verbatim() {
    let path = Path::new("/repo/ext/oxrsys/build-x64/runtime/liboxrsys-runtime.dylib");
    assert_eq!(
        missing_output_message(path),
        "expected build output missing: \
             /repo/ext/oxrsys/build-x64/runtime/liboxrsys-runtime.dylib"
    );
}

#[tokio::test]
async fn configure_and_build_specs_render_the_exact_argv() {
    use crate::executor::{DryRunExecutor, Executor};
    use crate::paths::Paths;
    use crate::stages::{null_sink, StageOptions};
    use tokio_util::sync::CancellationToken;

    let run_id = uuid::Uuid::new_v4();
    let executor: Arc<dyn Executor> = Arc::new(DryRunExecutor::new(
        run_id,
        null_sink(),
        CancellationToken::new(),
    ));
    let ctx = StageCtx::with_executor(
        Paths::new("/repo"),
        StageOptions::default(),
        null_sink(),
        CancellationToken::new(),
        executor,
        run_id,
    );
    let cmake = Path::new("/opt/homebrew/bin/cmake");

    let configure = configure_spec(
        &ctx,
        cmake,
        "/opt/homebrew/bin",
        step::BUILD_OXRSYS,
        &ctx.paths.oxrsys,
        &ctx.paths.oxr_build,
        &["-G", "Ninja", "-DCMAKE_BUILD_TYPE=Debug"],
    );
    assert_eq!(
        configure.display(),
        "/opt/homebrew/bin/cmake -S /repo/ext/oxrsys -B /repo/ext/oxrsys/build-x64 -G Ninja \
             -DCMAKE_BUILD_TYPE=Debug"
    );
    assert_eq!(configure.step, step::BUILD_OXRSYS);

    let build = build_spec(
        &ctx,
        cmake,
        "/opt/homebrew/bin",
        step::BUILD_HELPER,
        &ctx.paths.oxr_helper_build,
        &["--target", "oxrsys_encoder_helper"],
    );
    assert_eq!(
        build.display(),
        "/opt/homebrew/bin/cmake --build /repo/ext/oxrsys/build-helper-arm64 --target \
             oxrsys_encoder_helper -j8"
    );
    assert_eq!(build.step, step::BUILD_HELPER);
}

/// r1:A5-2 regression: the build-x64 configure passes the whole of build.sh's
/// argument list, ending in `-DOXRSYS_BUILD_ENCODER_HELPER=OFF` — CMake
/// `option()` cannot clear a cache already holding ON.
#[tokio::test]
async fn the_x64_configure_spec_renders_the_helper_off_flag() {
    let ctx = dry_run_ctx();
    let spec = configure_spec(
        &ctx,
        Path::new("/opt/homebrew/bin/cmake"),
        "/opt/homebrew/bin",
        step::BUILD_OXRSYS,
        &ctx.paths.oxrsys,
        &ctx.paths.oxr_build,
        &oxrsys_x64_configure_args(),
    );
    assert_eq!(
        spec.display(),
        "/opt/homebrew/bin/cmake -S /nonexistent/sabrage-build-test/ext/oxrsys \
             -B /nonexistent/sabrage-build-test/ext/oxrsys/build-x64 -G Ninja \
             -DCMAKE_BUILD_TYPE=Debug -DCMAKE_OSX_ARCHITECTURES=x86_64 \
             -DOXRSYS_ENABLE_ALVR=ON -DOXRSYS_BUILD_ENCODER_HELPER=OFF",
        "the whole configure command line; the tail is build.sh's argument list, in build.sh's \
             order"
    );
}

#[tokio::test]
async fn narrate_built_swaps_the_verb_and_the_severity_under_dry_run() {
    use crate::events::{Severity, StageEvent};
    use crate::paths::Paths;
    use crate::stages::{EventSink, StageOptions};
    use std::sync::Mutex as StdMutex;
    use tokio_util::sync::CancellationToken;

    for dry_run in [false, true] {
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let s = seen.clone();
        let sink: EventSink = Arc::new(move |ev| s.lock().unwrap().push(ev));
        let ctx = StageCtx::new(
            Paths::new("/nonexistent/sabrage-build-test"),
            StageOptions {
                dry_run,
                ..Default::default()
            },
            sink,
            CancellationToken::new(),
        );
        narrate_built(
            &ctx,
            step::BUILD_OXRSYS,
            dry_run,
            "oxrsys built",
            "would build oxrsys (build-x64)",
        );
        let evs = seen.lock().unwrap().clone();
        let StageEvent::Line { severity, text, .. } = &evs[0] else {
            panic!("expected a Line, got {evs:?}");
        };
        if dry_run {
            assert_eq!(*severity, Severity::Info);
            assert_eq!(text, "would build oxrsys (build-x64)");
            assert!(!text.contains("built"), "a dry run may not say 'built'");
        } else {
            assert_eq!(*severity, Severity::Ok);
            assert_eq!(text, "oxrsys built");
        }
    }
}

/// A [`StageCtx`] with a real executor whose every path lives under `root`.
fn real_ctx_at(root: &Path) -> (StageCtx, Arc<std::sync::Mutex<Vec<StageEvent>>>) {
    use crate::paths::Paths;
    use crate::stages::StageOptions;
    use std::sync::Mutex as StdMutex;
    use tokio_util::sync::CancellationToken;

    let seen = Arc::new(StdMutex::new(Vec::new()));
    let s = seen.clone();
    let sink: EventSink = Arc::new(move |ev| s.lock().unwrap().push(ev));
    let mut paths = Paths::new(root);
    // `Paths::new` derives these from the real `$HOME`; a real-executor
    // test must never be able to reach the developer's own state.
    paths.oxr_appsup = root.join("home/Library/Application Support/OXRSys");
    paths.toml_path = paths.oxr_appsup.join("oxrsys-runtime.toml");
    paths.sabrage_appsup = root.join("home/Library/Application Support/Sabrage");
    let ctx = StageCtx::new(
        paths,
        StageOptions::default(),
        sink,
        CancellationToken::new(),
    );
    (ctx, seen)
}

fn line_texts(seen: &Arc<std::sync::Mutex<Vec<StageEvent>>>) -> Vec<String> {
    seen.lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            StageEvent::Line { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// Stage a real arm64 executable (this test binary) as the "built" helper.
/// Returns `None` when this machine cannot satisfy the precondition, the
/// way `checks/build.rs`'s own arch tests skip.
fn seed_built_helper(ctx: &StageCtx) -> Option<()> {
    use std::os::unix::fs::PermissionsExt;
    let exe = std::env::current_exe().ok()?;
    if !helper_is_arm64(&exe) {
        return None; // not an arm64 build, or no usable lipo
    }
    std::fs::create_dir_all(ctx.paths.oxr_helper_built.parent()?).ok()?;
    std::fs::create_dir_all(ctx.paths.oxr_helper_staged.parent()?).ok()?;
    std::fs::copy(&exe, &ctx.paths.oxr_helper_built).ok()?;
    let mut perms = std::fs::metadata(&ctx.paths.oxr_helper_built)
        .ok()?
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&ctx.paths.oxr_helper_built, perms).ok()?;
    Some(())
}

#[tokio::test]
async fn a_byte_identical_but_non_executable_staged_helper_is_repaired() {
    use std::os::unix::fs::PermissionsExt;

    let root = scratch("staged-not-executable");
    let (ctx, seen) = real_ctx_at(&root);
    if seed_built_helper(&ctx).is_none() {
        std::fs::remove_dir_all(&root).ok();
        return;
    }
    // The exact shape `copy_if_changed` cannot see: right bytes, no +x.
    std::fs::copy(&ctx.paths.oxr_helper_built, &ctx.paths.oxr_helper_staged).unwrap();
    let mut perms = std::fs::metadata(&ctx.paths.oxr_helper_staged)
        .unwrap()
        .permissions();
    perms.set_mode(0o644);
    std::fs::set_permissions(&ctx.paths.oxr_helper_staged, perms).unwrap();
    assert!(!helper_is_arm64(&ctx.paths.oxr_helper_staged));

    stage_encoder_helper(&ctx, false)
        .await
        .expect("the stage repairs it rather than failing");

    assert!(
        helper_is_arm64(&ctx.paths.oxr_helper_staged),
        "doctor's build.helper-arm64 would still FAIL after a successful build"
    );
    let texts = line_texts(&seen);
    // Either layer may be the one that fixed it — `copy_if_changed` repairs
    // a mode mismatch ("installed: …"), this stage's destination-side
    // validation catches whatever that misses ("repaired: …") — but the row
    // must never be the do-nothing "unchanged: …".
    assert!(
        texts
            .iter()
            .any(|t| t.starts_with("installed: ") || t.starts_with("repaired: ")),
        "the repair is reported: {texts:?}"
    );
    assert!(
        !texts.iter().any(|t| t.starts_with("unchanged: ")),
        "a non-executable staged copy is not 'unchanged': {texts:?}"
    );
    assert!(texts
        .iter()
        .any(|t| t == "encoder helper built (arm64) and staged next to the runtime dylib"));
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn a_healthy_staged_helper_keeps_the_unchanged_row_and_is_not_touched() {
    let root = scratch("staged-healthy");
    let (ctx, seen) = real_ctx_at(&root);
    if seed_built_helper(&ctx).is_none() {
        std::fs::remove_dir_all(&root).ok();
        return;
    }
    std::fs::copy(&ctx.paths.oxr_helper_built, &ctx.paths.oxr_helper_staged).unwrap();
    assert!(helper_is_arm64(&ctx.paths.oxr_helper_staged));
    let before = std::fs::metadata(&ctx.paths.oxr_helper_staged)
        .unwrap()
        .modified()
        .unwrap();

    stage_encoder_helper(&ctx, false).await.unwrap();

    let texts = line_texts(&seen);
    assert!(
        texts.iter().any(|t| t.starts_with("unchanged: ")),
        "{texts:?}"
    );
    assert!(
        !texts.iter().any(|t| t.starts_with("repaired: ")),
        "nothing to repair: {texts:?}"
    );
    assert_eq!(
        std::fs::metadata(&ctx.paths.oxr_helper_staged)
            .unwrap()
            .modified()
            .unwrap(),
        before,
        "a healthy staged copy must not be rewritten"
    );
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn an_absent_staged_helper_is_installed_and_left_executable() {
    let root = scratch("staged-absent");
    let (ctx, seen) = real_ctx_at(&root);
    if seed_built_helper(&ctx).is_none() {
        std::fs::remove_dir_all(&root).ok();
        return;
    }
    assert!(!ctx.paths.oxr_helper_staged.exists());

    stage_encoder_helper(&ctx, false).await.unwrap();

    assert!(helper_is_arm64(&ctx.paths.oxr_helper_staged));
    let texts = line_texts(&seen);
    assert!(
        texts.iter().any(|t| t.starts_with("installed: ")),
        "{texts:?}"
    );
    assert!(
        !texts.iter().any(|t| t.starts_with("repaired: ")),
        "a fresh copy already carries the source's mode: {texts:?}"
    );
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn a_dry_run_stages_nothing_and_says_would_build() {
    let ctx = dry_run_ctx();
    // Paths under /nonexistent: no source, no destination, no cmake ran —
    // exactly the fresh-checkout dry run whose postconditions are skipped.
    stage_encoder_helper(&ctx, true).await.unwrap();
    assert!(!ctx.paths.oxr_helper_staged.exists());
}

#[test]
fn staged_helper_unusable_message_names_the_staged_path() {
    let path = Path::new("/repo/ext/oxrsys/build-x64/runtime/oxrsys-encoder-helper");
    assert_eq!(
        staged_helper_unusable_message(path),
        "staged encoder helper is not an arm64 executable () — delete \
             /repo/ext/oxrsys/build-x64/runtime/oxrsys-encoder-helper and re-run ./demo.sh build"
    );
}

fn dry_run_ctx() -> StageCtx {
    use crate::executor::{DryRunExecutor, Executor};
    use crate::paths::Paths;
    use crate::stages::{null_sink, StageOptions};
    use tokio_util::sync::CancellationToken;

    let run_id = uuid::Uuid::new_v4();
    let executor: Arc<dyn Executor> = Arc::new(DryRunExecutor::new(
        run_id,
        null_sink(),
        CancellationToken::new(),
    ));
    StageCtx::with_executor(
        Paths::new("/nonexistent/sabrage-build-test"),
        StageOptions::default(),
        null_sink(),
        CancellationToken::new(),
        executor,
        run_id,
    )
}

#[tokio::test]
async fn run_child_ok_never_spawns_under_dry_run_and_records_a_plan_entry() {
    let ctx = dry_run_ctx();
    let spec = ctx.child("/bin/false", step::BUILD_TOOLS);
    run_child_ok(&ctx, spec)
        .await
        .expect("dry run always succeeds");
    let planned = ctx.executor.planned();
    assert_eq!(planned.len(), 1);
    assert_eq!(planned[0].kind, crate::executor::PlannedKind::Spawn);
}

#[tokio::test]
async fn run_ninja_build_ok_never_spawns_under_dry_run_either() {
    let ctx = dry_run_ctx();
    let spec = ctx.child("/bin/false", step::BUILD_OXRSYS);
    run_ninja_build_ok(&ctx, spec)
        .await
        .expect("dry run always succeeds");
    assert_eq!(ctx.executor.planned().len(), 1);
}

#[tokio::test]
async fn run_child_ok_maps_a_real_failure_to_child_failed_with_no_tail() {
    use crate::executor::{Executor, RealExecutor};
    use crate::paths::Paths;
    use crate::stages::{null_sink, StageOptions};
    use tokio_util::sync::CancellationToken;

    let run_id = uuid::Uuid::new_v4();
    let executor: Arc<dyn Executor> = Arc::new(RealExecutor::new(
        run_id,
        null_sink(),
        CancellationToken::new(),
    ));
    let ctx = StageCtx::with_executor(
        Paths::new("/nonexistent/sabrage-build-test"),
        StageOptions::default(),
        null_sink(),
        CancellationToken::new(),
        executor,
        run_id,
    );
    let spec = ctx
        .child("/bin/sh", step::BUILD_OXRSYS)
        .arg("-c")
        .arg("exit 7");
    let err = run_child_ok(&ctx, spec).await.unwrap_err();
    match err {
        SabrageError::ChildFailed { status, tail, .. } => {
            assert_eq!(status, 7);
            assert!(tail.is_empty());
        }
        other => panic!("expected ChildFailed, got {other:?}"),
    }
}

#[tokio::test]
async fn run_ninja_build_ok_derives_progress_and_forwards_output_on_a_real_run() {
    use crate::executor::{Executor, RealExecutor};
    use crate::paths::Paths;
    use crate::stages::StageOptions;
    use std::sync::Mutex as StdMutex;
    use tokio_util::sync::CancellationToken;

    let seen = Arc::new(StdMutex::new(Vec::new()));
    let s = seen.clone();
    let sink: EventSink = Arc::new(move |ev| s.lock().unwrap().push(ev));
    let run_id = uuid::Uuid::new_v4();
    let executor: Arc<dyn Executor> = Arc::new(RealExecutor::new(
        run_id,
        sink.clone(),
        CancellationToken::new(),
    ));
    let ctx = StageCtx::with_executor(
        Paths::new("/nonexistent/sabrage-build-test"),
        StageOptions::default(),
        sink,
        CancellationToken::new(),
        executor,
        run_id,
    );
    let spec = ctx
        .child("/bin/sh", step::BUILD_OXRSYS)
        .arg("-c")
        .arg("printf '[1/2] Building CXX object a.o\\n'; printf '[2/2] Linking CXX foo\\n'");
    run_ninja_build_ok(&ctx, spec)
        .await
        .expect("script exits 0");

    let evs = seen.lock().unwrap();
    let progress: Vec<(u64, Option<u64>)> = evs
        .iter()
        .filter_map(|e| match e {
            StageEvent::Progress { current, total, .. } => Some((*current, *total)),
            _ => None,
        })
        .collect();
    assert_eq!(progress, vec![(1, Some(2)), (2, Some(2))]);
    // The raw Output chunks still reach the sink unchanged.
    assert!(evs.iter().any(|e| matches!(
        e,
        StageEvent::Output { chunk, .. } if chunk.contains("Building CXX object a.o")
    )));
}
