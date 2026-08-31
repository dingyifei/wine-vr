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
//!    only reported (and warned about when `protocol` is not `alvr`). The
//!    creation goes through [`crate::executor::Executor::create_new`] (`O_EXCL`)
//!    rather than an `exists()` probe plus
//!    [`crate::executor::Executor::write_atomic`]: the probe and the write are
//!    separated by an `await`, and the Sabrage operation lock covers neither an
//!    editor nor a concurrently running `demo.sh setup`, so the racy shape
//!    could rename a template over a hand-maintained config that was never
//!    backed up. When the kernel says somebody else created it first, the file
//!    is reported exactly as the already-present branch reports it.
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
//! [`crate::executor::Executor::is_dry_run`] is true. The check still runs for
//! real when the postcondition already happens to hold — a dry run over an
//! already-set-up checkout reports the same `ok` rows a real run would — and
//! only then. When the postcondition is currently **false**, the row swaps to
//! a future-tense `info` ("would initialize submodules …", "would extract
//! ext/dxmt-artifacts …") instead of the `ok`: a preview may say what it
//! plans to do, but it may not claim a checkout state that does not exist
//! (`sabrage setup --dry-run` over a fresh clone used to print three green
//! completed-state rows). This is the same verb swap `build.rs` applies to its
//! own staged-copy outcome, and PARITY.md's "would …" dry-run language row.
//!
//! The rule is the **whole** claimed postcondition, not the part of it that
//! happens to be observable. Two rows named a byte a dry run had not written:
//!
//! * the extraction row's parenthetical claims the provenance marker, but
//!   [`setup_pinned`] returns early ("already present") whenever the marker is
//!   current — so *reaching* that row at all means the marker is absent or
//!   stale, and under `--dry-run` nothing wrote it. There is therefore no
//!   dry-run shape in which the `ok` is truthful, and the row is always the
//!   future-tense one there;
//! * the config row said "wrote …" for a file [`crate::executor::DryRunExecutor`]
//!   only planned. It now says "would write …", and `sabrage setup --dry-run`
//!   over a fresh checkout leaves — and claims — nothing on disk.

use std::path::Path;

use crate::contract::contract;
use crate::error::{Result, SabrageError};
use crate::events::step;
use crate::process::{self, ChildSpec};
use crate::stages::{require_bottle, StageCtx};
use crate::util;

// ── dry-run "would …" rows (no shell counterpart; see the module doc) ────────

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

    // grep -q is_streaming_nonblocking "$ALVR/alvr/server_core/src/connection.rs"
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

    let extracted_ok = util::dxmt_files_ok(&ctx.paths);
    if extracted_ok || exec.is_dry_run() {
        // The marker write stays planned either way, so the dry-run plan still
        // records it; only the row's tense follows the postcondition.
        let marker_path = ctx.paths.dxmt_art.join(".sha256");
        let marker_bytes = util::contract_marker_bytes(&deps.dxmt_tgz_sha256);
        exec.write_atomic(&marker_path, marker_bytes.as_bytes())
            .await?;
        let st = ctx.step(step::SETUP_PINNED);
        // The `ok` row claims *both* halves of the postcondition — the files
        // and the marker. `util::dxmt_ok` (files **and** a current marker)
        // already returned early above, so reaching this line under a dry run
        // means the marker is missing or stale and nothing wrote it: a complete
        // file set alone may not buy the green row. See the module docs.
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

// ── 3. runtime config ────────────────────────────────────────────────────────

async fn setup_config(ctx: &StageCtx) -> Result<()> {
    let st = ctx.step(step::SETUP_CONFIG);
    let exec = ctx.executor_for(step::SETUP_CONFIG);
    exec.create_dir_all(&ctx.paths.oxr_appsup).await?;

    if ctx.paths.toml_path.is_file() {
        report_existing_config(st, &ctx.paths.toml_path);
    } else {
        // `O_EXCL`, not `exists()`-then-rename: whoever created the file
        // between the probe above and this line keeps it (see the module docs).
        let created = exec
            .create_new(&ctx.paths.toml_path, util::toml_template().as_bytes())
            .await?;
        if !created {
            // Someone else won the race. The file is now in exactly the shape
            // the branch above exists to describe, so describe it — the one
            // thing setup may never do to a config it did not write is
            // replace it.
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

/// setup.sh's two rows for a config this run did not write: the `info` when its
/// `protocol` is already `alvr`, the `warn` (verbatim, "not overwriting"
/// included) when it is anything else.
///
/// Two callers, one text: the ordinary already-present case, and the one where
/// [`crate::executor::Executor::create_new`] reports that another writer got
/// there first.
fn report_existing_config(st: crate::stages::StepEmitter<'_>, toml_path: &Path) {
    let text = std::fs::read_to_string(toml_path).unwrap_or_default();
    let proto = parse_protocol_awk(&text);
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

    /// The concurrent-writer race A5-3 named, made deterministic: an
    /// [`crate::executor::Executor`] that stands in for the other process —
    /// it creates the file itself (as an editor or a concurrent `demo.sh setup`
    /// would, between `setup_config`'s existence probe and its write) and then
    /// reports what the kernel reports to the loser of an `O_EXCL` open:
    /// `Ok(false)`, caller's bytes not written. Everything else forwards to the
    /// real executor underneath.
    #[derive(Debug)]
    struct LosesTheCreateRace {
        inner: Arc<dyn crate::executor::Executor>,
        other_writers_bytes: &'static str,
    }

    impl LosesTheCreateRace {
        fn around(
            inner: Arc<dyn crate::executor::Executor>,
            other_writers_bytes: &'static str,
        ) -> Arc<LosesTheCreateRace> {
            Arc::new(LosesTheCreateRace {
                inner,
                other_writers_bytes,
            })
        }
    }

    impl crate::executor::Executor for LosesTheCreateRace {
        fn with_step(&self, step: crate::events::StepId) -> Arc<dyn crate::executor::Executor> {
            Arc::new(LosesTheCreateRace {
                inner: self.inner.with_step(step),
                other_writers_bytes: self.other_writers_bytes,
            })
        }
        fn is_dry_run(&self) -> bool {
            self.inner.is_dry_run()
        }
        fn planned(&self) -> Vec<PlannedAction> {
            self.inner.planned()
        }
        fn create_new<'a>(
            &'a self,
            path: &'a Path,
            _bytes: &'a [u8],
        ) -> crate::executor::BoxFuture<'a, Result<bool>> {
            Box::pin(async move {
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, self.other_writers_bytes).unwrap();
                Ok(false)
            })
        }
        fn copy_if_changed<'a>(
            &'a self,
            src: &'a Path,
            dst: &'a Path,
        ) -> crate::executor::BoxFuture<'a, Result<crate::executor::Copied>> {
            self.inner.copy_if_changed(src, dst)
        }
        fn write_atomic<'a>(
            &'a self,
            path: &'a Path,
            bytes: &'a [u8],
        ) -> crate::executor::BoxFuture<'a, Result<()>> {
            self.inner.write_atomic(path, bytes)
        }
        fn remove_dir_all<'a>(&'a self, p: &'a Path) -> crate::executor::BoxFuture<'a, Result<()>> {
            self.inner.remove_dir_all(p)
        }
        fn remove_file<'a>(&'a self, p: &'a Path) -> crate::executor::BoxFuture<'a, Result<()>> {
            self.inner.remove_file(p)
        }
        fn create_dir_all<'a>(&'a self, p: &'a Path) -> crate::executor::BoxFuture<'a, Result<()>> {
            self.inner.create_dir_all(p)
        }
        fn dir_copy<'a>(
            &'a self,
            src: &'a Path,
            dst: &'a Path,
        ) -> crate::executor::BoxFuture<'a, Result<()>> {
            self.inner.dir_copy(src, dst)
        }
        fn download<'a>(
            &'a self,
            url: &'a str,
            dest: &'a Path,
            sha256: &'a str,
            label: &'a str,
        ) -> crate::executor::BoxFuture<'a, Result<crate::executor::Downloaded>> {
            self.inner.download(url, dest, sha256, label)
        }
        fn tar_xzf<'a>(
            &'a self,
            archive: &'a Path,
            into_dir: &'a Path,
        ) -> crate::executor::BoxFuture<'a, Result<()>> {
            self.inner.tar_xzf(archive, into_dir)
        }
        fn touch<'a>(&'a self, p: &'a Path) -> crate::executor::BoxFuture<'a, Result<()>> {
            self.inner.touch(p)
        }
        fn run_child<'a>(
            &'a self,
            spec: &'a crate::process::ChildSpec,
        ) -> crate::executor::BoxFuture<'a, Result<std::process::ExitStatus>> {
            self.inner.run_child(spec)
        }
        fn spawn_detached<'a>(
            &'a self,
            spec: &'a crate::process::ChildSpec,
            stdio: crate::executor::DetachedStdio,
        ) -> crate::executor::BoxFuture<'a, Result<Option<crate::executor::DetachedChild>>>
        {
            self.inner.spawn_detached(spec, stdio)
        }
    }

    /// A5-3: a config that appears between the `is_file()` probe and the write
    /// keeps its bytes. Before the fix the branch ended in `write_atomic`,
    /// which renames the template over whatever is at the destination — an
    /// irreversible loss of a hand-maintained config, with no backup.
    #[tokio::test]
    async fn a_config_created_by_another_writer_is_reported_not_replaced() {
        let fixture = scratch("config-race");
        let paths = fixture_paths(&fixture);
        let hand_edited = "# hand maintained\nprotocol = \"alvr\"\nbitrate_mbps = 80\n";
        let (mut ctx, seen) = ctx_with_paths(paths.clone(), StageOptions::default());
        assert!(!ctx.executor.is_dry_run());
        ctx.executor = LosesTheCreateRace::around(ctx.executor.clone(), hand_edited);

        // Nothing on disk when the stage probes: the other writer's file
        // appears only inside the create call, which is the window the old
        // `is_file()`-then-`write_atomic` shape renamed straight over.
        assert!(!paths.toml_path.exists());
        setup_config(&ctx).await.unwrap();

        assert_eq!(
            std::fs::read_to_string(&paths.toml_path).unwrap(),
            hand_edited,
            "the losing writer replaced a config it did not create"
        );
        let rows = lines(&seen);
        assert!(
            rows.contains(&(
                Severity::Info,
                format!(
                    "config present: {} (protocol=alvr)",
                    paths.toml_path.display()
                )
            )),
            "{rows:?}"
        );
        assert!(
            !rows
                .iter()
                .any(|(sev, t)| *sev == Severity::Ok && t.starts_with("wrote ")),
            "claimed a write that lost the race: {rows:?}"
        );
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

    // ── dry-run honesty: no green completed-state rows for planned work ──────

    /// Make `fixture` look like a checkout whose submodules are initialized and
    /// whose ALVR branch carries the oxrsys patch set.
    fn fake_submodule_checkout(paths: &Paths) {
        let header = paths.alvr.join("openvr/headers/openvr_driver.h");
        std::fs::create_dir_all(header.parent().unwrap()).unwrap();
        std::fs::write(&header, b"// stub").unwrap();
        let connection = paths.alvr.join("alvr/server_core/src/connection.rs");
        std::fs::create_dir_all(connection.parent().unwrap()).unwrap();
        std::fs::write(&connection, b"fn is_streaming_nonblocking() {}\n").unwrap();
    }

    #[tokio::test]
    async fn a_dry_run_over_a_fresh_checkout_never_claims_completed_state() {
        let fixture = scratch("dry-run-honesty-fresh");
        let paths = fixture_paths(&fixture);
        let (ctx, seen) = ctx_with_paths(
            paths.clone(),
            StageOptions {
                dry_run: true,
                ..Default::default()
            },
        );
        run(&ctx).await.unwrap();

        let rows = lines(&seen);
        for claim in [
            "submodules ready",
            "ALVR checkout carries the oxrsys patch set (branch oxrsys-v20.14.1)",
            "extracted ext/dxmt-artifacts (provenance marker written)",
        ] {
            assert!(
                !rows
                    .iter()
                    .any(|(sev, t)| *sev == Severity::Ok && t == claim),
                "a dry run claimed {claim:?} over a checkout that has none of it: {rows:?}"
            );
        }
        for planned in [
            SUBMODULES_WOULD_INIT_INFO,
            PATCHSET_WOULD_CHECKOUT_INFO,
            DXMT_WOULD_EXTRACT_INFO,
        ] {
            assert!(
                rows.iter()
                    .any(|(sev, t)| *sev == Severity::Info && t == planned),
                "missing the future-tense row {planned:?}: {rows:?}"
            );
        }
        // The marker write is still *planned* — only the row's tense changed.
        assert!(ctx.executor.planned().iter().any(|a| {
            a.kind == PlannedKind::Write
                && a.dst.as_deref() == Some(paths.dxmt_art.join(".sha256").as_path())
        }));
        std::fs::remove_dir_all(&fixture).ok();
    }

    #[tokio::test]
    async fn a_dry_run_over_an_already_set_up_checkout_still_reports_the_ok_rows() {
        let fixture = scratch("dry-run-honesty-ready");
        let paths = fixture_paths(&fixture);
        fake_submodule_checkout(&paths);
        // Every extracted file present but the provenance marker absent — the
        // shape a half-finished extraction leaves behind.
        for f in &contract().dxmt.files {
            let p = paths.dxmt_art.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, b"stub").unwrap();
        }
        let (ctx, seen) = ctx_with_paths(
            paths.clone(),
            StageOptions {
                dry_run: true,
                ..Default::default()
            },
        );
        run(&ctx).await.unwrap();

        let rows = lines(&seen);
        // These two postconditions are true on disk, so they keep their ok row.
        for claim in [
            "submodules ready",
            "ALVR checkout carries the oxrsys patch set (branch oxrsys-v20.14.1)",
        ] {
            assert!(
                rows.iter()
                    .any(|(sev, t)| *sev == Severity::Ok && t == claim),
                "a truthful postcondition lost its ok row {claim:?}: {rows:?}"
            );
        }
        // A5-1: the extraction row claims the *marker* too, and the marker is
        // exactly what this fixture lacks (a current one would have short-
        // circuited the whole step). A dry run wrote none, so the row must stay
        // future-tense — and the file must still be absent afterwards.
        assert!(
            rows.iter()
                .any(|(sev, t)| *sev == Severity::Info && t == DXMT_WOULD_EXTRACT_INFO),
            "missing the future-tense extraction row: {rows:?}"
        );
        assert!(
            !rows.iter().any(|(sev, t)| *sev == Severity::Ok
                && t == "extracted ext/dxmt-artifacts (provenance marker written)"),
            "a dry run claimed a provenance marker it never wrote: {rows:?}"
        );
        assert!(
            !paths.dxmt_art.join(".sha256").exists(),
            "the dry run wrote the marker for real"
        );
        std::fs::remove_dir_all(&fixture).ok();
    }

    /// A5-1, the config half: `setup_config`'s green row says "wrote …" of a
    /// file [`crate::executor::DryRunExecutor`] only planned. Under `--dry-run`
    /// it must be the future tense, and nothing may be on disk.
    #[tokio::test]
    async fn a_dry_run_never_claims_it_wrote_the_runtime_config() {
        let fixture = scratch("dry-run-honesty-config");
        let paths = fixture_paths(&fixture);
        let (ctx, seen) = ctx_with_paths(
            paths.clone(),
            StageOptions {
                dry_run: true,
                ..Default::default()
            },
        );
        setup_config(&ctx).await.unwrap();

        let rows = lines(&seen);
        assert!(
            rows.iter().any(|(sev, t)| *sev == Severity::Info
                && t == &format!(
                    "would write {} (protocol=alvr, 42 Mbps, encoder_process=auto)",
                    paths.toml_path.display()
                )),
            "missing the future-tense config row: {rows:?}"
        );
        assert!(
            !rows
                .iter()
                .any(|(sev, t)| *sev == Severity::Ok && t.starts_with("wrote ")),
            "a dry run claimed a config write it never performed: {rows:?}"
        );
        assert!(!paths.toml_path.exists(), "the dry run wrote the config");
        // The write is still *planned* — only the row's tense changed.
        assert!(ctx.executor.planned().iter().any(|a| {
            a.kind == PlannedKind::Write && a.dst.as_deref() == Some(paths.toml_path.as_path())
        }));
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
