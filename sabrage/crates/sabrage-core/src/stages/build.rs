//! `demo.sh build` — build oxrsys (x86_64 + embedded ALVR core), the native-arm64
//! encoder helper, wineopenxr, and the ALVR dashboard. Idempotent (all four
//! build systems are incremental).
//!
//! Reference: `scripts/demo/build.sh`. Six steps, in order:
//! [`step::BUILD_TOOLS`], [`step::BUILD_OXRSYS`], [`step::BUILD_HELPER`],
//! [`step::BUILD_WINEOPENXR`], [`step::BUILD_DASHBOARD`], [`step::BUILD_OUTPUTS`].
//!
//! Children are spawned with [`crate::process::default_child_path`]; the tool gates
//! probe that same list, so a Finder-launched `.app` missing `~/.cargo/bin` never
//! reports a false "missing" for a tool the spawn finds.
//!
//! Under `--dry-run` nothing is compiled: the helper postconditions, destination-side
//! validation, and seven-artifact sweep are skipped, and no row claims a build
//! (tests::a_dry_run_stages_nothing_and_says_would_build,
//! tests::narrate_built_swaps_the_verb_and_the_severity_under_dry_run).
//!
//! The staged helper is validated at its *destination*, which build.sh never checks:
//! a staged copy with the right bytes but no execute bit still FAILs doctor's
//! `build.helper-arm64`, so `stage_encoder_helper` re-validates and re-copies
//! (tests::a_byte_identical_but_non_executable_staged_helper_is_repaired).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::error::{Result, SabrageError};
use crate::events::{step, StageEvent, StepId, Stream};
use crate::executor::Copied;
use crate::process::{self, ChildSpec};
use crate::stages::{EventSink, StageCtx};
use crate::util::{helper_is_arm64, lipo_archs_stdout};

/// `die "rustup x86_64-apple-darwin target missing — …"`.
const RUSTUP_TARGET_MISSING_MESSAGE: &str = "rustup x86_64-apple-darwin target missing — install rustup via https://rustup.rs and source ~/.cargo/env, then: rustup toolchain install stable && rustup target add x86_64-apple-darwin";

/// `die "submodules not initialized — ./demo.sh setup"`.
const SUBMODULES_NOT_INITIALIZED_MESSAGE: &str = "submodules not initialized — ./demo.sh setup";

/// `die "alvr_dashboard build failed — retry with: …"` — the shell's only
/// explicit `die` in this file.
const DASHBOARD_BUILD_FAILED_MESSAGE: &str = "alvr_dashboard build failed — retry with: (cd ext/ALVR && cargo build -p alvr_dashboard --release)";

/// `die "$tool missing — brew install cmake ninja mingw-w64"`.
fn missing_tool_message(tool: &str) -> String {
    format!("{tool} missing — brew install cmake ninja mingw-w64")
}

/// `die "encoder helper build produced no binary at $OXR_HELPER_BIN_BUILT"`.
fn helper_missing_binary_message(path: &Path) -> String {
    format!(
        "encoder helper build produced no binary at {}",
        path.display()
    )
}

/// `die "encoder helper is not an arm64 executable ($(lipo -archs "$OXR_HELPER_BIN_BUILT" 2>/dev/null)) — delete $OXR_HELPER_BUILD and re-run ./demo.sh build"`.
fn helper_wrong_arch_message(built: &Path, helper_build_dir: &Path) -> String {
    format!(
        "encoder helper is not an arm64 executable ({}) — delete {} and re-run ./demo.sh build",
        lipo_archs_stdout(built),
        helper_build_dir.display()
    )
}

/// `die "expected build output missing: $f"`.
fn missing_output_message(path: &Path) -> String {
    format!("expected build output missing: {}", path.display())
}

/// No shell counterpart: build.sh never looks at the staged copy at all (see
/// the module doc). Emitted only when a re-copy could not make the staged file
/// pass `helper_is_arm64` either.
fn staged_helper_unusable_message(staged: &Path) -> String {
    format!(
        "staged encoder helper is not an arm64 executable ({}) — delete {} and re-run ./demo.sh build",
        lipo_archs_stdout(staged),
        staged.display()
    )
}

/// `cmake -S "$OXRSYS" -B "$OXR_BUILD" …` (build.sh), identical argument order.
///
/// `-DOXRSYS_BUILD_ENCODER_HELPER=OFF` (appended by `oxrsys_x64_configure_args`)
/// repairs a `build-x64` cache stuck at `ON` — CMake `option()` cannot clear it, and it
/// re-fatals on the thin-arm64 gate (r1:A5-2, tests::the_x64_configure_spec_renders_the_helper_off_flag).
const OXRSYS_X64_CONFIGURE_ARGS: [&str; 5] = [
    "-G",
    "Ninja",
    "-DCMAKE_BUILD_TYPE=Debug",
    "-DCMAKE_OSX_ARCHITECTURES=x86_64",
    "-DOXRSYS_ENABLE_ALVR=ON",
];

/// The cache-repair flag above, appended to [`OXRSYS_X64_CONFIGURE_ARGS`].
/// Separate only so the two halves can be asserted independently.
const HELPER_OFF_ARG: &str = "-DOXRSYS_BUILD_ENCODER_HELPER=OFF";

/// [`OXRSYS_X64_CONFIGURE_ARGS`] + [`HELPER_OFF_ARG`], in build.sh's order.
fn oxrsys_x64_configure_args() -> Vec<&'static str> {
    let mut args = OXRSYS_X64_CONFIGURE_ARGS.to_vec();
    args.push(HELPER_OFF_ARG);
    args
}

/// `ok(built)` on a real run; `info(would)` under `--dry-run`, where nothing
/// was compiled and an `Ok "… built"` row would contradict the stage's own
/// closing "nothing was built" notice.
fn narrate_built(ctx: &StageCtx, step_id: StepId, dry_run: bool, built: &str, would: &str) {
    let st = ctx.step(step_id);
    if dry_run {
        st.info(would);
    } else {
        st.ok(built);
    }
}

/// build.sh's `for tool in cmake ninja x86_64-w64-mingw32-gcc`, in that order.
const REQUIRED_TOOLS: [&str; 3] = ["cmake", "ninja", "x86_64-w64-mingw32-gcc"];

/// `command -v <name>` searched over `search_path` (a colon-joined list, e.g.
/// [`crate::process::default_child_path`]) rather than this process's own
/// inherited `PATH`.
fn resolve_tool(name: &str, search_path: &str) -> Option<PathBuf> {
    std::env::split_paths(search_path).find_map(|dir| {
        let candidate = dir.join(name);
        is_executable_file(&candidate).then_some(candidate)
    })
}

/// Is `path` an existing file with any execute bit set? (`command -v`
/// semantics for one candidate path.) Deliberately a private copy: `paths.rs`
/// and `checks/build.rs` each keep their own.
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// `Some(die text)` for the first of [`REQUIRED_TOOLS`] not found on
/// `search_path`, in shell order; `None` when all three are present.
fn tool_gate_message(search_path: &str) -> Option<String> {
    REQUIRED_TOOLS
        .iter()
        .find(|tool| resolve_tool(tool, search_path).is_none())
        .map(|tool| missing_tool_message(tool))
}

/// `rustup target list --installed 2>/dev/null | grep -q x86_64-apple-darwin`.
/// A missing `rustup` and a present-but-target-less one produce the same die text,
/// matching the shell (tests::rustup_gate_dies_unless_the_x86_64_target_is_installed).
///
/// Cancel-aware: races the child against `cancel.cancelled()` with
/// `kill_on_drop(true)`, so a Cancel during a cold `rustup` returns
/// [`SabrageError::Cancelled`] at once (tests::rustup_gate_is_cancel_aware_and_kills_the_child).
async fn rustup_gate_message(
    search_path: &str,
    cancel: &CancellationToken,
) -> Result<Option<&'static str>> {
    let Some(bin) = resolve_tool("rustup", search_path) else {
        return Ok(Some(RUSTUP_TARGET_MISSING_MESSAGE));
    };
    let mut cmd = tokio::process::Command::new(&bin);
    cmd.args(["target", "list", "--installed"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => return Ok(Some(RUSTUP_TARGET_MISSING_MESSAGE)),
    };
    let has_target = tokio::select! {
        out = child.wait_with_output() => {
            out.map(|o| String::from_utf8_lossy(&o.stdout).contains("x86_64-apple-darwin"))
                .unwrap_or(false)
        }
        _ = cancel.cancelled() => return Err(SabrageError::Cancelled),
    };
    Ok(if has_target {
        None
    } else {
        Some(RUSTUP_TARGET_MISSING_MESSAGE)
    })
}

/// Best-effort parse of ninja's default status prefix (`[12/340] Building CXX
/// object …`). `None` for anything else — a cmake configure line, a compiler
/// warning, wineopenxr's Makefiles-style `[ 50%]` (that tree has no
/// `-G Ninja`) — this is a decoration, never load-bearing.
fn parse_ninja_progress(line: &str) -> Option<(u64, u64)> {
    let rest = line.trim_start().strip_prefix('[')?;
    let close = rest.find(']')?;
    let (num, den) = rest[..close].split_once('/')?;
    let current: u64 = num.trim().parse().ok()?;
    let total: u64 = den.trim().parse().ok()?;
    Some((current, total))
}

fn configure_spec(
    ctx: &StageCtx,
    cmake_bin: &Path,
    search_path: &str,
    step: StepId,
    source: &Path,
    build_dir: &Path,
    extra_args: &[&str],
) -> ChildSpec {
    ctx.child(cmake_bin.to_path_buf(), step)
        .arg("-S")
        .arg(source.to_path_buf())
        .arg("-B")
        .arg(build_dir.to_path_buf())
        .args(extra_args.iter().copied())
        .env_path(search_path.to_string())
}

fn build_spec(
    ctx: &StageCtx,
    cmake_bin: &Path,
    search_path: &str,
    step: StepId,
    build_dir: &Path,
    extra_args: &[&str],
) -> ChildSpec {
    ctx.child(cmake_bin.to_path_buf(), step)
        .arg("--build")
        .arg(build_dir.to_path_buf())
        .args(extra_args.iter().copied())
        .arg("-j8")
        .env_path(search_path.to_string())
}

/// Run `spec` through `ctx.executor`, mapping a non-zero real exit to
/// [`SabrageError::ChildFailed`] with an empty tail: build.sh runs under a bare
/// `set -e` and has no bespoke `die` text for the `cmake` calls, and every line
/// the child printed already reached the event stream, so re-capturing a tail
/// buys nothing (tests::run_child_ok_maps_a_real_failure_to_child_failed_with_no_tail).
async fn run_child_ok(ctx: &StageCtx, spec: ChildSpec) -> Result<()> {
    let status = ctx.executor.run_child(&spec).await?;
    if status.success() {
        Ok(())
    } else {
        Err(SabrageError::ChildFailed {
            argv0: spec.argv0(),
            status: process::exit_code_of(status),
            tail: Vec::new(),
        })
    }
}

/// `run_child_ok`, plus best-effort [`StageEvent::Progress`] derived from
/// `spec`'s stdout. It needs its own sink because `Executor::run_child`'s is
/// fixed at construction, and it takes that path only on a real run so
/// `--dry-run` keeps `Executor::planned()`'s bookkeeping
/// (tests::run_ninja_build_ok_derives_progress_and_forwards_output_on_a_real_run,
/// tests::run_ninja_build_ok_never_spawns_under_dry_run_either).
async fn run_ninja_build_ok(ctx: &StageCtx, spec: ChildSpec) -> Result<()> {
    if ctx.executor.is_dry_run() {
        return run_child_ok(ctx, spec).await;
    }
    let run_id = ctx.run_id;
    let step_owned = spec.step.to_string();
    let outer_sink = ctx.sink.clone();
    let sink: EventSink = Arc::new(move |ev: StageEvent| {
        if let StageEvent::Output {
            step: ref s,
            stream: Stream::Stdout,
            ref chunk,
            ..
        } = ev
        {
            if s == &step_owned {
                if let Some((current, total)) = parse_ninja_progress(chunk) {
                    outer_sink(StageEvent::Progress {
                        run_id,
                        step: step_owned.clone(),
                        label: "ninja".to_string(),
                        current,
                        total: Some(total),
                    });
                }
            }
        }
        outer_sink(ev);
    });
    process::run_ok(&spec, &sink, &ctx.cancel).await?;
    Ok(())
}

/// Arch-gate the freshly built helper, stage it next to the runtime dylib, and
/// — the part the shell has no counterpart for — make sure the *staged* file
/// is what doctor and run's preflight will demand of it.
///
/// The source-side gates are skipped (not `die`'d) under a dry run that never
/// invoked cmake; so is the destination-side validation, whose subject may not
/// exist at all there. See the module doc for both.
async fn stage_encoder_helper(ctx: &StageCtx, dry_run: bool) -> Result<()> {
    let built = &ctx.paths.oxr_helper_built;
    let staged = &ctx.paths.oxr_helper_staged;

    // Postconditions on this stage's own output — skipped, not `die`'d, under
    // a dry run that never actually invoked cmake (see module doc).
    if !dry_run {
        if !built.is_file() {
            return Err(ctx.fatal(helper_missing_binary_message(built), None));
        }
        if !helper_is_arm64(built) {
            return Err(ctx.fatal(
                helper_wrong_arch_message(built, &ctx.paths.oxr_helper_build),
                None,
            ));
        }
    }

    let exec = ctx.executor_for(step::BUILD_HELPER);
    match exec.copy_if_changed(built, staged).await? {
        // Trustworthy even under dry-run (the executor's dry-run
        // `copy_if_changed` still does the real byte compare) — reproduced
        // unconditionally, matching `install.rs`'s `install_if_changed`.
        Copied::Unchanged => ctx
            .step(step::BUILD_HELPER)
            .info(format!("unchanged: {}", staged.display())),
        // Dry run takes `fixes/helper.rs`'s "would install" verb rather than
        // claiming a copy that did not happen.
        Copied::Copied => {
            let verb = if dry_run {
                "would install"
            } else {
                "installed"
            };
            ctx.step(step::BUILD_HELPER)
                .ok(format!("{verb}: {}", staged.display()));
        }
    }

    // `copy_if_changed` compares bytes, so `Unchanged` says nothing about the
    // destination's mode, and a staged helper without its execute bit FAILs
    // doctor's `build.helper-arm64` (tests::a_byte_identical_but_non_executable_staged_helper_is_repaired).
    // Remove first: a fresh `std::fs::copy` takes the source's mode.
    if !dry_run && !helper_is_arm64(staged) {
        exec.remove_file(staged).await?;
        exec.copy_if_changed(built, staged).await?;
        if !helper_is_arm64(staged) {
            return Err(ctx.fatal(staged_helper_unusable_message(staged), None));
        }
        ctx.step(step::BUILD_HELPER).ok(format!(
            "repaired: {} (the staged copy was not an executable arm64 binary)",
            staged.display()
        ));
    }

    narrate_built(
        ctx,
        step::BUILD_HELPER,
        dry_run,
        "encoder helper built (arm64) and staged next to the runtime dylib",
        "would build the encoder helper and stage it next to the runtime dylib",
    );
    Ok(())
}

/// Execute the stage.
pub async fn run(ctx: &StageCtx) -> Result<()> {
    let search_path = process::default_child_path();
    let dry_run = ctx.executor.is_dry_run();

    if let Some(msg) = tool_gate_message(&search_path) {
        return Err(ctx.fatal(msg, None));
    }
    if let Some(msg) = rustup_gate_message(&search_path, &ctx.cancel).await? {
        return Err(ctx.fatal(msg, None));
    }
    if !ctx.paths.oxrsys.join("runtime").is_dir() {
        return Err(ctx.fatal(SUBMODULES_NOT_INITIALIZED_MESSAGE, None));
    }
    let cmake_bin = resolve_tool("cmake", &search_path).expect("checked by tool_gate_message");

    ctx.step(step::BUILD_OXRSYS)
        .info("building oxrsys (build-x64: Ninja, Debug, x86_64, ALVR on)...");
    run_child_ok(
        ctx,
        configure_spec(
            ctx,
            &cmake_bin,
            &search_path,
            step::BUILD_OXRSYS,
            &ctx.paths.oxrsys,
            &ctx.paths.oxr_build,
            &oxrsys_x64_configure_args(),
        ),
    )
    .await?;
    run_ninja_build_ok(
        ctx,
        build_spec(
            ctx,
            &cmake_bin,
            &search_path,
            step::BUILD_OXRSYS,
            &ctx.paths.oxr_build,
            &[],
        ),
    )
    .await?;
    narrate_built(
        ctx,
        step::BUILD_OXRSYS,
        dry_run,
        "oxrsys built",
        "would build oxrsys (build-x64)",
    );

    ctx.step(step::BUILD_HELPER)
        .info("building oxrsys encoder helper (build-helper-arm64: Ninja, Debug, arm64)...");
    run_child_ok(
        ctx,
        configure_spec(
            ctx,
            &cmake_bin,
            &search_path,
            step::BUILD_HELPER,
            &ctx.paths.oxrsys,
            &ctx.paths.oxr_helper_build,
            &[
                "-G",
                "Ninja",
                "-DCMAKE_BUILD_TYPE=Debug",
                "-DCMAKE_OSX_ARCHITECTURES=arm64",
                "-DOXRSYS_BUILD_ENCODER_HELPER=ON",
            ],
        ),
    )
    .await?;
    run_ninja_build_ok(
        ctx,
        build_spec(
            ctx,
            &cmake_bin,
            &search_path,
            step::BUILD_HELPER,
            &ctx.paths.oxr_helper_build,
            &["--target", "oxrsys_encoder_helper"],
        ),
    )
    .await?;

    stage_encoder_helper(ctx, dry_run).await?;

    ctx.step(step::BUILD_WINEOPENXR)
        .info("building wineopenxr (PE dll via mingw + unix .so)...");
    let woxr_build = ctx.paths.woxr.join("build");
    run_child_ok(
        ctx,
        configure_spec(
            ctx,
            &cmake_bin,
            &search_path,
            step::BUILD_WINEOPENXR,
            &ctx.paths.woxr,
            &woxr_build,
            &[],
        ),
    )
    .await?;
    run_child_ok(
        ctx,
        build_spec(
            ctx,
            &cmake_bin,
            &search_path,
            step::BUILD_WINEOPENXR,
            &woxr_build,
            &[],
        ),
    )
    .await?;
    narrate_built(
        ctx,
        step::BUILD_WINEOPENXR,
        dry_run,
        "wineopenxr built",
        "would build wineopenxr",
    );

    ctx.step(step::BUILD_DASHBOARD)
        .info("building ALVR server dashboard (release)...");
    let cargo_bin = resolve_tool("cargo", &search_path).unwrap_or_else(|| PathBuf::from("cargo"));
    let cargo_spec = ctx
        .child(cargo_bin, step::BUILD_DASHBOARD)
        .args(["build", "-p", "alvr_dashboard", "--release"])
        .cwd(ctx.paths.alvr.clone())
        .env_path(search_path.clone());
    let status = ctx.executor.run_child(&cargo_spec).await?;
    if !status.success() {
        return Err(ctx.fatal(DASHBOARD_BUILD_FAILED_MESSAGE, None));
    }
    narrate_built(
        ctx,
        step::BUILD_DASHBOARD,
        dry_run,
        "ALVR dashboard built",
        "would build the ALVR dashboard",
    );

    // "all build outputs present" is a hard factual claim a dry run cannot
    // honestly make, so unlike the narrative "built" rows this one is skipped
    // entirely rather than swapped to a future-tense verb.
    if dry_run {
        ctx.step(step::BUILD_OUTPUTS)
            .info("build-output presence sweep skipped under --dry-run (nothing was built)");
    } else {
        for f in [
            &ctx.paths.oxr_dylib,
            &ctx.paths.oxr_alvr_dylib,
            &ctx.paths.oxr_runtime_json,
            &ctx.paths.oxr_helper_staged,
            &ctx.paths.woxr_dll,
            &ctx.paths.woxr_so,
            &ctx.paths.alvr_dashboard,
        ] {
            if !f.is_file() {
                return Err(ctx.fatal(missing_output_message(f), None));
            }
        }
        ctx.step(step::BUILD_OUTPUTS)
            .ok("all build outputs present");
    }

    Ok(())
    // The "build complete — next: …" line is the CLI renderer's, not this
    // stage's — see the frame's StageEvent::StageFinished bracketing (and
    // install.rs's identical note at its own equivalent line).
}

#[cfg(test)]
mod tests;
