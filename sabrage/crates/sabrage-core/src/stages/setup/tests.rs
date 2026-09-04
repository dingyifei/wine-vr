use super::*;
use crate::events::{Severity, StageEvent};
use crate::executor::{PlannedAction, PlannedKind};
use crate::paths::Paths;
use crate::stages::{EventSink, StageOptions};
use std::sync::{Arc, Mutex as StdMutex};
use tokio_util::sync::CancellationToken;

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("sabrage-setup-test-{}-{tag}", std::process::id()));
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

fn ctx_with_paths(paths: Paths, opts: StageOptions) -> (StageCtx, Arc<StdMutex<Vec<StageEvent>>>) {
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

/// A [`crate::executor::Executor`] that loses the `O_EXCL` create race
/// A5-3 names: `create_new` writes the other writer's bytes itself and
/// then returns `Ok(false)`, the caller's bytes unwritten. Every other
/// method forwards to the inner executor.
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
    ) -> crate::executor::BoxFuture<'a, Result<Option<crate::executor::DetachedChild>>> {
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
    // appears only inside the create call.
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
    // A5-1: the extraction row claims the marker too, and the marker is
    // exactly what this fixture lacks — so the row must stay future-tense
    // and the file must still be absent afterwards.
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
