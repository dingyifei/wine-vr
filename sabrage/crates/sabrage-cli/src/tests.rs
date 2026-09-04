use super::*;
// Production code counts SIGINTs in an `AtomicUsize`; the tests still use a
// plain flag for "did this callback fire".
use std::sync::atomic::AtomicBool;
use std::sync::Mutex as StdMutex;

fn args(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn parses_bottle_and_bs_dir() {
    let a = args(&["--bottle", "Steam", "--bs-dir", "/games/bs"]);
    let parsed = parse_doctor_args(&a).unwrap();
    assert_eq!(parsed.bottle.as_deref(), Some("Steam"));
    assert_eq!(parsed.bs_dir, Some(PathBuf::from("/games/bs")));
    assert_eq!(parsed.tap, None);
}

#[test]
fn parses_tap_and_boolean_flags() {
    let a = args(&[
        "--tap",
        "/tmp/tap.txt",
        "--wired",
        "--no-audio",
        "--no-dashboard",
        "--verbose",
    ]);
    let parsed = parse_doctor_args(&a).unwrap();
    assert_eq!(parsed.tap, Some(PathBuf::from("/tmp/tap.txt")));
    assert!(parsed.wired);
    assert!(parsed.no_audio);
    assert!(parsed.no_dashboard);
    assert!(parsed.verbose);
}

#[test]
fn doctor_parse_errors_match_demo_sh_verbatim() {
    let cases: &[(&str, &[&str], &str)] = &[
        (
            "missing --bottle value",
            &["--bottle"],
            "error: --bottle needs a name",
        ),
        (
            "missing --bs-dir value",
            &["--bs-dir"],
            "error: --bs-dir needs a path",
        ),
        (
            "missing doctor-only --tap value",
            &["--tap"],
            "error: --tap needs a path",
        ),
        (
            "unknown flag",
            &["--nope"],
            "error: unknown argument '--nope'",
        ),
        (
            "bare positional hits demo.sh's `*)` arm",
            &["Steam"],
            "error: unknown argument 'Steam'",
        ),
        (
            "first bad argument wins, no aggregation",
            &["--bottle", "Steam", "--nope", "--bs-dir", "/x"],
            "error: unknown argument '--nope'",
        ),
    ];
    for (label, argv, expected) in cases {
        assert_eq!(
            parse_doctor_args(&args(argv)).unwrap_err(),
            *expected,
            "{label}"
        );
    }
}

/// A14-4: demo.sh's `CMD="${1:-}"; shift` consumes the first token, so
/// the flag loop only sees the tail — `--bottle Steam run` reports `Steam`
/// and never reaches `--bottle`'s "needs a name" branch. `Ok(())` means
/// the shell fell through to its unknown-`CMD` `case` arm, whose text the
/// caller's `_` arm prints, not this helper.
#[test]
fn unknown_command_outcome_mirrors_the_shells_shift_then_parse() {
    let cases: &[(&str, &[&str], Result<(), &str>)] = &[
        (
            "A14-4 regression: flags before the command report the tail's first bad token",
            &["--bottle", "Steam", "run"],
            Err("error: unknown argument 'Steam'"),
        ),
        (
            "A14-4's other case: a clean tail falls through to the caller's usage/exit-2 arm",
            &["--verbose", "--bottle", "X"],
            Ok(()),
        ),
    ];
    for (label, argv, expected) in cases {
        let got = unknown_command_outcome(&args(argv));
        assert_eq!(
            got.as_ref().map(|_| ()).map_err(String::as_str),
            *expected,
            "{label}"
        );
    }
}

#[test]
fn doctor_outcome_rows_match_lib_sh_verbatim() {
    let cases: &[(&str, CheckOutcome, bool, bool, Option<&str>)] = &[
        (
            "pass, no color: three-space OK gap",
            CheckOutcome::pass("sys.arch", "Apple Silicon (Apple M-series)"),
            false,
            false,
            Some("  OK   Apple Silicon (Apple M-series)"),
        ),
        (
            "pass, verbose with no detail: nothing extra",
            CheckOutcome::pass("sys.arch", "Apple Silicon (Apple M-series)"),
            false,
            true,
            Some("  OK   Apple Silicon (Apple M-series)"),
        ),
        (
            "warn, no color: one-space WARN gap",
            CheckOutcome::warn("bottle.template", "bottle template is not win11_64"),
            false,
            false,
            Some("  WARN bottle template is not win11_64"),
        ),
        (
            "fail with remedy: seven-space remedy continuation",
            CheckOutcome::fail(
                "cx.version",
                "CrossOver 26.1 < 26.2",
                "upgrade CrossOver to 26.2+",
            ),
            false,
            false,
            Some("  FAIL CrossOver 26.1 < 26.2\n       remedy: upgrade CrossOver to 26.2+"),
        ),
        (
            "fail without remedy: no remedy continuation",
            CheckOutcome::fail_bare("sys.arch", "not an Apple Silicon Mac (x86_64)"),
            false,
            false,
            Some("  FAIL not an Apple Silicon Mac (x86_64)"),
        ),
        (
            "info: two-space indent, no status label",
            CheckOutcome::info(
                "game.present",
                "Beat Saber check skipped (needs --bottle or --bs-dir)",
            ),
            false,
            false,
            Some("  Beat Saber check skipped (needs --bottle or --bs-dir)"),
        ),
        (
            "skipped: prints nothing",
            CheckOutcome::skipped("hs.client", "no adb device".into()),
            false,
            false,
            None,
        ),
        (
            "not implemented: prints nothing",
            CheckOutcome::not_implemented("dep.dxmt"),
            false,
            false,
            None,
        ),
        (
            "pass with colors: the ANSI pair wraps the label, not the message",
            CheckOutcome::pass("sys.arch", "Apple Silicon"),
            true,
            false,
            Some("  \x1b[32mOK\x1b[0m   Apple Silicon"),
        ),
    ];
    for (label, outcome, colors, verbose, expected) in cases {
        assert_eq!(
            format_outcome(outcome, *colors, *verbose).as_deref(),
            *expected,
            "{label}"
        );
    }
}

/// A3b-3 (round 2): `detail` is sabrage-only and must never appear in
/// default (non-verbose) output — that is doctor's shell-parity byte
/// stream. Only `verbose: true` surfaces it, as a `detail:` continuation
/// line at the same column `remedy:` uses.
#[test]
fn detail_is_hidden_by_default_and_shown_only_when_verbose() {
    let o = CheckOutcome::warn("cfg.session-pins", "could not inspect session.json: oops")
        .with_detail("read error: oops");
    assert_eq!(
        format_outcome(&o, false, false).as_deref(),
        Some("  WARN could not inspect session.json: oops")
    );
    assert_eq!(
        format_outcome(&o, false, true).as_deref(),
        Some("  WARN could not inspect session.json: oops\n       detail: read error: oops")
    );
}

#[test]
fn footer_matches_doctor_sh_verbatim_both_branches() {
    assert_eq!(
        format_footer(0, "Steam", false),
        "doctor: all checks passed — ./demo.sh run --bottle Steam"
    );
    assert_eq!(
        format_footer(3, "Steam", false),
        "doctor: 3 check(s) failed — remedies above"
    );
    assert_eq!(
        format_footer(0, "<name>", true),
        format!(
            "doctor: {ANSI_GREEN}all checks passed{ANSI_RESET} — ./demo.sh run --bottle <name>"
        )
    );
}

#[test]
fn stage_parses_bottle_bs_dir_and_dry_run() {
    let a = args(&["--bottle", "Steam", "--bs-dir", "/games/bs", "--dry-run"]);
    let parsed = parse_stage_args(&a).unwrap();
    assert_eq!(parsed.bottle.as_deref(), Some("Steam"));
    assert_eq!(parsed.bs_dir, Some(PathBuf::from("/games/bs")));
    assert!(parsed.dry_run);
    assert!(!parsed.quiet);
}

#[test]
fn stage_parses_every_reserved_and_sabrage_only_flag() {
    let a = args(&[
        "--wired",
        "--no-audio",
        "--no-dashboard",
        "--verbose",
        "--dry-run",
        "--quiet",
    ]);
    let parsed = parse_stage_args(&a).unwrap();
    assert!(parsed.wired);
    assert!(parsed.no_audio);
    assert!(parsed.no_dashboard);
    assert!(parsed.verbose);
    assert!(parsed.dry_run);
    assert!(parsed.quiet);
}

#[test]
fn stage_missing_value_messages_match_demo_sh_verbatim() {
    assert_eq!(
        parse_stage_args(&args(&["--bottle"])).unwrap_err(),
        "error: --bottle needs a name"
    );
    assert_eq!(
        parse_stage_args(&args(&["--bs-dir"])).unwrap_err(),
        "error: --bs-dir needs a path"
    );
}

#[test]
fn stage_unknown_argument_is_the_exit_2_path() {
    // `--tap` is a doctor-only sabrage addition (demo.sh has no such flag),
    // so every stage command must reject it exactly like any other
    // unrecognized flag.
    assert_eq!(
        parse_stage_args(&args(&["--tap", "/tmp/x"])).unwrap_err(),
        "error: unknown argument '--tap'"
    );
    assert_eq!(
        parse_stage_args(&args(&["Steam"])).unwrap_err(),
        "error: unknown argument 'Steam'"
    );
}

#[test]
fn stage_first_bad_argument_wins_no_aggregation() {
    let a = args(&["--dry-run", "--nope", "--bottle", "Steam"]);
    assert_eq!(
        parse_stage_args(&a).unwrap_err(),
        "error: unknown argument '--nope'"
    );
}

#[test]
fn merge_stage_options_forwards_all_six_flags_onto_the_env_base() {
    // Matching demo.sh's flag grammar is not enough: each flag that has a
    // `StageOptions` field must actually reach it rather than being parsed
    // and dropped (`--quiet` is CLI-rendering only and has no field).
    let parsed = StageArgs {
        bottle: Some("Steam".to_string()),
        bs_dir: Some(PathBuf::from("/games/bs")),
        wired: true,
        no_audio: true,
        no_dashboard: true,
        verbose: true,
        dry_run: true,
        quiet: false,
    };
    let opts = merge_stage_options(StageOptions::default(), &parsed);
    assert_eq!(opts.bottle_name.as_deref(), Some("Steam"));
    assert_eq!(opts.bs_dir_override, Some(PathBuf::from("/games/bs")));
    assert!(opts.wired);
    assert!(opts.no_audio);
    assert!(opts.no_dashboard);
    assert!(opts.verbose);
    assert!(opts.dry_run);
}

#[test]
fn merge_stage_options_env_base_survives_when_no_flag_overrides_it() {
    // WINEVR_* is the base; a flag *absent* from the command line must
    // never clear a value the environment already set (the same
    // "exporting over whatever the shell already had" precedence
    // `StageOptions::from_env` documents).
    let env_opts = StageOptions {
        bottle_name: Some("FromEnv".to_string()),
        wired: true,
        no_audio: true,
        no_dashboard: true,
        verbose: true,
        ..Default::default()
    };
    let opts = merge_stage_options(env_opts, &StageArgs::default());
    assert_eq!(opts.bottle_name.as_deref(), Some("FromEnv"));
    assert!(opts.wired && opts.no_audio && opts.no_dashboard && opts.verbose);
    // `dry_run` has no env counterpart: `parsed.dry_run` (false here)
    // wins outright rather than only-if-true.
    assert!(!opts.dry_run);
}

#[test]
fn merge_stage_options_empty_cli_values_clear_a_preset_env_base() {
    // A14-1: `--bottle ""`/`--bs-dir ""` (from a wrapper interpolating
    // the flag even when its variable is unset) must behave like
    // `${WINEVR_BOTTLE:-}`/`${WINEVR_BS_DIR:-<default>}`: empty = absent,
    // not an override to the empty string.
    let env_opts = StageOptions {
        bottle_name: Some("Steam".to_string()),
        bs_dir_override: Some(PathBuf::from("/preset")),
        ..Default::default()
    };
    let parsed = StageArgs {
        bottle: Some(String::new()),
        bs_dir: Some(PathBuf::new()),
        ..Default::default()
    };
    let opts = merge_stage_options(env_opts, &parsed);
    assert_eq!(opts.bottle_name, None);
    assert_eq!(opts.bs_dir_override, None);
}

#[test]
fn merge_stage_options_empty_cli_values_stay_none_with_no_preset() {
    let parsed = StageArgs {
        bottle: Some(String::new()),
        bs_dir: Some(PathBuf::new()),
        ..Default::default()
    };
    let opts = merge_stage_options(StageOptions::default(), &parsed);
    assert_eq!(opts.bottle_name, None);
    assert_eq!(opts.bs_dir_override, None);
}

#[test]
fn merge_doctor_options_empty_cli_values_clear_a_preset_env_base() {
    // Same rule, doctor's merge path.
    let env_opts = CheckOptions {
        bottle_name: Some("Steam".to_string()),
        bs_dir_override: Some(PathBuf::from("/preset")),
        ..Default::default()
    };
    let parsed = DoctorArgs {
        bottle: Some(String::new()),
        bs_dir: Some(PathBuf::new()),
        ..Default::default()
    };
    let opts = merge_doctor_options(env_opts, &parsed);
    assert_eq!(opts.bottle_name, None);
    assert_eq!(opts.bs_dir_override, None);
}

#[test]
fn merge_doctor_options_empty_cli_values_stay_none_with_no_preset() {
    let parsed = DoctorArgs {
        bottle: Some(String::new()),
        bs_dir: Some(PathBuf::new()),
        ..Default::default()
    };
    let opts = merge_doctor_options(CheckOptions::default(), &parsed);
    assert_eq!(opts.bottle_name, None);
    assert_eq!(opts.bs_dir_override, None);
}

#[test]
fn stage_line_rendering_matches_the_doctor_row_shapes() {
    // Same helpers, so the two front-ends' text can never drift apart.
    assert_eq!(
        format_line_event(Severity::Ok, "ActiveRuntime registered", None, false),
        "  OK   ActiveRuntime registered"
    );
    assert_eq!(
        format_line_event(
            Severity::Warn,
            "bottle template is not win11_64",
            None,
            false
        ),
        "  WARN bottle template is not win11_64"
    );
    assert_eq!(
        format_line_event(
            Severity::Fail,
            "copy failed: a -> b",
            Some("re-run ./demo.sh build"),
            false
        ),
        "  FAIL copy failed: a -> b\n       remedy: re-run ./demo.sh build"
    );
    assert_eq!(
        format_line_event(
            Severity::Info,
            "downloading DXMT fork artifacts ...",
            None,
            false
        ),
        "  downloading DXMT fork artifacts ..."
    );
}

#[test]
fn closing_line_matches_each_stage_script_verbatim() {
    assert_eq!(
        closing_line(Stage::Setup, "<name>").as_deref(),
        Some("\nsetup complete — next: ./demo.sh build")
    );
    assert_eq!(
        closing_line(Stage::Build, "<name>").as_deref(),
        Some("\nbuild complete — next: ./demo.sh install --bottle <name>")
    );
    // build's line is the literal placeholder even when a bottle *is*
    // known — build.sh's own text never interpolates one.
    assert_eq!(
        closing_line(Stage::Build, "Steam").as_deref(),
        Some("\nbuild complete — next: ./demo.sh install --bottle <name>")
    );
    assert_eq!(
        closing_line(Stage::Install, "Steam").as_deref(),
        Some("\ninstall complete — next: ./demo.sh run --bottle Steam")
    );
    // stop.sh has no closing banner at all.
    assert_eq!(closing_line(Stage::Stop, "Steam"), None);
    assert_eq!(closing_line(Stage::Run, "Steam"), None);
}

// `stage_event_lines` is a pure projection, so "renders to nothing" is a
// value these tests assert on directly rather than an intercepted
// `println!`.

#[test]
fn text_event_prints_verbatim_including_empty_and_leading_spaces() {
    let empty = StageEvent::Text {
        run_id: Default::default(),
        step: None,
        text: String::new(),
    };
    assert_eq!(
        stage_event_lines(&empty, "<name>", Colors::OFF, false),
        vec![RenderedLine::Stdout(String::new())],
        "run.sh's `print \"\"` must still emit its (empty) line"
    );

    let indented = StageEvent::Text {
        run_id: Default::default(),
        step: Some("run.8.launch".to_string()),
        text: "   exe: Z:\\Beat Saber.exe".to_string(),
    };
    assert_eq!(
        stage_event_lines(&indented, "<name>", Colors::OFF, false),
        vec![RenderedLine::Stdout(
            "   exe: Z:\\Beat Saber.exe".to_string()
        )],
        "leading spaces belong to the shell's own line, not our indent"
    );
}

#[test]
fn structured_only_events_render_no_console_line() {
    use sabrage_core::{FixAction, Gate};

    let check = StageEvent::Check {
        run_id: Default::default(),
        step: "run.1.preflight".to_string(),
        outcome: CheckOutcome::pass("run.wine-exec", "wine present"),
        gate: Gate::Block,
    };
    assert_eq!(
        stage_event_lines(&check, "<name>", Colors::OFF, false),
        vec![]
    );

    let launched = StageEvent::Launched {
        run_id: Default::default(),
        pid: 4242,
        start_time: 1,
        log_path: "/x/beatsaber-20260829-000000.log".to_string(),
        started_at_unix_ms: 1,
    };
    assert_eq!(
        stage_event_lines(&launched, "<name>", Colors::OFF, false),
        vec![]
    );

    // Every fix already emits its own shell-verbatim `ok`/`warn` `Line` on
    // the same sink; printing this too would double the console row.
    let auto_fixed = StageEvent::AutoFixed {
        run_id: Default::default(),
        step: "run.1.preflight".to_string(),
        fix: FixAction::SetGraphicsBackend,
        description: "bottle graphics backend forced to dxmt".to_string(),
    };
    assert_eq!(
        stage_event_lines(&auto_fixed, "<name>", Colors::OFF, false),
        vec![]
    );

    let progress = StageEvent::Progress {
        run_id: Default::default(),
        step: "setup.1".to_string(),
        label: "curl".to_string(),
        current: 10,
        total: Some(100),
    };
    assert_eq!(
        stage_event_lines(&progress, "<name>", Colors::OFF, false),
        vec![]
    );
}

#[test]
fn needs_admin_event_renders_as_an_info_row() {
    let ev = StageEvent::NeedsAdmin {
        run_id: Default::default(),
        step: "install.4.host-manifest".to_string(),
        reason: "macOS will ask for your password".to_string(),
    };
    assert_eq!(
        stage_event_lines(&ev, "<name>", Colors::OFF, false),
        vec![RenderedLine::Stdout(
            "  macOS will ask for your password".to_string()
        )]
    );
}

#[test]
fn stage_started_section_and_line_events_compose_through_the_shared_helpers() {
    let started = StageEvent::StageStarted {
        run_id: Default::default(),
        stage: Stage::Run,
    };
    assert_eq!(
        stage_event_lines(&started, "<name>", Colors::OFF, false),
        vec![RenderedLine::Stdout("== wine-vr demo run ==".to_string())]
    );

    let section = StageEvent::Section {
        run_id: Default::default(),
        title: "Goldberg".to_string(),
    };
    assert_eq!(
        stage_event_lines(&section, "<name>", Colors::OFF, false),
        vec![RenderedLine::Stdout("-- Goldberg".to_string())]
    );

    let line = StageEvent::Line {
        run_id: Default::default(),
        step: None,
        severity: Severity::Ok,
        text: "wineserver down".to_string(),
        remedy: None,
    };
    assert_eq!(
        stage_event_lines(&line, "<name>", Colors::OFF, false),
        vec![RenderedLine::Stdout("  OK   wineserver down".to_string())]
    );
}

#[test]
fn output_event_respects_quiet_and_routes_by_stream() {
    let out = StageEvent::Output {
        run_id: Default::default(),
        step: "build.1".to_string(),
        stream: Stream::Stdout,
        chunk: "[1/9] cc foo.c".to_string(),
        end: sabrage_core::process::ChunkEnd::Lf,
    };
    assert_eq!(
        stage_event_lines(&out, "<name>", Colors::OFF, false),
        vec![RenderedLine::Stdout("[1/9] cc foo.c".to_string())]
    );
    assert_eq!(
        stage_event_lines(&out, "<name>", Colors::OFF, true),
        vec![],
        "--quiet suppresses a child's own output passthrough"
    );

    let err = StageEvent::Output {
        run_id: Default::default(),
        step: "build.1".to_string(),
        stream: Stream::Stderr,
        chunk: "warning: x".to_string(),
        end: sabrage_core::process::ChunkEnd::Lf,
    };
    assert_eq!(
        stage_event_lines(&err, "<name>", Colors::OFF, false),
        vec![RenderedLine::Stderr("warning: x".to_string())]
    );
}

#[test]
fn cr_chunk_repaints_only_on_a_real_terminal() {
    // A14-3: a bare `\r` progress-bar segment (curl/ninja-style) repaints
    // the current terminal line when the destination is a real tty, but
    // falls back to the ordinary newline-per-chunk treatment (identical
    // to `Lf`/`Eof`) for a non-tty consumer such as a redirected file or
    // `--tap` pipe, which has no "current line" to overwrite.
    let tty = Colors {
        stdout: false,
        stderr: false,
        stdout_tty: true,
        stderr_tty: true,
    };
    let stdout_cr = StageEvent::Output {
        run_id: Default::default(),
        step: "install.1".to_string(),
        stream: Stream::Stdout,
        chunk: "###### 42%".to_string(),
        end: sabrage_core::process::ChunkEnd::Cr,
    };
    assert_eq!(
        stage_event_lines(&stdout_cr, "<name>", tty, false),
        vec![RenderedLine::StdoutRepaint("###### 42%".to_string())],
        "stdout is a tty: a Cr chunk repaints in place"
    );
    assert_eq!(
        stage_event_lines(&stdout_cr, "<name>", Colors::OFF, false),
        vec![RenderedLine::Stdout("###### 42%".to_string())],
        "stdout is not a tty: a Cr chunk falls back to one line per chunk"
    );

    let stderr_cr = StageEvent::Output {
        run_id: Default::default(),
        step: "install.1".to_string(),
        stream: Stream::Stderr,
        chunk: "###### 42%".to_string(),
        end: sabrage_core::process::ChunkEnd::Cr,
    };
    assert_eq!(
        stage_event_lines(&stderr_cr, "<name>", tty, false),
        vec![RenderedLine::StderrRepaint("###### 42%".to_string())],
        "stderr is a tty: a Cr chunk repaints in place"
    );
    assert_eq!(
        stage_event_lines(&stderr_cr, "<name>", Colors::OFF, false),
        vec![RenderedLine::Stderr("###### 42%".to_string())],
        "stderr is not a tty: a Cr chunk falls back to one line per chunk"
    );

    // `Eof` (end of stream, no delimiter at all) never repaints, tty or
    // not — it's a distinct terminator from `Cr`, not a synonym for it.
    let stdout_eof = StageEvent::Output {
        run_id: Default::default(),
        step: "install.1".to_string(),
        stream: Stream::Stdout,
        chunk: "done".to_string(),
        end: sabrage_core::process::ChunkEnd::Eof,
    };
    assert_eq!(
        stage_event_lines(&stdout_eof, "<name>", tty, false),
        vec![RenderedLine::Stdout("done".to_string())],
        "Eof never repaints even on a tty"
    );

    assert_eq!(
        stage_event_lines(&stdout_cr, "<name>", tty, true),
        vec![],
        "--quiet suppresses a repaint chunk too"
    );
}

#[test]
fn fatal_and_stage_finished_events_compose_through_the_shared_helpers() {
    let fatal = StageEvent::Fatal {
        run_id: Default::default(),
        message: "boom".to_string(),
        remedy: Some("fix it".to_string()),
        fix: None,
    };
    // Finding #4: `Fatal.remedy` must reach a CLI user — it carries the
    // App Management deep link that finding #6's
    // `privilege::upgrade_write_error` puts on the `Fatal` it emits.
    assert_eq!(
        stage_event_lines(&fatal, "<name>", Colors::OFF, false),
        vec![
            RenderedLine::Stderr("FATAL boom".to_string()),
            RenderedLine::Stderr("       remedy: fix it".to_string()),
        ]
    );

    let finished_ok = StageEvent::StageFinished {
        run_id: Default::default(),
        stage: Stage::Setup,
        ok: true,
        exit_code_equiv: 0,
    };
    assert_eq!(
        stage_event_lines(&finished_ok, "<name>", Colors::OFF, false),
        vec![RenderedLine::Stdout(
            "\nsetup complete — next: ./demo.sh build".to_string()
        )]
    );

    // run.sh's own closing text arrives as a `Text` event, not a banner
    // here — `closing_line(Run, _)` is `None`.
    let finished_run_ok = StageEvent::StageFinished {
        run_id: Default::default(),
        stage: Stage::Run,
        ok: true,
        exit_code_equiv: 0,
    };
    assert_eq!(
        stage_event_lines(&finished_run_ok, "<name>", Colors::OFF, false),
        vec![]
    );

    let finished_failed = StageEvent::StageFinished {
        run_id: Default::default(),
        stage: Stage::Setup,
        ok: false,
        exit_code_equiv: 1,
    };
    assert_eq!(
        stage_event_lines(&finished_failed, "<name>", Colors::OFF, false),
        vec![]
    );
}

// A14-5: color is gated per stream, not once for the process.

#[test]
fn colors_from_gates_each_stream_on_its_own_tty_unless_no_color() {
    let cases: &[(&str, (bool, bool, bool), Colors)] = &[
        (
            "NO_COLOR forces both streams off regardless of tty",
            (true, true, true),
            Colors {
                stdout: false,
                stderr: false,
                stdout_tty: true,
                stderr_tty: true,
            },
        ),
        (
            "stdout piped, stderr a tty — the other half of the same bug",
            (false, false, true),
            Colors {
                stdout: false,
                stderr: true,
                stdout_tty: false,
                stderr_tty: true,
            },
        ),
        (
            "mirrored: stdout a tty, stderr redirected to a file",
            (false, true, false),
            Colors {
                stdout: true,
                stderr: false,
                stdout_tty: true,
                stderr_tty: false,
            },
        ),
    ];
    for (label, (no_color, stdout_tty, stderr_tty), expected) in cases {
        assert_eq!(
            colors_from(*no_color, *stdout_tty, *stderr_tty),
            *expected,
            "{label}"
        );
    }
}

#[test]
fn fatal_uses_stderr_colors_while_a_line_event_uses_stdout_colors() {
    // A14-5: `Fatal` gates its color on stderr's terminal-ness and an
    // ordinary `Line` on stdout's, so with stdout piped and stderr a
    // terminal the `Fatal` row is colored and the `Line` row is not.
    let colors = Colors {
        stdout: false,
        stderr: true,
        stdout_tty: false,
        stderr_tty: true,
    };

    let fatal = StageEvent::Fatal {
        run_id: Default::default(),
        message: "boom".to_string(),
        remedy: None,
        fix: None,
    };
    assert_eq!(
        stage_event_lines(&fatal, "<name>", colors, false),
        vec![RenderedLine::Stderr(format!(
            "{ANSI_RED}FATAL{ANSI_RESET} boom"
        ))]
    );

    let line = StageEvent::Line {
        run_id: Default::default(),
        step: None,
        severity: Severity::Fail,
        text: "copy failed".to_string(),
        remedy: None,
    };
    assert_eq!(
        stage_event_lines(&line, "<name>", colors, false),
        vec![RenderedLine::Stdout("  FAIL copy failed".to_string())]
    );

    // Mirrored: stdout a terminal (colored), stderr piped (no color) —
    // `Fatal` must lose its color while the `Line` gains it.
    let mirrored = Colors {
        stdout: true,
        stderr: false,
        stdout_tty: true,
        stderr_tty: false,
    };
    assert_eq!(
        stage_event_lines(&fatal, "<name>", mirrored, false),
        vec![RenderedLine::Stderr("FATAL boom".to_string())]
    );
    assert_eq!(
        stage_event_lines(&line, "<name>", mirrored, false),
        vec![RenderedLine::Stdout(format!(
            "  {ANSI_RED}FAIL{ANSI_RESET} copy failed"
        ))]
    );
}

#[test]
fn dry_run_plan_has_a_header_then_one_line_per_action_in_order() {
    use sabrage_core::executor::PlannedKind;

    let plan = vec![
        PlannedAction {
            kind: PlannedKind::Copy,
            src: Some(PathBuf::from("/a")),
            dst: Some(PathBuf::from("/b")),
            reason: "differs from source".to_string(),
        },
        PlannedAction {
            kind: PlannedKind::Skip,
            src: None,
            dst: Some(PathBuf::from("/c")),
            reason: "already current".to_string(),
        },
    ];
    let lines = render_dry_run_plan(&plan);
    assert_eq!(
        lines,
        vec![
            "-- plan (dry run)".to_string(),
            format!("  {}", plan[0]),
            format!("  {}", plan[1]),
        ]
    );
}

#[test]
fn dry_run_plan_with_no_actions_says_so_rather_than_printing_nothing() {
    assert_eq!(
        render_dry_run_plan(&[]),
        vec![
            "-- plan (dry run)".to_string(),
            "  (nothing planned)".to_string(),
        ]
    );
}

// No test below may reach a stage body: setup/build/install/run touch the
// real machine. Argument errors return before `resolve_repo_root`, and
// `run_all`'s bottle precheck (hand-built `opts`) dies before `run_chain`.

fn capturing_sink() -> (EventSink, Arc<StdMutex<Vec<StageEvent>>>) {
    let seen = Arc::new(StdMutex::new(Vec::new()));
    let s = seen.clone();
    (Arc::new(move |ev| s.lock().unwrap().push(ev)), seen)
}

#[tokio::test]
async fn run_stage_rejects_a_bad_argument_before_touching_anything() {
    assert_eq!(cmd_stage(Stage::Run, &args(&["--nope"])).await, 2);
    assert_eq!(cmd_stage(Stage::Run, &args(&["--bottle"])).await, 2);
}

#[tokio::test]
async fn all_rejects_a_bad_argument_before_touching_anything() {
    assert_eq!(cmd_all(&args(&["--nope"])).await, 2);
    assert_eq!(cmd_all(&args(&["--bs-dir"])).await, 2);
}

#[tokio::test]
async fn run_chain_stops_at_the_first_nonzero_exit_code() {
    let calls = Arc::new(StdMutex::new(Vec::new()));
    let c = calls.clone();
    let code = run_chain(&Stage::ALL_CHAIN, move |stage| {
        let c = c.clone();
        async move {
            c.lock().unwrap().push(stage);
            if stage == Stage::Build {
                5
            } else {
                0
            }
        }
    })
    .await;
    assert_eq!(code, 5);
    assert_eq!(
        *calls.lock().unwrap(),
        vec![Stage::Setup, Stage::Build],
        "must stop before Install/Run once Build has already failed"
    );
}

#[tokio::test]
async fn run_chain_runs_every_stage_when_all_of_them_succeed() {
    let calls = Arc::new(StdMutex::new(Vec::new()));
    let c = calls.clone();
    let code = run_chain(&Stage::ALL_CHAIN, move |stage| {
        let c = c.clone();
        async move {
            c.lock().unwrap().push(stage);
            0
        }
    })
    .await;
    assert_eq!(code, 0);
    assert_eq!(*calls.lock().unwrap(), Stage::ALL_CHAIN.to_vec());
}

#[tokio::test]
async fn run_all_requires_a_bottle_before_touching_run_stage() {
    let paths = Paths::new("/nonexistent/sabrage/repo");
    let opts = StageOptions {
        bottle_name: Some("Sabrage-Cli-Test-Bottle-Does-Not-Exist".to_string()),
        ..Default::default()
    };
    let (sink, seen) = capturing_sink();
    let code = run_all(&paths, &opts, &sink).await;
    assert_eq!(code, 1);
    let seen = seen.lock().unwrap();
    assert!(
        seen.iter()
            .all(|e| !matches!(e, StageEvent::StageStarted { .. })),
        "require_bottle must fail before any stage starts: {seen:?}"
    );
    assert!(seen.iter().any(|e| matches!(e, StageEvent::Fatal { .. })));
}

#[tokio::test]
async fn run_all_with_no_bottle_name_dies_before_touching_run_stage() {
    let paths = Paths::new("/nonexistent/sabrage/repo");
    let (sink, seen) = capturing_sink();
    let code = run_all(&paths, &StageOptions::default(), &sink).await;
    assert_eq!(code, 1);
    assert!(seen
        .lock()
        .unwrap()
        .iter()
        .all(|e| !matches!(e, StageEvent::StageStarted { .. })));
}

#[test]
fn watch_flag_invokes_the_callback_once_the_flag_is_set() {
    // A private static, never touched by `watch_termination_signals`'s
    // real `SIGNAL_COUNT` — this exercises the same polling primitive in
    // isolation, with no signal handler involved.
    static TEST_COUNT: AtomicUsize = AtomicUsize::new(0);
    let fired = Arc::new(AtomicBool::new(false));
    let f = fired.clone();
    watch_flag(&TEST_COUNT, move || f.store(true, Ordering::SeqCst));

    assert!(
        !fired.load(Ordering::SeqCst),
        "fired before the flag was set"
    );
    TEST_COUNT.fetch_add(1, Ordering::Relaxed);
    std::thread::sleep(CTRL_C_POLL_INTERVAL * 4);
    assert!(fired.load(Ordering::SeqCst), "callback never ran");
}

#[test]
fn watcher_keeps_polling_past_the_first_fire_so_a_second_signal_is_reachable() {
    // Finding #7: the watcher keeps polling past its first callback, so a
    // second Ctrl-C is observed. Uses `watch_flag_with_second_action`
    // because the real path (`terminate_via_default_disposition_of_last_signal`)
    // `raise()`s on this process and would kill the test binary.
    static TEST_COUNT: AtomicUsize = AtomicUsize::new(0);
    let first_calls = Arc::new(AtomicUsize::new(0));
    let second_calls = Arc::new(AtomicUsize::new(0));
    let f1 = first_calls.clone();
    let f2 = second_calls.clone();
    watch_flag_with_second_action(
        &TEST_COUNT,
        move || {
            f1.fetch_add(1, Ordering::SeqCst);
        },
        move || {
            f2.fetch_add(1, Ordering::SeqCst);
        },
    );

    TEST_COUNT.fetch_add(1, Ordering::Relaxed);
    std::thread::sleep(CTRL_C_POLL_INTERVAL * 4);
    assert_eq!(first_calls.load(Ordering::SeqCst), 1, "first signal");
    assert_eq!(
        second_calls.load(Ordering::SeqCst),
        0,
        "second action must not fire on the first signal"
    );

    TEST_COUNT.fetch_add(1, Ordering::Relaxed);
    std::thread::sleep(CTRL_C_POLL_INTERVAL * 4);
    assert_eq!(
        first_calls.load(Ordering::SeqCst),
        1,
        "first action must not fire again on the second signal"
    );
    assert_eq!(second_calls.load(Ordering::SeqCst), 1, "second signal");
}

#[test]
fn two_signals_inside_one_poll_interval_are_not_collapsed_into_one() {
    // Finding #6: a burst inside one poll interval (a fast double-tap, or
    // `kill -INT` pair) counts as two signals and is fatal. Both deliveries
    // land before the watcher's first poll — the collapsing window.
    static TEST_COUNT: AtomicUsize = AtomicUsize::new(0);
    TEST_COUNT.fetch_add(1, Ordering::Relaxed);
    TEST_COUNT.fetch_add(1, Ordering::Relaxed);

    let first_calls = Arc::new(AtomicUsize::new(0));
    let second_calls = Arc::new(AtomicUsize::new(0));
    let f1 = first_calls.clone();
    let f2 = second_calls.clone();
    watch_flag_with_second_action(
        &TEST_COUNT,
        move || {
            f1.fetch_add(1, Ordering::SeqCst);
        },
        move || {
            f2.fetch_add(1, Ordering::SeqCst);
        },
    );

    std::thread::sleep(CTRL_C_POLL_INTERVAL * 4);
    assert_eq!(first_calls.load(Ordering::SeqCst), 1, "first signal");
    assert_eq!(
        second_calls.load(Ordering::SeqCst),
        1,
        "the second signal of a same-interval burst must still be fatal"
    );
}

#[test]
fn a_second_signal_of_a_different_kind_is_still_fatal_and_reraises_that_kind() {
    // Production shares one `SIGNAL_COUNT` between `SIGINT` and `SIGTERM`
    // handlers, with `LAST_SIGNAL` holding the most recent delivery: first
    // of either kind cancels, second of either kind is fatal and re-raises
    // whichever just arrived. Process-local fakes stand in because a real
    // second signal would kill the test binary.
    static TEST_COUNT: AtomicUsize = AtomicUsize::new(0);
    static TEST_LAST_KIND: AtomicI32 = AtomicI32::new(0);

    let deliver = |kind: i32| {
        TEST_LAST_KIND.store(kind, Ordering::Relaxed);
        TEST_COUNT.fetch_add(1, Ordering::Relaxed);
    };

    let first_calls = Arc::new(AtomicUsize::new(0));
    let reraised_kind = Arc::new(AtomicI32::new(0));
    let f1 = first_calls.clone();
    let rk = reraised_kind.clone();
    watch_flag_with_second_action(
        &TEST_COUNT,
        move || {
            f1.fetch_add(1, Ordering::SeqCst);
        },
        move || {
            rk.store(TEST_LAST_KIND.load(Ordering::Relaxed), Ordering::SeqCst);
        },
    );

    deliver(SIGINT); // stands in for the first Ctrl-C / `kill -INT`
    std::thread::sleep(CTRL_C_POLL_INTERVAL * 4);
    assert_eq!(
        first_calls.load(Ordering::SeqCst),
        1,
        "first signal of either kind cancels"
    );
    assert_eq!(
        reraised_kind.load(Ordering::SeqCst),
        0,
        "second action must not fire on the first signal"
    );

    deliver(SIGTERM); // a *different* kind arrives second — still fatal
    std::thread::sleep(CTRL_C_POLL_INTERVAL * 4);
    assert_eq!(
        first_calls.load(Ordering::SeqCst),
        1,
        "first action must not fire again on the second signal"
    );
    assert_eq!(
        reraised_kind.load(Ordering::SeqCst),
        SIGTERM,
        "the second signal reraises whichever kind actually arrived, not the first kind"
    );
}
