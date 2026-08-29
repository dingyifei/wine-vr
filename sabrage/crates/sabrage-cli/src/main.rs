//! `sabrage` — the native Sabrage pipeline CLI.
//!
//! Phase 1 shipped `sabrage doctor`. Phase 2 adds the four mutating stages —
//! `setup`, `build`, `install`, `stop` — as thin renderers over
//! [`sabrage_core::stages::run_stage`]: this file owns argument parsing and
//! turning a [`sabrage_core::StageEvent`] stream into the exact console text
//! its `demo.sh` equivalent prints, nothing more (the stage bodies themselves
//! live in `sabrage-core`). `run`/`all` still fall through to the usage text,
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
//! too, even though only `run` (not yet implemented here) reads the last
//! four:
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
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use sabrage_core::checks::{run_doctor, CheckCtx, CheckOptions, CheckOutcome, CheckStatus};
use sabrage_core::tap::render_tap;
use sabrage_core::{
    resolve_repo_root, run_stage, EventSink, Paths, PlannedAction, SabrageError, Severity, Stage,
    StageCtx, StageEvent, StageOptions, Stream,
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
  stop                stop the game + wineserver for a bottle (like ./demo.sh stop)

Options (every command above):
  --bottle <name>     CrossOver bottle (or env WINEVR_BOTTLE)
  --bs-dir <path>     Beat Saber 1.29.4 install dir (or env WINEVR_BS_DIR)
  --no-audio          (reserved; unused before the run stage)
  --no-dashboard      (reserved; unused before the run stage)
  --wired             (reserved; unused before the run stage)
  --verbose           (reserved; unused before the run stage)

Options (doctor only):
  --tap <file>        write parity tap lines (\"<slug> <status>\") to file, truncated first

Options (setup/build/install/stop only):
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
        "stop" => exit(cmd_stage(Stage::Stop, &args[1..]).await),
        _ => {
            print!("{USAGE}");
            exit(2);
        }
    }
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

/// Parse `doctor`'s argument list. Returns the `demo.sh`-verbatim error message
/// (sans the `error: ` prefix's destination — the caller decides where it goes)
/// on the first bad argument, exactly like the shell's `case` loop: no
/// aggregation, first failure wins.
fn parse_doctor_args(args: &[String]) -> Result<DoctorArgs, String> {
    let mut out = DoctorArgs::default();
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--bottle" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| "error: --bottle needs a name".to_string())?;
                out.bottle = Some(v.clone());
                i += 2;
            }
            "--bs-dir" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| "error: --bs-dir needs a path".to_string())?;
                out.bs_dir = Some(PathBuf::from(v));
                i += 2;
            }
            "--tap" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| "error: --tap needs a path".to_string())?;
                out.tap = Some(PathBuf::from(v));
                i += 2;
            }
            "--no-audio" => {
                out.no_audio = true;
                i += 1;
            }
            "--no-dashboard" => {
                out.no_dashboard = true;
                i += 1;
            }
            "--wired" => {
                out.wired = true;
                i += 1;
            }
            "--verbose" => {
                out.verbose = true;
                i += 1;
            }
            other => return Err(format!("error: unknown argument '{other}'")),
        }
    }
    Ok(out)
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
    let mut opts = CheckOptions::from_env();
    if let Some(b) = parsed.bottle {
        opts.bottle_name = Some(b);
    }
    if let Some(d) = parsed.bs_dir {
        opts.bs_dir_override = Some(d);
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

    let ctx = CheckCtx::new(Paths::new(&repo_root), opts);
    let colors = use_colors();

    println!("{BANNER}");
    let report = run_doctor(&ctx, |outcome| {
        if let Some(line) = format_outcome(&outcome, colors) {
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
fn format_outcome(o: &CheckOutcome, colors: bool) -> Option<String> {
    if o.quiet {
        return None;
    }
    match o.status {
        CheckStatus::Pass => Some(ok_row(&o.message, colors)),
        CheckStatus::Warn => Some(warn_row(&o.message, colors)),
        CheckStatus::Fail => Some(fail_row(&o.message, o.remedy.as_deref(), colors)),
        CheckStatus::Info => Some(info_row(&o.message)),
        CheckStatus::Skipped | CheckStatus::NotImplemented => None,
    }
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

/// isatty(stdout) && !NO_COLOR — the one deliberate divergence from `lib.sh`,
/// which emits its `$'\e[32m'`-style codes unconditionally. Native output is
/// more likely to be piped/captured (by the GUI, by `--tap` consumers), so it
/// earns the gate the shell script never needed. Shared by doctor and the
/// stage commands; a stage's `Fatal` line goes to stderr rather than stdout,
/// but is still gated on stdout's terminal-ness for simplicity — the common
/// case is both streams sharing one terminal.
fn use_colors() -> bool {
    if env::var_os("NO_COLOR").is_some() {
        return false;
    }
    std::io::stdout().is_terminal()
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
    let mut out = StageArgs::default();
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--bottle" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| "error: --bottle needs a name".to_string())?;
                out.bottle = Some(v.clone());
                i += 2;
            }
            "--bs-dir" => {
                let v = args
                    .get(i + 1)
                    .ok_or_else(|| "error: --bs-dir needs a path".to_string())?;
                out.bs_dir = Some(PathBuf::from(v));
                i += 2;
            }
            "--no-audio" => {
                out.no_audio = true;
                i += 1;
            }
            "--no-dashboard" => {
                out.no_dashboard = true;
                i += 1;
            }
            "--wired" => {
                out.wired = true;
                i += 1;
            }
            "--verbose" => {
                out.verbose = true;
                i += 1;
            }
            "--dry-run" => {
                out.dry_run = true;
                i += 1;
            }
            "--quiet" => {
                out.quiet = true;
                i += 1;
            }
            other => return Err(format!("error: unknown argument '{other}'")),
        }
    }
    Ok(out)
}

/// Render one [`StageEvent`] exactly the way its `demo.sh` equivalent prints
/// it. `bottle_label` is the actual bottle name once one is known, else the
/// literal `<name>` placeholder — [`format_footer`]'s convention, reused here
/// for `install`'s closing line.
fn render_stage_event(ev: &StageEvent, bottle_label: &str, colors: bool, quiet: bool) {
    match ev {
        StageEvent::StageStarted { stage, .. } => {
            println!("== wine-vr demo {stage} ==");
        }
        StageEvent::Section { title, .. } => {
            println!("-- {title}");
        }
        StageEvent::Line {
            severity,
            text,
            remedy,
            ..
        } => {
            println!(
                "{}",
                format_line_event(*severity, text, remedy.as_deref(), colors)
            );
        }
        StageEvent::Output { stream, chunk, .. } => {
            if !quiet {
                match stream {
                    Stream::Stdout => println!("{chunk}"),
                    Stream::Stderr => eprintln!("{chunk}"),
                }
            }
        }
        StageEvent::Progress { .. } => {
            // Nothing extra: ninja's "[n/m]" and curl's progress bar already
            // arrive as `Output` chunks straight from the child.
        }
        StageEvent::AutoFixed { description, .. } => {
            // Not reachable from setup/build/install/stop today — only a
            // launch preflight applies fixes (Phase 3) — rendered defensively
            // in the same `ok` shape lib.sh's own self-heals use.
            println!("{}", ok_row(&format!("auto-fixed: {description}"), colors));
        }
        StageEvent::NeedsAdmin { reason, .. } => {
            println!("{}", info_row(reason));
        }
        StageEvent::Fatal {
            message, remedy, ..
        } => {
            for line in fatal_lines(message, remedy.as_deref(), colors) {
                eprintln!("{line}");
            }
        }
        StageEvent::StageFinished { stage, ok, .. } => {
            if *ok {
                if let Some(line) = closing_line(*stage, bottle_label) {
                    println!("{line}");
                }
            }
        }
    }
}

/// Run one stage and return the process exit code (never calls `exit` itself,
/// so `main` stays the only place that does). Today's stage bodies are all
/// `todo!()` — see `sabrage-core/src/stages/*.rs` — so this wires the
/// plumbing up to and through [`run_stage`] correctly for when they land; it
/// is exercised up to that boundary by this file's tests.
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

    // WINEVR_* env is the base; CLI flags override — same precedence as doctor.
    let mut opts = StageOptions::from_env();
    if let Some(b) = parsed.bottle {
        opts.bottle_name = Some(b);
    }
    if let Some(d) = parsed.bs_dir {
        opts.bs_dir_override = Some(d);
    }
    if parsed.verbose {
        opts.verbose = true;
    }
    opts.dry_run = parsed.dry_run;
    // `--wired`/`--no-audio`/`--no-dashboard` are parsed (so the shell's own
    // "accept, don't reject" grammar is matched byte-for-byte) but otherwise
    // dropped here: `StageOptions` carries no field for them yet — they are
    // Phase 3 launch options (see `StageOptions`'s own doc comment) and
    // `demo.sh` never reads them before `run` either.

    let bottle_label = opts
        .bottle_name
        .clone()
        .unwrap_or_else(|| "<name>".to_string());
    let colors = use_colors();
    let quiet = parsed.quiet;

    let sink: EventSink = Arc::new(move |ev: StageEvent| {
        render_stage_event(&ev, &bottle_label, colors, quiet);
    });

    let ctx = StageCtx::new(Paths::new(&repo_root), opts, sink, Default::default());
    {
        let cancel = ctx.cancel.clone();
        watch_sigint(move || cancel.cancel());
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
fn error_already_reported_as_fatal(e: &SabrageError) -> bool {
    matches!(
        e,
        SabrageError::Fatal { .. } | SabrageError::TccDenied { .. } | SabrageError::AdminDeclined
    )
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

// ── Ctrl-C handling ──────────────────────────────────────────────────────────
//
// `tokio::signal::ctrl_c()` needs the `signal` feature; sabrage-cli's `tokio`
// dependency only has `rt-multi-thread`/`macros` (see `Cargo.toml`'s own
// comment), and this crate may not edit Cargo manifests (Frame-owned — see
// this task's hard rules). A raw `SIGINT` trap plus a polling `std::thread`
// needs neither: every Rust binary already links the C runtime that provides
// `signal(2)`, and the handler itself does the one thing that is
// async-signal-safe here — a relaxed atomic store — leaving the actual
// cancellation (`sabrage_core`'s `CancellationToken::cancel()`, which is what
// makes `process::spawn_streamed`'s `killpg(SIGTERM)`→`SIGKILL` escalation
// fire for a running cmake/ninja/curl child) to a plain OS thread.
//
// `watch_sigint`/`watch_flag` are generic over a plain `FnOnce` callback
// rather than over the cancellation type itself, so this file never has to
// name `tokio_util::sync::CancellationToken` — sabrage-cli has no direct
// dependency on `tokio-util` (only `sabrage-core` does; see its Cargo.toml).
// `cmd_stage` supplies it pre-captured in a closure instead.
//
// The watcher must not go one-shot after the first Ctrl-C: a second SIGINT
// has to be able to kill the process even while the first one's cancellation
// is still winding down a child (or during a phase with no child at all, e.g.
// `stop`'s non-executor code paths) — the same "impatient user, no more Mr.
// Nice trap" escape hatch a `./demo.sh` invocation gets for free from the
// shell's own default SIGINT disposition. So the loop keeps running past the
// first fire: the first delivery runs `on_signal` once (unchanged), every
// later one restores `SIG_DFL` and re-raises SIGINT on ourselves, so the
// *kernel's* default action (terminate) applies — the process dies "killed by
// SIGINT", exit 130, rather than us picking that number by hand. The handler
// counts deliveries rather than setting a flag, so two signals arriving inside
// one poll interval still read as two.

/// Poll interval for the Ctrl-C watcher thread — frequent enough that a
/// human doesn't perceive the delay, infrequent enough to cost nothing idle.
const CTRL_C_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// How many `SIGINT`s the handler has seen. A **count**, not a flag: two
/// signals delivered inside one [`CTRL_C_POLL_INTERVAL`] (a fast double-tap,
/// or the `kill -INT` pair an automated test sends) collapsed into a single
/// observation under a `swap(false, ..)`'d `AtomicBool`, so the second Ctrl-C
/// was silently lost and the process stayed in its "cancelling" state — the
/// one gap left in "a second Ctrl-C is always fatal". `fetch_add` on an
/// `AtomicUsize` is as async-signal-safe as the store it replaces.
static SIGINT_COUNT: AtomicUsize = AtomicUsize::new(0);

/// `SIG_DFL` — restoring it before re-raising `SIGINT` on the second Ctrl-C
/// lets the OS's own default disposition (terminate) apply, rather than us
/// synthesizing the exit code by hand.
const SIG_DFL: usize = 0;

extern "C" fn on_sigint(_signum: i32) {
    SIGINT_COUNT.fetch_add(1, Ordering::Relaxed);
}

extern "C" {
    /// `signal(2)`. Both the handler parameter and the return value are typed
    /// `usize` rather than a function-pointer type: the return value is never
    /// read (its bits are simply discarded, sound regardless of declared
    /// width), and the `usize` parameter type is what lets this one
    /// declaration pass *either* a real handler (`on_sigint as usize`) *or*
    /// the `SIG_DFL` sentinel (`0`) without a raw function-pointer transmute.
    fn signal(signum: i32, handler: usize) -> usize;
    /// `raise(2)` — used only to re-deliver `SIGINT` to this process itself
    /// after restoring `SIG_DFL`, on a second Ctrl-C.
    fn raise(signum: i32) -> i32;
}

/// Install the real `SIGINT` trap and spawn the watcher thread that calls
/// `on_signal` once, the moment the trap first fires (and is fatal to the
/// process on the next one — see the module doc above).
fn watch_sigint(on_signal: impl FnOnce() + Send + 'static) {
    watch_flag(&SIGINT_COUNT, on_signal);
    unsafe {
        signal(2, on_sigint as *const () as usize); // SIGINT == 2 on every platform this ships to (macOS).
    }
}

/// Restore `SIGINT`'s default disposition and re-raise it on ourselves, so
/// the kernel's own default action (terminate) applies. This is the real
/// production "second Ctrl-C" action; tests exercise
/// [`watch_flag_with_second_action`] with a fake in its place; a real second
/// SIGINT during a test run would otherwise kill the test binary.
fn terminate_via_default_sigint_disposition() {
    unsafe {
        signal(2, SIG_DFL);
        raise(2);
    }
}

/// The polling primitive `watch_sigint` builds on, factored out so it is
/// testable without installing a real signal handler or touching the
/// process-wide [`SIGINT_COUNT`] counter. Delegates to
/// [`watch_flag_with_second_action`] with the real fatal second action.
fn watch_flag(counter: &'static AtomicUsize, on_signal: impl FnOnce() + Send + 'static) {
    watch_flag_with_second_action(counter, on_signal, terminate_via_default_sigint_disposition);
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

    // ── doctor rendering ─────────────────────────────────────────────────────

    #[test]
    fn pass_row_has_three_space_gap_like_ok() {
        let o = CheckOutcome::pass("sys.arch", "Apple Silicon (Apple M-series)");
        assert_eq!(
            format_outcome(&o, false).as_deref(),
            Some("  OK   Apple Silicon (Apple M-series)")
        );
    }

    #[test]
    fn warn_row_has_one_space_gap_like_warn() {
        let o = CheckOutcome::warn("bottle.template", "bottle template is not win11_64");
        assert_eq!(
            format_outcome(&o, false).as_deref(),
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
            format_outcome(&o, false).as_deref(),
            Some("  FAIL CrossOver 26.1 < 26.2\n       remedy: upgrade CrossOver to 26.2+")
        );
    }

    #[test]
    fn fail_row_without_remedy_has_no_remedy_line() {
        let o = CheckOutcome::fail_bare("sys.arch", "not an Apple Silicon Mac (x86_64)");
        assert_eq!(
            format_outcome(&o, false).as_deref(),
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
            format_outcome(&o, false).as_deref(),
            Some("  Beat Saber check skipped (needs --bottle or --bs-dir)")
        );
    }

    #[test]
    fn skipped_and_not_implemented_print_nothing() {
        let skipped = CheckOutcome::skipped("hs.client", "no adb device".into());
        assert_eq!(format_outcome(&skipped, false), None);
        let ni = CheckOutcome::not_implemented("dep.dxmt");
        assert_eq!(format_outcome(&ni, false), None);
    }

    #[test]
    fn colors_wrap_only_the_label_text() {
        let o = CheckOutcome::pass("sys.arch", "Apple Silicon");
        assert_eq!(
            format_outcome(&o, true).as_deref(),
            Some("  \x1b[32mOK\x1b[0m   Apple Silicon")
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
    fn fatal_line_has_no_leading_spaces_unlike_the_other_rows() {
        assert_eq!(
            fatal_line("run lands in Phase 3", false),
            "FATAL run lands in Phase 3"
        );
        assert_eq!(
            fatal_line("boom", true),
            format!("{ANSI_RED}FATAL{ANSI_RESET} boom")
        );
    }

    #[test]
    fn a_fatal_with_a_remedy_gets_the_same_continuation_line_a_fail_row_does() {
        // Finding #4: `render_stage_event` dropped `Fatal.remedy` entirely, so
        // the App Management deep link — the whole point of finding #6's
        // `upgrade_write_error` — never reached a CLI user.
        assert_eq!(
            fatal_lines("cannot write /x", None, false),
            vec!["FATAL cannot write /x".to_string()],
            "no remedy, no continuation — lib.sh's die shape exactly"
        );
        assert_eq!(
            fatal_lines(
                "cannot write /x",
                Some("grant it in System Settings"),
                false
            ),
            vec![
                "FATAL cannot write /x".to_string(),
                "       remedy: grant it in System Settings".to_string(),
            ]
        );
        // Same seven-space indent `fail_row` uses, so a remedy reads the same
        // wherever it appears.
        assert_eq!(
            fail_row("x", Some("y"), false),
            "  FAIL x\n       remedy: y"
        );
    }

    #[test]
    fn errors_that_already_emitted_a_fatal_are_not_reported_a_second_time() {
        // `Fatal` is the `die`-shaped one; the other two are emitted by
        // `privilege` before it returns them (see the predicate's doc).
        for e in [
            SabrageError::Fatal {
                message: "boom".into(),
                remedy: None,
            },
            SabrageError::TccDenied {
                path: PathBuf::from("/x"),
            },
            SabrageError::AdminDeclined,
        ] {
            assert!(error_already_reported_as_fatal(&e), "{e:?}");
        }
        // Everything else must still print its `error: {e}` tail.
        assert!(!error_already_reported_as_fatal(&SabrageError::Cancelled));
        assert!(!error_already_reported_as_fatal(&SabrageError::io(
            std::path::Path::new("/x"),
            std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        )));
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

    // ── Ctrl-C watcher ───────────────────────────────────────────────────────

    #[test]
    fn watch_flag_invokes_the_callback_once_the_flag_is_set() {
        // A private static, never touched by `watch_sigint`'s real
        // `SIGINT_COUNT` — this exercises the same polling primitive in
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
        // `watch_flag`/`terminate_via_default_sigint_disposition` directly —
        // that path really does `raise(SIGINT)` on the calling process, which
        // would kill this test binary.
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
}
