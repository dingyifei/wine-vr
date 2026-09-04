use super::*;
use crate::fixes::FixAction;
use crate::paths::{Bottle, Paths};
use crate::stages::{null_sink, StageCtx, StageOptions};
use std::sync::{Arc, Mutex as StdMutex};
use tokio_util::sync::CancellationToken;

fn matches_doctor_anchor(s: &str) -> bool {
    s.lines().any(|l| l == TARGET_LINE)
}

/// Every branch of `rewrite_graphics_backend` over literal `cxbottle.conf`
/// bodies: (label, input, expected branch, expected bytes out, whether the
/// result contains the line doctor greps for).
///
/// Two rows pin measured shell behaviour: a key line not shaped
/// `"CX_GRAPHICS_BACKEND" = "..."` is left alone as sed would, so the anchor
/// column is `false`; and the header-with-no-trailing-newline row is a
/// deliberate improvement over BSD sed's `a\` (review finding #10).
#[test]
fn branch_rewrite_cases() {
    let cases: &[(&str, &str, Branch, &str, bool)] = &[
            (
                "rewrites an existing key line in place",
                "\"Template\" = \"win11_64\"\n\"CX_GRAPHICS_BACKEND\" = \"auto\"\n\"Other\" = \"1\"\n",
                Branch::Rewrote,
                "\"Template\" = \"win11_64\"\n\"CX_GRAPHICS_BACKEND\" = \"dxmt\"\n\"Other\" = \"1\"\n",
                true,
            ),
            (
                "empty existing value is still rewritten",
                "\"CX_GRAPHICS_BACKEND\" = \"\"\n",
                Branch::Rewrote,
                "\"CX_GRAPHICS_BACKEND\" = \"dxmt\"\n",
                true,
            ),
            (
                "no trailing newline is preserved on rewrite",
                "\"CX_GRAPHICS_BACKEND\" = \"auto\"",
                Branch::Rewrote,
                "\"CX_GRAPHICS_BACKEND\" = \"dxmt\"",
                true,
            ),
            (
                "an unquoted value must not be touched, exactly as sed's anchored s/// would skip it",
                "\"CX_GRAPHICS_BACKEND\" = auto\n",
                Branch::Rewrote,
                "\"CX_GRAPHICS_BACKEND\" = auto\n",
                false,
            ),
            (
                "inserts immediately after the [EnvironmentVariables] header",
                "[EnvironmentVariables]\n\"SOME_OTHER\" = \"1\"\n",
                Branch::InsertedAfterEnvSection,
                "[EnvironmentVariables]\n\"CX_GRAPHICS_BACKEND\" = \"dxmt\"\n\"SOME_OTHER\" = \"1\"\n",
                true,
            ),
            (
                "header and inserted line must be joined by a real newline, not concatenated the \
                 way BSD sed's `a\\` mangles this exact case, and must not gain a trailing newline \
                 the original file never had",
                "[EnvironmentVariables]",
                Branch::InsertedAfterEnvSection,
                "[EnvironmentVariables]\n\"CX_GRAPHICS_BACKEND\" = \"dxmt\"",
                true,
            ),
            (
                "appends a new section when neither exists",
                "\"Template\" = \"win11_64\"\n",
                Branch::AppendedSection,
                "\"Template\" = \"win11_64\"\n\n[EnvironmentVariables]\n\"CX_GRAPHICS_BACKEND\" = \"dxmt\"\n",
                true,
            ),
            (
                "append does not care whether the original had a trailing newline (input has none)",
                "\"Template\" = \"win11_64\"",
                Branch::AppendedSection,
                "\"Template\" = \"win11_64\"\n[EnvironmentVariables]\n\"CX_GRAPHICS_BACKEND\" = \"dxmt\"\n",
                true,
            ),
        ];

    for (label, conf, expected_branch, expected_out, expected_anchor) in cases {
        let (out, branch) = rewrite_graphics_backend(conf);
        assert_eq!(branch, *expected_branch, "{label}");
        assert_eq!(out.as_str(), *expected_out, "{label}");
        assert_eq!(matches_doctor_anchor(&out), *expected_anchor, "{label}");
    }
}

// `scan_wineservers`'s OS-process scan is deliberately not exercised by
// spawning a stand-in child: system-wide process state is not a fixture this
// suite can pin down (cargo test runs in parallel; a sandboxed runner has
// SIGKILLed a copied-to-`/tmp` executable before it could be scanned).

#[test]
fn wineservers_indicate_live_decides_by_wineprefix() {
    let cases: &[(&str, &[Option<&str>], &str, bool)] = &[
        (
            "exact prefix matches",
            &[Some("/bottles/A")],
            "/bottles/A",
            true,
        ),
        (
            "same observation, a different candidate bottle",
            &[Some("/bottles/A")],
            "/bottles/B",
            false,
        ),
        (
            "an unreadable WINEPREFIX alone refuses: cannot tell which bottle it belongs to",
            &[None],
            "/anything",
            true,
        ),
        (
            "a different bottle plus one unreadable still refuses: the unreadable one might \
                 be this bottle's",
            &[Some("/bottles/Other"), None],
            "/bottles/A",
            true,
        ),
        ("nothing running", &[], "/anything", false),
        (
            "every match is a different bottle",
            &[Some("/bottles/Other1"), Some("/bottles/Other2")],
            "/bottles/A",
            false,
        ),
    ];

    for (label, observed, want_prefix, expected) in cases {
        let observed: Vec<Option<String>> =
            observed.iter().map(|p| p.map(str::to_string)).collect();
        assert_eq!(
            wineservers_indicate_live(&observed, want_prefix),
            *expected,
            "{label}"
        );
    }
}

/// Sanity check for the sysinfo-backed plumbing itself: a path nothing on
/// the machine could ever resolve to must report "not alive", regardless
/// of what else is running system-wide. Deterministic and side-effect free
/// (process.rs's own `find_processes_by_exe` test uses the same trick).
#[test]
fn scan_wineservers_finds_nothing_for_a_path_that_cannot_exist() {
    let nowhere = Path::new("/nonexistent/sabrage/not-a-real-wineserver");
    assert!(!any_wineserver_alive(nowhere));
    assert!(!bottle_wineserver_is_live(nowhere, Path::new("/anything")));
}

fn scratch(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("sabrage-backend-fix-{tag}-{}", std::process::id()))
}

/// A [`StageCtx`] whose bottle is a **fixture** directory under
/// `std::env::temp_dir()`, never the real `~/Library/Application
/// Support/CrossOver/Bottles` — `Bottle`'s fields are public precisely so
/// tests can build one without going through `Bottle::unvalidated`, which
/// always derives from `$HOME`. Returns the ctx and the fixture's
/// `cxbottle.conf` path.
fn fixture_ctx(root: &Path, dry_run: bool) -> (StageCtx, std::path::PathBuf) {
    let prefix = root.join("bottle");
    std::fs::create_dir_all(&prefix).unwrap();
    let bottle = Bottle {
        name: "FixtureBottle".to_string(),
        sys32: prefix.join("drive_c/windows/system32"),
        prefix: prefix.clone(),
    };
    let opts = StageOptions {
        // `require_bottle` checks `opts.bottle_name` first, before ever
        // looking at `ctx.bottle` — both must agree, or the fix dies on
        // "bottle name required" before it ever sees the fixture bottle.
        bottle_name: Some(bottle.name.clone()),
        dry_run,
        ..StageOptions::default()
    };
    let mut ctx = StageCtx::new(
        Paths::new(root),
        opts,
        null_sink(),
        CancellationToken::new(),
    );
    ctx.bottle = Some(bottle);
    (ctx, prefix.join("cxbottle.conf"))
}

#[tokio::test]
async fn set_graphics_backend_is_a_noop_when_already_current() {
    let root = scratch("noop");
    let (ctx, conf) = fixture_ctx(&root, false);
    std::fs::write(&conf, "\"CX_GRAPHICS_BACKEND\" = \"dxmt\"\n").unwrap();

    let sink: EventSink = Arc::new(|_| {});
    let report = set_graphics_backend(&ctx, &sink).await.unwrap();
    assert!(!report.changed);
    assert_eq!(report.action, FixAction::SetGraphicsBackend);
    assert_eq!(
        std::fs::read_to_string(&conf).unwrap(),
        "\"CX_GRAPHICS_BACKEND\" = \"dxmt\"\n",
        "must not rewrite a file that already matches"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn set_graphics_backend_rewrites_and_reports_the_verbatim_description() {
    let root = scratch("rewrite");
    let (ctx, conf) = fixture_ctx(&root, false);
    std::fs::write(&conf, "\"CX_GRAPHICS_BACKEND\" = \"auto\"\n").unwrap();

    let seen: Arc<StdMutex<Vec<crate::events::StageEvent>>> = Arc::new(StdMutex::new(Vec::new()));
    let s = seen.clone();
    let sink: EventSink = Arc::new(move |ev| s.lock().unwrap().push(ev));

    let report = set_graphics_backend(&ctx, &sink).await.unwrap();
    assert!(report.changed);
    assert_eq!(report.description, FORCED_DESCRIPTION);
    assert!(matches_doctor_anchor(
        &std::fs::read_to_string(&conf).unwrap()
    ));
    assert!(seen.lock().unwrap().iter().any(|e| matches!(
        e,
        crate::events::StageEvent::Line { text, .. } if text == FORCED_DESCRIPTION
    )));

    std::fs::remove_dir_all(&root).ok();
}

/// A `CX_GRAPHICS_BACKEND` line the anchored rewrite cannot touch (an
/// unquoted value, unusual spacing) is a failure through both doors: the
/// fix dies with run.sh's post-fix text instead of reporting "forced to
/// dxmt" over bytes that still lack the target line.
#[tokio::test]
async fn a_line_the_rewrite_cannot_canonicalize_is_a_failure_not_a_success() {
    for (tag, original) in [
        ("unquoted", "\"CX_GRAPHICS_BACKEND\" = auto\n"),
        ("no-spaces", "\"CX_GRAPHICS_BACKEND\"=\"auto\"\n"),
        ("wide", "\"CX_GRAPHICS_BACKEND\"  =  \"auto\"\n"),
    ] {
        let root = scratch(&format!("malformed-{tag}"));
        let (ctx, conf) = fixture_ctx(&root, false);
        std::fs::write(&conf, original).unwrap();

        let seen: Arc<StdMutex<Vec<crate::events::StageEvent>>> =
            Arc::new(StdMutex::new(Vec::new()));
        let s = seen.clone();
        let sink: EventSink = Arc::new(move |ev| s.lock().unwrap().push(ev));

        let err = set_graphics_backend(&ctx, &sink).await.unwrap_err();
        assert!(
            err.to_string()
                .starts_with("could not force graphics backend to dxmt in "),
            "{tag}: {err}"
        );
        assert_eq!(
            std::fs::read_to_string(&conf).unwrap(),
            original,
            "{tag}: a failed rewrite must not write anything"
        );
        assert!(
            !seen
                .lock()
                .unwrap()
                .iter()
                .any(|e| matches!(e, crate::events::StageEvent::Line { text, .. }
                        if text == FORCED_DESCRIPTION)),
            "{tag}: must never claim the backend was forced"
        );

        // The launch door shares the body, so it refuses identically.
        let err = set_graphics_backend_for_launch(&ctx, &sink)
            .await
            .unwrap_err();
        assert!(err
            .to_string()
            .starts_with("could not force graphics backend to dxmt in "));

        std::fs::remove_dir_all(&root).ok();
    }
}

#[tokio::test]
async fn set_graphics_backend_under_dry_run_does_not_touch_the_file() {
    let root = scratch("dry");
    let (ctx, conf) = fixture_ctx(&root, true);
    std::fs::write(&conf, "\"CX_GRAPHICS_BACKEND\" = \"auto\"\n").unwrap();

    let sink: EventSink = Arc::new(|_| {});
    let report = set_graphics_backend(&ctx, &sink).await.unwrap();
    assert!(report.changed, "dry run still reports what WOULD change");
    assert_eq!(
        std::fs::read_to_string(&conf).unwrap(),
        "\"CX_GRAPHICS_BACKEND\" = \"auto\"\n",
        "dry run must never write"
    );
    assert!(!ctx.executor.planned().is_empty());

    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn set_graphics_backend_requires_a_bottle() {
    let root = scratch("no-bottle");
    let ctx = StageCtx::new(
        Paths::new(&root),
        StageOptions::default(),
        null_sink(),
        CancellationToken::new(),
    );
    let sink: EventSink = Arc::new(|_| {});
    let err = set_graphics_backend(&ctx, &sink).await.unwrap_err();
    assert!(err
        .to_string()
        .starts_with("CrossOver bottle name required"));
}

#[tokio::test]
async fn set_graphics_backend_refuses_while_the_bottles_own_wineserver_is_live() {
    let root = scratch("refuse");
    let (mut ctx, conf) = fixture_ctx(&root, false);
    std::fs::write(&conf, "\"CX_GRAPHICS_BACKEND\" = \"auto\"\n").unwrap();

    // Stand in for a live wineserver with this test binary's own process —
    // guaranteed alive, nothing to spawn (the trick
    // `process::tests::finds_this_test_binary_by_its_exe_path` uses). A
    // `cargo test` process normally lacks `WINEPREFIX`, hitting
    // `wineservers_indicate_live`'s "cannot rule this one out" branch.
    ctx.paths.wineserver = Some(std::env::current_exe().expect("current_exe resolves"));

    let sink: EventSink = Arc::new(|_| {});
    let err = set_graphics_backend(&ctx, &sink).await.unwrap_err();
    assert!(err.to_string().contains("live wineserver"));
    assert_eq!(
        std::fs::read_to_string(&conf).unwrap(),
        "\"CX_GRAPHICS_BACKEND\" = \"auto\"\n",
        "must not edit while refusing"
    );

    std::fs::remove_dir_all(&root).ok();
}

/// The whole point of the second entry point: run.sh rewrites
/// `cxbottle.conf` in preflight and kills that wineserver two blocks
/// later, so a live wineserver must not block a launch. Same fixture as
/// the refusal test above, opposite verdict.
#[tokio::test]
async fn for_launch_edits_even_while_the_bottles_wineserver_is_live() {
    let root = scratch("for-launch-live");
    let (mut ctx, conf) = fixture_ctx(&root, false);
    std::fs::write(&conf, "\"CX_GRAPHICS_BACKEND\" = \"auto\"\n").unwrap();
    ctx.paths.wineserver = Some(std::env::current_exe().expect("current_exe resolves"));

    let sink: EventSink = Arc::new(|_| {});
    let report = set_graphics_backend_for_launch(&ctx, &sink).await.unwrap();
    assert!(report.changed);
    assert_eq!(report.action, FixAction::SetGraphicsBackend);
    assert_eq!(report.description, FORCED_DESCRIPTION);
    assert!(matches_doctor_anchor(
        &std::fs::read_to_string(&conf).unwrap()
    ));

    std::fs::remove_dir_all(&root).ok();
}
