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
//! The narrative rows follow suit rather than claiming something that did
//! not happen: the helper's `copy_if_changed` outcome swaps to `fixes/
//! helper.rs`'s `restage_helper`-established "would install"/"installed"
//! verb pair for `Copied::Copied` (its `Copied::Unchanged` text is
//! reproduced unconditionally either way — that branch is trustworthy even
//! under dry-run, since the executor's dry-run `copy_if_changed` still does
//! the real byte compare); the closing "all build outputs present" row
//! is skipped in favor of a plain dry-run notice, since — unlike a copy
//! outcome — file existence has no honest hypothetical phrasing; and each of
//! the four per-component completion rows (`"oxrsys built"`, the helper's
//! closing line, `"wineopenxr built"`, `"ALVR dashboard built"`) swaps to its
//! own future-tense `info` ([`narrate_built`]) under a dry run, so no `Ok` row
//! ever says "built" in the same invocation that ends with "nothing was
//! built".
//!
//! # The staged helper is validated at its *destination*, not only at its source
//!
//! A staged helper carrying the right bytes with its execute bit lost (an
//! `unzip`ped or `rsync -rt --chmod`ed tree, a restore from a backup that
//! dropped modes) is *not* installed — `checks/build.rs`'s `build.helper-arm64`
//! and run's preflight both require `[ -x ]` — yet build used to arch-gate only
//! `oxr_helper_built`, its own *source*, and then report success, leaving
//! doctor FAILing with no stage able to repair it (a byte-only comparison sees
//! nothing to do). [`crate::executor::Executor::copy_if_changed`] now repairs a
//! mode mismatch itself, so the common case is fixed one layer down;
//! [`stage_encoder_helper`] still re-validates `oxr_helper_staged` after the
//! copy and, when it *still* fails, removes and re-copies it (a fresh copy takes
//! the source's mode) before giving up with a remedy naming the staged path —
//! this stage may not report a build as complete while the artifact it just
//! staged is one doctor FAIL. The shell has neither half (`lib.sh`'s
//! `install_if_changed` is `cmp -s` + `cp`); both are additive — no shell text
//! changes, and a healthy tree behaves identically.

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

// ── the build-x64 configure arguments ────────────────────────────────────────

/// `cmake -S "$OXRSYS" -B "$OXR_BUILD" …` (build.sh).
///
/// `-DOXRSYS_BUILD_ENCODER_HELPER=OFF` is load-bearing, not decoration: CMake
/// `option()` is a no-op against an existing cache entry, so a `build-x64` tree
/// that was ever configured with the helper enabled (its default for *every*
/// Apple configure before the arch gate landed, and a failed configure still
/// writes the cache) keeps `ON` forever and re-fatals on the thin-arm64 gate at
/// every retry. Passing it explicitly repairs such a tree in place and makes
/// CLAUDE.md's arch-gate invariant (2) — `OXRSYS_BUILD_ENCODER_HELPER:BOOL=OFF`
/// in `build-x64` — true by construction. Kept identical, and in the same
/// position, to build.sh's own argument list.
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

// ── narrative completion rows ────────────────────────────────────────────────

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

// ── encoder-helper staging ────────────────────────────────────────────────────

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
                .ok(format!("{verb}: {}", staged.display()));
        }
    }

    // `copy_if_changed` compares bytes, so `Unchanged` says nothing about the
    // destination's mode — and a staged helper without its execute bit fails
    // `build.helper-arm64` while build reports success (see the module doc).
    // Remove and re-copy: `std::fs::copy` gives the new file the source's mode.
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

    stage_encoder_helper(ctx, dry_run).await?;

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
    narrate_built(
        ctx,
        step::BUILD_WINEOPENXR,
        dry_run,
        "wineopenxr built",
        "would build wineopenxr",
    );

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
    narrate_built(
        ctx,
        step::BUILD_DASHBOARD,
        dry_run,
        "ALVR dashboard built",
        "would build the ALVR dashboard",
    );

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

    // ── build-x64 configure arguments ────────────────────────────────────────

    #[test]
    fn the_x64_configure_forces_the_encoder_helper_option_off() {
        // CLAUDE.md's arch gate requires `OXRSYS_BUILD_ENCODER_HELPER:BOOL=OFF`
        // in build-x64's cache; CMake `option()` cannot establish that against a
        // tree already cached with ON, so the flag has to be passed explicitly.
        let args = oxrsys_x64_configure_args();
        assert_eq!(
            args,
            vec![
                "-G",
                "Ninja",
                "-DCMAKE_BUILD_TYPE=Debug",
                "-DCMAKE_OSX_ARCHITECTURES=x86_64",
                "-DOXRSYS_ENABLE_ALVR=ON",
                "-DOXRSYS_BUILD_ENCODER_HELPER=OFF",
            ],
            "build.sh's argument list, in build.sh's order"
        );
    }

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
        assert!(
            spec.display().ends_with(
                "-G Ninja -DCMAKE_BUILD_TYPE=Debug -DCMAKE_OSX_ARCHITECTURES=x86_64 \
                 -DOXRSYS_ENABLE_ALVR=ON -DOXRSYS_BUILD_ENCODER_HELPER=OFF"
            ),
            "{}",
            spec.display()
        );
    }

    // ── narrative rows never claim a build a dry run did not do ──────────────

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

    // ── the staged helper is validated at its destination ────────────────────

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
