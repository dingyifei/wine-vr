//! `sabrage` — the native Sabrage pipeline CLI.
//!
//! Owns argument parsing and turning a [`sabrage_core::StageEvent`] stream into
//! the exact console text the equivalent `demo.sh` stage prints; the stage
//! bodies themselves live in `sabrage-core`. `all` is a caller-level loop over
//! [`sabrage_core::Stage::ALL_CHAIN`], never a sixth [`Stage`]. Anything else on
//! the command line falls through to the usage text, exit 2.
//!
//! `demo.sh`'s flag loop runs before `case "$CMD"`, so every subcommand accepts
//! all six `WINEVR_*` flags even though only `run` (and `all`) reads the last
//! four; row shapes follow `scripts/demo/{doctor,setup,build,install,stop}.sh`.
//! `--tap`, `--dry-run` and `--quiet` are sabrage-only; colors are gated on
//! isatty + `NO_COLOR` where the shell emits ANSI unconditionally (PARITY.md
//! § CLI / GUI, "`--dry-run` / `--quiet` flags"; PARITY.md § Doctor / checks,
//! "Console colors gated on isatty").
//!
//! Argument and output parity is pinned by
//! tests::{stage_parses_every_reserved_and_sabrage_only_flag,
//! doctor_outcome_rows_match_lib_sh_verbatim,
//! closing_line_matches_each_stage_script_verbatim}.

use std::env;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::exit;
use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use sabrage_core::checks::{run_doctor, CheckCtx, CheckOptions, CheckOutcome, CheckStatus};
use sabrage_core::tap::render_tap;
use sabrage_core::{
    require_bottle, resolve_repo_root, run_stage, EventSink, Paths, PlannedAction, SabrageError,
    Severity, Stage, StageCtx, StageEvent, StageOptions, StageOutcome, Stream,
};

const ANSI_GREEN: &str = "\x1b[32m";
const ANSI_YELLOW: &str = "\x1b[33m";
const ANSI_RED: &str = "\x1b[31m";
const ANSI_RESET: &str = "\x1b[0m";

const BANNER: &str = "== wine-vr demo doctor ==";

const USAGE: &str = "\
Usage: sabrage <command> [options]

Commands:
  doctor              check every prerequisite, print remedies (like ./demo.sh doctor)
  setup               fetch submodules + pinned binaries, write config (like ./demo.sh setup)
  build               build oxrsys (with ALVR core) + wineopenxr (like ./demo.sh build)
  install             install the bridge into CrossOver + the bottle + host loader
                      (like ./demo.sh install; the only stage that may ask for a password)
  run                 launch Beat Saber through the bridge (like ./demo.sh run)
  stop                stop the game + wineserver for a bottle (like ./demo.sh stop)
  all                 setup, then build, then install, then run, stopping at the first
                      failure (like ./demo.sh all)

Options (every command above):
  --bottle <name>     CrossOver bottle (or env WINEVR_BOTTLE)
  --bs-dir <path>     Beat Saber 1.29.4 install dir (or env WINEVR_BS_DIR)
  --no-audio          run only: leave the Mac's audio output alone (or env WINEVR_NO_AUDIO)
  --no-dashboard      run only: don't launch the ALVR server dashboard (or env WINEVR_NO_DASHBOARD)
  --wired             run only: USB streaming — forward tcp:9943/tcp:9944 instead of
                      clearing them (or env WINEVR_WIRED)
  --verbose           run only: restore the wine/openxr debug firehose in the console/log
                      (or env WINEVR_VERBOSE)

  (every command above accepts all six flags, matching demo.sh's own flag loop, which
  runs before it dispatches on the subcommand; only run — and, by extension, all — reads
  the last four. sabrage's own --help lists --wired/--verbose in full: demo.sh's usage
  text truncates them, a declared divergence, see sabrage/PARITY.md)

Options (doctor only):
  --tap <file>        write parity tap lines (\"<slug> <status>\") to file, truncated first

Options (setup/build/install/run/stop/all only):
  --dry-run           plan the stage's writes/spawns without touching anything (sabrage-only)
  --quiet             don't pass a child process's own output through (sabrage-only)

  sabrage --version   print the CLI version
  sabrage -h|--help   print this message
";

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().skip(1).collect();

    if args.is_empty() {
        print!("{USAGE}");
        exit(2);
    }

    match args[0].as_str() {
        "--version" => {
            println!("sabrage {}", env!("CARGO_PKG_VERSION"));
            exit(0);
        }
        "-h" | "--help" => {
            print!("{USAGE}");
            exit(0);
        }
        "doctor" => cmd_doctor(&args[1..]),
        "setup" => exit(cmd_stage(Stage::Setup, &args[1..]).await),
        "build" => exit(cmd_stage(Stage::Build, &args[1..]).await),
        "install" => exit(cmd_stage(Stage::Install, &args[1..]).await),
        "run" => exit(cmd_stage(Stage::Run, &args[1..]).await),
        "stop" => exit(cmd_stage(Stage::Stop, &args[1..]).await),
        "all" => exit(cmd_all(&args[1..]).await),
        _ => {
            if let Err(msg) = unknown_command_outcome(&args) {
                eprintln!("{msg}");
                exit(2);
            }
            print!("{USAGE}");
            exit(2);
        }
    }
}

/// The `_` arm's decision for an unrecognized first token, factored into a pure
/// helper so it is testable without `exit` (A14-4).
///
/// `demo.sh` consumes `CMD="${1:-}"` unconditionally before its flag loop runs,
/// so `args[0]` is treated as already-consumed and `args[1..]` is parsed with
/// the six shared flags. `Err` is the shell's own first-bad-argument message;
/// `Ok(())` means the shell would have fallen through to its unknown-`CMD` arm
/// instead — this file's usage-text-then-exit-2 fallback. Pinned by
/// tests::unknown_command_outcome_mirrors_the_shells_shift_then_parse.
fn unknown_command_outcome(args: &[String]) -> Result<(), String> {
    parse_stage_args(&args[1..]).map(|_| ())
}

/// Print a bootstrap failure as `error: <msg>`, plus [`SabrageError`]'s remedy
/// line when it carries one.
///
/// Sabrage-only: repo-root and `HOME` resolution run before any
/// [`StageCtx`]/[`EventSink`] exists (doctor has no `StageCtx` at all), so there
/// is no `StageEvent::Fatal` or `die()` line to ride on and no shell text to
/// match — finding the repo root has no shell equivalent (`paths.rs`'s doc).
fn print_bootstrap_error(e: &SabrageError) {
    eprintln!("error: {e}");
    if let Some(remedy) = e.remedy() {
        eprintln!("       remedy: {remedy}");
    }
}

/// Parsed `doctor` arguments, before merging onto the `WINEVR_*` environment.
#[derive(Debug, Default, PartialEq, Eq)]
struct DoctorArgs {
    bottle: Option<String>,
    bs_dir: Option<PathBuf>,
    tap: Option<PathBuf>,
    wired: bool,
    no_audio: bool,
    no_dashboard: bool,
    verbose: bool,
}

/// The six `demo.sh`-verbatim flags every stage script (and `doctor`) shares:
/// `--bottle`/`--bs-dir`/`--no-audio`/`--no-dashboard`/`--wired`/`--verbose`.
/// Shared by `parse_doctor_args`/`parse_stage_args` so each `demo.sh`-verbatim
/// error string lives in exactly one place (S-C3-cli-ipc).
#[derive(Debug, Default, PartialEq, Eq)]
struct CommonArgs {
    bottle: Option<String>,
    bs_dir: Option<PathBuf>,
    wired: bool,
    no_audio: bool,
    no_dashboard: bool,
    verbose: bool,
}

/// Try to consume one of the six `CommonArgs` flags at `args[i]`.
///
/// `Ok(Some(next_i))` — recognized and consumed, resume the caller's loop at
/// `next_i`. `Ok(None)` — `args[i]` is not one of the six; the caller tries its
/// own extra flags (`--tap`, `--dry-run`, `--quiet`) before the shared "unknown
/// argument" error. `Err` — a value-taking flag with no value; first bad
/// argument wins, no aggregation.
fn parse_common_flag(
    out: &mut CommonArgs,
    args: &[String],
    i: usize,
) -> Result<Option<usize>, String> {
    match args[i].as_str() {
        "--bottle" => {
            let v = args
                .get(i + 1)
                .ok_or_else(|| "error: --bottle needs a name".to_string())?;
            out.bottle = Some(v.clone());
            Ok(Some(i + 2))
        }
        "--bs-dir" => {
            let v = args
                .get(i + 1)
                .ok_or_else(|| "error: --bs-dir needs a path".to_string())?;
            out.bs_dir = Some(PathBuf::from(v));
            Ok(Some(i + 2))
        }
        "--no-audio" => {
            out.no_audio = true;
            Ok(Some(i + 1))
        }
        "--no-dashboard" => {
            out.no_dashboard = true;
            Ok(Some(i + 1))
        }
        "--wired" => {
            out.wired = true;
            Ok(Some(i + 1))
        }
        "--verbose" => {
            out.verbose = true;
            Ok(Some(i + 1))
        }
        _ => Ok(None),
    }
}

/// Parse `doctor`'s argument list. Returns the `demo.sh`-verbatim error message
/// (sans the `error: ` prefix's destination — the caller decides where it goes)
/// on the first bad argument, exactly like the shell's `case` loop: no
/// aggregation, first failure wins.
fn parse_doctor_args(args: &[String]) -> Result<DoctorArgs, String> {
    let mut common = CommonArgs::default();
    let mut tap = None;
    let mut i = 0usize;
    while i < args.len() {
        if let Some(next) = parse_common_flag(&mut common, args, i)? {
            i = next;
            continue;
        }
        match args[i].as_str() {
            "--tap" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| "error: --tap needs a path".to_string())?;
                tap = Some(PathBuf::from(v));
                i += 2;
            }
            other => return Err(format!("error: unknown argument '{other}'")),
        }
    }
    Ok(DoctorArgs {
        bottle: common.bottle,
        bs_dir: common.bs_dir,
        tap,
        wired: common.wired,
        no_audio: common.no_audio,
        no_dashboard: common.no_dashboard,
        verbose: common.verbose,
    })
}

/// Merge parsed `doctor` flags onto the env-derived base — `WINEVR_*` is the
/// base, flags override — `merge_stage_options`'s `doctor` counterpart,
/// separate so it is testable without a repo root or a [`CheckCtx`].
///
/// A14-1: an explicit empty `--bottle`/`--bs-dir` clears the env-derived value
/// rather than overriding it with `Some("")`, because `demo.sh`'s
/// `${WINEVR_BOTTLE:-}` treats an empty export as absent. Pinned by
/// tests::merge_doctor_options_empty_cli_values_clear_a_preset_env_base.
fn merge_doctor_options(env_opts: CheckOptions, parsed: &DoctorArgs) -> CheckOptions {
    let mut opts = env_opts;
    if let Some(b) = &parsed.bottle {
        opts.bottle_name = (!b.is_empty()).then(|| b.clone());
    }
    if let Some(d) = &parsed.bs_dir {
        opts.bs_dir_override = (!d.as_os_str().is_empty()).then(|| d.clone());
    }
    if parsed.wired {
        opts.wired = true;
    }
    if parsed.no_audio {
        opts.no_audio = true;
    }
    if parsed.no_dashboard {
        opts.no_dashboard = true;
    }
    if parsed.verbose {
        opts.verbose = true;
    }
    opts
}

fn cmd_doctor(args: &[String]) -> ! {
    let parsed = match parse_doctor_args(args) {
        Ok(p) => p,
        Err(msg) => {
            eprintln!("{msg}");
            exit(2);
        }
    };

    let repo_root = match resolve_repo_root(None) {
        Ok(p) => p,
        Err(e) => {
            print_bootstrap_error(&e);
            exit(e.exit_code());
        }
    };

    let opts = merge_doctor_options(CheckOptions::from_env(), &parsed);
    let verbose = opts.verbose;

    let ctx = CheckCtx::new(Paths::new(&repo_root), opts);
    let colors = use_colors().stdout;

    println!("{BANNER}");
    let report = run_doctor(&ctx, |outcome| {
        if let Some(line) = format_outcome(&outcome, colors, verbose) {
            println!("{line}");
        }
    });

    println!();
    println!(
        "{}",
        format_footer(report.fail_count, ctx.bottle_label(), colors)
    );

    if let Some(tap_path) = &parsed.tap {
        // "truncate first" (the `--help` text): `fs::write` creates-or-truncates
        // and writes the payload in one shot, never `tap::append_tap`'s
        // incremental `>>` channel.
        if let Err(e) = std::fs::write(tap_path, render_tap(&report.outcomes)) {
            eprintln!(
                "warning: could not write --tap file {}: {e}",
                tap_path.display()
            );
        }
    }

    // doctor.sh: `[ -n "${WINEVR_DOCTOR_SOFT:-}" ] || exit "$FAILCOUNT"` — a
    // non-empty value short-circuits the exit, so the process exits 0.
    if env_flag("WINEVR_DOCTOR_SOFT") {
        exit(0);
    }
    exit(report.exit_code());
}

/// `[ -n "${NAME:-}" ]`: any non-empty value is truthy, matching every
/// `WINEVR_*` boolean flag across the shell pipeline.
fn env_flag(name: &str) -> bool {
    env::var_os(name).is_some_and(|v| !v.is_empty())
}

/// `lib.sh`'s `ok()` row: `"  ${_G}OK${_N}   $*"`.
fn ok_row(text: &str, colors: bool) -> String {
    format!("  {}   {}", label("OK", ANSI_GREEN, colors), text)
}

/// `lib.sh`'s `warn()` row: `"  ${_Y}WARN${_N} $*"`.
fn warn_row(text: &str, colors: bool) -> String {
    format!("  {} {}", label("WARN", ANSI_YELLOW, colors), text)
}

/// `lib.sh`'s `fail()` row, plus its optional `remedy:` line:
/// `"  ${_R}FAIL${_N} $1"` / `"       remedy: $2"`.
fn fail_row(text: &str, remedy: Option<&str>, colors: bool) -> String {
    let mut s = format!("  {} {}", label("FAIL", ANSI_RED, colors), text);
    if let Some(remedy) = remedy {
        s.push('\n');
        s.push_str("       remedy: ");
        s.push_str(remedy);
    }
    s
}

/// `lib.sh`'s `info()` row: `"  $*"` — two-space indent, no marker.
fn info_row(text: &str) -> String {
    format!("  {text}")
}

/// `lib.sh`'s `ok`/`warn`/`fail`/`info`, transcribed onto a [`CheckOutcome`].
///
/// Returns `None` for `Skipped`/`NotImplemented` and for a `quiet` pass — rows
/// that only ever reach doctor.sh's silent `tap` channel, never stdout.
///
/// `verbose` (A3b-3) appends one `       detail: <d>` continuation for a row
/// carrying a [`CheckOutcome::detail`], the sabrage-only explainability field
/// zsh's row helpers have no slot for; with `verbose` false (doctor's default)
/// the output stays byte-identical to the shell's. Pinned by
/// tests::{doctor_outcome_rows_match_lib_sh_verbatim,
/// detail_is_hidden_by_default_and_shown_only_when_verbose}.
fn format_outcome(o: &CheckOutcome, colors: bool, verbose: bool) -> Option<String> {
    if o.quiet {
        return None;
    }
    let mut line = match o.status {
        CheckStatus::Pass => Some(ok_row(&o.message, colors)),
        CheckStatus::Warn => Some(warn_row(&o.message, colors)),
        CheckStatus::Fail => Some(fail_row(&o.message, o.remedy.as_deref(), colors)),
        CheckStatus::Info => Some(info_row(&o.message)),
        CheckStatus::Skipped | CheckStatus::NotImplemented => None,
    };
    if verbose {
        if let (Some(l), Some(detail)) = (&mut line, &o.detail) {
            l.push('\n');
            l.push_str("       detail: ");
            l.push_str(detail);
        }
    }
    line
}

/// The identical four rows, transcribed onto a [`sabrage_core::StageEvent::Line`]
/// instead — the `setup`/`build`/`install`/`stop` counterpart of
/// [`format_outcome`]. Both delegate to the same `{ok,warn,fail,info}_row`
/// helpers, so a source string reads the same whichever front-end printed it.
fn format_line_event(severity: Severity, text: &str, remedy: Option<&str>, colors: bool) -> String {
    match severity {
        Severity::Ok => ok_row(text, colors),
        Severity::Warn => warn_row(text, colors),
        Severity::Fail => fail_row(text, remedy, colors),
        Severity::Info => info_row(text),
    }
}

/// `lib.sh`'s `die()`: `"${_R}FATAL${_N} $*"` — no leading indent (unlike
/// `ok`/`warn`/`fail`/`info`, which all indent two) and no separate `remedy:`
/// line. A `die` call folds its remedy into the message text at the call site
/// (`require_bottle`'s two-line message is the canonical example), so this
/// prints no line `die()` never printed.
fn fatal_line(message: &str, colors: bool) -> String {
    format!("{} {}", label("FATAL", ANSI_RED, colors), message)
}

/// `fatal_line` plus a `       remedy: <r>` continuation when the event carries
/// one, at `fail_row`'s own indent.
///
/// Sabrage-only: `lib.sh`'s `die` has no remedy slot, so this adds text where the
/// shell prints none — PARITY.md § CLI / GUI, "A `FATAL` row may be followed by".
/// Two `Fatal`s come from [`sabrage_core::privilege`] rather than a `die`-shaped
/// call site and carry their only actionable instruction in `remedy`.
fn fatal_lines(message: &str, remedy: Option<&str>, colors: bool) -> Vec<String> {
    let mut lines = vec![fatal_line(message, colors)];
    if let Some(remedy) = remedy {
        lines.push(format!("       remedy: {remedy}"));
    }
    lines
}

/// doctor.sh's footer line: `doctor: all checks passed — ./demo.sh run --bottle
/// <label>` when `fail_count` is 0, else `doctor: <n> check(s) failed — remedies
/// above`.
///
/// The caller prints the preceding blank line itself. `fail_count` is the
/// *uncapped* tally; the 255 cap in
/// [`sabrage_core::checks::DoctorReport::exit_code`] is an exit-code concern, not
/// a display one. Pinned by
/// tests::footer_matches_doctor_sh_verbatim_both_branches.
fn format_footer(fail_count: usize, bottle_label: &str, colors: bool) -> String {
    if fail_count == 0 {
        format!(
            "doctor: {} — ./demo.sh run --bottle {bottle_label}",
            label("all checks passed", ANSI_GREEN, colors)
        )
    } else {
        format!(
            "doctor: {} — remedies above",
            label(&format!("{fail_count} check(s) failed"), ANSI_RED, colors)
        )
    }
}

/// The blank-line-prefixed "next:" banner each stage script ends with on
/// success, verbatim from `scripts/demo/{setup,build,install}.sh`'s closing
/// `print` line; `None` for `stop` and `run`, which print none.
///
/// `bottle_label` is used only by `install`'s line, by which point
/// `require_bottle` has guaranteed a real name; `build`'s line quotes the literal
/// `<name>` placeholder because build.sh's own text does. Pinned by
/// tests::closing_line_matches_each_stage_script_verbatim.
fn closing_line(stage: Stage, bottle_label: &str) -> Option<String> {
    match stage {
        Stage::Setup => Some("\nsetup complete — next: ./demo.sh build".to_string()),
        Stage::Build => {
            Some("\nbuild complete — next: ./demo.sh install --bottle <name>".to_string())
        }
        Stage::Install => Some(format!(
            "\ninstall complete — next: ./demo.sh run --bottle {bottle_label}"
        )),
        Stage::Stop | Stage::Run => None,
    }
}

fn label(text: &str, color: &str, colors: bool) -> String {
    if colors {
        format!("{color}{text}{ANSI_RESET}")
    } else {
        text.to_string()
    }
}

/// Per-stream color eligibility: `isatty(<stream>) && !NO_COLOR`.
///
/// Each stream gets its own `is_terminal()` read (A14-5), because a stage's
/// `Fatal` row goes to stderr while every other row goes to stdout and the two
/// are redirected independently. The gate itself is the deliberate divergence
/// from `lib.sh`, which emits its ANSI codes unconditionally — PARITY.md
/// § Doctor / checks, "Console colors gated on isatty". Pinned by
/// tests::{colors_from_gates_each_stream_on_its_own_tty_unless_no_color,
/// fatal_uses_stderr_colors_while_a_line_event_uses_stdout_colors}.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Colors {
    stdout: bool,
    stderr: bool,
    /// Raw `isatty()`, independent of `NO_COLOR`: A14-3 decides from this whether
    /// a `\r`-terminated [`StageEvent::Output`] chunk repaints the terminal line
    /// or falls back to newline-per-chunk (a non-tty consumer — a log file, a
    /// `--tap` pipe — has no "current line" to repaint). Pinned by
    /// tests::cr_chunk_repaints_only_on_a_real_terminal.
    stdout_tty: bool,
    stderr_tty: bool,
}

impl Colors {
    /// Both streams uncolored and non-tty — the shared shorthand every
    /// `stage_event_lines` test that doesn't care about color uses.
    #[cfg(test)]
    const OFF: Colors = Colors {
        stdout: false,
        stderr: false,
        stdout_tty: false,
        stderr_tty: false,
    };
}

/// [`use_colors`]'s rule, as a pure function of the raw inputs — testable
/// without mutating this process's real environment or terminal state.
fn colors_from(no_color: bool, stdout_tty: bool, stderr_tty: bool) -> Colors {
    Colors {
        stdout: !no_color && stdout_tty,
        stderr: !no_color && stderr_tty,
        stdout_tty,
        stderr_tty,
    }
}

/// Read both streams' terminal-ness once (`NO_COLOR` disables both at once,
/// matching every other color gate in this file). Doctor only ever writes to
/// stdout, so it uses `.stdout` alone; the stage commands thread the whole
/// struct through so `Fatal` (stderr) and everything else (stdout) each get
/// the right one.
fn use_colors() -> Colors {
    colors_from(
        env::var_os("NO_COLOR").is_some(),
        std::io::stdout().is_terminal(),
        std::io::stderr().is_terminal(),
    )
}

/// Parsed arguments for the four stage commands.
///
/// The six flags are `demo.sh`'s own grammar (see this file's module doc);
/// `--dry-run`/`--quiet` are sabrage-only, parsed the same "boolean, no value"
/// way as `--verbose`.
#[derive(Debug, Default, PartialEq, Eq)]
struct StageArgs {
    bottle: Option<String>,
    bs_dir: Option<PathBuf>,
    wired: bool,
    no_audio: bool,
    no_dashboard: bool,
    verbose: bool,
    dry_run: bool,
    quiet: bool,
}

/// Parse a stage command's argument list — [`parse_doctor_args`]'s sibling,
/// same first-bad-argument-wins semantics, same error text.
fn parse_stage_args(args: &[String]) -> Result<StageArgs, String> {
    let mut common = CommonArgs::default();
    let mut dry_run = false;
    let mut quiet = false;
    let mut i = 0usize;
    while i < args.len() {
        if let Some(next) = parse_common_flag(&mut common, args, i)? {
            i = next;
            continue;
        }
        match args[i].as_str() {
            "--dry-run" => {
                dry_run = true;
                i += 1;
            }
            "--quiet" => {
                quiet = true;
                i += 1;
            }
            other => return Err(format!("error: unknown argument '{other}'")),
        }
    }
    Ok(StageArgs {
        bottle: common.bottle,
        bs_dir: common.bs_dir,
        wired: common.wired,
        no_audio: common.no_audio,
        no_dashboard: common.no_dashboard,
        verbose: common.verbose,
        dry_run,
        quiet,
    })
}

/// One line `stage_event_lines` wants printed, tagged with the stream it
/// belongs on.
///
/// A plain value type rather than a `println!` side effect, so "this event prints
/// nothing" — [`StageEvent::Check`], [`StageEvent::Launched`],
/// [`StageEvent::AutoFixed`], [`StageEvent::Progress`] — is directly assertable
/// (tests::structured_only_events_render_no_console_line).
#[derive(Debug, Clone, PartialEq, Eq)]
enum RenderedLine {
    Stdout(String),
    Stderr(String),
    /// A `\r`-terminated repaint chunk on a tty (A14-3): written with a
    /// trailing `\r` and no `\n`, so a curl/ninja-style progress line
    /// overwrites itself instead of scrolling once per update.
    StdoutRepaint(String),
    StderrRepaint(String),
}

/// The pure projection [`render_stage_event`] prints: exactly the lines
/// `demo.sh`'s equivalent would have written for one [`StageEvent`], on
/// whichever stream they belong on. `bottle_label` is the actual bottle name
/// once one is known, else the literal `<name>` placeholder —
/// [`format_footer`]'s convention, reused here for `install`'s closing line.
fn stage_event_lines(
    ev: &StageEvent,
    bottle_label: &str,
    colors: Colors,
    quiet: bool,
) -> Vec<RenderedLine> {
    match ev {
        StageEvent::StageStarted { stage, .. } => {
            vec![RenderedLine::Stdout(format!("== wine-vr demo {stage} =="))]
        }
        StageEvent::Section { title, .. } => vec![RenderedLine::Stdout(format!("-- {title}"))],
        StageEvent::Line {
            severity,
            text,
            remedy,
            ..
        } => vec![RenderedLine::Stdout(format_line_event(
            *severity,
            text,
            remedy.as_deref(),
            colors.stdout,
        ))],
        StageEvent::Output {
            stream, chunk, end, ..
        } => {
            if quiet {
                vec![]
            } else {
                // A `\r` repaint only skips the newline on a real terminal — a
                // non-tty consumer has no current line to overwrite, so it keeps
                // one newline per chunk (tests::cr_chunk_repaints_only_on_a_real_terminal).
                let repaint = matches!(end, sabrage_core::process::ChunkEnd::Cr)
                    && match stream {
                        Stream::Stdout => colors.stdout_tty,
                        Stream::Stderr => colors.stderr_tty,
                    };
                match (stream, repaint) {
                    (Stream::Stdout, true) => vec![RenderedLine::StdoutRepaint(chunk.clone())],
                    (Stream::Stdout, false) => vec![RenderedLine::Stdout(chunk.clone())],
                    (Stream::Stderr, true) => vec![RenderedLine::StderrRepaint(chunk.clone())],
                    (Stream::Stderr, false) => vec![RenderedLine::Stderr(chunk.clone())],
                }
            }
        }
        // Nothing extra: ninja's "[n/m]" and curl's progress bar already
        // arrive as `Output` chunks straight from the child.
        StageEvent::Progress { .. } => vec![],
        // Every fix already emits its own shell-verbatim `ok`/`warn` row onto
        // this sink; `AutoFixed` is a structured signal for the GUI, so printing
        // it here would double the console row.
        StageEvent::AutoFixed { .. } => vec![],
        StageEvent::NeedsAdmin { reason, .. } => vec![RenderedLine::Stdout(info_row(reason))],
        // A raw `print -r --` line: verbatim, no marker, no indent of our
        // own. `print ""` arrives here as an empty string and must still
        // print its (empty) line.
        StageEvent::Text { text, .. } => vec![RenderedLine::Stdout(text.clone())],
        // run.sh's preflight prints nothing for a check that passes and
        // `die`s (a `Fatal`) for one that does not, so the console has
        // nothing to add here. The GUI shows every row.
        StageEvent::Check { .. } => vec![],
        // The launch banner is printed as `Text` lines, exactly as run.sh
        // prints it; this event is state for the GUI, not console output.
        StageEvent::Launched { .. } => vec![],
        StageEvent::Fatal {
            message, remedy, ..
        } => fatal_lines(message, remedy.as_deref(), colors.stderr)
            .into_iter()
            .map(RenderedLine::Stderr)
            .collect(),
        StageEvent::StageFinished { stage, ok, .. } => {
            if *ok {
                closing_line(*stage, bottle_label)
                    .into_iter()
                    .map(RenderedLine::Stdout)
                    .collect()
            } else {
                vec![]
            }
        }
    }
}

/// Render one [`StageEvent`] exactly the way its `demo.sh` equivalent prints
/// it, by printing [`stage_event_lines`]'s projection of it.
fn render_stage_event(ev: &StageEvent, bottle_label: &str, colors: Colors, quiet: bool) {
    use std::io::Write as _;
    for line in stage_event_lines(ev, bottle_label, colors, quiet) {
        match line {
            RenderedLine::Stdout(s) => println!("{s}"),
            RenderedLine::Stderr(s) => eprintln!("{s}"),
            RenderedLine::StdoutRepaint(s) => {
                print!("{s}\r");
                let _ = std::io::stdout().flush();
            }
            RenderedLine::StderrRepaint(s) => {
                eprint!("{s}\r");
                let _ = std::io::stderr().flush();
            }
        }
    }
}

/// Merge parsed CLI flags onto the env-derived base — `WINEVR_*` is the base,
/// flags override, exactly [`StageOptions::from_env`]'s own precedence rule
/// applied to all six flags in one place. `--dry-run` has no env counterpart, so
/// `parsed.dry_run` wins outright rather than only-if-true.
///
/// Separate from `cmd_stage`/`cmd_all` so the forwarding is testable without
/// a repo root or a [`StageCtx`]: pinned by
/// tests::{merge_stage_options_forwards_all_six_flags_onto_the_env_base,
/// merge_stage_options_env_base_survives_when_no_flag_overrides_it}.
fn merge_stage_options(env_opts: StageOptions, parsed: &StageArgs) -> StageOptions {
    let mut opts = env_opts;
    if let Some(b) = &parsed.bottle {
        // A14-1: an explicit `--bottle ""` must clear the env-derived value, never
        // survive as `Some("")` — `demo.sh` treats an empty export as unset. Pinned
        // by tests::merge_stage_options_empty_cli_values_clear_a_preset_env_base.
        opts.bottle_name = (!b.is_empty()).then(|| b.clone());
    }
    if let Some(d) = &parsed.bs_dir {
        // Same rule for `--bs-dir`: `${WINEVR_BS_DIR:-<default>}` (lib.sh)
        // treats an empty value as absent too.
        opts.bs_dir_override = (!d.as_os_str().is_empty()).then(|| d.clone());
    }
    if parsed.verbose {
        opts.verbose = true;
    }
    if parsed.wired {
        opts.wired = true;
    }
    if parsed.no_audio {
        opts.no_audio = true;
    }
    if parsed.no_dashboard {
        opts.no_dashboard = true;
    }
    opts.dry_run = parsed.dry_run;
    opts
}

/// Run one stage and return the process exit code (never calls `exit` itself, so
/// `main` stays the only place that does).
///
/// All five stage bodies really touch the machine — [`Stage::Run`] launches wine
/// and the game — so this function is driven end-to-end only up to the point it
/// resolves a repo root and builds a [`StageCtx`]: an argument error, a
/// `resolve_repo_root` failure, or a `Paths::new_checked` failure returns before
/// any of that (tests::run_stage_rejects_a_bad_argument_before_touching_anything).
/// Everything past that boundary is covered through the pure helpers
/// (`merge_stage_options`, `stage_event_lines`, `report_stage_result`).
async fn cmd_stage(stage: Stage, args: &[String]) -> i32 {
    let parsed = match parse_stage_args(args) {
        Ok(p) => p,
        Err(msg) => {
            eprintln!("{msg}");
            return 2;
        }
    };

    let repo_root = match resolve_repo_root(None) {
        Ok(p) => p,
        Err(e) => {
            print_bootstrap_error(&e);
            return e.exit_code();
        }
    };

    // A2-8: every stage writes under `~/Library/Application Support`, so an unset,
    // empty, or relative `HOME` fails closed here rather than silently redirecting
    // those writes (sabrage_core::paths::tests::home_is_required_to_be_absolute_and_non_empty).
    let paths = match Paths::new_checked(&repo_root) {
        Ok(p) => p,
        Err(e) => {
            print_bootstrap_error(&e);
            return e.exit_code();
        }
    };

    let opts = merge_stage_options(StageOptions::from_env(), &parsed);

    let bottle_label = opts
        .bottle_name
        .clone()
        .unwrap_or_else(|| "<name>".to_string());
    let colors = use_colors();
    let quiet = parsed.quiet;

    let sink: EventSink = Arc::new(move |ev: StageEvent| {
        render_stage_event(&ev, &bottle_label, colors, quiet);
    });

    let ctx = StageCtx::new(paths, opts, sink, Default::default());
    {
        let cancel = ctx.cancel.clone();
        watch_termination_signals(move || cancel.cancel());
    }

    let is_dry_run = ctx.executor.is_dry_run();
    let result = run_stage(stage, &ctx).await;

    // The dry-run plan prints trailing, after every narrative row a real run would
    // also print, and is keyed on the executor (`is_dry_run()`) rather than the raw
    // `--dry-run` flag, so a non-dry run's output is untouched byte-for-byte
    // (finding #13; PARITY.md § CLI / GUI, "A dry run ends with a trailing").
    if is_dry_run {
        for line in render_dry_run_plan(&ctx.executor.planned()) {
            println!("{line}");
        }
    }

    report_stage_result(result)
}

/// Project a finished stage's [`run_stage`] result onto the exit code
/// `./demo.sh <stage>` would have produced — for `run`, that is wine's own
/// exit status, [`StageOutcome::exit_code_equiv`]. Factored out of
/// [`cmd_stage`] so [`cmd_all`]'s per-stage loop body reports an error
/// identically instead of drifting from it.
fn report_stage_result(result: std::result::Result<StageOutcome, SabrageError>) -> i32 {
    match result {
        Ok(outcome) => outcome.exit_code_equiv,
        Err(e) => {
            // Suppressed for every error whose condition already emitted a
            // `Fatal` (see `error_already_reported_as_fatal`); every remaining
            // variant has no shell equivalent and would otherwise vanish.
            if !error_already_reported_as_fatal(&e) {
                eprintln!("error: {e}");
                for line in e.tail() {
                    eprintln!("    {line}");
                }
            }
            e.exit_code()
        }
    }
}

/// True when this error's condition already emitted a [`StageEvent::Fatal`], so
/// printing `error: {e}` after it would double-report one failure.
///
/// A CLI-flavored name over [`SabrageError::already_reported`], which carries the
/// rule once for both front-ends (the GUI needs the identical predicate to decide
/// whether a failure banner would double the `Fatal` already in its run log) —
/// see sabrage_core::error::tests::already_reported_covers_the_variants_that_emit_their_own_row
/// and PARITY.md § CLI / GUI, "`error: <e>` is suppressed for every error".
fn error_already_reported_as_fatal(e: &SabrageError) -> bool {
    e.already_reported()
}

/// The trailing "-- plan (dry run)" section's lines: the section header in
/// [`StageEvent::Section`]'s own `"-- <title>"` shape, then one `info`-indented
/// line per recorded [`PlannedAction`].
///
/// Title and body come from [`sabrage_core::DRY_RUN_PLAN_TITLE`] /
/// [`sabrage_core::dry_run_plan_body`], never from literals here, so the CLI and
/// the GUI cannot word the same plan differently — PARITY.md § CLI / GUI, "A dry
/// run ends with a trailing". Pinned by
/// tests::dry_run_plan_has_a_header_then_one_line_per_action_in_order.
fn render_dry_run_plan(plan: &[PlannedAction]) -> Vec<String> {
    let mut lines = vec![format!("-- {}", sabrage_core::DRY_RUN_PLAN_TITLE)];
    lines.extend(
        sabrage_core::dry_run_plan_body(plan)
            .into_iter()
            .map(|line| info_row(&line)),
    );
    lines
}

// `sabrage all` is `demo.sh`'s `all)` loop, native-side: one `require_bottle` up
// front so it fails before the expensive fetch/build stages, then
// `Stage::ALL_CHAIN` in order (`Stage` deliberately has no `All` member — see its
// own doc). Divergences, including the dropped `##### demo.sh: <stage> #####`
// separator each stage's own `StageStarted` banner already covers:
// PARITY.md § Run (launch), "`sabrage all` chains".

/// Run `stages` in order via `run_one`, stopping at the first stage whose exit
/// code is non-zero and returning that code, else `0` — `demo.sh`'s own
/// `|| exit $?`.
///
/// `run_one` is a parameter so a test can substitute a fake and assert the
/// stop-at-first-failure rule without invoking a real stage body
/// (tests::{run_chain_stops_at_the_first_nonzero_exit_code,
/// run_chain_runs_every_stage_when_all_of_them_succeed}).
async fn run_chain<F, Fut>(stages: &[Stage], mut run_one: F) -> i32
where
    F: FnMut(Stage) -> Fut,
    Fut: std::future::Future<Output = i32>,
{
    for &stage in stages {
        let code = run_one(stage).await;
        if code != 0 {
            return code;
        }
    }
    0
}

/// The core of `all`: requires a bottle up front via [`require_bottle`] (lib.sh's
/// own die text), then chains [`Stage::ALL_CHAIN`] through [`run_stage`] via
/// `run_chain`.
///
/// Each stage gets a **fresh** [`StageCtx`] — fresh `run_id`, fresh executor —
/// over the same `paths`/`opts`/`sink`, plus one cancellation token shared across
/// every stage so a single Ctrl-C reaches whichever stage is running. `--dry-run`
/// applies uniformly because every per-stage context inherits `opts.dry_run` and
/// prints its own trailing plan. Taking `paths`/`opts` as arguments — never
/// [`StageOptions::from_env`] — is what keeps
/// tests::run_all_requires_a_bottle_before_touching_run_stage independent of the
/// real machine's `WINEVR_BOTTLE`.
async fn run_all(paths: &Paths, opts: &StageOptions, sink: &EventSink) -> i32 {
    // Doubles as the source of the `CancellationToken` every per-stage `StageCtx`
    // below shares — reached through `StageCtx`'s own field, so this file never
    // names `tokio_util`'s type.
    let precheck = StageCtx::new(
        paths.clone(),
        opts.clone(),
        sink.clone(),
        Default::default(),
    );
    if require_bottle(&precheck).is_err() {
        // `require_bottle` already emitted the `Fatal` onto `sink`.
        return 1;
    }
    let cancel = precheck.cancel.clone();
    {
        let c = cancel.clone();
        watch_termination_signals(move || c.cancel());
    }

    run_chain(&Stage::ALL_CHAIN, |stage| {
        let ctx = StageCtx::new(paths.clone(), opts.clone(), sink.clone(), cancel.clone());
        async move {
            let is_dry_run = ctx.executor.is_dry_run();
            let result = run_stage(stage, &ctx).await;
            if is_dry_run {
                for line in render_dry_run_plan(&ctx.executor.planned()) {
                    println!("{line}");
                }
            }
            report_stage_result(result)
        }
    })
    .await
}

/// `sabrage all`'s CLI glue: parse (same six flags + `--dry-run`/`--quiet` as
/// every other stage command — [`parse_stage_args`]), resolve a repo root,
/// merge onto the `WINEVR_*` base ([`merge_stage_options`]), then hand off to
/// [`run_all`].
async fn cmd_all(args: &[String]) -> i32 {
    let parsed = match parse_stage_args(args) {
        Ok(p) => p,
        Err(msg) => {
            eprintln!("{msg}");
            return 2;
        }
    };

    let repo_root = match resolve_repo_root(None) {
        Ok(p) => p,
        Err(e) => {
            print_bootstrap_error(&e);
            return e.exit_code();
        }
    };

    // `all` chains setup/build/install/run, all of which write into the user
    // store, so it fails closed on an unusable `HOME` the same way `cmd_stage`
    // does (sabrage_core::paths::tests::home_is_required_to_be_absolute_and_non_empty).
    let paths = match Paths::new_checked(&repo_root) {
        Ok(p) => p,
        Err(e) => {
            print_bootstrap_error(&e);
            return e.exit_code();
        }
    };

    let opts = merge_stage_options(StageOptions::from_env(), &parsed);
    let bottle_label = opts
        .bottle_name
        .clone()
        .unwrap_or_else(|| "<name>".to_string());
    let colors = use_colors();
    let quiet = parsed.quiet;
    let sink: EventSink = Arc::new(move |ev: StageEvent| {
        render_stage_event(&ev, &bottle_label, colors, quiet);
    });

    run_all(&paths, &opts, &sink).await
}

// Ctrl-C / SIGTERM
//
// A raw `signal(2)` trap plus a polling `std::thread`, not `tokio::signal`:
// sabrage-cli's tokio carries only `rt-multi-thread`/`macros` (see its
// Cargo.toml). Each handler does only async-signal-safe work — one relaxed store
// (`LAST_SIGNAL`) and one relaxed `fetch_add` (`SIGNAL_COUNT`) — leaving the
// cancellation itself to the thread. `watch_termination_signals`/`watch_flag`
// take a plain `FnOnce` so this file never has to name `tokio_util`'s
// `CancellationToken`.
//
// SIGINT and SIGTERM share one counter and one state machine, matching run.sh's
// identical `INT`/`TERM` traps: the first delivery of either cancels, and every
// later one — of either kind — restores `SIG_DFL` for both and re-raises whichever
// signal actually arrived, so the kernel picks the exit code (130 or 143) instead
// of this file. A *completed* cancellation always exits 130 whichever signal
// started it: PARITY.md § CLI / GUI, "SIGTERM is trapped like SIGINT". Pinned by
// tests::{watcher_keeps_polling_past_the_first_fire_so_a_second_signal_is_reachable,
// two_signals_inside_one_poll_interval_are_not_collapsed_into_one,
// a_second_signal_of_a_different_kind_is_still_fatal_and_reraises_that_kind}.
//
// Landmine: a second signal skips teardown and wine is spawned detached, so an
// impatient double-tap leaves the game, audio and dashboard up until the next
// `sabrage run`/`stop` reconciles (PARITY.md § Run (launch), "The wine child is
// spawned in its"; `sabrage_core::session::reconcile`).

/// Poll interval for the signal watcher thread — frequent enough that a
/// human doesn't perceive the delay, infrequent enough to cost nothing idle.
const CTRL_C_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// `SIGINT`'s signal number — 2 on every platform this ships to (macOS).
const SIGINT: i32 = 2;

/// `SIGTERM`'s signal number — 15 on every platform this ships to (macOS).
/// `run.sh` traps it with the same teardown as its `INT` trap.
const SIGTERM: i32 = 15;

/// How many `SIGINT`/`SIGTERM` deliveries the handlers have seen, combined.
///
/// A **count**, not a flag: two deliveries inside one `CTRL_C_POLL_INTERVAL`
/// must still read as two, or the second is lost and the process stays merely
/// "cancelling" (tests::two_signals_inside_one_poll_interval_are_not_collapsed_into_one).
/// `fetch_add` on an `AtomicUsize` is as async-signal-safe as a plain store. Both
/// signals share it on purpose: one state machine here, not two.
static SIGNAL_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Which of `SIGINT`/`SIGTERM` the *most recent* delivery was — read only by
/// the fatal second action, to decide which one to re-raise. Each handler
/// stores here *before* bumping `SIGNAL_COUNT`, so by the time the watcher
/// observes a given increment this holds at least as recent a value as that
/// delivery.
static LAST_SIGNAL: AtomicI32 = AtomicI32::new(SIGINT);

/// `SIG_DFL` — restoring it before re-raising a signal on the second
/// delivery lets the OS's own default disposition (terminate) apply, rather
/// than us synthesizing the exit code by hand.
const SIG_DFL: usize = 0;

extern "C" fn on_sigint(_signum: i32) {
    LAST_SIGNAL.store(SIGINT, Ordering::Relaxed);
    SIGNAL_COUNT.fetch_add(1, Ordering::Relaxed);
}

extern "C" fn on_sigterm(_signum: i32) {
    LAST_SIGNAL.store(SIGTERM, Ordering::Relaxed);
    SIGNAL_COUNT.fetch_add(1, Ordering::Relaxed);
}

extern "C" {
    /// `signal(2)`. Both the handler parameter and the return value are typed
    /// `usize` rather than a function-pointer type: the return value is never
    /// read (its bits are simply discarded, sound regardless of declared
    /// width), and the `usize` parameter type is what lets this one
    /// declaration pass *either* a real handler (`on_sigint as usize`) *or*
    /// the `SIG_DFL` sentinel (`0`) without a raw function-pointer transmute.
    fn signal(signum: i32, handler: usize) -> usize;
    /// `raise(2)` — used only to re-deliver a signal to this process itself
    /// after restoring `SIG_DFL`, on a second delivery.
    fn raise(signum: i32) -> i32;
}

/// Install the real `SIGINT` **and** `SIGTERM` traps and spawn the one
/// watcher thread that calls `on_signal` once, the moment either trap first
/// fires (and is fatal to the process on the next delivery of *either*
/// kind — see the module doc above). Both signals feed the same
/// `SIGNAL_COUNT`, so this spawns exactly one watcher for both — calling
/// `watch_flag` a second time (once per signal) would instead spawn two
/// independent pollers racing each other over that one counter.
fn watch_termination_signals(on_signal: impl FnOnce() + Send + 'static) {
    watch_flag(&SIGNAL_COUNT, on_signal);
    unsafe {
        signal(SIGINT, on_sigint as *const () as usize);
        signal(SIGTERM, on_sigterm as *const () as usize);
    }
}

/// Restore both `SIGINT`'s and `SIGTERM`'s default disposition and re-raise
/// whichever one was actually received (`LAST_SIGNAL`), so the process dies
/// under that signal's own name — exit 143 for a second `kill <pid>`, 130 for a
/// second Ctrl-C — rather than this file picking a number.
///
/// The production second-signal action both traps share; tests drive
/// `watch_flag_with_second_action` with a fake instead, since a real second
/// signal would kill the test binary
/// (tests::a_second_signal_of_a_different_kind_is_still_fatal_and_reraises_that_kind).
fn terminate_via_default_disposition_of_last_signal() {
    unsafe {
        signal(SIGINT, SIG_DFL);
        signal(SIGTERM, SIG_DFL);
        raise(LAST_SIGNAL.load(Ordering::Relaxed));
    }
}

/// The polling primitive [`watch_termination_signals`] builds on, factored
/// out so it is testable without installing a real signal handler or
/// touching the process-wide [`SIGNAL_COUNT`] counter. Delegates to
/// [`watch_flag_with_second_action`] with the real fatal second action.
fn watch_flag(counter: &'static AtomicUsize, on_signal: impl FnOnce() + Send + 'static) {
    watch_flag_with_second_action(
        counter,
        on_signal,
        terminate_via_default_disposition_of_last_signal,
    );
}

/// `watch_flag`'s full generality: polls `counter` for as long as the process
/// runs, calling `on_first_signal` once on the first delivery it observes and
/// `on_second_signal` on any delivery after that, then returning.
///
/// It reads a monotone **count** and remembers how much of it it has consumed
/// rather than swapping a flag back to `false`: a burst delivered inside one
/// `CTRL_C_POLL_INTERVAL` is one observation but two deliveries, and a flag
/// cannot tell those apart. Pinned by
/// tests::{watcher_keeps_polling_past_the_first_fire_so_a_second_signal_is_reachable,
/// two_signals_inside_one_poll_interval_are_not_collapsed_into_one}.
fn watch_flag_with_second_action(
    counter: &'static AtomicUsize,
    on_first_signal: impl FnOnce() + Send + 'static,
    on_second_signal: impl Fn() + Send + 'static,
) {
    std::thread::spawn(move || {
        let mut on_first_signal = Some(on_first_signal);
        let mut consumed = 0usize;
        loop {
            let seen = counter.load(Ordering::Relaxed);
            while consumed < seen {
                consumed += 1;
                match on_first_signal.take() {
                    Some(f) => f(),
                    None => {
                        on_second_signal();
                        return;
                    }
                }
            }
            std::thread::sleep(CTRL_C_POLL_INTERVAL);
        }
    });
}

#[cfg(test)]
mod tests {
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
}
