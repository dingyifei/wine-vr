use super::*;
use crate::executor::PlannedKind;
use crate::paths::Paths;
use crate::stages::{EventSink, StageOptions};
use std::sync::{Arc, Mutex as StdMutex};

// No test here may execute `osascript` or `sudo`: every test exercises a
// pure function, stages a file in a fixture directory, or drives the
// dry-run path, which records argv through the executor and spawns nothing.

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "sabrage-priv-{}-{tag}-{}",
        std::process::id(),
        uuid::Uuid::new_v4().as_simple()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn ctx_with(opts: StageOptions) -> (StageCtx, Arc<StdMutex<Vec<StageEvent>>>) {
    let seen = Arc::new(StdMutex::new(Vec::new()));
    let s = seen.clone();
    let sink: EventSink = Arc::new(move |ev| s.lock().unwrap().push(ev));
    let ctx = StageCtx::new(
        Paths::new("/nonexistent/sabrage/repo"),
        opts,
        sink,
        CancellationToken::new(),
    );
    (ctx, seen)
}

/// Reverse [`applescript_escape`]: the AppleScript string-literal rules.
fn applescript_unescape(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some(other) => out.push(other), // \\ and \"
            None => out.push('\\'),
        }
    }
    out
}

/// A minimal `/bin/sh` word splitter understanding unquoted words,
/// single-quoted strings and backslash escapes — the only constructs
/// [`shell_quote`] can emit.
fn sh_words(s: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut have = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' => {
                if have {
                    words.push(std::mem::take(&mut cur));
                    have = false;
                }
            }
            '\'' => {
                have = true;
                for d in chars.by_ref() {
                    if d == '\'' {
                        break;
                    }
                    cur.push(d);
                }
            }
            '\\' => {
                have = true;
                if let Some(d) = chars.next() {
                    cur.push(d);
                }
            }
            _ => {
                have = true;
                cur.push(c);
            }
        }
    }
    if have {
        words.push(cur);
    }
    words
}

/// The `/bin/sh` line back out of an `osascript -e` argument.
fn unwrap_do_shell_script(script: &str) -> String {
    let inner = script
        .strip_prefix("do shell script \"")
        .and_then(|s| s.strip_suffix("\" with administrator privileges"))
        .expect("do shell script wrapper");
    applescript_unescape(inner)
}

#[test]
fn applescript_escape_covers_every_special() {
    assert_eq!(applescript_escape(r"a\b"), r"a\\b");
    assert_eq!(applescript_escape("say \"hi\""), "say \\\"hi\\\"");
    assert_eq!(applescript_escape("a\nb\tc\rd"), "a\\nb\\tc\\rd");
    // Nothing else is touched — single quotes are not special to AppleScript.
    assert_eq!(
        applescript_escape("it's $HOME `x` 100%"),
        "it's $HOME `x` 100%"
    );
}

#[test]
fn shell_quote_neutralizes_every_metacharacter() {
    assert_eq!(shell_quote("plain"), "'plain'");
    assert_eq!(shell_quote("with space"), "'with space'");
    assert_eq!(shell_quote("it's"), r"'it'\''s'");
    assert_eq!(shell_quote("$(rm -rf /)"), "'$(rm -rf /)'");
    assert_eq!(
        sh_words(&shell_quote("a b; rm -rf /")),
        vec!["a b; rm -rf /"]
    );
}

#[test]
fn nasty_paths_round_trip_through_both_quoting_layers() {
    let cases: &[&str] = &[
        "/Users/me/wine-vr",
        "/Users/me/My Repos/wine vr",
        "/Users/me/it's mine/repo",
        "/Users/me/say \"hi\"/repo",
        r"/Users/me/back\slash/repo",
        "/Users/me/$(touch /tmp/pwned)/repo",
        "/Users/me/`id`;rm -rf ~/repo",
        "/Users/me/a&&b||c|d>e<f/repo",
        "/Users/me/new\nline/repo",
        "/Users/me/ünïcødé 🎯/repo",
    ];
    for root in cases {
        let tmp = PathBuf::from(format!("{root}/tmp/host-manifest-abc.json"));
        let dest = PathBuf::from(format!("{root}/openxr/1/active_runtime.x86_64.json"));
        let dir = dest.parent().unwrap();

        let cmd = privileged_install_command(&tmp, &dest);
        let script = do_shell_script(&cmd);
        // Layer 1: the AppleScript literal survives verbatim.
        assert_eq!(
            unwrap_do_shell_script(&script),
            cmd,
            "applescript layer: {root}"
        );
        // Layer 2: /bin/sh sees exactly the argv we intended, one word per
        // path, no matter what the path contains.
        assert_eq!(
            sh_words(&cmd),
            vec![
                MKDIR.to_string(),
                "-p".to_string(),
                dir.to_string_lossy().into_owned(),
                "&&".to_string(),
                INSTALL.to_string(),
                "-m".to_string(),
                "0644".to_string(),
                "-o".to_string(),
                "root".to_string(),
                "-g".to_string(),
                "wheel".to_string(),
                tmp.to_string_lossy().into_owned(),
                dest.to_string_lossy().into_owned(),
            ],
            "sh layer: {root}"
        );
    }
}

#[test]
fn the_command_creates_the_directory_and_installs_root_wheel_0644() {
    let cmd = privileged_install_command(
        Path::new("/Users/me/Library/Application Support/Sabrage/tmp/host-manifest-1.json"),
        Path::new("/usr/local/share/openxr/1/active_runtime.x86_64.json"),
    );
    assert_eq!(
        cmd,
        "/bin/mkdir -p '/usr/local/share/openxr/1' && /usr/bin/install -m 0644 -o root -g wheel \
             '/Users/me/Library/Application Support/Sabrage/tmp/host-manifest-1.json' \
             '/usr/local/share/openxr/1/active_runtime.x86_64.json'"
    );
}

#[test]
fn elevation_argv_is_one_osascript_or_install_shs_two_sudo_calls() {
    let tmp = Path::new("/tmp-dir/staged.json");
    let dest = Path::new("/usr/local/share/openxr/1/active_runtime.x86_64.json");

    let osa = elevation_argv(AdminMethod::Osascript, tmp, dest);
    assert_eq!(osa.len(), 1, "one prompt total");
    assert_eq!(osa[0][0], OsString::from(OSASCRIPT));
    assert_eq!(osa[0][1], OsString::from("-e"));
    assert_eq!(
        unwrap_do_shell_script(&osa[0][2].to_string_lossy()),
        privileged_install_command(tmp, dest)
    );

    let sudo = elevation_argv(AdminMethod::Sudo, tmp, dest);
    assert_eq!(sudo.len(), SUDO_DIE.len(), "one die string per command");
    assert_eq!(
        sudo[0],
        vec![
            OsString::from(SUDO),
            OsString::from(MKDIR),
            OsString::from("-p"),
            OsString::from("/usr/local/share/openxr/1"),
        ]
    );
    assert_eq!(
        sudo[1],
        vec![
            OsString::from(SUDO),
            OsString::from(INSTALL),
            OsString::from("-m"),
            OsString::from("0644"),
            OsString::from("-o"),
            OsString::from("root"),
            OsString::from("-g"),
            OsString::from("wheel"),
            OsString::from("/tmp-dir/staged.json"),
            OsString::from("/usr/local/share/openxr/1/active_runtime.x86_64.json"),
        ]
    );
}

#[test]
fn staged_temp_is_0600_and_deletes_itself() {
    let dir = scratch("staging");
    let path;
    {
        let staged = StagedTemp::create(&dir, "{\"file_format_version\": \"1.0.0\"}\n").unwrap();
        path = staged.path.clone();
        let meta = std::fs::metadata(&path).unwrap();
        assert_eq!(
            meta.permissions().mode() & 0o777,
            0o600,
            "the file root installs must not be readable or writable by anyone else"
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\"file_format_version\": \"1.0.0\"}\n"
        );
        assert!(path.starts_with(&dir));
        assert!(path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("host-manifest-"));
    }
    assert!(
        !path.exists(),
        "the staging file must not outlive the write"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn staging_creates_the_directory_0700_and_never_reuses_a_name() {
    let dir = scratch("staging-dir").join("nested/tmp");
    let a = StagedTemp::create(&dir, "a").unwrap();
    let b = StagedTemp::create(&dir, "b").unwrap();
    assert_ne!(a.path, b.path, "names are randomized per write");
    assert_eq!(
        std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let _ = std::fs::remove_dir_all(dir.parent().unwrap().parent().unwrap());
}

#[tokio::test]
async fn a_current_destination_is_skipped_without_prompting() {
    let dir = scratch("skip");
    let dest = dir.join("active_runtime.x86_64.json");
    let dylib = PathBuf::from("/repo/runtime/lib.dylib");
    let content = crate::util::host_manifest_file_bytes(&dylib);
    std::fs::write(&dest, &content).unwrap();

    let (ctx, seen) = ctx_with(StageOptions::default());
    assert_eq!(
        write_host_manifest_privileged(&ctx, &dylib, &dest)
            .await
            .unwrap(),
        PrivilegedWrite::Skipped
    );
    // No prompt was even announced — that is what keeps demo.sh and Sabrage
    // from re-authorizing each other's installs.
    assert!(seen.lock().unwrap().is_empty());

    // install.sh's own currency test is command-substitution based, so extra
    // trailing newlines on disk still read as current.
    std::fs::write(&dest, format!("{content}\n\n")).unwrap();
    assert_eq!(
        write_host_manifest_privileged(&ctx, &dylib, &dest)
            .await
            .unwrap(),
        PrivilegedWrite::Skipped
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_dry_run_plans_the_staging_write_and_the_elevated_argv() {
    let dir = scratch("dry-run");
    let dest = dir.join("openxr/1/active_runtime.x86_64.json");
    // The dylib path is the only variable part of the rendered manifest, so
    // marking it is how we prove the JSON never reaches a command line.
    let dylib = PathBuf::from("/repo/NEVER-ON-A-COMMAND-LINE/liboxrsys-runtime.dylib");
    let content = host_manifest_bytes(&dylib);

    let (ctx, seen) = ctx_with(StageOptions {
        dry_run: true,
        bottle_name: Some("Beat Saber".into()),
        ..Default::default()
    });
    assert_eq!(
        write_host_manifest_privileged(&ctx, &dylib, &dest)
            .await
            .unwrap(),
        // Planned, never Written: the caller renders this, and a preview
        // that prints the completed-install row is indistinguishable from
        // a completed install in the event log.
        PrivilegedWrite::Planned
    );

    assert!(!dest.exists());
    // A dry run prompts for nothing, so NeedsAdmin — which the GUI renders
    // as "macOS will ask for your password" — must not be emitted; the
    // "would prompt" row stands in for it.
    let evs = seen.lock().unwrap().clone();
    assert!(
        !evs.iter()
            .any(|e| matches!(e, StageEvent::NeedsAdmin { .. })),
        "{evs:?}"
    );
    assert!(
        evs.iter().any(|e| matches!(
            e,
            StageEvent::Line { text, severity: crate::events::Severity::Info, .. }
                if text == WOULD_PROMPT_DRY_RUN
        )),
        "{evs:?}"
    );

    let plan = ctx.executor.planned();
    let kinds: Vec<PlannedKind> = plan.iter().map(|p| p.kind).collect();
    let method = AdminMethod::detect();
    let spawns = elevation_argv(method, Path::new("/x"), &dest).len();
    let mut want = vec![PlannedKind::CreateDir, PlannedKind::Write];
    want.extend(std::iter::repeat_n(PlannedKind::Spawn, spawns));
    assert_eq!(kinds, want);

    assert_eq!(plan[0].dst.as_deref(), Some(sabrage_temp_dir().as_path()));
    let staged = plan[1].dst.clone().expect("staged path");
    assert!(staged.starts_with(sabrage_temp_dir()));
    assert_eq!(plan[1].reason, format!("{} bytes", content.len()));

    for action in &plan[2..] {
        assert!(
            action
                .reason
                .contains(&staged.to_string_lossy().into_owned())
                && action.reason.contains(&dest.to_string_lossy().into_owned()),
            "the elevated command names the staging file and the destination: {}",
            action.reason
        );
    }
    // The JSON itself never rides on a command line.
    for action in &plan {
        assert!(
            !action.reason.contains("NEVER-ON-A-COMMAND-LINE"),
            "content leaked into {:?}",
            action
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The cancel arm must not return while the child it just signalled is
/// still running: the caller's next act is to drop the staging file, and a
/// privileged `install` reading it would get an ENOENT half-way through the
/// pipeline's only root write.
#[tokio::test]
async fn a_cancelled_child_is_reaped_before_the_call_returns() {
    let dir = scratch("cancel-reap");
    let marker = dir.join("marker");
    let argv = vec![
        OsString::from("/bin/sh"),
        OsString::from("-c"),
        OsString::from(format!(
            "sleep 0.4; touch {}",
            shell_quote(&marker.to_string_lossy())
        )),
    ];
    // Cancelled *after* the child is up: an already-cancelled token never reaches
    // the spawn (see tests::an_already_cancelled_token_never_spawns_the_elevated_child),
    // so pre-cancelling here would prove nothing about the reap.
    let cancel = CancellationToken::new();
    let stopper = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(60)).await;
        stopper.cancel();
    });

    let err = run_inheriting(&argv, &cancel).await.unwrap_err();
    assert!(matches!(err, SabrageError::Cancelled), "{err:?}");
    // Reaped, not merely signalled: the child is gone *now*, so the work it
    // would have done cannot land after this point.
    assert!(!marker.exists(), "the child was killed before it wrote");
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(
        !marker.exists(),
        "a signalled-but-unreaped child would have finished by now"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The other end of the same rule: a run that is *already* over spawns
/// nothing. `select!` can only cancel a prompt that is already on screen —
/// the dialog would still have appeared, and `sudo` would still have taken
/// the terminal, after Stop.
#[tokio::test]
async fn an_already_cancelled_token_never_spawns_the_elevated_child() {
    let dir = scratch("cancel-nospawn");
    let marker = dir.join("marker");
    let argv = vec![
        OsString::from("/bin/sh"),
        OsString::from("-c"),
        OsString::from(format!("touch {}", shell_quote(&marker.to_string_lossy()))),
    ];
    let cancel = CancellationToken::new();
    cancel.cancel();

    for err in [
        run_inheriting(&argv, &cancel).await.unwrap_err(),
        run_capturing(&argv, &cancel).await.unwrap_err(),
    ] {
        assert!(matches!(err, SabrageError::Cancelled), "{err:?}");
    }
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(!marker.exists(), "a cancelled run spawned a child anyway");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Install layer 3 can hand this function a run that was cancelled while
/// `reg add` was in flight. Nothing below the announcement is undoable from
/// our side (the elevated `/bin/sh` is not in our process tree), so the
/// token is checked before `NeedsAdmin`, before the staging file, and before
/// any child — the user does not get an authorization prompt after Stop.
#[tokio::test]
async fn a_cancelled_run_neither_announces_nor_stages_the_privileged_write() {
    let dir = scratch("cancel-before-prompt");
    let dest = dir.join("openxr/1/active_runtime.x86_64.json");
    let dylib = PathBuf::from("/repo/runtime/lib.dylib");
    assert!(!crate::util::host_manifest_is_current(
        &dest,
        &crate::util::render_host_manifest(&dylib)
    ));

    let seen = Arc::new(StdMutex::new(Vec::new()));
    let s = seen.clone();
    let sink: EventSink = Arc::new(move |ev| s.lock().unwrap().push(ev));
    let cancel = CancellationToken::new();
    cancel.cancel();
    // Not a dry run: this is the branch that would prompt.
    let ctx = StageCtx::new(
        Paths::new("/nonexistent/sabrage/repo"),
        StageOptions::default(),
        sink,
        cancel,
    );
    assert!(!ctx.executor.is_dry_run());

    let before = staging_file_count();
    let err = write_host_manifest_privileged(&ctx, &dylib, &dest)
        .await
        .unwrap_err();
    assert!(matches!(err, SabrageError::Cancelled), "{err:?}");
    assert!(!dest.exists());
    let evs = seen.lock().unwrap();
    assert!(evs.is_empty(), "nothing is announced after Stop: {evs:#?}");
    assert_eq!(
        staging_file_count(),
        before,
        "a cancelled run stages nothing"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// How many `host-manifest-*.json` are staged right now. Read-only.
fn staging_file_count() -> usize {
    std::fs::read_dir(sabrage_temp_dir())
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .starts_with("host-manifest-")
                })
                .count()
        })
        .unwrap_or(0)
}

/// install.sh's escaping is two substitutions, so a path with a raw control
/// character renders non-JSON over the root:wheel file the OpenXR loader
/// reads. The guard fails closed, before the currency test and before any
/// prompt; `json_escape_string` stays those same two substitutions, so every
/// accepted path renders byte-identically on both front-ends.
#[tokio::test]
async fn a_control_character_in_the_path_is_refused_before_any_prompt() {
    let dir = scratch("control-char");
    let dest = dir.join("active_runtime.x86_64.json");
    let (ctx, seen) = ctx_with(StageOptions {
        dry_run: true,
        bottle_name: Some("Beat Saber".into()),
        ..Default::default()
    });

    for nasty in ["/repo/two\nlines/lib.dylib", "/repo/a\tb/lib.dylib"] {
        let dylib = PathBuf::from(nasty);
        let err = write_host_manifest_privileged(&ctx, &dylib, &dest)
            .await
            .unwrap_err();
        assert!(matches!(err, SabrageError::Fatal { .. }), "{err:?}");
        assert!(err.to_string().contains("control character"), "{err}");
    }
    // …and an ordinary path is untouched by the guard.
    assert!(reject_unrepresentable_manifest_path(
        &ctx,
        Path::new("/Users/me/wine vr/it's/liboxrsys-runtime.dylib")
    )
    .is_ok());

    assert!(
        !seen
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, StageEvent::NeedsAdmin { .. })),
        "no prompt is announced for a path that cannot be written"
    );
    assert!(!dest.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// The staging file is deleted on every exit path *except* cancellation,
/// where the elevated (root) write may still be reading it.
#[test]
fn a_defused_staging_file_outlives_its_drop() {
    let dir = scratch("defuse");
    let path;
    {
        let mut staged = StagedTemp::create(&dir, "x").unwrap();
        path = staged.path.clone();
        staged.defuse();
    }
    assert!(
        path.exists(),
        "a cancelled elevation must not pull the file out from under root"
    );

    // …and the next privileged write is what collects it — but only once it
    // is old enough that no concurrent run can still be using it.
    sweep_stale_staging(&dir);
    assert!(path.exists(), "a fresh staging file is never swept");
    let old = std::time::SystemTime::now() - STAGING_SWEEP_AGE - Duration::from_secs(60);
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(old))
        .unwrap();
    let bystander = dir.join("session.json");
    std::fs::write(&bystander, "keep me").unwrap();
    sweep_stale_staging(&dir);
    assert!(!path.exists(), "a stale staging file is swept");
    assert!(bystander.exists(), "only host-manifest-*.json is swept");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_announcement_names_the_method_detect_actually_picked() {
    assert_eq!(
        needs_admin_reason(AdminMethod::Osascript),
        NEEDS_ADMIN_REASON
    );
    assert_eq!(
        needs_admin_reason(AdminMethod::Sudo),
        NEEDS_ADMIN_REASON_SUDO
    );
    // The sudo path prompts on a terminal that may well be *behind* the
    // window the user is looking at (`npm run tauri dev`), so the row has
    // to say where to look.
    assert!(
        needs_admin_reason(AdminMethod::Sudo).contains("terminal that launched Sabrage"),
        "{}",
        NEEDS_ADMIN_REASON_SUDO
    );
    assert!(!needs_admin_reason(AdminMethod::Osascript).contains("terminal"));
    // Both still say what the password buys, which is the other half of
    // design-core § 5. Privilege boundary, implementation step 4.
    for method in [AdminMethod::Osascript, AdminMethod::Sudo] {
        assert!(
            needs_admin_reason(method).contains("host OpenXR registration")
                && needs_admin_reason(method).contains("repo path changes"),
            "{}",
            needs_admin_reason(method)
        );
    }
}

#[test]
fn a_refused_cp_into_a_bundle_is_upgraded_like_a_refused_write() {
    let (ctx, seen) = ctx_with(StageOptions {
        bottle_name: Some("BS".into()),
        ..Default::default()
    });
    let backup = PathBuf::from(
        "/Applications/CrossOver.app/Contents/SharedSupport/CrossOver/lib/dxmt.stock-backup",
    );
    let refused = || SabrageError::ChildFailed {
        argv0: "cp".into(),
        status: 1,
        tail: vec![format!("cp: {}: Permission denied", backup.display())],
    };

    let upgraded = upgrade_child_write_error(&ctx, refused(), &backup);
    assert_eq!(upgraded.kind(), "tcc_denied");
    assert!(matches!(&upgraded, SabrageError::TccDenied { path } if path == &backup));
    let evs = seen.lock().unwrap().clone();
    assert_eq!(evs.len(), 1, "the prose is emitted once, here: {evs:?}");
    let StageEvent::Fatal {
        message, remedy, ..
    } = &evs[0]
    else {
        panic!("expected Fatal, got {:?}", evs[0]);
    };
    assert_eq!(message, &app_management_message(&backup));
    assert_eq!(
        remedy.as_deref(),
        Some(app_management_remedy(Some("BS")).as_str())
    );

    // A destination outside a bundle, a tail that is not a refusal, and a
    // non-child error all pass through untouched and emit nothing more.
    let outside = PathBuf::from("/usr/local/share/openxr/1");
    assert_eq!(
        upgrade_child_write_error(&ctx, refused(), &outside).kind(),
        "child_failed"
    );
    let full = SabrageError::ChildFailed {
        argv0: "cp".into(),
        status: 1,
        tail: vec!["cp: no space left on device".into()],
    };
    assert_eq!(
        upgrade_child_write_error(&ctx, full, &backup).kind(),
        "child_failed"
    );
    assert_eq!(
        upgrade_child_write_error(&ctx, SabrageError::Cancelled, &backup).kind(),
        "cancelled"
    );
    assert_eq!(seen.lock().unwrap().len(), 1, "no further events");
}

#[test]
fn a_permission_tail_is_recognised_in_either_spelling() {
    assert!(tail_is_permission_denied(&[
        "cp: /x: Permission denied".into()
    ]));
    assert!(tail_is_permission_denied(&[
        "cp: /x: Operation not permitted".into()
    ]));
    assert!(!tail_is_permission_denied(&["cp: /x: No such file".into()]));
    assert!(!tail_is_permission_denied(&[]));
}

#[test]
fn declined_and_failed_authorization_are_told_apart_by_stderr() {
    assert!(is_user_cancelled(
        "execution error: User canceled. (-128)\n"
    ));
    assert!(!is_user_cancelled(
        "execution error: The administrator user name or password was incorrect. (-60007)\n"
    ));
    assert!(!is_user_cancelled(""));
}

#[test]
fn write_errors_classify_by_errno_and_path() {
    use std::io::{Error, ErrorKind};
    let in_bundle = Path::new(
            "/Applications/CrossOver.app/Contents/SharedSupport/CrossOver/lib/dxmt/x86_64-windows/d3d11.dll",
        );
    let outside = Path::new("/usr/local/share/openxr/1/active_runtime.x86_64.json");
    let table: &[(ErrorKind, &Path, WriteErrorKind)] = &[
        (
            ErrorKind::PermissionDenied,
            in_bundle,
            WriteErrorKind::TccAppManagementLikely,
        ),
        (
            ErrorKind::PermissionDenied,
            outside,
            WriteErrorKind::PermissionDenied,
        ),
        (ErrorKind::NotFound, in_bundle, WriteErrorKind::Other),
        (ErrorKind::NotFound, outside, WriteErrorKind::Other),
        (ErrorKind::StorageFull, in_bundle, WriteErrorKind::Other),
    ];
    for (kind, path, want) in table {
        assert_eq!(
            classify_write_error(&Error::from(*kind), path),
            *want,
            "{kind:?} at {}",
            path.display()
        );
    }
}

#[test]
fn only_a_tcc_shaped_io_error_is_upgraded() {
    let (ctx, seen) = ctx_with(StageOptions {
        bottle_name: Some("BS".into()),
        ..Default::default()
    });
    let bundle = PathBuf::from("/Applications/CrossOver.app/Contents/x/d3d11.dll");

    let upgraded = upgrade_write_error(
        &ctx,
        SabrageError::io(
            &bundle,
            std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        ),
    );
    assert_eq!(upgraded.kind(), "tcc_denied");
    assert!(matches!(&upgraded, SabrageError::TccDenied { path } if path == &bundle));

    // The prose reaches the user through the event, once.
    let evs = seen.lock().unwrap().clone();
    assert_eq!(evs.len(), 1);
    let StageEvent::Fatal {
        message, remedy, ..
    } = &evs[0]
    else {
        panic!("expected Fatal, got {:?}", evs[0]);
    };
    assert_eq!(message, &app_management_message(&bundle));
    assert_eq!(
        remedy.as_deref(),
        Some(app_management_remedy(Some("BS")).as_str())
    );

    // Anything else is passed through untouched, with no extra event.
    let passthrough = upgrade_write_error(
        &ctx,
        SabrageError::io(&bundle, std::io::Error::from(std::io::ErrorKind::NotFound)),
    );
    assert_eq!(passthrough.kind(), "io");
    let outside = PathBuf::from("/usr/local/share/openxr/1/active_runtime.x86_64.json");
    let denied = upgrade_write_error(
        &ctx,
        SabrageError::io(
            &outside,
            std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        ),
    );
    assert_eq!(denied.kind(), "io");
    assert_eq!(seen.lock().unwrap().len(), 1, "no further events");
}

#[test]
fn the_app_management_strings_stay_a_hypothesis_with_a_way_out() {
    let msg = app_management_message(Path::new("/Applications/CrossOver.app/lib/x.dll"));
    assert!(
        msg.contains("likely macOS App Management permission"),
        "{msg}"
    );
    assert!(msg.contains("sudo cannot grant"), "{msg}");

    let remedy = app_management_remedy(Some("BeatSaber"));
    assert!(remedy.contains(APP_MANAGEMENT_SETTINGS_URL), "{remedy}");
    assert!(remedy.contains("relaunch Sabrage"), "{remedy}");
    assert!(
        remedy.contains("./demo.sh install --bottle BeatSaber"),
        "{remedy}"
    );
    assert!(
        app_management_remedy(None).contains("./demo.sh install --bottle <name>"),
        "doctor's placeholder is <name>"
    );
}

#[test]
fn the_declined_fallback_is_doctors_host_manifest_remedy() {
    let (ctx, _) = ctx_with(StageOptions {
        bottle_name: Some("BeatSaber".into()),
        ..Default::default()
    });
    assert_eq!(
        terminal_fallback_remedy(&ctx),
        "./demo.sh install --bottle BeatSaber (sudo writes it)"
    );
}

#[test]
fn app_bundle_detection_is_component_wise() {
    assert!(is_inside_app_bundle(Path::new(
        "/Applications/CrossOver.app/Contents/SharedSupport/CrossOver/lib/dxmt/d3d11.dll"
    )));
    assert!(is_inside_app_bundle(Path::new("/x/Sabrage.app")));
    assert!(!is_inside_app_bundle(Path::new(
        "/usr/local/share/openxr/1/active_runtime.x86_64.json"
    )));
    // A file merely *named* like a bundle deeper in a normal tree still
    // counts — it is a bundle by macOS's own rule.
    assert!(is_inside_app_bundle(Path::new("/home/me/thing.app/x")));
    assert!(!is_inside_app_bundle(Path::new("/home/me/thing.apple/x")));
}

#[test]
fn support_dir_is_under_application_support() {
    assert!(sabrage_temp_dir().ends_with("Library/Application Support/Sabrage/tmp"));
    assert!(
        !sabrage_temp_dir().starts_with("/tmp"),
        "staging in a world-writable directory is a swap race"
    );
}

#[test]
fn admin_method_is_decided_by_the_controlling_terminal_never_by_stdout() {
    // Injected inputs, so the rule is pinned without needing a tty (or the
    // absence of one) in the test process.
    let table: &[(bool, bool, AdminMethod)] = &[
        // stdin is the terminal: the ordinary `sabrage install` case, and
        // `sabrage install | tee log` — stdout is a pipe there and it must
        // make no difference, exactly like `./demo.sh install | tee`.
        (true, true, AdminMethod::Sudo),
        // stdin redirected from a file, terminal still reachable through
        // /dev/tty — where sudo reads the password from anyway.
        (false, true, AdminMethod::Sudo),
        // A tty on stdin with no controlling terminal is not a shape macOS
        // produces, but "either probe is sufficient" is the rule.
        (true, false, AdminMethod::Sudo),
        // The GUI: no terminal at all, so the authorization dialog.
        (false, false, AdminMethod::Osascript),
    ];
    for (stdin_is_tty, controlling_tty, want) in table {
        assert_eq!(
            AdminMethod::choose(*stdin_is_tty, *controlling_tty),
            *want,
            "stdin_is_tty={stdin_is_tty} controlling_tty={controlling_tty}"
        );
    }
}
