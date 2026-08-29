//! `demo.sh setup` — one-time fetch of sources + pinned binaries and config
//! bootstrap. Idempotent, no sudo.
//!
//! Reference: `scripts/demo/setup.sh`. Four steps, in order:
//!
//! 1. [`step::SETUP_SUBMODULES`] — `git submodule update --init` for
//!    `ext/{oxrsys,wineopenxr,ALVR}`, then wineopenxr's nested submodules with
//!    `--filter=blob:none` and ALVR's `openvr`; die if `openvr_driver.h` did not
//!    materialize; grep `ext/ALVR/alvr/server_core/src/connection.rs` for
//!    `is_streaming_nonblocking` to prove the oxrsys patch set is checked out.
//! 2. [`step::SETUP_PINNED`] — Goldberg dll (a present-but-unpinned dll is kept
//!    with a warn, never re-fetched) and the DXMT tarball: fetch, `rm -rf` the
//!    artifact dir, extract into `ext/`, re-check completeness, then write the
//!    `.sha256` provenance marker ([`crate::util::contract_marker_bytes`]).
//! 3. [`step::SETUP_CONFIG`] — **write-once** `oxrsys-runtime.toml` from
//!    [`crate::util::toml_template`]; an existing file is never overwritten,
//!    only reported (and warned about when `protocol` is not `alvr`).
//! 4. [`step::SETUP_GAME`] — Beat Saber presence probe; skipped entirely when
//!    neither `--bottle` nor `--bs-dir` was given.
//!
//! Byte contracts: the toml template is [`crate::util::toml_template`] verbatim,
//! and the marker file is the pin plus one newline — nothing else.
//!
//! # Every mutation through the executor, and what that means for `--dry-run`
//!
//! `git submodule update`, the two pinned fetches, the DXMT `rm -rf` +
//! extraction, and the config write all go through `ctx.executor` (narrowed to
//! their step with [`crate::stages::StageCtx::executor_for`]) so a dry run
//! plans instead of acting — [`crate::executor::DryRunExecutor`] never actually
//! spawns `git`, `curl`, or `tar`, and never writes a byte.
//!
//! That means the shell's *postcondition* assertions — `openvr_driver.h`
//! materialized, the patch-set grep, the extracted DXMT set being complete —
//! would always read false after a planned-but-not-executed mutation. Rather
//! than report those as failures a dry run could not itself have caused, each
//! such check is skipped (never `die`s) when
//! [`crate::executor::Executor::is_dry_run`] is true; the check still runs for
//! real when the precondition already happens to hold (a dry run over an
//! already-set-up checkout still reports truthfully), and the accompanying `ok`
//! row is still emitted, matching [`crate::executor::DryRunExecutor`]'s own
//! convention of optimistically reporting success for a planned mutation.

use std::path::Path;

use crate::contract::contract;
use crate::error::{Result, SabrageError};
use crate::events::step;
use crate::process::{self, ChildSpec};
use crate::stages::{require_bottle, StageCtx};
use crate::util;

/// Execute the stage.
pub async fn run(ctx: &StageCtx) -> Result<()> {
    setup_submodules(ctx).await?;
    setup_pinned(ctx).await?;
    setup_config(ctx).await?;
    setup_game(ctx).await?;
    Ok(())
}

// ── 1. submodules ────────────────────────────────────────────────────────────

/// A `git -C <dir> submodule update <rest...>` spec, attributed to
/// [`step::SETUP_SUBMODULES`].
fn git_submodule_spec(ctx: &StageCtx, dir: &Path, rest: &[&str]) -> ChildSpec {
    ctx.child("git", step::SETUP_SUBMODULES)
        .arg("-C")
        .arg(dir.to_path_buf())
        .args(rest.iter().copied())
}

async fn setup_submodules(ctx: &StageCtx) -> Result<()> {
    let st = ctx.step(step::SETUP_SUBMODULES);
    st.info("initializing submodules (first fetch is large: ALVR + the wine source tree)...");

    // git -C "$ROOT" submodule update --init ext/oxrsys ext/wineopenxr ext/ALVR
    run_child_ok(
        ctx,
        git_submodule_spec(
            ctx,
            &ctx.paths.root,
            &[
                "submodule",
                "update",
                "--init",
                "ext/oxrsys",
                "ext/wineopenxr",
                "ext/ALVR",
            ],
        ),
    )
    .await?;
    // blob:none keeps the wine-mirror clone to tens of MB instead of full
    // history: git -C "$WOXR" submodule update --init --filter=blob:none
    run_child_ok(
        ctx,
        git_submodule_spec(
            ctx,
            &ctx.paths.woxr,
            &["submodule", "update", "--init", "--filter=blob:none"],
        ),
    )
    .await?;
    // openvr (alvr_session build dep): git -C "$ALVR" submodule update --init
    run_child_ok(
        ctx,
        git_submodule_spec(ctx, &ctx.paths.alvr, &["submodule", "update", "--init"]),
    )
    .await?;

    let openvr_header = ctx.paths.alvr.join("openvr/headers/openvr_driver.h");
    if openvr_header.is_file() || ctx.executor.is_dry_run() {
        st.ok("submodules ready");
    } else {
        return Err(st.fatal(
            "ALVR openvr submodule did not materialize — check network/auth and re-run setup",
            None,
        ));
    }

    // grep -q is_streaming_nonblocking "$ALVR/alvr/server_core/src/connection.rs"
    let connection_rs = ctx.paths.alvr.join("alvr/server_core/src/connection.rs");
    let patchset_present = std::fs::read(&connection_rs)
        .map(|bytes| String::from_utf8_lossy(&bytes).contains("is_streaming_nonblocking"))
        .unwrap_or(false);
    if patchset_present || ctx.executor.is_dry_run() {
        st.ok("ALVR checkout carries the oxrsys patch set (branch oxrsys-v20.14.1)");
        Ok(())
    } else {
        Err(st.fatal(
            format!(
                "ALVR submodule is missing the oxrsys patches — run: git -C \"{}\" submodule update --checkout ext/ALVR",
                ctx.paths.root.display()
            ),
            None,
        ))
    }
}

// ── 2. pinned binaries ───────────────────────────────────────────────────────

async fn setup_pinned(ctx: &StageCtx) -> Result<()> {
    let exec = ctx.executor_for(step::SETUP_PINNED);
    let deps = &contract().deps;

    // Goldberg Steam emulator dll: a present-but-wrong-hash file is kept, never
    // silently replaced.
    let gbe = &ctx.paths.gbe_dll;
    if gbe.is_file() && !util::file_sha256_matches(gbe, &deps.gbe_dll_sha256) {
        ctx.step(step::SETUP_PINNED).warn(format!(
            "Goldberg dll present with a non-pinned hash — keeping it (delete {} to re-fetch the pinned build)",
            gbe.display()
        ));
    } else {
        let url = format!("{}/{}", deps.url, deps.gbe_dll_asset);
        exec.download(
            &url,
            gbe,
            &deps.gbe_dll_sha256,
            "Goldberg Steam emulator dll",
        )
        .await?;
    }

    // DXMT fork artifacts.
    if util::dxmt_ok(&ctx.paths) {
        ctx.step(step::SETUP_PINNED)
            .info("already present: dxmt-artifacts (sha256 marker matches)");
        return Ok(());
    }
    let tgz_dest = ctx
        .paths
        .root
        .join("third_party/downloads")
        .join(&deps.dxmt_tgz_asset);
    let tgz_url = format!("{}/{}", deps.url, deps.dxmt_tgz_asset);
    exec.download(
        &tgz_url,
        &tgz_dest,
        &deps.dxmt_tgz_sha256,
        "DXMT fork artifacts",
    )
    .await?;

    exec.remove_dir_all(&ctx.paths.dxmt_art).await?;

    let ext_dir = ctx.paths.root.join("ext");
    match exec.tar_xzf(&tgz_dest, &ext_dir).await {
        Ok(()) => {}
        Err(SabrageError::Cancelled) => return Err(SabrageError::Cancelled),
        Err(_) => {
            return Err(ctx
                .step(step::SETUP_PINNED)
                .fatal("extraction failed", None))
        }
    }

    if util::dxmt_files_ok(&ctx.paths) || exec.is_dry_run() {
        let marker_path = ctx.paths.dxmt_art.join(".sha256");
        let marker_bytes = util::contract_marker_bytes(&deps.dxmt_tgz_sha256);
        exec.write_atomic(&marker_path, marker_bytes.as_bytes())
            .await?;
        ctx.step(step::SETUP_PINNED)
            .ok("extracted ext/dxmt-artifacts (provenance marker written)");
        Ok(())
    } else {
        Err(ctx.step(step::SETUP_PINNED).fatal(
            format!(
                "extracted dxmt-artifacts are incomplete — delete {} and re-run setup",
                ctx.paths.dxmt_art.display()
            ),
            None,
        ))
    }
}

// ── 3. runtime config ────────────────────────────────────────────────────────

async fn setup_config(ctx: &StageCtx) -> Result<()> {
    let st = ctx.step(step::SETUP_CONFIG);
    let exec = ctx.executor_for(step::SETUP_CONFIG);
    exec.create_dir_all(&ctx.paths.oxr_appsup).await?;

    if ctx.paths.toml_path.is_file() {
        let text = std::fs::read_to_string(&ctx.paths.toml_path).unwrap_or_default();
        let proto = parse_protocol_awk(&text);
        if proto == "alvr" {
            st.info(format!(
                "config present: {} (protocol=alvr)",
                ctx.paths.toml_path.display()
            ));
        } else {
            st.warn(format!(
                "config present with protocol='{proto}' — the demo needs protocol = \"alvr\"; edit {} yourself (not overwriting)",
                ctx.paths.toml_path.display()
            ));
        }
    } else {
        exec.write_atomic(&ctx.paths.toml_path, util::toml_template().as_bytes())
            .await?;
        st.ok(format!(
            "wrote {} (protocol=alvr, 42 Mbps, encoder_process=auto)",
            ctx.paths.toml_path.display()
        ));
    }

    st.info(format!(
        "note: the embedded ALVR core keeps its session.json under '{}/alvr/' — auto-created on first run, LAN clients auto-trusted",
        ctx.paths.oxr_appsup.display()
    ));
    Ok(())
}

/// `awk -F'"' '/^[[:space:]]*protocol[[:space:]]*=/{print $2; exit}'`.
///
/// Duplicates [`crate::checks::config`]'s private `parse_protocol` (identical
/// algorithm, independently verified against the same awk recipe below)
/// rather than reusing it: that helper is not `pub`, and this file may not
/// edit `checks/`.
fn parse_protocol_awk(toml_text: &str) -> String {
    for line in toml_text.lines() {
        let after_leading_ws = line.trim_start();
        let Some(rest) = after_leading_ws.strip_prefix("protocol") else {
            continue;
        };
        if !rest.trim_start().starts_with('=') {
            continue;
        }
        let mut fields = line.split('"');
        let _before_first_quote = fields.next();
        return fields.next().unwrap_or("").to_string();
    }
    String::new()
}

// ── 4. Beat Saber presence ───────────────────────────────────────────────────

async fn setup_game(ctx: &StageCtx) -> Result<()> {
    let st = ctx.step(step::SETUP_GAME);
    if ctx.opts.bottle_name.is_none() && ctx.opts.bs_dir_override.is_none() {
        st.info(
            "Beat Saber check skipped (no --bottle/--bs-dir given); ./demo.sh doctor will verify it",
        );
        return Ok(());
    }
    // `[ -n "${WINEVR_BOTTLE:-}" ] && require_bottle || { BS_DIR=...; DEPOT_CMD=... }`:
    // when a bottle name was given, require_bottle both dies (verbatim text)
    // on a missing bottle and pins BS_DIR the same way `StageCtx` already did;
    // when only --bs-dir was given, ctx.bs_dir already equals it. Either way
    // the DepotDownloader command is the same formula.
    if ctx.opts.bottle_name.is_some() {
        require_bottle(ctx)?;
    }
    if ctx.bs_dir.join("Beat Saber.exe").is_file() {
        st.ok(format!("Beat Saber found at {}", ctx.bs_dir.display()));
    } else {
        st.warn(format!(
            "Beat Saber 1.29.4 not found at {}",
            ctx.bs_dir.display()
        ));
        st.info("download it with your Steam account (owning Beat Saber):");
        st.info(format!("  {}", contract().depot_command(&ctx.bs_dir)));
    }
    Ok(())
}

// ── shared child-spawn helper ────────────────────────────────────────────────

/// Run `spec` through the stage's executor, mapping a non-zero real exit to
/// [`SabrageError::ChildFailed`].
///
/// The shell aborts on these commands via a bare `set -e` — there is no
/// bespoke `die()` text to reproduce, so this is [`process::run_ok`]'s
/// mapping, minus the output tail: [`crate::executor::Executor::run_child`]
/// returns a plain [`std::process::ExitStatus`], not the tail
/// `spawn_streamed` captures internally, but every line the child printed
/// already reached the event stream (and the log) as it ran. In `--dry-run`,
/// [`crate::executor::DryRunExecutor::run_child`] never spawns and always
/// reports success, so this always returns `Ok` there.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{Severity, StageEvent};
    use crate::executor::{PlannedAction, PlannedKind};
    use crate::paths::Paths;
    use crate::stages::{EventSink, StageOptions};
    use std::sync::{Arc, Mutex as StdMutex};
    use tokio_util::sync::CancellationToken;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sabrage-setup-test-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A [`Paths`] rooted entirely under `fixture` — including `oxr_appsup`/
    /// `toml_path`, which [`Paths::new`] otherwise derives from the real
    /// `$HOME`. A test must never touch the real `~/Library`.
    fn fixture_paths(fixture: &Path) -> Paths {
        let mut paths = Paths::new(fixture);
        paths.oxr_appsup = fixture.join("home/Library/Application Support/OXRSys");
        paths.toml_path = paths.oxr_appsup.join("oxrsys-runtime.toml");
        paths
    }

    fn ctx_with_paths(
        paths: Paths,
        opts: StageOptions,
    ) -> (StageCtx, Arc<StdMutex<Vec<StageEvent>>>) {
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let s = seen.clone();
        let sink: EventSink = Arc::new(move |ev| s.lock().unwrap().push(ev));
        let ctx = StageCtx::new(paths, opts, sink, CancellationToken::new());
        (ctx, seen)
    }

    fn ctx_with(fixture: &Path, opts: StageOptions) -> (StageCtx, Arc<StdMutex<Vec<StageEvent>>>) {
        ctx_with_paths(fixture_paths(fixture), opts)
    }

    fn lines(seen: &Arc<StdMutex<Vec<StageEvent>>>) -> Vec<(Severity, String)> {
        seen.lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                StageEvent::Line { severity, text, .. } => Some((*severity, text.clone())),
                _ => None,
            })
            .collect()
    }

    // ── parse_protocol_awk ───────────────────────────────────────────────────

    #[test]
    fn parse_protocol_awk_matches_the_shell_recipe() {
        assert_eq!(parse_protocol_awk("protocol = \"alvr\"\n"), "alvr");
        assert_eq!(parse_protocol_awk("  protocol=\"oxrsys\"\n"), "oxrsys");
        assert_eq!(parse_protocol_awk("# protocol = \"alvr\"\n"), "");
        assert_eq!(parse_protocol_awk("protocol_extra = \"alvr\"\n"), "");
        assert_eq!(parse_protocol_awk("protocol=alvr\n"), "");
        assert_eq!(parse_protocol_awk(""), "");
        assert_eq!(
            parse_protocol_awk("video_codec = \"h264\"\nprotocol = \"alvr\"\n"),
            "alvr"
        );
        // First matching line wins.
        assert_eq!(
            parse_protocol_awk("protocol = \"first\"\nprotocol = \"second\"\n"),
            "first"
        );
    }

    // ── marker bytes ─────────────────────────────────────────────────────────

    #[test]
    fn dxmt_marker_bytes_are_the_pin_plus_one_newline() {
        let pin = &contract().deps.dxmt_tgz_sha256;
        assert_eq!(util::contract_marker_bytes(pin), format!("{pin}\n"));
    }

    // ── setup_game / skip-advice text ────────────────────────────────────────

    #[tokio::test]
    async fn game_check_is_skipped_without_bottle_or_bs_dir() {
        let fixture = scratch("game-skip");
        let (ctx, seen) = ctx_with(&fixture, StageOptions::default());
        setup_game(&ctx).await.unwrap();
        assert_eq!(
            lines(&seen),
            vec![(
                Severity::Info,
                "Beat Saber check skipped (no --bottle/--bs-dir given); ./demo.sh doctor will verify it"
                    .to_string()
            )]
        );
        std::fs::remove_dir_all(&fixture).ok();
    }

    #[tokio::test]
    async fn game_check_reports_found_via_bs_dir_override() {
        let fixture = scratch("game-found");
        let bs_dir = fixture.join("BeatSaber");
        std::fs::create_dir_all(&bs_dir).unwrap();
        std::fs::write(bs_dir.join("Beat Saber.exe"), b"stub").unwrap();
        let (ctx, seen) = ctx_with(
            &fixture,
            StageOptions {
                bs_dir_override: Some(bs_dir.clone()),
                ..Default::default()
            },
        );
        setup_game(&ctx).await.unwrap();
        assert_eq!(
            lines(&seen),
            vec![(
                Severity::Ok,
                format!("Beat Saber found at {}", bs_dir.display())
            )]
        );
        std::fs::remove_dir_all(&fixture).ok();
    }

    #[tokio::test]
    async fn game_check_warns_with_the_depot_command_when_missing() {
        let fixture = scratch("game-missing");
        let bs_dir = fixture.join("NoGameHere");
        let (ctx, seen) = ctx_with(
            &fixture,
            StageOptions {
                bs_dir_override: Some(bs_dir.clone()),
                ..Default::default()
            },
        );
        setup_game(&ctx).await.unwrap();
        let rows = lines(&seen);
        assert_eq!(
            rows,
            vec![
                (
                    Severity::Warn,
                    format!("Beat Saber 1.29.4 not found at {}", bs_dir.display())
                ),
                (
                    Severity::Info,
                    "download it with your Steam account (owning Beat Saber):".to_string()
                ),
                (
                    Severity::Info,
                    format!("  {}", contract().depot_command(&bs_dir))
                ),
            ]
        );
        std::fs::remove_dir_all(&fixture).ok();
    }

    #[tokio::test]
    async fn game_check_dies_the_require_bottle_way_for_a_missing_bottle() {
        let fixture = scratch("game-nobottle");
        let (ctx, _seen) = ctx_with(
            &fixture,
            StageOptions {
                bottle_name: Some("NoSuchSabrageTestBottle".into()),
                ..Default::default()
            },
        );
        let err = setup_game(&ctx).await.unwrap_err();
        assert!(
            err.to_string()
                .starts_with("bottle 'NoSuchSabrageTestBottle' not found at "),
            "{err}"
        );
        std::fs::remove_dir_all(&fixture).ok();
    }

    // ── setup_config ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn config_write_once_never_overwrites_an_existing_file() {
        let fixture = scratch("config-existing-alvr");
        let paths = fixture_paths(&fixture);
        std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
        std::fs::write(
            &paths.toml_path,
            "protocol = \"alvr\"\nvideo_codec = \"h264\"\n",
        )
        .unwrap();
        let (ctx, seen) = ctx_with_paths(paths.clone(), StageOptions::default());
        setup_config(&ctx).await.unwrap();
        assert_eq!(
            std::fs::read_to_string(&paths.toml_path).unwrap(),
            "protocol = \"alvr\"\nvideo_codec = \"h264\"\n"
        );
        assert!(lines(&seen).contains(&(
            Severity::Info,
            format!(
                "config present: {} (protocol=alvr)",
                paths.toml_path.display()
            )
        )));
        std::fs::remove_dir_all(&fixture).ok();
    }

    #[tokio::test]
    async fn config_warns_verbatim_when_protocol_is_not_alvr() {
        let fixture = scratch("config-oxrsys");
        let paths = fixture_paths(&fixture);
        std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
        std::fs::write(&paths.toml_path, "protocol = \"oxrsys\"\n").unwrap();
        let (ctx, seen) = ctx_with_paths(paths.clone(), StageOptions::default());
        setup_config(&ctx).await.unwrap();
        assert!(lines(&seen).contains(&(
            Severity::Warn,
            format!(
                "config present with protocol='oxrsys' — the demo needs protocol = \"alvr\"; edit {} yourself (not overwriting)",
                paths.toml_path.display()
            )
        )));
        std::fs::remove_dir_all(&fixture).ok();
    }

    #[tokio::test]
    async fn config_writes_the_shared_template_when_absent() {
        let fixture = scratch("config-fresh");
        let paths = fixture_paths(&fixture);
        let (ctx, seen) = ctx_with_paths(paths.clone(), StageOptions::default());
        setup_config(&ctx).await.unwrap();
        assert_eq!(
            std::fs::read_to_string(&paths.toml_path).unwrap(),
            util::toml_template()
        );
        assert!(lines(&seen).contains(&(
            Severity::Ok,
            format!(
                "wrote {} (protocol=alvr, 42 Mbps, encoder_process=auto)",
                paths.toml_path.display()
            )
        )));
        std::fs::remove_dir_all(&fixture).ok();
    }

    // ── setup_pinned ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn goldberg_present_with_wrong_hash_is_kept_and_not_redownloaded() {
        let fixture = scratch("gbe-mismatch");
        let paths = fixture_paths(&fixture);
        std::fs::create_dir_all(paths.gbe_dll.parent().unwrap()).unwrap();
        std::fs::write(&paths.gbe_dll, b"not the pinned build").unwrap();
        let (ctx, seen) = ctx_with_paths(
            paths.clone(),
            StageOptions {
                dry_run: true,
                ..Default::default()
            },
        );
        setup_pinned(&ctx).await.unwrap();

        assert!(lines(&seen).contains(&(
            Severity::Warn,
            format!(
                "Goldberg dll present with a non-pinned hash — keeping it (delete {} to re-fetch the pinned build)",
                paths.gbe_dll.display()
            )
        )));
        // Only the DXMT tarball is planned; the Goldberg dll is left alone.
        let downloads = ctx
            .executor
            .planned()
            .iter()
            .filter(|a| a.kind == PlannedKind::Download)
            .count();
        assert_eq!(downloads, 1);
        assert_eq!(
            std::fs::read(&paths.gbe_dll).unwrap(),
            b"not the pinned build"
        );
        std::fs::remove_dir_all(&fixture).ok();
    }

    #[tokio::test]
    async fn dxmt_already_current_skips_the_fetch_and_extract() {
        let fixture = scratch("dxmt-current");
        let paths = fixture_paths(&fixture);
        for f in &contract().dxmt.files {
            let p = paths.dxmt_art.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, b"stub").unwrap();
        }
        std::fs::write(
            paths.dxmt_art.join(".sha256"),
            util::contract_marker_bytes(&contract().deps.dxmt_tgz_sha256),
        )
        .unwrap();
        let (ctx, seen) = ctx_with_paths(
            paths.clone(),
            StageOptions {
                dry_run: true,
                ..Default::default()
            },
        );
        setup_pinned(&ctx).await.unwrap();

        assert!(lines(&seen).contains(&(
            Severity::Info,
            "already present: dxmt-artifacts (sha256 marker matches)".to_string()
        )));
        let plan = ctx.executor.planned();
        // Only the Goldberg fetch is planned; dxmt short-circuited before any
        // download/extract action.
        assert_eq!(
            plan.iter()
                .filter(|a| a.kind == PlannedKind::Download)
                .count(),
            1
        );
        assert!(!plan.iter().any(|a| a.kind == PlannedKind::Extract));
        assert!(!plan.iter().any(|a| a.kind == PlannedKind::RemoveDir));
        std::fs::remove_dir_all(&fixture).ok();
    }

    // ── full-stage dry run ───────────────────────────────────────────────────

    #[tokio::test]
    async fn dry_run_over_a_fresh_checkout_plans_every_mutation_without_touching_disk() {
        let fixture = scratch("dry-run-full");
        let paths = fixture_paths(&fixture);
        let (ctx, seen) = ctx_with_paths(
            paths.clone(),
            StageOptions {
                dry_run: true,
                ..Default::default()
            },
        );
        run(&ctx).await.unwrap();

        let plan = ctx.executor.planned();
        let kinds: Vec<PlannedKind> = plan.iter().map(|a| a.kind).collect();
        assert_eq!(
            kinds,
            vec![
                PlannedKind::Spawn,    // git -C ROOT submodule update ext/{oxrsys,wineopenxr,ALVR}
                PlannedKind::Spawn,    // git -C WOXR submodule update --filter=blob:none
                PlannedKind::Spawn,    // git -C ALVR submodule update
                PlannedKind::Download, // Goldberg dll
                PlannedKind::Download, // DXMT tarball
                PlannedKind::RemoveDir, // rm -rf ext/dxmt-artifacts
                PlannedKind::Extract,  // tar -xzf into ext/
                PlannedKind::Write,    // .sha256 marker
                PlannedKind::CreateDir, // mkdir -p OXR_APPSUP
                PlannedKind::Write,    // oxrsys-runtime.toml
            ]
        );

        let spawns: Vec<&PlannedAction> = plan
            .iter()
            .filter(|a| a.kind == PlannedKind::Spawn)
            .collect();
        assert_eq!(
            spawns[0].reason,
            format!(
                "git -C {} submodule update --init ext/oxrsys ext/wineopenxr ext/ALVR",
                paths.root.display()
            )
        );
        assert_eq!(
            spawns[1].reason,
            format!(
                "git -C {} submodule update --init --filter=blob:none",
                paths.woxr.display()
            )
        );
        assert_eq!(
            spawns[2].reason,
            format!("git -C {} submodule update --init", paths.alvr.display())
        );

        // Nothing was actually written.
        assert!(!paths.toml_path.exists());
        assert!(!paths.dxmt_art.exists());
        assert!(!paths.gbe_dll.exists());
        assert!(!paths.oxr_appsup.exists());

        // The stage still finished cleanly and reported the skip-advice line
        // for the game check (no --bottle/--bs-dir given).
        assert!(lines(&seen)
            .iter()
            .any(|(_, t)| t.contains("Beat Saber check skipped")));
        std::fs::remove_dir_all(&fixture).ok();
    }
}
