//! `sabrage` — the native Sabrage pipeline CLI.
//!
//! Phase 1 scope: `sabrage doctor`, plus the top-level `--version` / no-args /
//! `-h`/`--help` surface. Every other `demo.sh` subcommand (`setup`, `build`,
//! `install`, `run`, `stop`, `all`) is not implemented yet and falls through to
//! the usage text, exit 2 — the same fallback `demo.sh`'s own `case` uses for an
//! unrecognized `CMD`.
//!
//! # Argument parsing
//!
//! Hand-parsed, deliberately not `clap` (see the comment in
//! `sabrage-cli/Cargo.toml`), to match `demo.sh`'s own loop byte-for-byte:
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
//! `--tap <file>` has no `demo.sh` counterpart (the shell side uses the
//! `WINEVR_DOCTOR_TAP` env var instead, opt-in the same way `WINEVR_DOCTOR_SOFT`
//! is) — it is sabrage's own equivalent, parsed with the same "needs a value or
//! exit 2" shape and its own message.
//!
//! # Human output
//!
//! Mirrors `scripts/demo/doctor.sh`'s rendering (via `lib.sh`'s `ok`/`warn`/
//! `fail`/`info`), with one deliberate divergence: colors are gated on
//! `isatty(stdout)` and `NO_COLOR`, where the shell emits ANSI codes
//! unconditionally. `Skipped`/`NotImplemented` rows print nothing to the human
//! console — matching `tap()`, which only ever writes to `$WINEVR_DOCTOR_TAP`
//! and never to stdout.

use std::env;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::exit;

use sabrage_core::checks::{run_doctor, CheckCtx, CheckOptions, CheckOutcome, CheckStatus};
use sabrage_core::tap::render_tap;
use sabrage_core::Paths;

const ANSI_GREEN: &str = "\x1b[32m";
const ANSI_YELLOW: &str = "\x1b[33m";
const ANSI_RED: &str = "\x1b[31m";
const ANSI_RESET: &str = "\x1b[0m";

const BANNER: &str = "== wine-vr demo doctor ==";

const USAGE: &str = "\
Usage: sabrage <command> [options]

Commands:
  doctor              check every prerequisite, print remedies (like ./demo.sh doctor)

Options (doctor):
  --bottle <name>     CrossOver bottle (or env WINEVR_BOTTLE)
  --bs-dir <path>     Beat Saber 1.29.4 install dir (or env WINEVR_BS_DIR)
  --tap <file>        write parity tap lines (\"<slug> <status>\") to file, truncated first
  --no-audio          (reserved; unused by doctor)
  --no-dashboard      (reserved; unused by doctor)
  --wired             (reserved; unused by doctor)
  --verbose           (reserved; unused by doctor)

  sabrage --version   print the CLI version
  sabrage -h|--help   print this message
";

fn main() {
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
        _ => {
            print!("{USAGE}");
            exit(2);
        }
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

    let repo_root = match resolve_repo_root() {
        Ok(p) => p,
        Err(msg) => {
            eprintln!("{msg}");
            exit(1);
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

// ── rendering ─────────────────────────────────────────────────────────────────

/// `lib.sh`'s `ok`/`warn`/`fail`/`info`, transcribed:
///
/// ```zsh
/// ok()   { print -r -- "  ${_G}OK${_N}   $*"; }
/// warn() { print -r -- "  ${_Y}WARN${_N} $*"; }
/// fail() { print -r -- "  ${_R}FAIL${_N} $1"; [ $# -gt 1 ] && print -r -- "       remedy: $2"; }
/// info() { print -r -- "  $*"; }
/// ```
///
/// `Skipped`/`NotImplemented` return `None` — those statuses only ever reach
/// zsh's silent `tap()` channel, never stdout. So do `quiet` passes: rows
/// doctor.sh taps (`tap <slug> ok`) without printing a console line.
fn format_outcome(o: &CheckOutcome, colors: bool) -> Option<String> {
    if o.quiet {
        return None;
    }
    match o.status {
        CheckStatus::Pass => Some(format!(
            "  {}   {}",
            label("OK", ANSI_GREEN, colors),
            o.message
        )),
        CheckStatus::Warn => Some(format!(
            "  {} {}",
            label("WARN", ANSI_YELLOW, colors),
            o.message
        )),
        CheckStatus::Fail => {
            let mut s = format!("  {} {}", label("FAIL", ANSI_RED, colors), o.message);
            if let Some(remedy) = &o.remedy {
                s.push('\n');
                s.push_str("       remedy: ");
                s.push_str(remedy);
            }
            Some(s)
        }
        CheckStatus::Info => Some(format!("  {}", o.message)),
        CheckStatus::Skipped | CheckStatus::NotImplemented => None,
    }
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
/// earns the gate the shell script never needed.
fn use_colors() -> bool {
    if env::var_os("NO_COLOR").is_some() {
        return false;
    }
    std::io::stdout().is_terminal()
}

// ── repo root resolution ─────────────────────────────────────────────────────

/// `SABRAGE_REPO_ROOT` env var, else walk up from the running executable
/// looking for the `demo.sh` + `scripts/demo/lib.sh` pair that identifies the
/// wine-vr checkout, else a clear error.
///
/// Unlike `demo.sh` (`ROOT="$(cd "$(dirname "$0")" && pwd)"`), Sabrage's binary
/// does not live inside the repo it operates on, so this has no shell
/// equivalent to mirror — see `paths.rs`'s module doc.
fn resolve_repo_root() -> Result<PathBuf, String> {
    if let Some(over) = repo_root_override(env::var("SABRAGE_REPO_ROOT").ok().as_deref()) {
        return Ok(over);
    }
    let exe = env::current_exe()
        .map_err(|e| format!("error: cannot resolve sabrage's own executable path: {e}"))?;
    find_repo_root_from_exe(&exe).ok_or_else(|| {
        format!(
            "error: could not locate the wine-vr repo root (looked for demo.sh + \
             scripts/demo/lib.sh in every directory above {}); set SABRAGE_REPO_ROOT \
             to override",
            exe.display()
        )
    })
}

fn repo_root_override(v: Option<&str>) -> Option<PathBuf> {
    // Canonicalize so a symlinked or `..`-containing override still satisfies
    // host.manifest's exact string equality on library_path.
    v.filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .map(|p| p.canonicalize().unwrap_or(p))
}

/// Walk `exe`'s ancestors (not `exe` itself — it need not exist for this to be
/// tested) for the first directory containing both `demo.sh` and
/// `scripts/demo/lib.sh`.
fn find_repo_root_from_exe(exe: &Path) -> Option<PathBuf> {
    let mut dir = exe.parent();
    while let Some(d) = dir {
        if d.join("demo.sh").is_file() && d.join("scripts/demo/lib.sh").is_file() {
            return Some(d.to_path_buf());
        }
        dir = d.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── arg parsing ──────────────────────────────────────────────────────────

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

    // ── rendering ────────────────────────────────────────────────────────────

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

    // ── repo root ────────────────────────────────────────────────────────────

    fn real_repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .canonicalize()
            .expect("repo root resolves from CARGO_MANIFEST_DIR")
    }

    #[test]
    fn override_wins_and_ignores_empty() {
        assert_eq!(
            repo_root_override(Some("/custom/root")),
            Some(PathBuf::from("/custom/root"))
        );
        assert_eq!(repo_root_override(Some("")), None);
        assert_eq!(repo_root_override(None), None);
    }

    #[test]
    fn finds_repo_root_by_walking_up_from_a_plausible_exe_path() {
        let root = real_repo_root();
        // The exe file itself need not exist — only demo.sh/lib.sh under each
        // candidate ancestor are actually stat'd.
        let fake_exe = root.join("target/debug/sabrage");
        assert_eq!(find_repo_root_from_exe(&fake_exe), Some(root.clone()));

        // Nested deeper still finds it.
        let deeper = root.join("some/nested/install/dir/sabrage");
        assert_eq!(find_repo_root_from_exe(&deeper), Some(root));
    }

    #[test]
    fn returns_none_when_nothing_above_has_the_pair() {
        assert_eq!(
            find_repo_root_from_exe(Path::new("/nonexistent/sabrage/bin/sabrage")),
            None
        );
    }
}
