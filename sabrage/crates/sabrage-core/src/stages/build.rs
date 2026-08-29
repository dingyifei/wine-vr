//! `demo.sh build` — build oxrsys (x86_64 + embedded ALVR core), the native-arm64
//! encoder helper, wineopenxr, and the ALVR dashboard. Idempotent (all four
//! build systems are incremental).
//!
//! Reference: `scripts/demo/build.sh`. Six steps, in order:
//!
//! 1. [`step::BUILD_TOOLS`] — `cmake`, `ninja`, `x86_64-w64-mingw32-gcc` on
//!    `PATH`; `rustup target list --installed` contains `x86_64-apple-darwin`;
//!    `ext/oxrsys/runtime` exists. Each failure has its own verbatim die text
//!    (the shell's `for tool in …; do … || die "$tool missing — …"; done`
//!    aborts on the *first* missing tool, in that exact order — so does this).
//! 2. [`step::BUILD_OXRSYS`] — configure `build-x64` (Ninja, Debug, x86_64,
//!    `OXRSYS_ENABLE_ALVR=ON`) then build.
//! 3. [`step::BUILD_HELPER`] — configure `build-helper-arm64` (thin arm64,
//!    `OXRSYS_BUILD_ENCODER_HELPER=ON`), build the `oxrsys_encoder_helper`
//!    target, verify the product is arm64 ([`crate::util::helper_is_arm64`] —
//!    `arm64e` alone must NOT satisfy), then stage it next to the runtime dylib
//!    with `copy_if_changed`.
//! 4. [`step::BUILD_WINEOPENXR`] — configure + build `ext/wineopenxr/build`
//!    (no explicit generator — cmake's platform default, unlike the two Ninja
//!    trees above).
//! 5. [`step::BUILD_DASHBOARD`] — `cargo build -p alvr_dashboard --release`,
//!    run **in** `ext/ALVR` (native arch, no cross target). The shell's only
//!    explicit `die` in this file lives here.
//! 6. [`step::BUILD_OUTPUTS`] — the seven expected artifacts must all exist.
//!
//! # `PATH`
//!
//! Every child here is spawned with [`crate::process::default_child_path`] as
//! its `PATH` (Homebrew, `/usr/local`, `~/.cargo/bin`, ahead of whatever this
//! process inherited): `demo.sh` runs from a login shell that already has
//! cmake/ninja/mingw/rustup on `PATH`, but a GUI-launched Sabrage may not, and
//! `rustup`/`cargo` specifically live in `~/.cargo/bin`, which a
//! Finder-launched `.app` does not inherit. The tool-gate probes below search
//! that same resolved list (not just this process's own inherited `PATH`) so
//! they never report a false "missing" for a tool the spawn itself would have
//! found.
//!
//! # Cancellation outside the executor
//!
//! [`rustup_gate_message`]'s probe is a bare `tokio::process::Command`, not a
//! [`crate::stages::StageCtx::child`] routed through
//! [`crate::executor::Executor::run_child`] (it is a plain read-only query, not
//! a mutation the executor needs to plan or skip under `--dry-run`) — but a
//! cold `rustup` invocation can itself be slow, so it still needs to notice a
//! Cancel promptly rather than block the whole stage on it. It races the
//! child's output against `ctx.cancel.cancelled()` with `tokio::select!`,
//! exactly like `privilege.rs`'s child helpers of the same shape, and spawns
//! with `kill_on_drop(true)` so losing that race actually kills the child.
//!
//! # No explicit `die` text for the six `cmake` calls
//!
//! Unlike `install.sh`'s `reg add` or this file's own `cargo build`, none of
//! the `cmake`/`cmake --build` invocations has a bespoke `die "…"` — build.sh
//! runs under a bare `set -e`, so the shell just stops on the child's exit
//! code. [`run_child_ok`] is that shape (mirrors `setup.rs`'s helper of the
//! same name and same empty-tail rationale: every line the child printed
//! already reached the event stream as it ran, so re-capturing a tail buys
//! nothing here).
//!
//! # Ninja progress
//!
//! The two `-G Ninja` trees (oxrsys, the encoder helper) get best-effort
//! [`StageEvent::Progress`] derived from ninja's default `[n/m]` status
//! prefix ([`parse_ninja_progress`]). `Executor::run_child`'s sink is fixed at
//! construction (same `Arc` as `ctx.sink` — see `executor.rs`'s module doc),
//! so there is no way to derive a second event stream from it without a
//! second sink. [`run_ninja_build_ok`] gets one by calling
//! [`crate::process::run_ok`] directly on the real-run path only, with a sink
//! that forwards every event to `ctx.sink` unchanged and *additionally* emits
//! Progress for a matching stdout chunk; the dry-run path is untouched
//! (delegates straight to [`run_child_ok`], which is exactly `ctx.executor`),
//! so `--dry-run` still plans instead of acting and nothing here duplicates
//! the plan bookkeeping `Executor::planned()` owns.
//!
//! # Post-build assertions and `--dry-run`
//!
//! `[ -f "$OXR_HELPER_BIN_BUILT" ]`, the arm64 gate, and the final
//! seven-artifact sweep all assert that *this stage's own* cmake/cargo
//! invocations actually produced something. Under `DryRunExecutor` none of
//! them ran, so on a checkout that has genuinely never been built these three
//! checks would `die` for the sole reason that the dry run correctly did not
//! build anything — the exact false negative `setup.rs`'s module doc names
//! for its own postcondition checks. They are therefore skipped (not
//! `die`'d) when [`crate::executor::Executor::is_dry_run`] is true.
//!
//! Two narrative rows follow suit rather than claiming something that did
//! not happen: the helper's `copy_if_changed` outcome swaps to `fixes/
//! helper.rs`'s `restage_helper`-established "would install"/"installed"
//! verb pair for `Copied::Copied` (its `Copied::Unchanged` text is
//! reproduced unconditionally either way — that branch is trustworthy even
//! under dry-run, since the executor's dry-run `copy_if_changed` still does
//! the real byte compare); and the closing "all build outputs present" row
//! is skipped in favor of a plain dry-run notice, since — unlike a copy
//! outcome — file existence has no honest hypothetical phrasing. Every other
//! narrative `info`/`ok` (`"oxrsys built"`, `"wineopenxr built"`, `"ALVR
//! dashboard built"`, the helper's closing line) stays unconditional,
//! matching `install.rs`'s `install_if_changed` convention of not
//! special-casing dry-run for plain narration.

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

// ── verbatim die/message text (build.sh) ─────────────────────────────────────

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

// ── tool gates ────────────────────────────────────────────────────────────────

/// `for tool in cmake ninja x86_64-w64-mingw32-gcc`, in that order.
const REQUIRED_TOOLS: [&str; 3] = ["cmake", "ninja", "x86_64-w64-mingw32-gcc"];

/// `command -v <name>` searched over `search_path` (a colon-joined list, e.g.
/// [`crate::process::default_child_path`]) rather than this process's own
/// inherited `PATH` — see the module doc's `PATH` section.
fn resolve_tool(name: &str, search_path: &str) -> Option<PathBuf> {
    std::env::split_paths(search_path).find_map(|dir| {
        let candidate = dir.join(name);
        is_executable_file(&candidate).then_some(candidate)
    })
}

/// Is `path` an existing file with any execute bit set? (`command -v`
/// semantics for one candidate path.) A private copy — `paths.rs` and
/// `checks/build.rs` each already carry their own; a fourth is cheaper than a
/// shared one three separate task owners would all have had to agree on.
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
/// A missing `rustup` binary and a present-but-target-less one produce the
/// same die text, matching the shell (a failed command substitution pipes
/// empty output into `grep -q`, which fails the same way either way).
///
/// Cancel-aware like `privilege.rs`'s child helpers of the same shape
/// (`run_capturing`): a `tokio::select!` races the child's output against
/// `cancel.cancelled()`, so a Cancel that lands while `rustup` is still
/// spawning its own subprocess (it can be slow on a cold toolchain) returns
/// [`SabrageError::Cancelled`] immediately instead of blocking the caller
/// until `rustup` finishes on its own. `kill_on_drop(true)` means losing that
/// race actually kills the child rather than leaking it.
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

// ── ninja progress ────────────────────────────────────────────────────────────

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

// ── child spec builders ───────────────────────────────────────────────────────

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

// ── spawn helpers (see module doc) ───────────────────────────────────────────

/// Run `spec` through `ctx.executor`, mapping a non-zero real exit to
/// [`SabrageError::ChildFailed`] with an empty tail — see the module doc for
/// why there is no bespoke `die` text to reproduce here and why the tail is
/// deliberately not re-captured (identical rationale to `setup.rs`'s helper
/// of the same name and shape).
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

/// [`run_child_ok`], plus best-effort [`StageEvent::Progress`] derived from
/// `spec`'s stdout (see the module doc's "Ninja progress" section for why
/// this needs its own sink on the real-run path only).
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

// ── the stage ─────────────────────────────────────────────────────────────────

/// Execute the stage.
pub async fn run(ctx: &StageCtx) -> Result<()> {
    let search_path = process::default_child_path();
    let dry_run = ctx.executor.is_dry_run();

    // ── 1. tool gates + submodules ────────────────────────────────────────
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

    // ── 2. oxrsys: build-x64 (Debug, x86_64, ALVR on) ────────────────────────
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
            &[
                "-G",
                "Ninja",
                "-DCMAKE_BUILD_TYPE=Debug",
                "-DCMAKE_OSX_ARCHITECTURES=x86_64",
                "-DOXRSYS_ENABLE_ALVR=ON",
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
            step::BUILD_OXRSYS,
            &ctx.paths.oxr_build,
            &[],
        ),
    )
    .await?;
    ctx.step(step::BUILD_OXRSYS).ok("oxrsys built");

    // ── 3. native-arm64 encoder helper ───────────────────────────────────────
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

    // Postconditions on this stage's own output — skipped, not `die`'d, under
    // a dry run that never actually invoked cmake (see module doc).
    if !dry_run {
        if !ctx.paths.oxr_helper_built.is_file() {
            return Err(ctx.fatal(
                helper_missing_binary_message(&ctx.paths.oxr_helper_built),
                None,
            ));
        }
        if !helper_is_arm64(&ctx.paths.oxr_helper_built) {
            return Err(ctx.fatal(
                helper_wrong_arch_message(&ctx.paths.oxr_helper_built, &ctx.paths.oxr_helper_build),
                None,
            ));
        }
    }
    match ctx
        .executor_for(step::BUILD_HELPER)
        .copy_if_changed(&ctx.paths.oxr_helper_built, &ctx.paths.oxr_helper_staged)
        .await?
    {
        // Trustworthy even under dry-run (the executor's dry-run
        // `copy_if_changed` still does the real byte compare) — reproduced
        // unconditionally, matching `install.rs`'s `install_if_changed`.
        Copied::Unchanged => ctx.step(step::BUILD_HELPER).info(format!(
            "unchanged: {}",
            ctx.paths.oxr_helper_staged.display()
        )),
        // Dry-run gets `fixes/helper.rs`'s "would install" verb (its own
        // `restage_helper` uses the same swap for the same
        // `copy_if_changed` outcome) rather than claiming a copy that did
        // not happen.
        Copied::Copied => {
            let verb = if dry_run {
                "would install"
            } else {
                "installed"
            };
            ctx.step(step::BUILD_HELPER)
                .ok(format!("{verb}: {}", ctx.paths.oxr_helper_staged.display()));
        }
    }
    ctx.step(step::BUILD_HELPER)
        .ok("encoder helper built (arm64) and staged next to the runtime dylib");

    // ── 4. wineopenxr (PE dll via mingw + unix .so) ──────────────────────────
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
    ctx.step(step::BUILD_WINEOPENXR).ok("wineopenxr built");

    // ── 5. ALVR server dashboard (native arch, release) ──────────────────────
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
    ctx.step(step::BUILD_DASHBOARD).ok("ALVR dashboard built");

    // ── 6. final outputs presence sweep ──────────────────────────────────────
    // Same dry-run exemption as the helper's postconditions above — "all
    // build outputs present" is a hard factual claim a dry run cannot
    // honestly make (nothing was actually built), so unlike the narrative
    // "built" rows above, this row is skipped entirely rather than
    // reproduced unconditionally.
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
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sabrage-build-test-{}-{tag}", std::process::id()));
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

    // ── ninja progress parser ────────────────────────────────────────────────

    #[test]
    fn parse_ninja_progress_matches_the_default_status_prefix() {
        assert_eq!(
            parse_ninja_progress("[12/340] Building CXX object foo.cpp.o"),
            Some((12, 340))
        );
        // Leading whitespace tolerated (a chunk boundary could land mid-line
        // padding in principle; harmless either way).
        assert_eq!(
            parse_ninja_progress("  [1/2] Linking CXX foo"),
            Some((1, 2))
        );
        // A configure line, a compiler warning, wineopenxr's Makefiles-style
        // `[ 50%]` — none of these are ninja's shape.
        assert_eq!(parse_ninja_progress("-- Configuring done"), None);
        assert_eq!(
            parse_ninja_progress("foo.cpp:12:3: warning: unused variable"),
            None
        );
        assert_eq!(parse_ninja_progress("[ 50%] Building CXX object foo"), None);
        assert_eq!(parse_ninja_progress(""), None);
        assert_eq!(parse_ninja_progress("[1/]"), None);
        assert_eq!(parse_ninja_progress("[/2]"), None);
        assert_eq!(parse_ninja_progress("[abc/2]"), None);
        assert_eq!(parse_ninja_progress("no brackets at all"), None);
        // ninja's final summary line.
        assert_eq!(
            parse_ninja_progress("[340/340] Linking CXX executable oxrsys-runtime"),
            Some((340, 340))
        );
    }

    // ── tool gate, fake PATH dir ─────────────────────────────────────────────

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
        assert_eq!(
            missing_tool_message("cmake"),
            "cmake missing — brew install cmake ninja mingw-w64"
        );
        assert_eq!(
            missing_tool_message("ninja"),
            "ninja missing — brew install cmake ninja mingw-w64"
        );
        assert_eq!(
            missing_tool_message("x86_64-w64-mingw32-gcc"),
            "x86_64-w64-mingw32-gcc missing — brew install cmake ninja mingw-w64"
        );
    }

    #[test]
    fn fixed_die_texts_are_verbatim() {
        assert_eq!(
            RUSTUP_TARGET_MISSING_MESSAGE,
            "rustup x86_64-apple-darwin target missing — install rustup via https://rustup.rs \
             and source ~/.cargo/env, then: rustup toolchain install stable && rustup target \
             add x86_64-apple-darwin"
        );
        assert_eq!(
            SUBMODULES_NOT_INITIALIZED_MESSAGE,
            "submodules not initialized — ./demo.sh setup"
        );
        assert_eq!(
            DASHBOARD_BUILD_FAILED_MESSAGE,
            "alvr_dashboard build failed — retry with: (cd ext/ALVR && cargo build -p \
             alvr_dashboard --release)"
        );
    }

    // ── rustup gate ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn rustup_gate_missing_binary_dies() {
        let dir = scratch("rustup-absent");
        let search_path = dir.display().to_string();
        assert_eq!(
            rustup_gate_message(&search_path, &CancellationToken::new())
                .await
                .unwrap(),
            Some(RUSTUP_TARGET_MISSING_MESSAGE)
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn rustup_gate_missing_target_dies() {
        let dir = scratch("rustup-no-target");
        write_fake_tool(&dir, "rustup", "#!/bin/sh\necho aarch64-apple-darwin\n");
        let search_path = dir.display().to_string();
        assert_eq!(
            rustup_gate_message(&search_path, &CancellationToken::new())
                .await
                .unwrap(),
            Some(RUSTUP_TARGET_MISSING_MESSAGE)
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn rustup_gate_passes_when_the_target_is_installed() {
        let dir = scratch("rustup-present");
        write_fake_tool(
            &dir,
            "rustup",
            "#!/bin/sh\necho aarch64-apple-darwin\necho x86_64-apple-darwin\n",
        );
        let search_path = dir.display().to_string();
        assert_eq!(
            rustup_gate_message(&search_path, &CancellationToken::new())
                .await
                .unwrap(),
            None
        );
        std::fs::remove_dir_all(&dir).ok();
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

    // ── encoder-helper arch-gate message texts ──────────────────────────────

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

    // ── configure_spec / build_spec argv shape ──────────────────────────────

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

    // ── run_child_ok / run_ninja_build_ok, dry-run and real ─────────────────

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
}
