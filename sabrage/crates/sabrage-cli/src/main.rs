//! `sabrage` — the native Sabrage pipeline CLI.
//!
//! Phase 1 shipped `sabrage doctor`. Phase 2 added the four mutating stages —
//! `setup`, `build`, `install`, `stop` — as thin renderers over
//! [`sabrage_core::stages::run_stage`]. Phase 3 adds `run` (the same renderer,
//! over [`sabrage_core::stages::run::run`], which is real — a running
//! `sabrage run` launches wine and the game exactly as `./demo.sh run` does;
//! `cmd_stage`'s own doc explains why that already-real body still stays out
//! of this file's unit tests) and `all` (a caller-level loop over
//! [`sabrage_core::Stage::ALL_CHAIN`] — never a sixth [`Stage`], mirroring
//! `demo.sh`'s own `for stage in setup build install run` re-exec loop). This
//! file owns argument parsing and turning a [`sabrage_core::StageEvent`]
//! stream into the exact console text its `demo.sh` equivalent prints,
//! nothing more (the stage bodies themselves live in `sabrage-core`).
//! Anything else on the command line still falls through to the usage text,
//! exit 2 — the same fallback `demo.sh`'s own `case` uses for an unrecognized
//! `CMD`.
//!
//! # Argument parsing
//!
//! Hand-parsed, deliberately not `clap` (see the comment in
//! `sabrage-cli/Cargo.toml`), to match `demo.sh`'s own loop byte-for-byte —
//! and that loop parses the same six flags for **every** subcommand
//! (`demo.sh`'s flag `while` runs before the `case "$CMD"` dispatch, lines
//! 30-42), so `setup`/`build`/`install`/`stop` all accept
//! `--bottle`/`--bs-dir`/`--no-audio`/`--no-dashboard`/`--wired`/`--verbose`
//! too, even though only `run` (and, by inheriting `run`, `all`) reads the
//! last four:
//!
//! ```zsh
//! while [ $# -gt 0 ]; do
//!   case "$1" in
//!     --bottle) [ $# -ge 2 ] || { echo "error: --bottle needs a name" >&2; exit 2; }
//!               export WINEVR_BOTTLE="$2"; shift 2 ;;
//!     --bs-dir) [ $# -ge 2 ] || { echo "error: --bs-dir needs a path" >&2; exit 2; }
//!               export WINEVR_BS_DIR="$2"; shift 2 ;;
//!     --no-audio) export WINEVR_NO_AUDIO=1; shift ;;
//!     --no-dashboard) export WINEVR_NO_DASHBOARD=1; shift ;;
//!     --wired)    export WINEVR_WIRED=1; shift ;;
//!     --verbose)  export WINEVR_VERBOSE=1; shift ;;
//!     *) echo "error: unknown argument '$1'" >&2; exit 2 ;;
//!   esac
//! done
//! ```
//!
//! `--tap <file>` (doctor only) and `--dry-run`/`--quiet` (the four stage
//! commands) have no `demo.sh` counterpart — sabrage's own additions, parsed
//! with the same "boolean, no value" or "needs a value or exit 2" shapes.
//!
//! # Human output
//!
//! Doctor mirrors `scripts/demo/doctor.sh`'s rendering (via `lib.sh`'s
//! `ok`/`warn`/`fail`/`info`); the four stage commands mirror
//! `scripts/demo/{setup,build,install,stop}.sh` the same way, reusing the
//! identical row layout (see `ok_row`/`warn_row`/`fail_row`/`info_row`) over
//! [`sabrage_core::StageEvent::Line`] instead of
//! [`sabrage_core::checks::CheckOutcome`]. One deliberate divergence carries
//! over to both: colors are gated on `isatty(stdout)` and `NO_COLOR`, where
//! the shell emits ANSI codes unconditionally.

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

/// Finding A14-4: what the `_` arm above does with an unrecognized first
/// token, factored into a pure helper so it is testable without `exit`.
///
/// `demo.sh` shifts `CMD="${1:-}"` off unconditionally *before* its flag
/// `while` loop runs (lines 30-42 of this file's module doc), so when the
/// first token is a flag rather than a real subcommand — `--bottle Steam
/// run` — the loop parses the *remaining* tokens (`Steam run`) and reports
/// the first one it does not recognize (`Steam`, since `--bottle`'s value
/// was consumed as `CMD` instead of by the flag) before ever reaching its
/// `case "$CMD"` dispatch. Mirror that sequencing: treat `args[0]` as the
/// shell's `CMD` (already consumed, never re-examined) and parse `args[1..]`
/// with the same grammar the six flags share. `Ok(())` means the shell would
/// have fallen through to its unknown-`CMD` `case` arm instead — this file's
/// usage-text-then-exit-2 fallback.
fn unknown_command_outcome(args: &[String]) -> Result<(), String> {
    parse_stage_args(&args[1..]).map(|_| ())
}

// ── shared bootstrap ─────────────────────────────────────────────────────────

/// Repo-root resolution fails before any [`StageCtx`]/[`EventSink`] exists
/// (doctor has no `StageCtx` at all), so it has no `StageEvent::Fatal` or
/// `die()` line to ride on. Render it in the same `error: <msg>` shape the
/// argument-parsing errors already use, plus [`SabrageError`]'s remedy line
/// when there is one — a plain addition, since finding the repo root has no
/// shell equivalent to match (`paths.rs`'s module doc).
fn print_bootstrap_error(e: &SabrageError) {
    eprintln!("error: {e}");
    if let Some(remedy) = e.remedy() {
        eprintln!("       remedy: {remedy}");
    }
}

// ── `sabrage doctor` ─────────────────────────────────────────────────────────

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
/// Factored out of [`parse_doctor_args`]/[`parse_stage_args`] (finding
/// S-C3-cli-ipc: the two hand-rolled loops repeated these six arms
/// identically, including the exact `demo.sh`-verbatim error text) so each
/// error string lives in exactly one place.
#[derive(Debug, Default, PartialEq, Eq)]
struct CommonArgs {
    bottle: Option<String>,
    bs_dir: Option<PathBuf>,
    wired: bool,
    no_audio: bool,
    no_dashboard: bool,
    verbose: bool,
}

/// Try to consume one of the six [`CommonArgs`] flags at `args[i]`.
///
/// `Ok(Some(next_i))` — recognized and consumed, resume the caller's loop at
/// `next_i`. `Ok(None)` — `args[i]` is not one of the six; the caller tries
/// its own extra flags (`--tap`, `--dry-run`, `--quiet`) before falling back
/// to the shared "unknown argument" error. `Err` — a value-taking flag with
/// no value, the same first-bad-argument-wins short-circuit both callers
/// already relied on.
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

/// Merge parsed `doctor` flags onto the env-derived base — `doctor`'s
/// counterpart of [`merge_stage_options`], same precedence rule
/// (`WINEVR_*` is the base, flags override), factored out for the same
/// reason: directly testable without resolving a repo root or building a
/// [`CheckCtx`].
///
/// Finding A14-1: an explicit empty `--bottle`/`--bs-dir` must clear the
/// env-derived value rather than override it with `Some("")` — see
/// [`merge_stage_options`]'s matching comment for the full rationale
/// (`demo.sh`'s `${WINEVR_BOTTLE:-}`/`${WINEVR_BS_DIR:-<default>}` both treat
/// an empty export as absent).
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

    // WINEVR_BOTTLE / WINEVR_BS_DIR (and the other WINEVR_* flags) are the base;
    // CLI flags override — the same precedence `demo.sh` gets from exporting
    // over whatever the caller's shell already had set.
    let opts = merge_doctor_options(CheckOptions::from_env(), &parsed);
    let verbose = opts.verbose;

    let ctx = CheckCtx::new(Paths::new(&repo_root), opts);
    // Doctor only ever writes to stdout — no stderr rows exist in its output
    // — so it keeps using a single bool, the way it always has.
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
        // "truncate first": std::fs::write creates-or-truncates then writes the
        // whole payload in one shot — never zsh's incremental `>>` (that channel
        // is `tap::append_tap`, a renderer primitive this CLI does not use here).
        if let Err(e) = std::fs::write(tap_path, render_tap(&report.outcomes)) {
            eprintln!(
                "warning: could not write --tap file {}: {e}",
                tap_path.display()
            );
        }
    }

    // doctor.sh: `[ -n "${WINEVR_DOCTOR_SOFT:-}" ] || exit "$FAILCOUNT"` — a
    // non-empty WINEVR_DOCTOR_SOFT short-circuits the exit entirely, so the
    // process's own exit code is 0 (the successful `[ -n ... ]` test).
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

// ── rendering (shared by doctor and the stage commands) ─────────────────────

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
/// `Skipped`/`NotImplemented` return `None` — those statuses only ever reach
/// zsh's silent `tap()` channel, never stdout. So do `quiet` passes: rows
/// doctor.sh taps (`tap <slug> ok`) without printing a console line.
///
/// `verbose` (A3b-3, round 2) gates one extra continuation line —
/// `       detail: <d>` — appended when the row has a
/// [`CheckOutcome::detail`] and the caller asked for it: `detail` is the
/// "sabrage-only explainability field" (`checks/mod.rs`'s doc comment) that
/// zsh's own `ok`/`warn`/`fail`/`info` have no slot for, so with `verbose`
/// false (doctor's default) this function's output stays exactly what it was
/// before `detail` existed — shell-parity byte-identical.
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

/// `lib.sh`'s `die()`: `"${_R}FATAL${_N} $*"` — no leading spaces (unlike
/// `ok`/`warn`/`fail`/`info`, which all indent two) and no separate `remedy:`
/// line. A `die` call's remedy, when it has one, is folded into the message
/// text itself at the call site (see `require_bottle`'s two-line message for
/// the canonical example) rather than carried as structured data the way
/// `fail`'s is — this mirrors that, rather than inventing a line `die()`
/// never printed.
fn fatal_line(message: &str, colors: bool) -> String {
    format!("{} {}", label("FATAL", ANSI_RED, colors), message)
}

/// [`fatal_line`] plus a `       remedy: <r>` continuation when the event
/// carries one.
///
/// **Sabrage-only** (declared in `sabrage/PARITY.md`, "CLI"): `lib.sh`'s `die`
/// has no remedy slot, so there is no shell text this can diverge from. It
/// exists because two `Fatal`s are emitted by [`sabrage_core::privilege`]
/// rather than by a `die`-shaped call site — the App Management refusal and
/// the declined authorization dialog — and both put the only actionable
/// instruction the user gets (the `x-apple.systempreferences:` deep link, the
/// relaunch requirement) in `remedy`. Dropping it left CLI users with a
/// diagnosis and no fix, while the GUI got both over the channel. The
/// indent is `fail_row`'s, so a remedy reads identically wherever it appears.
fn fatal_lines(message: &str, remedy: Option<&str>, colors: bool) -> Vec<String> {
    let mut lines = vec![fatal_line(message, colors)];
    if let Some(remedy) = remedy {
        lines.push(format!("       remedy: {remedy}"));
    }
    lines
}

/// doctor.sh's footer:
///
/// ```zsh
/// print ""
/// if [ "$FAILCOUNT" -eq 0 ]; then print -r -- "doctor: ${_G}all checks passed${_N} — ./demo.sh run --bottle $WINEVR_BOTTLE"
/// else print -r -- "doctor: ${_R}$FAILCOUNT check(s) failed${_N} — remedies above"; fi
/// ```
///
/// The caller prints the blank line separately (`println!()`); this returns
/// just the message line. `fail_count` here is the *uncapped* tally — the 255
/// cap in [`sabrage_core::checks::DoctorReport::exit_code`] is a CLI exit-code
/// concern, not a display one.
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
/// success — verbatim from `scripts/demo/{setup,build,install}.sh`'s closing
/// `print "\n... complete — next: ..."` line (`stop.sh` has none: `None`).
/// `bottle_label` is only used by `install`'s line, and only once `install`
/// has actually succeeded — by which point `require_bottle` has already
/// guaranteed a real bottle name, never the `<name>` placeholder. `build`'s
/// line quotes the literal placeholder too — build.sh's own text does, since
/// `build` never resolves a bottle at all.
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
/// Finding A14-5: a stage's `Fatal` row goes to stderr, but coloring it was
/// gated on *stdout*'s terminal-ness alone — with stdout attached to a
/// terminal and stderr redirected to a file, ANSI escapes leaked into the
/// error file; with stdout piped and stderr on a terminal, `Fatal` lost
/// color it should have had. Each stream now gets its own `is_terminal()`
/// read, both still short-circuited by `NO_COLOR` (the one deliberate
/// divergence from `lib.sh`, which emits its `$'\e[32m'`-style codes
/// unconditionally — native output is more likely to be piped/captured, by
/// the GUI or a `--tap` consumer, so it earns the gate the shell script
/// never needed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Colors {
    stdout: bool,
    stderr: bool,
    /// Raw `isatty()`, independent of `NO_COLOR` — A14-3 needs this to decide
    /// whether a `\r`-terminated [`StageEvent::Output`] chunk repaints the
    /// terminal line or falls back to newline-per-chunk (a non-tty consumer,
    /// e.g. a log file or `--tap` pipe, has no "current line" to repaint).
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

// ── `sabrage setup|build|install|stop` ──────────────────────────────────────

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

/// One line [`stage_event_lines`] wants printed, tagged with the stream it
/// belongs on.
///
/// A plain value type purely so "this event prints nothing" — [`Check`],
/// [`Launched`], [`AutoFixed`], [`Progress`] — is something a test can assert
/// on directly (`stage_event_lines(...) == vec![]`) instead of a `println!`
/// side effect it would otherwise have to intercept.
///
/// [`Check`]: StageEvent::Check
/// [`Launched`]: StageEvent::Launched
/// [`AutoFixed`]: StageEvent::AutoFixed
/// [`Progress`]: StageEvent::Progress
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
                // A bare `\r` (a progress-bar repaint) only gets the
                // no-newline treatment when the destination is a real
                // terminal; a non-tty consumer (redirected to a file, piped,
                // or `--tap`) has no "current line" to overwrite, so it keeps
                // today's one-newline-per-chunk behavior — same as `Lf`/`Eof`.
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
        // Every fix already emits its own shell-verbatim `ok`/`warn` row (a
        // `Line`) onto this same sink — printing this too would double the
        // row on the console. `AutoFixed` stays a structured signal the GUI
        // renders (which fix ran); the console has nothing to add.
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
/// (`demo.sh` exporting over whatever the caller's shell already had set)
/// applied to all six flags in one place. `--dry-run` has no env counterpart,
/// so `parsed.dry_run` wins outright rather than only-if-true.
///
/// Factored out of [`cmd_stage`]/[`cmd_all`] so the forwarding of
/// `--wired`/`--no-audio`/`--no-dashboard` — parsed since Phase 2 for
/// grammar parity, but silently dropped on the floor until now because
/// `StageOptions` carried no field for them — is directly testable without
/// resolving a repo root or building a [`StageCtx`].
fn merge_stage_options(env_opts: StageOptions, parsed: &StageArgs) -> StageOptions {
    let mut opts = env_opts;
    if let Some(b) = &parsed.bottle {
        // Finding A14-1: an explicit `--bottle ""` (a wrapper that always
        // interpolates the flag, even when its own variable is unset) must
        // NOT survive as `Some("")` — that resolves to a different
        // missing-bottle path than "no override at all". `demo.sh` never
        // sees this ambiguity: `export WINEVR_BOTTLE=""` then `[ -n
        // "${WINEVR_BOTTLE:-}" ]` (lib.sh) is false, so the shell treats an
        // empty export exactly like an unset one. Mirror that here: an empty
        // CLI value clears whatever `StageOptions::from_env` already put in
        // `bottle_name`, rather than overriding it with an empty string.
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

/// Run one stage and return the process exit code (never calls `exit` itself,
/// so `main` stays the only place that does).
///
/// `setup`/`build`/`install`/`stop`'s bodies are real as of Phase 2, and
/// [`Stage::Run`]'s (`sabrage_core::stages::run::run`) is real as of Phase 3
/// — it actually launches wine and the game. That makes all five stage
/// bodies equally off-limits for this file's unit tests (this task's hard
/// rules: no launching the game, no touching the machine), so `cmd_stage`'s
/// tests may drive `cmd_stage(Stage::Run, …)` (and the other four stages)
/// end-to-end **only** up to the point this function resolves a repo root
/// and builds a [`StageCtx`]: an argument error, a `resolve_repo_root`
/// failure, or a `Paths::new_checked` failure (an unusable `HOME`) returns
/// before any of that and is exercised directly; anything past that boundary
/// is exercised through the pure helpers instead ([`merge_stage_options`],
/// [`stage_event_lines`], [`report_stage_result`]).
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

    // Cross-area packet (from A2-8): `setup`/`build`/`install`/`run`/`stop`
    // all write under `~/Library/Application Support/{OXRSys,Sabrage}` —
    // an unset, empty, or relative `HOME` must fail closed here, before any
    // of that, rather than silently redirecting those writes under the
    // process's working directory (empty `HOME`) or a root-owned path
    // (unset `HOME`'s `/` fallback, `Paths::new`'s own doc comment). Doctor
    // keeps `Paths::new` — it only ever reads/probes, never writes.
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

    // Finding #13: `DryRunExecutor::planned()` recorded a real plan from
    // Phase 2 onward, but nothing ever printed it — `--dry-run`'s own
    // `--help` text ("plan the stage's writes/spawns without touching
    // anything") was a promise the console renderer never kept. Trailing,
    // after every narrative row a real run would also have printed (this
    // stays keyed on the executor, `ctx.executor.is_dry_run()`, not the raw
    // `--dry-run` flag — the same source-of-truth precedent `is_dry_run()`
    // callers elsewhere in this crate already follow), so a non-dry run's
    // output is untouched byte-for-byte.
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
            // Every error whose *condition* already emitted a `Fatal` event
            // is suppressed here — otherwise the user reads the FATAL row and
            // then a second, differently-worded line for the same thing.
            // That is not only `SabrageError::Fatal`: `upgrade_write_error`
            // emits the App Management `Fatal` and returns `TccDenied`, and
            // `elevate_osascript` does the same before returning
            // `AdminDeclined` (both documented as "the caller must propagate,
            // not re-emit"). Every remaining variant has no shell equivalent
            // to reproduce and would otherwise vanish silently — design-core
            // §6.6, "no swallowed diagnostics".
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

/// True when this error's condition already emitted a [`StageEvent::Fatal`],
/// so printing `error: {e}` after it would double-report one failure.
///
/// [`SabrageError::Fatal`] is the `die`-shaped case (emitted by
/// [`sabrage_core::StageCtx::fatal`] itself). The other two are
/// [`sabrage_core::privilege`]'s: `upgrade_write_error` emits the App
/// Management explanation and returns `TccDenied`; `elevate_osascript` emits
/// the declined-authorization row and returns `AdminDeclined`. Both document
/// that the caller must propagate rather than re-emit — which makes them
/// exactly as "already reported" as a `Fatal`, and the reason this is a
/// predicate over the condition instead of a single-variant `matches!`.
///
/// [`SabrageError::Cancelled`] is here for a different reason: it is the
/// user's own Ctrl-C (or `kill`). The run stage already printed run.sh's
/// `-- interrupted: stopping wine` section on that path, a build stage's
/// child simply stops, and `demo.sh` itself prints nothing after its
/// INT/TERM trap re-raises the signal — a trailing `error: cancelled` would be
/// the one line the shell never shows. Exit 130 remains the signal.
///
/// A thin CLI-flavored name over [`SabrageError::already_reported`], which
/// carries the actual rule (and its own test) once for both front-ends — the
/// GUI needs the identical predicate to decide whether a failure banner would
/// double up the `Fatal` row already in its run log.
fn error_already_reported_as_fatal(e: &SabrageError) -> bool {
    e.already_reported()
}

/// The trailing "-- plan (dry run)" section's lines: the section header (same
/// `"-- <title>"` shape [`StageEvent::Section`] renders), then one line per
/// recorded [`PlannedAction`], each indented like an `info` row.
///
/// The title and the body come from `sabrage-core`
/// ([`sabrage_core::DRY_RUN_PLAN_TITLE`], [`sabrage_core::dry_run_plan_body`]),
/// not from string literals here: the GUI renders the same plan as a section
/// plus `info` rows in its run log, and the two front-ends must say the same
/// thing word for word. This function is only the CLI's console *shape* around
/// that shared text — kept separate from the `println!`s at its one call site
/// so tests can assert on it without capturing stdout.
fn render_dry_run_plan(plan: &[PlannedAction]) -> Vec<String> {
    let mut lines = vec![format!("-- {}", sabrage_core::DRY_RUN_PLAN_TITLE)];
    lines.extend(
        sabrage_core::dry_run_plan_body(plan)
            .into_iter()
            .map(|line| info_row(&line)),
    );
    lines
}

// ── `sabrage all` ────────────────────────────────────────────────────────────
//
// `demo.sh`'s `all)` case is a caller-level loop that re-execs itself once per
// stage (`for stage in setup build install run; do … "$ROOT/demo.sh" "$stage"
// || exit $?; done`), after one `require_bottle` up front "to fail fast before
// the expensive fetch/build stages". `Stage` deliberately has no `All` member
// (see its own doc comment) — this is that same loop, native-side, over
// [`Stage::ALL_CHAIN`].
//
// One divergence, declared here for `sabrage/PARITY.md`: `demo.sh` prints
// `"\n##### demo.sh: $stage #####"` before re-exec'ing each stage; `sabrage
// all` does not reproduce that separator, because every stage already
// announces itself on the same sink via its own `StageStarted` banner
// (`"== wine-vr demo <stage> =="`, rendered by `stage_event_lines` exactly as
// it is for a standalone `sabrage <stage>`) — printing both would say which
// stage is starting twice.

/// Run `stages` in order via `run_one`, stopping at the first stage whose
/// exit code is non-zero and returning that code (or `0` once every stage in
/// `stages` has exited `0`) — `demo.sh`'s own `|| exit $?`.
///
/// Factored out of [`run_all`] so a test can substitute a fake `run_one` and
/// assert "stops at the first failure" without going anywhere near
/// [`run_stage`]: every one of `Stage::Setup`/`Build`/`Install`/`Run`'s
/// bodies really does touch the machine (`Run` launches wine and the game),
/// and none of them is something a unit test may invoke (this task's hard
/// rules).
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

/// The core of `all`, split from [`cmd_all`] so it can be driven with a
/// caller-built `paths`/`opts` pair — never [`StageOptions::from_env`] —
/// which is what makes this deterministic in a test regardless of the *real*
/// `WINEVR_BOTTLE`/CrossOver state on whatever machine runs this suite (hard
/// rule: a test must never depend on, let alone mutate, real machine state).
///
/// Requires a bottle up front via [`require_bottle`] — lib.sh's own die text,
/// through the exact `StageCtx`-shaped path that function expects — then
/// chains [`Stage::ALL_CHAIN`] through [`run_stage`] via [`run_chain`],
/// building a **fresh** [`StageCtx`] per stage (fresh `run_id`, fresh
/// executor — [`StageCtx::new`] mints both) but the same `paths`/`opts`/
/// `sink`, and one cancellation token shared across every stage so a single
/// Ctrl-C reaches whichever stage is currently running. `--dry-run` applies
/// uniformly because every per-stage `StageCtx` inherits `opts.dry_run`, each
/// printing its own trailing dry-run plan exactly as a standalone
/// `sabrage <stage> --dry-run` would.
async fn run_all(paths: &Paths, opts: &StageOptions, sink: &EventSink) -> i32 {
    // Doubles as the source of the `CancellationToken` every per-stage
    // `StageCtx` below shares — `StageCtx`'s own field, so this file never has
    // to name the type (see the Ctrl-C section's comment on that).
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

    // See `cmd_stage`'s matching check: `all` chains setup/build/install/run,
    // every one of which writes into the user store, so it fails closed on
    // an unusable `HOME` the same way, before any of those stages run.
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

// ── Ctrl-C / SIGTERM handling ─────────────────────────────────────────────
//
// `tokio::signal::ctrl_c()` needs the `signal` feature; sabrage-cli's `tokio`
// dependency only has `rt-multi-thread`/`macros` (see `Cargo.toml`'s own
// comment), and this crate may not edit Cargo manifests (Frame-owned — see
// this task's hard rules). A raw signal trap plus a polling `std::thread`
// needs neither: every Rust binary already links the C runtime that provides
// `signal(2)`, and each handler does the one thing that is
// async-signal-safe here — two relaxed atomic stores — leaving the actual
// cancellation (`sabrage_core`'s `CancellationToken::cancel()`, which is what
// makes `process::spawn_streamed`'s `killpg(SIGTERM)`→`SIGKILL` escalation
// fire for a running cmake/ninja/curl child) to a plain OS thread.
//
// `watch_termination_signals`/`watch_flag` are generic over a plain `FnOnce`
// callback rather than over the cancellation type itself, so this file never
// has to name `tokio_util::sync::CancellationToken` — sabrage-cli has no
// direct dependency on `tokio-util` (only `sabrage-core` does; see its
// Cargo.toml). `cmd_stage`/`run_all` supply it pre-captured in a closure
// instead.
//
// `demo.sh`'s `run.sh` traps *both* `INT` and `TERM` (lines 180-181) with the
// same teardown body — stop wine, close the dashboard, reap the encoder
// helper, restore audio — so `kill <pid>` on a running `./demo.sh run` takes
// the identical path a Ctrl-C does. `sabrage run` matches that: `SIGINT` and
// `SIGTERM` share one state machine here, not two independent ones, so
// either kind reaches the same cancellation.
//
// The watcher must not go one-shot after the first signal: a second signal —
// of *either* kind — has to be able to kill the process even while the first
// one's cancellation is still winding down a child (or during a phase with
// no child at all, e.g. `stop`'s non-executor code paths) — the same
// "impatient user, no more Mr. Nice trap" escape hatch a `./demo.sh`
// invocation gets for free from the shell's own default disposition. So the
// loop keeps running past the first fire: the first delivery of either
// SIGINT or SIGTERM runs `on_signal` once (unchanged), every later one of
// *either* kind restores `SIG_DFL` for *both* signals and re-raises
// whichever one was actually received, so the *kernel's* default action
// (terminate) applies — the process dies "killed by SIGINT" (exit 130) or
// "killed by SIGTERM" (exit 143) depending on what the second delivery was,
// rather than us picking either number by hand. The handler counts
// deliveries rather than setting a flag, so two signals arriving inside one
// poll interval still read as two.
//
// # A second signal of a *different* kind is still fatal
//
// `SIGINT` and `SIGTERM` increment the *same* `SIGNAL_COUNT` — there is no
// separate "SIGINT slot" and "SIGTERM slot" for the watcher to fall behind on
// independently. A `SIGINT` first, `SIGTERM` second (or the reverse) is
// exactly as fatal as two of the same kind: the first of either cancels, the
// second of either — regardless of which one arrived first — restores both
// dispositions and re-raises itself. `LAST_SIGNAL` records which signal
// number the *most recent* delivery was, written by each handler immediately
// before it bumps the shared counter, so the fatal action re-raises the one
// the user actually just sent rather than always SIGINT.
//
// # A signal during `run` (and `all`, while its chain is on `Run`)
//
// The first signal — SIGINT or SIGTERM — cancels the token `cmd_stage`/
// `run_all` handed to the stage's `StageCtx`, which is `run`'s own
// cancellation path: it stops wine, restores audio, and returns
// `Err(SabrageError::Cancelled)` — `report_stage_result` maps that to exit
// 130 through the ordinary error tail (`sabrage_core::error`'s
// `exit_code()`), **regardless of which signal triggered the cancellation**.
// That is a declared divergence from `demo.sh`: `run.sh`'s own TERM trap
// re-raises TERM after its teardown and so exits 143 on a TERM-initiated
// shutdown, while `sabrage run` collapses both signals onto the same
// "cancelled" outcome and always exits 130 for a *completed* cancellation —
// the signal's identity only survives long enough to pick the exit code of
// an *impatient* second signal (see `sabrage/PARITY.md`). A second signal
// does **not** wait for that teardown to finish: per the state machine above
// it restores `SIG_DFL` for both signals and re-raises whichever one just
// arrived, so `sabrage` itself dies immediately. Because the wine child is
// spawned *detached* — its own process group, never `kill_on_drop(true)` —
// an impatient double-tap during `run` leaves wine (and the game) running
// unsupervised: the audio device may still be routed to BlackHole, the ALVR
// dashboard may still be open, and `session-state.json` still describes
// both. Nothing left in this process can finish that teardown once the
// second signal lands — the next `sabrage run` or `sabrage stop` for that
// bottle is what reconciles the guards (`sabrage_core::session::reconcile`).

/// Poll interval for the signal watcher thread — frequent enough that a
/// human doesn't perceive the delay, infrequent enough to cost nothing idle.
const CTRL_C_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// `SIGINT`'s signal number — 2 on every platform this ships to (macOS).
const SIGINT: i32 = 2;

/// `SIGTERM`'s signal number — 15 on every platform this ships to (macOS).
/// `run.sh`'s line 181 traps this one too, with the same teardown as its
/// `INT` trap — see the module doc above.
const SIGTERM: i32 = 15;

/// How many `SIGINT`/`SIGTERM` deliveries the handlers have seen, combined.
/// A **count**, not a flag: two signals delivered inside one
/// [`CTRL_C_POLL_INTERVAL`] (a fast double-tap, or the `kill` pair an
/// automated test sends) collapsed into a single observation under a
/// `swap(false, ..)`'d `AtomicBool`, so the second signal was silently lost
/// and the process stayed in its "cancelling" state — the one gap left in "a
/// second signal is always fatal". `fetch_add` on an `AtomicUsize` is as
/// async-signal-safe as the store it replaces. Shared by both signals on
/// purpose (see the module doc's "second signal of a different kind" note):
/// there is one state machine here, not two.
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
/// whichever one was actually received (`LAST_SIGNAL`), so the kernel's own
/// default action (terminate) applies and the process dies under the
/// signal's own name — "killed by SIGTERM" (exit 143) for a second `kill
/// <pid>`, "killed by SIGINT" (exit 130) for a second Ctrl-C/`kill -INT` —
/// rather than us picking either number by hand. This is the real production
/// second-signal action (both traps installed by [`watch_termination_signals`]
/// share it); tests exercise [`watch_flag_with_second_action`] with a fake in
/// its place, since a real second SIGINT/SIGTERM during a test run would
/// otherwise kill the test binary.
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

/// `watch_flag`'s full generality: keeps polling `counter` for as long as the
/// process runs, calling `on_first_signal` (once) the first time it observes a
/// delivery, and `on_second_signal` on any delivery after that — so a caller
/// who never Ctrl-C's twice never runs the second action, and one who does
/// gets a live watcher for it rather than a one-shot that went quiet after the
/// first fire (the bug this replaces: `on_signal` used to `return` right after
/// firing, leaving nobody reading the flag for any later SIGINT).
///
/// It reads a monotone **count** and remembers how much of it it has already
/// consumed, rather than swapping a flag back to `false`: a burst delivered
/// inside one [`CTRL_C_POLL_INTERVAL`] is one observation but two deliveries,
/// and a flag cannot tell those apart — so a fast double-tap used to leave the
/// process merely "cancelling" instead of dead.
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

    // ── doctor arg parsing ───────────────────────────────────────────────────

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
    fn missing_value_messages_match_demo_sh_verbatim() {
        assert_eq!(
            parse_doctor_args(&args(&["--bottle"])).unwrap_err(),
            "error: --bottle needs a name"
        );
        assert_eq!(
            parse_doctor_args(&args(&["--bs-dir"])).unwrap_err(),
            "error: --bs-dir needs a path"
        );
        assert_eq!(
            parse_doctor_args(&args(&["--tap"])).unwrap_err(),
            "error: --tap needs a path"
        );
    }

    #[test]
    fn unknown_argument_message_matches_demo_sh_verbatim() {
        assert_eq!(
            parse_doctor_args(&args(&["--nope"])).unwrap_err(),
            "error: unknown argument '--nope'"
        );
        // A bare positional (no leading `--`) hits the same `*)` branch in
        // demo.sh's case loop.
        assert_eq!(
            parse_doctor_args(&args(&["Steam"])).unwrap_err(),
            "error: unknown argument 'Steam'"
        );
    }

    #[test]
    fn first_bad_argument_wins_no_aggregation() {
        let a = args(&["--bottle", "Steam", "--nope", "--bs-dir", "/x"]);
        assert_eq!(
            parse_doctor_args(&a).unwrap_err(),
            "error: unknown argument '--nope'"
        );
    }

    // ── A14-4: flags before the command ──────────────────────────────────────

    #[test]
    fn unknown_command_outcome_reports_the_first_bad_remaining_token() {
        // `--bottle Steam run`: demo.sh's `CMD="${1:-}"; shift` consumes
        // `--bottle` as `CMD` unconditionally, so its flag loop then sees
        // `Steam run` and reports `Steam` — never routing through `--bottle`'s
        // "needs a name" branch at all.
        let a = args(&["--bottle", "Steam", "run"]);
        assert_eq!(
            unknown_command_outcome(&a).unwrap_err(),
            "error: unknown argument 'Steam'"
        );
    }

    #[test]
    fn unknown_command_outcome_is_ok_when_the_remaining_tokens_parse_clean() {
        // `--verbose --bottle X`: once `--verbose` is consumed as `CMD`, the
        // remaining `--bottle X` parses fine under the shared six-flag
        // grammar — the shell's `case "$CMD"` would then fall through to its
        // own unknown-command text, which this file's usage/exit-2 fallback
        // mirrors (the caller's `_` arm, not this helper, prints it).
        let a = args(&["--verbose", "--bottle", "X"]);
        assert_eq!(unknown_command_outcome(&a), Ok(()));
    }

    // ── doctor rendering ─────────────────────────────────────────────────────

    #[test]
    fn pass_row_has_three_space_gap_like_ok() {
        let o = CheckOutcome::pass("sys.arch", "Apple Silicon (Apple M-series)");
        assert_eq!(
            format_outcome(&o, false, false).as_deref(),
            Some("  OK   Apple Silicon (Apple M-series)")
        );
    }

    #[test]
    fn warn_row_has_one_space_gap_like_warn() {
        let o = CheckOutcome::warn("bottle.template", "bottle template is not win11_64");
        assert_eq!(
            format_outcome(&o, false, false).as_deref(),
            Some("  WARN bottle template is not win11_64")
        );
    }

    #[test]
    fn fail_row_with_remedy_aligns_at_column_seven() {
        let o = CheckOutcome::fail(
            "cx.version",
            "CrossOver 26.1 < 26.2",
            "upgrade CrossOver to 26.2+",
        );
        assert_eq!(
            format_outcome(&o, false, false).as_deref(),
            Some("  FAIL CrossOver 26.1 < 26.2\n       remedy: upgrade CrossOver to 26.2+")
        );
    }

    #[test]
    fn fail_row_without_remedy_has_no_remedy_line() {
        let o = CheckOutcome::fail_bare("sys.arch", "not an Apple Silicon Mac (x86_64)");
        assert_eq!(
            format_outcome(&o, false, false).as_deref(),
            Some("  FAIL not an Apple Silicon Mac (x86_64)")
        );
    }

    #[test]
    fn info_row_is_two_space_indent_no_label() {
        let o = CheckOutcome::info(
            "game.present",
            "Beat Saber check skipped (needs --bottle or --bs-dir)",
        );
        assert_eq!(
            format_outcome(&o, false, false).as_deref(),
            Some("  Beat Saber check skipped (needs --bottle or --bs-dir)")
        );
    }

    #[test]
    fn skipped_and_not_implemented_print_nothing() {
        let skipped = CheckOutcome::skipped("hs.client", "no adb device".into());
        assert_eq!(format_outcome(&skipped, false, false), None);
        let ni = CheckOutcome::not_implemented("dep.dxmt");
        assert_eq!(format_outcome(&ni, false, false), None);
    }

    #[test]
    fn colors_wrap_only_the_label_text() {
        let o = CheckOutcome::pass("sys.arch", "Apple Silicon");
        assert_eq!(
            format_outcome(&o, true, false).as_deref(),
            Some("  \x1b[32mOK\x1b[0m   Apple Silicon")
        );
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
    fn verbose_with_no_detail_prints_nothing_extra() {
        let o = CheckOutcome::pass("sys.arch", "Apple Silicon (Apple M-series)");
        assert_eq!(
            format_outcome(&o, false, true).as_deref(),
            Some("  OK   Apple Silicon (Apple M-series)")
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

    // ── stage arg parsing ────────────────────────────────────────────────────

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
        // `--tap` is doctor-only; every other stage command must still reject
        // it (`sabrage doctor`'s own addition, not part of demo.sh's shared
        // flag grammar), exactly like any other unrecognized flag.
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

    // ── merge_stage_options ──────────────────────────────────────────────────

    #[test]
    fn merge_stage_options_forwards_all_six_flags_onto_the_env_base() {
        // The bug this closes: `--wired`/`--no-audio`/`--no-dashboard` were
        // parsed (so the shell's own grammar was matched byte-for-byte) but
        // then silently dropped — `StageOptions` carried no field for them at
        // the time, and nothing ever set the three that were added since.
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

    // ── A14-1: empty CLI values clear an env-derived preset ─────────────────

    #[test]
    fn merge_stage_options_empty_cli_values_clear_a_preset_env_base() {
        // `--bottle ""`/`--bs-dir ""` (a wrapper that always interpolates the
        // flag, even with its own variable unset) must behave exactly like
        // `${WINEVR_BOTTLE:-}`/`${WINEVR_BS_DIR:-<default>}` treating an
        // empty export as absent — not like an override to the empty string.
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

    // ── stage rendering ──────────────────────────────────────────────────────

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

    // ── stage_event_lines (Phase 3: run) ─────────────────────────────────────
    //
    // Pure projection, so "renders to nothing" is a value these assert on
    // directly instead of a `println!` side effect they would have to
    // intercept.

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

        // `--quiet` suppresses a Cr chunk exactly like any other Output,
        // regardless of tty-ness.
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
        // Finding #4: `render_stage_event` dropped `Fatal.remedy` entirely, so
        // the App Management deep link — the whole point of finding #6's
        // `upgrade_write_error` — never reached a CLI user.
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

    // ── A14-5: color gating is per-stream ────────────────────────────────────

    #[test]
    fn no_color_forces_both_streams_off_regardless_of_tty() {
        assert_eq!(
            colors_from(true, true, true),
            Colors {
                stdout: false,
                stderr: false,
                stdout_tty: true,
                stderr_tty: true,
            }
        );
    }

    #[test]
    fn colors_from_is_independent_per_stream() {
        // stdout piped (no color), stderr on a terminal (colored) — the
        // "stderr redirected to a file, stdout a tty" case inverted: this is
        // "stdout piped, stderr a tty", the other half of the same bug.
        assert_eq!(
            colors_from(false, false, true),
            Colors {
                stdout: false,
                stderr: true,
                stdout_tty: false,
                stderr_tty: true,
            }
        );
        // The mirrored case: stdout a terminal, stderr redirected to a file.
        assert_eq!(
            colors_from(false, true, false),
            Colors {
                stdout: true,
                stderr: false,
                stdout_tty: true,
                stderr_tty: false,
            }
        );
    }

    #[test]
    fn fatal_uses_stderr_colors_while_a_line_event_uses_stdout_colors() {
        // Finding A14-5: `Fatal` rows go to stderr but color was previously
        // gated on stdout's terminal-ness alone. With stdout piped (no
        // color) and stderr a terminal (colored), `Fatal` must still come
        // out colored — and an ordinary `Line` in the same call must stay
        // uncolored, since it renders on stdout.
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

    // ── dry-run plan rendering ───────────────────────────────────────────────

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

    // ── `sabrage run` / `sabrage all` wiring ─────────────────────────────────
    //
    // `Stage::Run`'s body (`sabrage_core::stages::run::run`) is real as of
    // Phase 3 — it actually launches wine and the game — and `Stage::Setup`/
    // `Build`/`Install`'s bodies really do touch the machine too; none of the
    // four may be reached from a unit test (this task's hard rules). Every
    // test below stays on the safe side of that boundary: an argument error
    // returns before `resolve_repo_root` is even called, and `run_all`'s
    // bottle precheck (driven with a hand-built `opts`, never
    // `StageOptions::from_env()`, so it is deterministic regardless of the
    // *real* machine's `WINEVR_BOTTLE`/CrossOver state) dies before
    // `run_chain`/`run_stage` are ever reached.

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

    // ── Ctrl-C / SIGTERM watcher ─────────────────────────────────────────────

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
        // Finding #7: the old `watch_flag` returned right after its one
        // callback fired, so nothing was left reading the flag for a second
        // Ctrl-C. Exercised here via the injectable second action
        // (`watch_flag_with_second_action`) rather than
        // `watch_flag`/`terminate_via_default_disposition_of_last_signal`
        // directly — that path really does `raise()` on the calling process,
        // which would kill this test binary.
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

        // The bug: after the first `return`, nobody was polling any more, so
        // this second delivery would never be observed at all.
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
        // Finding #6: the watcher used to consume the flag with
        // `swap(false, ..)`, so a burst delivered between two polls — a fast
        // double-tap, or the `kill -INT` pair an automated test sends — read
        // as a single Ctrl-C and left the process merely "cancelling" instead
        // of dead. Both deliveries land here before the watcher's first poll
        // can possibly run, which is exactly the collapsing window.
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
        // Production shares one `SIGNAL_COUNT` between the real `SIGINT` and
        // `SIGTERM` handlers, with `LAST_SIGNAL` recording which signal
        // number the *most recent* delivery was (see the module doc above):
        // the first delivery of *either* kind cancels, and the second — of
        // either kind, not necessarily the same one — is fatal and re-raises
        // whichever one actually just arrived. Reproduced here with
        // process-local fakes standing in for the two real handlers and
        // `LAST_SIGNAL`, for the same reason the tests above avoid installing
        // a real trap: a real second SIGINT/SIGTERM during a test run would
        // kill the test binary.
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
