//! `demo.sh setup` — one-time fetch of sources + pinned binaries and config
//! bootstrap. Idempotent, no sudo.
//!
//! Reference: `scripts/demo/setup.sh`. Four steps, in order:
//!
//! 1. [`step::SETUP_SUBMODULES`] — the `ext/` submodules.
//! 2. [`step::SETUP_PINNED`] — the pinned Goldberg dll and DXMT artifacts.
//! 3. [`step::SETUP_CONFIG`] — the runtime `oxrsys-runtime.toml`.
//! 4. [`step::SETUP_GAME`] — the Beat Saber presence probe.
//!
//! The config is write-once via [`crate::executor::Executor::create_new`]
//! (`O_EXCL`, not `exists()` then rename): a config this run did not write
//! is reported, never replaced —
//! tests::a_config_created_by_another_writer_is_reported_not_replaced.
//!
//! Every mutation goes through `ctx.executor`, so a dry run plans instead of
//! acting. Postcondition checks are skipped (never fatal) when the run only
//! planned the mutation; a false one swaps the `ok` row for a future-tense
//! `info` — a preview may not claim a state that does not exist. See
//! tests::a_dry_run_over_a_fresh_checkout_never_claims_completed_state and
//! PARITY.md § CLI / GUI, "Dry-run rows swap the verb to".

use std::path::Path;

use crate::contract::contract;
use crate::error::{Result, SabrageError};
use crate::events::step;
use crate::process::{self, ChildSpec};
use crate::stages::{require_bottle, StageCtx};
use crate::util;

/// Replaces `ok "submodules ready"` under `--dry-run` when the submodules are
/// not in fact checked out yet.
const SUBMODULES_WOULD_INIT_INFO: &str =
    "would initialize submodules (nothing was fetched under --dry-run)";

/// Replaces the patch-set `ok` row under `--dry-run` when the grep does not
/// (yet) find `is_streaming_nonblocking`.
const PATCHSET_WOULD_CHECKOUT_INFO: &str = "would check out the ALVR oxrsys patch set \
     (branch oxrsys-v20.14.1; nothing was fetched under --dry-run)";

/// Replaces the extraction `ok` row under `--dry-run` when `ext/dxmt-artifacts`
/// is not (yet) complete on disk.
const DXMT_WOULD_EXTRACT_INFO: &str =
    "would extract ext/dxmt-artifacts and write the provenance marker";

/// Execute the stage.
pub async fn run(ctx: &StageCtx) -> Result<()> {
    setup_submodules(ctx).await?;
    setup_pinned(ctx).await?;
    setup_config(ctx).await?;
    setup_game(ctx).await?;
    Ok(())
}

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
    // history.
    run_child_ok(
        ctx,
        git_submodule_spec(
            ctx,
            &ctx.paths.woxr,
            &["submodule", "update", "--init", "--filter=blob:none"],
        ),
    )
    .await?;
    // openvr is an alvr_session build dependency.
    run_child_ok(
        ctx,
        git_submodule_spec(ctx, &ctx.paths.alvr, &["submodule", "update", "--init"]),
    )
    .await?;

    let openvr_header = ctx.paths.alvr.join("openvr/headers/openvr_driver.h");
    if openvr_header.is_file() {
        st.ok("submodules ready");
    } else if ctx.executor.is_dry_run() {
        // The postcondition is false *because* the dry run planned the fetch
        // instead of performing it — say what would happen, never claim it did.
        st.info(SUBMODULES_WOULD_INIT_INFO);
    } else {
        return Err(st.fatal(
            "ALVR openvr submodule did not materialize — check network/auth and re-run setup",
            None,
        ));
    }

    let connection_rs = ctx.paths.alvr.join("alvr/server_core/src/connection.rs");
    let patchset_present = std::fs::read(&connection_rs)
        .map(|bytes| String::from_utf8_lossy(&bytes).contains("is_streaming_nonblocking"))
        .unwrap_or(false);
    if patchset_present {
        st.ok("ALVR checkout carries the oxrsys patch set (branch oxrsys-v20.14.1)");
        Ok(())
    } else if ctx.executor.is_dry_run() {
        st.info(PATCHSET_WOULD_CHECKOUT_INFO);
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

    let extracted_ok = util::dxmt_files_ok(&ctx.paths);
    if extracted_ok || exec.is_dry_run() {
        // The marker write stays planned either way, so the dry-run plan still
        // records it; only the row's tense follows the postcondition.
        let marker_path = ctx.paths.dxmt_art.join(".sha256");
        let marker_bytes = util::contract_marker_bytes(&deps.dxmt_tgz_sha256);
        exec.write_atomic(&marker_path, marker_bytes.as_bytes())
            .await?;
        let st = ctx.step(step::SETUP_PINNED);
        // The `ok` row claims the files *and* the marker; reaching this line
        // under a dry run means the marker is missing or stale and nothing
        // wrote it — tests::a_dry_run_over_an_already_set_up_checkout_still_reports_the_ok_rows.
        if extracted_ok && !exec.is_dry_run() {
            st.ok("extracted ext/dxmt-artifacts (provenance marker written)");
        } else {
            st.info(DXMT_WOULD_EXTRACT_INFO);
        }
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

async fn setup_config(ctx: &StageCtx) -> Result<()> {
    let st = ctx.step(step::SETUP_CONFIG);
    let exec = ctx.executor_for(step::SETUP_CONFIG);
    exec.create_dir_all(&ctx.paths.oxr_appsup).await?;

    if ctx.paths.toml_path.is_file() {
        report_existing_config(st, &ctx.paths.toml_path);
    } else {
        // `O_EXCL`, not `exists()`-then-rename: whoever created the file
        // between the probe above and this line keeps it —
        // tests::a_config_created_by_another_writer_is_reported_not_replaced.
        let created = exec
            .create_new(&ctx.paths.toml_path, util::toml_template().as_bytes())
            .await?;
        if !created {
            // Someone else won the race; setup may never replace a config it
            // did not write, so report it exactly as the branch above does.
            report_existing_config(st, &ctx.paths.toml_path);
        } else if exec.is_dry_run() {
            // Planned, not performed: no bytes are on disk to have written.
            st.info(format!(
                "would write {} (protocol=alvr, 42 Mbps, encoder_process=auto)",
                ctx.paths.toml_path.display()
            ));
        } else {
            st.ok(format!(
                "wrote {} (protocol=alvr, 42 Mbps, encoder_process=auto)",
                ctx.paths.toml_path.display()
            ));
        }
    }

    st.info(format!(
        "note: the embedded ALVR core keeps its session.json under '{}/alvr/' — auto-created on first run, LAN clients auto-trusted",
        ctx.paths.oxr_appsup.display()
    ));
    Ok(())
}

/// Emits the row for a config this run did not write: `info` when its
/// `protocol` is already `alvr`, otherwise the `warn` that reproduces
/// setup.sh's "not overwriting" text verbatim —
/// tests::config_warns_verbatim_when_protocol_is_not_alvr. `protocol` is read
/// with the same last-match recipe as doctor.sh and run.sh (lib.sh's
/// `toml_string_value`).
///
/// Reference: `scripts/demo/setup.sh`.
fn report_existing_config(st: crate::stages::StepEmitter<'_>, toml_path: &Path) {
    let text = std::fs::read_to_string(toml_path).unwrap_or_default();
    let proto = crate::checks::config::parse_protocol(&text);
    if proto == "alvr" {
        st.info(format!(
            "config present: {} (protocol=alvr)",
            toml_path.display()
        ));
    } else {
        st.warn(format!(
            "config present with protocol='{proto}' — the demo needs protocol = \"alvr\"; edit {} yourself (not overwriting)",
            toml_path.display()
        ));
    }
}

async fn setup_game(ctx: &StageCtx) -> Result<()> {
    let st = ctx.step(step::SETUP_GAME);
    if ctx.opts.bottle_name.is_none() && ctx.opts.bs_dir_override.is_none() {
        st.info(
            "Beat Saber check skipped (no --bottle/--bs-dir given); ./demo.sh doctor will verify it",
        );
        return Ok(());
    }
    // A given bottle name must still die the require_bottle way on a missing
    // bottle; with only --bs-dir, ctx.bs_dir already holds it —
    // tests::game_check_dies_the_require_bottle_way_for_a_missing_bottle.
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

/// Runs `spec` through the stage's executor.
///
/// # Errors
///
/// [`SabrageError::ChildFailed`] with an empty tail on a non-zero exit: every
/// line the child printed already reached the event stream as it ran. A dry
/// run never spawns and always reports success, so this returns `Ok` there.
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
mod tests;
