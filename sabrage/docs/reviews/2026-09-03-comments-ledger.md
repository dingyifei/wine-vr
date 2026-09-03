# Sabrage — comment scan 2026-09-03: replacement text

*Companion to `2026-09-03-comments.md`: the exact replacement of every row the program rewrites or adds, keyed by the same ids. The current text of each block is what stands at the cited lines of the file at `7688529`; the full before/after ledger with that text is `evidence/comments-2026-09-03/ledger.md` (local artifact). Deleted rows are listed by id only. Strike a row by id.*

## `sabrage/crates/sabrage-cli/src/main.rs`

Deleted (nothing carried): B0003, B0005, B0011, B0012, B0016, B0032, B0076, B0077, B0079, B0081, B0083, B0087, B0090, B0098, B0104, B0106, B0109

### B0001 · l.1–59 · REWRITE (confirm) ·p · rule 1.2 · 59 → 20

````text
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
````

### B0002 · l.159–172 · REWRITE (confirm) · rule 1.7 · 14 → 9

````text
/// The `_` arm's decision for an unrecognized first token, factored into a pure
/// helper so it is testable without `exit` (A14-4).
///
/// `demo.sh` consumes `CMD="${1:-}"` unconditionally before its flag loop runs,
/// so `args[0]` is treated as already-consumed and `args[1..]` is parsed with
/// the six shared flags. `Err` is the shell's own first-bad-argument message;
/// `Ok(())` means the shell would have fallen through to its unknown-`CMD` arm
/// instead — this file's usage-text-then-exit-2 fallback. Pinned by
/// tests::unknown_command_outcome_mirrors_the_shells_shift_then_parse.
````

### B0004 · l.179–184 · REWRITE (confirm) · rule 1.2 · 6 → 7

````text
/// Print a bootstrap failure as `error: <msg>`, plus [`SabrageError`]'s remedy
/// line when it carries one.
///
/// Sabrage-only: repo-root and `HOME` resolution run before any
/// [`StageCtx`]/[`EventSink`] exists (doctor has no `StageCtx` at all), so there
/// is no `StageEvent::Fatal` or `die()` line to ride on and no shell text to
/// match — finding the repo root has no shell equivalent (`paths.rs`'s doc).
````

### B0007 · l.206–211 · REWRITE (confirm) · rule 1.7 · 6 → 4

````text
/// The six `demo.sh`-verbatim flags every stage script (and `doctor`) shares:
/// `--bottle`/`--bs-dir`/`--no-audio`/`--no-dashboard`/`--wired`/`--verbose`.
/// Shared by [`parse_doctor_args`]/[`parse_stage_args`] so each `demo.sh`-verbatim
/// error string lives in exactly one place (S-C3-cli-ipc).
````

### B0008 · l.222–229 · REWRITE (confirm) · rule 1.2 · 8 → 7

````text
/// Try to consume one of the six [`CommonArgs`] flags at `args[i]`.
///
/// `Ok(Some(next_i))` — recognized and consumed, resume the caller's loop at
/// `next_i`. `Ok(None)` — `args[i]` is not one of the six; the caller tries its
/// own extra flags (`--tap`, `--dry-run`, `--quiet`) before the shared "unknown
/// argument" error. `Err` — a value-taking flag with no value; first bad
/// argument wins, no aggregation.
````

### B0010 · l.305–315 · REWRITE (confirm) · rule 1.5 · 11 → 8

````text
/// Merge parsed `doctor` flags onto the env-derived base — `WINEVR_*` is the
/// base, flags override — [`merge_stage_options`]'s `doctor` counterpart,
/// separate so it is testable without a repo root or a [`CheckCtx`].
///
/// A14-1: an explicit empty `--bottle`/`--bs-dir` clears the env-derived value
/// rather than overriding it with `Some("")`, because `demo.sh`'s
/// `${WINEVR_BOTTLE:-}` treats an empty export as absent. Pinned by
/// tests::merge_doctor_options_empty_cli_values_clear_a_preset_env_base.
````

### B0013 · l.381–383 · REWRITE (confirm) · rule 1.3 · 3 → 3

````text
        // "truncate first" (the `--help` text): `fs::write` creates-or-truncates
        // and writes the payload in one shot, never `tap::append_tap`'s
        // incremental `>>` channel.
````

### B0014 · l.392–394 · REWRITE (confirm) · rule 1.6 · 3 → 2

````text
    // doctor.sh: `[ -n "${WINEVR_DOCTOR_SOFT:-}" ] || exit "$FAILCOUNT"` — a
    // non-empty value short-circuits the exit, so the process exits 0.
````

### B0021 · l.436–448 · REWRITE (confirm) · rule 1.2 · 13 → 11

````text
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
````

### B0023 · l.483–489 · REWRITE (confirm) · rule 1.2 · 7 → 5

````text
/// `lib.sh`'s `die()`: `"${_R}FATAL${_N} $*"` — no leading indent (unlike
/// `ok`/`warn`/`fail`/`info`, which all indent two) and no separate `remedy:`
/// line. A `die` call folds its remedy into the message text at the call site
/// (`require_bottle`'s two-line message is the canonical example), so this
/// prints no line `die()` never printed.
````

### B0024 · l.494–505 · REWRITE (confirm) · rule 1.5 · 12 → 7

````text
/// [`fatal_line`] plus a `       remedy: <r>` continuation when the event carries
/// one, at `fail_row`'s own indent.
///
/// Sabrage-only: `lib.sh`'s `die` has no remedy slot, so this adds text where the
/// shell prints none — PARITY.md § CLI / GUI, "A `FATAL` row may be followed by".
/// Two `Fatal`s come from [`sabrage_core::privilege`] rather than a `die`-shaped
/// call site and carry their only actionable instruction in `remedy`.
````

### B0025 · l.514–525 · REWRITE (confirm) · rule 1.6 · 12 → 9

````text
/// doctor.sh's footer line: `doctor: all checks passed — ./demo.sh run --bottle
/// <label>` when `fail_count` is 0, else `doctor: <n> check(s) failed — remedies
/// above`.
///
/// The caller prints the preceding blank line itself. `fail_count` is the
/// *uncapped* tally; the 255 cap in
/// [`sabrage_core::checks::DoctorReport::exit_code`] is an exit-code concern, not
/// a display one. Pinned by
/// tests::footer_matches_doctor_sh_verbatim_both_branches.
````

### B0026 · l.540–547 · REWRITE (confirm) · rule 1.6 · 8 → 8

````text
/// The blank-line-prefixed "next:" banner each stage script ends with on
/// success, verbatim from `scripts/demo/{setup,build,install}.sh`'s closing
/// `print` line; `None` for `stop` and `run`, which print none.
///
/// `bottle_label` is used only by `install`'s line, by which point
/// `require_bottle` has guaranteed a real name; `build`'s line quotes the literal
/// `<name>` placeholder because build.sh's own text does. Pinned by
/// tests::closing_line_matches_each_stage_script_verbatim.
````

### B0027 · l.569–580 · REWRITE (confirm) · rule 1.7 · 12 → 9

````text
/// Per-stream color eligibility: `isatty(<stream>) && !NO_COLOR`.
///
/// Each stream gets its own `is_terminal()` read (A14-5), because a stage's
/// `Fatal` row goes to stderr while every other row goes to stdout and the two
/// are redirected independently. The gate itself is the deliberate divergence
/// from `lib.sh`, which emits its ANSI codes unconditionally — PARITY.md
/// § Doctor / checks, "Console colors gated on isatty". Pinned by
/// tests::{colors_from_gates_each_stream_on_its_own_tty_unless_no_color,
/// fatal_uses_stderr_colors_while_a_line_event_uses_stdout_colors}.
````

### B0028 · l.585–588 · REWRITE (confirm) · rule 1.5 · 4 → 5

````text
    /// Raw `isatty()`, independent of `NO_COLOR`: A14-3 decides from this whether
    /// a `\r`-terminated [`StageEvent::Output`] chunk repaints the terminal line
    /// or falls back to newline-per-chunk (a non-tty consumer — a log file, a
    /// `--tap` pipe — has no "current line" to repaint). Pinned by
    /// tests::cr_chunk_repaints_only_on_a_real_terminal.
````

### B0035 · l.684–695 · REWRITE (confirm) · rule 1.2 · 12 → 7

````text
/// One line [`stage_event_lines`] wants printed, tagged with the stream it
/// belongs on.
///
/// A plain value type rather than a `println!` side effect, so "this event prints
/// nothing" — [`StageEvent::Check`], [`StageEvent::Launched`],
/// [`StageEvent::AutoFixed`], [`StageEvent::Progress`] — is directly assertable
/// (tests::structured_only_events_render_no_console_line).
````

### B0038 · l.740–744 · REWRITE (confirm) ·p · rule 1.3 · 5 → 3

````text
                // A `\r` repaint only skips the newline on a real terminal — a
                // non-tty consumer has no current line to overwrite, so it keeps
                // one newline per chunk (tests::cr_chunk_repaints_only_on_a_real_terminal).
````

### B0040 · l.761–764 · REWRITE (confirm) · rule 1.3 · 4 → 3

````text
        // Every fix already emits its own shell-verbatim `ok`/`warn` row onto
        // this sink; `AutoFixed` is a structured signal for the GUI, so printing
        // it here would double the console row.
````

### B0045 · l.817–827 · REWRITE (confirm) · rule 1.7 · 11 → 9

````text
/// Merge parsed CLI flags onto the env-derived base — `WINEVR_*` is the base,
/// flags override, exactly [`StageOptions::from_env`]'s own precedence rule
/// applied to all six flags in one place. `--dry-run` has no env counterpart, so
/// `parsed.dry_run` wins outright rather than only-if-true.
///
/// Separate from [`cmd_stage`]/[`cmd_all`] so the forwarding is testable without
/// a repo root or a [`StageCtx`]: pinned by
/// tests::{merge_stage_options_forwards_all_six_flags_onto_the_env_base,
/// merge_stage_options_env_base_survives_when_no_flag_overrides_it}.
````

### B0046 · l.831–839 · REWRITE (confirm) · rule 1.5 · 9 → 3

````text
        // A14-1: an explicit `--bottle ""` must clear the env-derived value, never
        // survive as `Some("")` — `demo.sh` treats an empty export as unset. Pinned
        // by tests::merge_stage_options_empty_cli_values_clear_a_preset_env_base.
````

### B0048 · l.863–877 · REWRITE (confirm) · rule 1.7 · 15 → 10

````text
/// Run one stage and return the process exit code (never calls `exit` itself, so
/// `main` stays the only place that does).
///
/// All five stage bodies really touch the machine — [`Stage::Run`] launches wine
/// and the game — so this function is driven end-to-end only up to the point it
/// resolves a repo root and builds a [`StageCtx`]: an argument error, a
/// `resolve_repo_root` failure, or a `Paths::new_checked` failure returns before
/// any of that (tests::run_stage_rejects_a_bad_argument_before_touching_anything).
/// Everything past that boundary is covered through the pure helpers
/// ([`merge_stage_options`], [`stage_event_lines`], [`report_stage_result`]).
````

### B0049 · l.895–901 · REWRITE (confirm) · rule 1.5 · 7 → 3

````text
    // A2-8: every stage writes under `~/Library/Application Support`, so an unset,
    // empty, or relative `HOME` fails closed here rather than silently redirecting
    // those writes (sabrage_core::paths::tests::home_is_required_to_be_absolute_and_non_empty).
````

### B0050 · l.932–940 · REWRITE (confirm) · rule 1.7 · 9 → 4

````text
    // The dry-run plan prints trailing, after every narrative row a real run would
    // also print, and is keyed on the executor (`is_dry_run()`) rather than the raw
    // `--dry-run` flag, so a non-dry run's output is untouched byte-for-byte
    // (finding #13; PARITY.md § CLI / GUI, "A dry run ends with a trailing").
````

### B0052 · l.959–968 · REWRITE (confirm) · rule 1.3 · 10 → 3

````text
            // Suppressed for every error whose condition already emitted a
            // `Fatal` (see [`error_already_reported_as_fatal`]); every remaining
            // variant has no shell equivalent and would otherwise vanish.
````

### B0053 · l.980–1002 · REWRITE (amend) · rule 1.2 · 23 → 8

````text
/// True when this error's condition already emitted a [`StageEvent::Fatal`], so
/// printing `error: {e}` after it would double-report one failure.
///
/// A CLI-flavored name over [`SabrageError::already_reported`], which carries the
/// rule once for both front-ends (the GUI needs the identical predicate to decide
/// whether a failure banner would double the `Fatal` already in its run log) —
/// see sabrage_core::error::tests::already_reported_covers_the_variants_that_emit_their_own_row
/// and PARITY.md § CLI / GUI, "`error: <e>` is suppressed for every error".
````

### B0054 · l.1007–1017 · REWRITE (confirm) · rule 1.2 · 11 → 9

````text
/// The trailing "-- plan (dry run)" section's lines: the section header in
/// [`StageEvent::Section`]'s own `"-- <title>"` shape, then one `info`-indented
/// line per recorded [`PlannedAction`].
///
/// Title and body come from [`sabrage_core::DRY_RUN_PLAN_TITLE`] /
/// [`sabrage_core::dry_run_plan_body`], never from literals here, so the CLI and
/// the GUI cannot word the same plan differently — PARITY.md § CLI / GUI, "A dry
/// run ends with a trailing". Pinned by
/// tests::dry_run_plan_has_a_header_then_one_line_per_action_in_order.
````

### B0055 · l.1028–1043 · REWRITE (amend) · rule 1.6 · 16 → 6

````text
// `sabrage all` is `demo.sh`'s `all)` loop, native-side: one `require_bottle` up
// front so it fails before the expensive fetch/build stages, then
// `Stage::ALL_CHAIN` in order (`Stage` deliberately has no `All` member — see its
// own doc). Divergences, including the dropped `##### demo.sh: <stage> #####`
// separator each stage's own `StageStarted` banner already covers:
// PARITY.md § Run (launch), "`sabrage all` chains".
````

### B0056 · l.1045–1054 · REWRITE (confirm) · rule 1.7 · 10 → 8

````text
/// Run `stages` in order via `run_one`, stopping at the first stage whose exit
/// code is non-zero and returning that code, else `0` — `demo.sh`'s own
/// `|| exit $?`.
///
/// `run_one` is a parameter so a test can substitute a fake and assert the
/// stop-at-first-failure rule without invoking a real stage body
/// (tests::{run_chain_stops_at_the_first_nonzero_exit_code,
/// run_chain_runs_every_stage_when_all_of_them_succeed}).
````

### B0057 · l.1069–1084 · REWRITE (confirm) · rule 1.7 · 16 → 12

````text
/// The core of `all`: requires a bottle up front via [`require_bottle`] (lib.sh's
/// own die text), then chains [`Stage::ALL_CHAIN`] through [`run_stage`] via
/// [`run_chain`].
///
/// Each stage gets a **fresh** [`StageCtx`] — fresh `run_id`, fresh executor —
/// over the same `paths`/`opts`/`sink`, plus one cancellation token shared across
/// every stage so a single Ctrl-C reaches whichever stage is running. `--dry-run`
/// applies uniformly because every per-stage context inherits `opts.dry_run` and
/// prints its own trailing plan. Taking `paths`/`opts` as arguments — never
/// [`StageOptions::from_env`] — is what keeps
/// tests::run_all_requires_a_bottle_before_touching_run_stage independent of the
/// real machine's `WINEVR_BOTTLE`.
````

### B0058 · l.1086–1088 · REWRITE (amend) · rule 1.3 · 3 → 3

````text
    // Doubles as the source of the `CancellationToken` every per-stage `StageCtx`
    // below shares — reached through `StageCtx`'s own field, so this file never
    // names `tokio_util`'s type.
````

### B0061 · l.1142–1144 · REWRITE (confirm) ·p · rule 1.3 · 3 → 3

````text
    // `all` chains setup/build/install/run, all of which write into the user
    // store, so it fails closed on an unusable `HOME` the same way `cmd_stage`
    // does (sabrage_core::paths::tests::home_is_required_to_be_absolute_and_non_empty).
````

### B0062 · l.1167–1245 · REWRITE (amend) · rule 1.5 · 79 → 24

````text
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
````

### B0065 · l.1254–1256 · REWRITE (confirm) · rule 1.6 · 3 → 2

````text
/// `SIGTERM`'s signal number — 15 on every platform this ships to (macOS).
/// `run.sh` traps it with the same teardown as its `INT` trap.
````

### B0066 · l.1259–1268 · REWRITE (confirm) · rule 1.7 · 10 → 7

````text
/// How many `SIGINT`/`SIGTERM` deliveries the handlers have seen, combined.
///
/// A **count**, not a flag: two deliveries inside one [`CTRL_C_POLL_INTERVAL`]
/// must still read as two, or the second is lost and the process stays merely
/// "cancelling" (tests::two_signals_inside_one_poll_interval_are_not_collapsed_into_one).
/// `fetch_add` on an `AtomicUsize` is as async-signal-safe as a plain store. Both
/// signals share it on purpose: one state machine here, not two.
````

### B0072 · l.1321–1330 · REWRITE (confirm) · rule 1.2 · 10 → 9

````text
/// Restore both `SIGINT`'s and `SIGTERM`'s default disposition and re-raise
/// whichever one was actually received ([`LAST_SIGNAL`]), so the process dies
/// under that signal's own name — exit 143 for a second `kill <pid>`, 130 for a
/// second Ctrl-C — rather than this file picking a number.
///
/// The production second-signal action both traps share; tests drive
/// [`watch_flag_with_second_action`] with a fake instead, since a real second
/// signal would kill the test binary
/// (tests::a_second_signal_of_a_different_kind_is_still_fatal_and_reraises_that_kind).
````

### B0074 · l.1351–1363 · REWRITE (confirm) · rule 1.7 · 13 → 10

````text
/// `watch_flag`'s full generality: polls `counter` for as long as the process
/// runs, calling `on_first_signal` once on the first delivery it observes and
/// `on_second_signal` on any delivery after that, then returning.
///
/// It reads a monotone **count** and remembers how much of it it has consumed
/// rather than swapping a flag back to `false`: a burst delivered inside one
/// [`CTRL_C_POLL_INTERVAL`] is one observation but two deliveries, and a flag
/// cannot tell those apart. Pinned by
/// tests::{watcher_keeps_polling_past_the_first_fire_so_a_second_signal_is_reachable,
/// two_signals_inside_one_poll_interval_are_not_collapsed_into_one}.
````

### B0078 · l.1475–1479 · REWRITE (confirm) ·p · rule 1.7 · 5 → 5

````text
    /// A14-4: demo.sh's `CMD="${1:-}"; shift` consumes the first token, so
    /// the flag loop only sees the tail — `--bottle Steam run` reports `Steam`
    /// and never reaches `--bottle`'s "needs a name" branch. `Ok(())` means
    /// the shell fell through to its unknown-`CMD` `case` arm, whose text the
    /// caller's `_` arm prints, not this helper.
````

### B0082 · l.1670–1672 · REWRITE (amend) · rule 1.3 · 3 → 3

````text
        // `--tap` is a doctor-only sabrage addition (demo.sh has no such flag),
        // so every stage command must reject it exactly like any other
        // unrecognized flag.
````

### B0084 · l.1696–1699 · REWRITE (amend) · rule 1.7 · 4 → 3

````text
        // Matching demo.sh's flag grammar is not enough: each flag that has a
        // `StageOptions` field must actually reach it rather than being parsed
        // and dropped (`--quiet` is CLI-rendering only and has no field).
````

### B0088 · l.1746–1749 · REWRITE (confirm) ·p · rule 1.7 · 4 → 4

````text
        // A14-1: `--bottle ""`/`--bs-dir ""` (from a wrapper interpolating
        // the flag even when its variable is unset) must behave like
        // `${WINEVR_BOTTLE:-}`/`${WINEVR_BS_DIR:-<default>}`: empty = absent,
        // not an override to the empty string.
````

### B0094 · l.1870–1874 · REWRITE (confirm) · rule 1.4 · 5 → 3

````text
    // `stage_event_lines` is a pure projection, so "renders to nothing" is a
    // value these tests assert on directly rather than an intercepted
    // `println!`.
````

### B0099 · l.2117–2119 · REWRITE (amend) · rule 1.7 · 3 → 3

````text
        // Finding #4: `Fatal.remedy` must reach a CLI user — it carries the
        // App Management deep link that finding #6's
        // `privilege::upgrade_write_error` puts on the `Fatal` it emits.
````

### B0101 · l.2166 · REWRITE (amend) · rule 1.4 · 1 → 1

````text
    // A14-5: color is gated per stream, not once for the process.
````

### B0102 · l.2213–2217 · REWRITE (confirm) · rule 1.7 · 5 → 3

````text
        // A14-5: `Fatal` gates its color on stderr's terminal-ness and an
        // ordinary `Line` on stdout's, so with stdout piped and stderr a
        // terminal the `Fatal` row is colored and the `Line` row is not.
````

### B0105 · l.2312–2323 · REWRITE (amend) · rule 1.4 · 12 → 3

````text
    // No test below may reach a stage body: setup/build/install/run touch the
    // real machine. Argument errors return before `resolve_repo_root`, and
    // `run_all`'s bottle precheck (hand-built `opts`) dies before `run_chain`.
````

### B0108 · l.2438–2444 · REWRITE (amend) ·p · rule 1.7 · 7 → 4

````text
        // Finding #7: the watcher keeps polling past its first callback, so a
        // second Ctrl-C is observed. Uses `watch_flag_with_second_action`
        // because the real path (`terminate_via_default_disposition_of_last_signal`)
        // `raise()`s on this process and would kill the test binary.
````

### B0110 · l.2483–2488 · REWRITE (confirm) ·p · rule 1.7 · 6 → 3

````text
        // Finding #6: a burst inside one poll interval (a fast double-tap, or
        // `kill -INT` pair) counts as two signals and is fatal. Both deliveries
        // land before the watcher's first poll — the collapsing window.
````

### B0111 · l.2518–2527 · REWRITE (confirm) ·p · rule 1.3 · 10 → 5

````text
        // Production shares one `SIGNAL_COUNT` between `SIGINT` and `SIGTERM`
        // handlers, with `LAST_SIGNAL` holding the most recent delivery: first
        // of either kind cancels, second of either kind is fatal and re-raises
        // whichever just arrived. Process-local fakes stand in because a real
        // second signal would kill the test binary.
````

## `sabrage/crates/sabrage-contract-gen/src/lib.rs`

Deleted (nothing carried): B0120, B0121, B0125, B0144

### B0114 · l.1–28 · REWRITE (amend) · rule 1.2 · 28 → 12

````text
//! Generator for `scripts/demo/contract.gen.sh`, the zsh mirror of
//! `contract/pipeline.toml` that lib.sh sources so the shell needs no TOML parser.
//!
//! Two tripwires cover it: the `# contract-sha256:` header lets doctor's
//! `meta.contract-sync` catch a contract edited without regenerating, and
//! [`check`] catches a hand-edited generated file. The committed bytes are
//! pinned by sabrage-parity's
//! `tests::contract_gen_parity::generate_matches_the_committed_contract_gen_sh`.
//!
//! The contract subset below is re-declared rather than imported from
//! `sabrage-core`: a field added to `pipeline.toml` but not here is visibly
//! absent from the shell instead of silently coupled through a shared struct.
````

### B0116 · l.39–41 · REWRITE (confirm) ·p · rule 1.6 · 3 → 3

````text
/// Repo-relative paths of the three contract inputs, in the order their bytes
/// are concatenated for the `contract-sha256` header.
/// Reference: scripts/demo/doctor.sh section 0, same order and recipe.
````

### B0128 · l.230–241 · REWRITE (amend) · rule 1.4 · 12 → 5

````text
// A value here is shell code unless quoted, and sabrage-core reads the same TOML
// literally — an unquoted `$(...)`/backtick/`$VAR`/space/glob silently diverges the
// two front-ends (tests::hostile_contract_values_are_emitted_as_zsh_literals).
// Encoding stays minimal so the committed contract.gen.sh is byte-identical
// (tests::zsh_encoders_are_minimal_for_ordinary_contract_values).
````

### B0143 · l.383–389 · REWRITE (confirm) · rule 1.7 · 7 → 5

````text
    /// Every contract scalar the zsh side consumes must be *emitted*, not
    /// hard-coded in lib.sh/setup.sh: mutate it in `pipeline.toml` and a body
    /// line has to change. A field that only moves the header hash never
    /// reaches the shell, yet `--regen`, `--check` and `meta.contract-sync`
    /// all report "in sync" while the two front-ends use different values.
````

## `sabrage/crates/sabrage-contract-gen/src/main.rs`

### B0154 · l.1–9 · REWRITE (confirm) · rule 1.4 · 9 → 6

````text
//! Thin `--write` / `--check` shim over [`sabrage_contract_gen`].
//!
//! `--check` regenerates in memory and exits 1 when the committed
//! `scripts/demo/contract.gen.sh` differs; `--write` regenerates it in place.
//! `scripts/dev/parity.sh --regen` drives `--write`; the tier-1 parity test
//! drives the library directly.
````

## `sabrage/crates/sabrage-core/src/checks/audio.rs`

### B0157 · l.18–26 · REWRITE (confirm) ·p · rule 1.6 · 9 → 3

````text
/// Warns when `SwitchAudioSource` is absent or `BlackHole 2ch` is not an
/// exact line in `-a -t output` (`grep -qx`—a longer device name won't match).
/// Reference: `scripts/demo/doctor.sh` section 15.
````

## `sabrage/crates/sabrage-core/src/checks/bottle.rs`

Deleted (nothing carried): B0164

### B0160 · l.1–24 · REWRITE (amend) ·p · rule 1.6 · 24 → 9

````text
//! Group `bottle` — doctor.sh section 3: resolves the bottle context every
//! later section consumes. Slug order pinned by
//! checks::tests::registry_binds_in_contract_order_and_covers_every_slug.
//!
//! Every evaluator is a read-only `fn(&CheckCtx) -> CheckOutcome`.
//! `bottle.template`/`bottle.gfx-dxmt`/`bottle.zdrive` report
//! [`CheckStatus::Skipped`] whenever `ctx.bottle` is `None` — see
//! [`skip_reason_for_missing_bottle`] for the reason text and
//! tests::bottle_named_fail_without_a_name_skips_the_rest_of_the_section.
````

## `sabrage/crates/sabrage-core/src/checks/bridge.rs`

Deleted (nothing carried): B0178, B0180

### B0173 · l.1–21 · REWRITE (amend) ·p · rule 1.6 · 21 → 9

````text
//! Group `bottle-bridge` (doctor.sh section 11): the per-bottle half of the
//! bridge install. Binds `bottle.woxr-dll`, `bottle.manifest` and
//! `bottle.registry` in contract order; every evaluator is a read-only probe.
//! With no bottle all three return `skipped` carrying doctor.sh's verbatim
//! info line as the [`SkipReason`]. Message and remedy prose must match
//! `scripts/demo/doctor.sh` verbatim.
//!
//! See tests::{ordered_substring_scan_matches_grep_semantics,
//! no_bottle_skips_all_three_with_the_verbatim_reason}.
````

### B0175 · l.30–37 · REWRITE (confirm) · rule 1.2 · 8 → 7

````text
/// Whether `text` carries `ActiveRuntime`, `openxr` and `wineopenxr64.json`
/// in that order on one line: the semantics of doctor.sh's
/// `grep -q 'ActiveRuntime.*openxr.*wineopenxr64.json' "$PREFIX/system.reg"`.
///
/// A left-to-right chained `find` decides that existence question exactly:
/// `.*` is greedy but existence-only, and `.` never spans a newline.
/// See tests::ordered_substring_scan_matches_grep_semantics.
````

### B0177 · l.124–126 · REWRITE (confirm) · rule 1.3 · 3 → 2

````text
        // A realistic wine system.reg line: the middle `openxr` comes from
        // the `C:\openxr\` path segment, not from a key of its own.
````

## `sabrage/crates/sabrage-core/src/checks/build.rs`

Deleted (nothing carried): B0186, B0188, B0195, B0197

### B0185 · l.1–19 · REWRITE (amend) · rule 1.5 · 19 → 12

````text
//! Group `build` — doctor.sh section 9, 9b: build outputs, including the native-arm64 encoder helper.
//!
//! Slugs owned here, in contract order: `build.oxr-dylib`, `build.alvr-core`,
//! `build.runtime-json`, `build.woxr-dll`, `build.woxr-so`, `build.dashboard`,
//! `build.helper-staged`, `build.helper-arm64`.
//!
//! Every evaluator is `fn(&CheckCtx) -> CheckOutcome`: a read-only probe whose
//! message and remedy strings must match `scripts/demo/doctor.sh` verbatim.
//!
//! `build.helper-arm64` must not accept `arm64e` alone: a wrong-arch binary
//! staged next to the runtime dylib shadows the good one and silently drops the
//! session to in-process H.264 (tests::helper_is_arm64_rejects_arm64e_only_binaries).
````

### B0187 · l.28–30 · REWRITE (confirm) · rule 1.2 · 3 → 3

````text
/// Shared shape of the six `build.*` output-presence checks: passes with
/// `built: <relpath>`, fails with `missing build output: <relpath>` and the
/// `./demo.sh build` remedy, where `<relpath>` is `Paths::rel_display`.
````

### B0189 · l.70–73 · REWRITE (amend) · rule 1.2 · 4 → 3

````text
/// True when `p` is a regular file with any execute bit set — `[ -x "$1" ]`
/// to the same approximation the `paths` module's `which()` uses (no
/// euid/egid resolution, which `lib.sh` never relied on either).
````

### B0190 · l.81–85 · REWRITE (confirm) ·p · rule 1.2 · 5 → 3

````text
/// `lipo -archs <path>` stdout with trailing newlines stripped as `$(...)` does;
/// empty when `lipo` cannot run or writes nothing. Exit status is ignored; the
/// FAIL message of `build.helper-arm64` embeds this value.
````

### B0191 · l.99–110 · REWRITE (confirm) ·p · rule 1.6 · 12 → 7

````text
/// True when `path` is executable and `lipo -archs` lists `arm64` as a whole
/// word. Single home of lib.sh's `helper_is_arm64()`; `crate::util` re-exports
/// it for the fix and stage layers.
///
/// A fat `x86_64 arm64e` binary must NOT match, while `x86_64 arm64` and thin
/// `arm64` must (tests::helper_is_arm64_rejects_arm64e_only_binaries,
/// tests::helper_is_arm64_is_true_for_the_thin_arm64_test_binary_itself).
````

### B0194 · l.277–281 · REWRITE (amend) · rule 1.3 · 5 → 3

````text
        // The compiled test binary is itself a thin-arm64 Mach-O on this
        // repo's target machine: a real positive case with no compiler.
        // Skipped where that cannot hold (Intel Mac, Linux CI, no usable lipo).
````

## `sabrage/crates/sabrage-core/src/checks/config.rs`

Deleted (nothing carried): B0203, B0214, B0215, B0228, B0231, B0236

### B0202 · l.1–47 · REWRITE (amend) · rule 1.6 · 47 → 17

````text
//! Group `config` — the oxrsys runtime config and ALVR session state.
//!
//! Binds `cfg.protocol.supported`, `cfg.protocol.legacy-oxrsys` and
//! `cfg.session-pins`, in contract order, to read-only
//! `fn(&CheckCtx) -> CheckOutcome` probes whose message and remedy strings
//! match the shell verbatim except where noted below.
//! Reference: scripts/demo/doctor.sh sections 13 and 13b.
//!
//! Multi-pin WARN entries keep session.json file order because this crate
//! enables `serde_json/preserve_order`; see PARITY.md § Doctor / checks,
//! "`host.manifest` / `cfg.session-pins` parse JSON natively (serde)".
//!
//! TODO(A3b-3): an unreadable or malformed `session.json` Warns here where
//! doctor.sh's `try/except: sys.exit(0)` reports the clean Pass; the
//! divergence still owes a `scripts/demo/doctor.sh` change or a
//! `sabrage/PARITY.md` row. Pinned by tests::malformed_json_warns and
//! tests::unreadable_session_json_warns.
````

### B0205 · l.59 · REWRITE (amend) · rule 1.6 · 1 → 1

````text
    /// No regular file at the configured `oxrsys-runtime.toml` path.
````

### B0207 · l.67–81 · REWRITE (amend) ·p · rule 1.2 · 15 → 7

````text
/// The `protocol` value doctor.sh's last-match `awk` recipe would resolve,
/// or the empty string when no line assigns a quoted one.
///
/// Table-blind and last-assignment-wins, matching the runtime's line-oriented
/// reader and the `awk` form doctor.sh and run.sh share; `#`-commented lines
/// and keys like `protocol_foo` never match; an unquoted assignment resolves
/// to the empty string. Pinned by tests::parse_protocol_matches_the_awk_recipe.
````

### B0208 · l.99–102 · REWRITE (confirm) · rule 1.6 · 4 → 5

````text
/// The [`ProtocolState`] of the configured `oxrsys-runtime.toml`.
///
/// A read error after the existence check (a permission race, say)
/// degrades to an empty `protocol`, like the shell's unredirected `awk`
/// failing silently into an empty capture.
````

### B0211 · l.128–136 · REWRITE (confirm) · rule 1.6 · 9 → 4

````text
/// `cfg.protocol.supported`: Pass for `protocol = "alvr"`, silent Pass for
/// `oxrsys` (the shell prints that row from `cfg.protocol.legacy-oxrsys`
/// instead), Fail when the toml is missing or names any other value.
/// Reference: scripts/demo/doctor.sh section 13.
````

### B0212 · l.148–150 · REWRITE (confirm) ·p · rule 1.3 · 3 → 1

````text
        // The shell prints this row from cfg.protocol.legacy-oxrsys instead (PARITY.md § Doctor / checks).
````

### B0213 · l.163–169 · REWRITE (confirm) · rule 1.6 · 7 → 4

````text
/// `cfg.protocol.legacy-oxrsys`: Fail on `protocol = "oxrsys"` (the legacy
/// USB/adb-reverse path), silent Pass on `alvr`, Skipped when the toml is
/// missing or names any other value.
/// Reference: scripts/demo/doctor.sh section 13.
````

### B0217 · l.197–203 · REWRITE (confirm) · rule 1.5 · 7 → 4

````text
    /// `fs::read_to_string` failed (missing between the `is_file()` gate and
    /// the read, permissions, …). Warned rather than collapsed into `Clean`
    /// the way doctor.sh's `try/except: sys.exit(0)` would (A3b-3): "could
    /// not tell" is the degraded state this check exists to surface.
````

### B0218 · l.205–207 · REWRITE (confirm) · rule 1.5 · 3 → 3

````text
    /// `serde_json::from_str` failed (malformed JSON). Warned for the same
    /// reason, and with the same deliberate doctor.sh divergence, as
    /// `Unreadable` (A3b-3).
````

### B0219 · l.209–215 · REWRITE (amend) · rule 1.2 · 7 → 5

````text
    /// The JSON parsed, but its shape breaks the walk over
    /// `client_connections` (the top level, `client_connections` itself, an
    /// entry under it, or a non-empty `manual_ips` with the wrong type).
    /// doctor.sh's Python raises outside its `try/except` here too, so its
    /// "broken python3?" WARN is mirrored.
````

### B0221 · l.219–222 · REWRITE (confirm) · rule 1.5 · 4 → 4

````text
    /// Space-joined `"name=ip,ip "` entries carrying doctor.sh's trailing
    /// space, so concatenating directly before `"— fine while …"` reproduces
    /// the single space the shell gets. Pinned by
    /// tests::one_pinned_client_warns_with_the_trailing_space_quirk.
````

### B0223 · l.239–246 · REWRITE (confirm) · rule 1.6 · 8 → 4

````text
/// The [`SessionPinState`] of an ALVR `session.json`, mirroring doctor.sh's
/// inline python inspector: every `client_connections` entry with a
/// non-empty `manual_ips` contributes one `name=ip,ip` entry.
/// Reference: scripts/demo/doctor.sh section 13b.
````

### B0224 · l.301–302 · REWRITE (confirm) · rule 1.2 · 2 → 4

````text
/// `cfg.session-pins`: Skipped when `session.json` is absent, Pass when no
/// client carries a manual IP pin, Warn otherwise — including the A3b-3
/// read/parse failures doctor.sh reports as clean.
/// Reference: scripts/demo/doctor.sh section 13b.
````

### B0225 · l.316–321 · REWRITE (confirm) · rule 1.5 · 6 → 3

````text
        // A3b-3: a read or parse failure is a degraded state, not a clean one,
        // and no python3 is in this process to blame for it.
        // Pinned by tests::malformed_json_warns, tests::unreadable_session_json_warns.
````

### B0226 · l.335–339 · REWRITE (confirm) · rule 1.3 · 5 → 2

````text
        // `Corrupt` mirrors doctor.sh: these shape violations raise outside the
        // shell probe's try/except, so "broken python3?" is accurate here.
````

### B0233 · l.468–474 · REWRITE (confirm) · rule 1.7 · 7 → 5

````text
    /// The `cfg.protocol.supported` / `cfg.protocol.legacy-oxrsys` pair for the
    /// `oxrsys-runtime.toml` bodies that differ only in the protocol value
    /// doctor.sh's last-match `awk` resolves to. `{toml}` in an expected remedy
    /// is the row's own scratch toml path. The absent file and the A3b-1
    /// shadowing regression keep their own functions.
````

### B0243 · l.704–708 · REWRITE (confirm) · rule 1.7 · 5 → 3

````text
    /// `cfg.session-pins` for the session.json bodies whose shape alone decides
    /// the verdict. `{session}` in an expected message is the row's own scratch
    /// session.json path.
````

## `sabrage/crates/sabrage-core/src/checks/game.rs`

### B0246 · l.1–12 · REWRITE (amend) ·p · rule 1.6 · 12 → 8

````text
//! Group `game` — doctor.sh section 8: the Beat Saber 1.29.4 install.
//!
//! Binds `game.present` and `game.version` in contract order; every evaluator
//! is `fn(&CheckCtx) -> CheckOutcome`, a read-only probe.
//!
//! Printed strings are reproduced verbatim (`docs/troubleshooting.md` quotes
//! them and nothing tests it). Tap-only strings are impl-owned prose, marked
//! at their site.
````

### B0247 · l.21–23 · REWRITE (confirm) · rule 1.6 · 3 → 2

````text
/// True when doctor skips the whole Beat Saber section: neither a resolved
/// bottle nor an explicit `--bs-dir` override.
````

### B0248 · l.28–33 · REWRITE (amend) · rule 1.5 · 6 → 3

````text
/// doctor's skip line, verbatim (doctor.sh section 8). doctor taps both
/// `game.*` slugs skipped with no per-slug text; sabrage carries this text as
/// the [`SkipReason`] on both (tests::no_bottle_no_override_skips_both_slugs_with_the_verbatim_reason).
````

### B0249 · l.45–48 · REWRITE (confirm) · rule 1.3 · 4 → 3

````text
        // doctor.sh taps `game.present ok` and prints no row for the found
        // case; the tap channel carries slug+status only, so this message is
        // impl-owned prose, not a verbatim doctor.sh string.
````

## `sabrage/crates/sabrage-core/src/checks/headset.rs`

Deleted (nothing carried): B0259, B0260, B0262

### B0252 · l.1–22 · REWRITE (confirm) ·p · rule 1.6 · 22 → 7

````text
//! Group `headset` — scripts/demo/doctor.sh section 14: `hs.adb` and
//! `hs.client`, both warning-only (WiFi streaming needs no USB). Every
//! evaluator is a read-only probe over `adb`.
//!
//! `CheckOptions::allow_adb_probes = false` is a Sabrage-only state with no zsh
//! counterpart: both slugs report `Skipped` instead of probing, which is outside
//! the doctor parity contract (tests::probes_disabled_skips_both_slugs).
````

### B0254 · l.41–44 · REWRITE (confirm) · rule 1.6 · 4 → 3

````text
/// Serial of the first `adb devices` row after the header whose state field is
/// exactly `device` (not `offline`, `unauthorized`, …); `None` when none
/// qualifies.
````

### B0255 · l.63–68 · REWRITE (amend) ·p · rule 1.6 · 6 → 3

````text
/// `hs.adb`: Pass with the Quest's serial, Warn when adb is missing or no
/// device is connected (tests::no_adb_binary_warns_hs_adb_and_skips_hs_client),
/// Skipped when probing is disabled. Reference: scripts/demo/doctor.sh, section 14.
````

### B0256 · l.82–84 · REWRITE (confirm) · rule 1.6 · 3 → 3

````text
/// True when the package list on `serial` mentions `alvr` anywhere in stdout
/// (case-sensitive substring, matching the shell's bare `grep`); false when the
/// command fails to run.
````

### B0257 · l.93–94 · REWRITE (confirm) · rule 1.2 · 2 → 3

````text
/// `hs.client`: Pass when the ALVR client is installed on the connected Quest,
/// Warn when it is not, Skipped when no Quest is connected or probing is
/// disabled.
````

### B0261 · l.177–179 · REWRITE (confirm) · rule 1.3 · 3 → 3

````text
    // Paths::new probes the real machine, so this asserts the no-adb shape only
    // when the field genuinely came back None — the invariant, not a fixed
    // machine state.
````

## `sabrage/crates/sabrage-core/src/checks/host.rs`

### B0264 · l.22–34 · REWRITE (amend) ·p · rule 1.6 · 13 → 4

````text
/// `host.manifest`: the root-owned host OpenXR manifest exists and its
/// `runtime.library_path` routes to the expected `oxr_dylib`.
///
/// Reference: scripts/demo/doctor.sh `# 12. host loader registration`
````

### B0265 · l.85–100 · REWRITE (amend) ·p · rule 1.7 · 16 → 11

````text
/// Parses `runtime.library_path` out of the host OpenXR manifest at `path`.
///
/// Returns `None` for every way the shell's `PYRC != 0` branch is reachable:
/// unreadable file, malformed JSON, missing or non-object `"runtime"`, or
/// missing `"library_path"`. A non-string `library_path` also yields `None`
/// (real Python would stringify it); `contract/active_runtime.x86_64.json.template`
/// never writes one, and both routes end in FAIL.
///
/// `pub` because `src-tauri/src/commands.rs`'s `get_repo_info` reuses this
/// parse for its `hostManifestLibraryPath`/`hostManifestPointsHere` fields
/// rather than poking the JSON a second time.
````

## `sabrage/crates/sabrage-core/src/checks/meta.rs`

### B0268 · l.1–17 · REWRITE (confirm) ·p · rule 1.6 · 17 → 14

````text
//! Group `meta` — doctor.sh section 0: the generated shell contract mirror is in sync with contract/.
//!
//! Slugs owned here, in contract order:
//!
//! * `meta.contract-sync` — compares the sha256 recomputed from `contract/` on disk against
//!   the `# contract-sha256:` header of `scripts/demo/contract.gen.sh`
//!   (`util::contract_hash` / `util::contract_gen_recorded_hash`), and (Sabrage-only)
//!   against the contract this binary was compiled from.
//!
//! Every evaluator is `fn(&CheckCtx) -> CheckOutcome`: a read-only probe.
//! Message and remedy strings of the on-disk half must match `scripts/demo/doctor.sh`
//! verbatim; the compiled-vs-checkout half has no shell counterpart and is declared in
//! PARITY.md § Declared by the 2026-08-30 adversarial review (round 1 fixes),
//! "**Contract identity.**".
````

### B0269 · l.25–62 · REWRITE (amend) ·p · rule 1.6 · 38 → 13

````text
/// Doctor row `meta.contract-sync`: Pass only when the sha256 recomputed from
/// `contract/` under `ctx.paths.root` equals both `contract.gen.sh`'s
/// `# contract-sha256:` header and this binary's compiled-in contract hash.
///
/// A missing or empty header is a Fail — doctor.sh's `[ -n "$_have" ]` guard;
/// only the missing case is pinned
/// (tests::fails_closed_when_the_repo_root_is_wrong). Neither side reads the
/// body of `contract.gen.sh`, so a hand-edited body under a current header
/// passes this row (A1-4); tier-1's `sabrage-contract-gen::generate() ==
/// include_str!` test catches that drift. The compiled-vs-checkout half is
/// Sabrage-only, finding A1-1
/// (tests::fails_when_the_binary_was_compiled_from_a_different_contract).
/// Reference: scripts/demo/doctor.sh section 0.
````

### B0274 · l.122–140 · REWRITE (confirm) ·p · rule 1.7 · 19 → 14

````text
/// Sabrage-only compiled-vs-checkout identity guard, usable outside Doctor so
/// callers can refuse to act on a mismatch (round-1 finding A1-1).
///
/// Returns `Ok(())` only when `root`'s `contract/` hashes to exactly the
/// contract this binary was compiled from. Returns `(message, remedy)` rather
/// than `CheckOutcome` so callers outside `checks::` need no [`CheckCtx`];
/// the strings are the Doctor row's own, preventing drift.
///
/// # Errors
///
/// `Err((message, remedy))` when `contract/` under `root` cannot be read or
/// hashed (fails closed with the Doctor row's message), and when a
/// self-consistent checkout disagrees with
/// [`crate::contract::COMPILED_CONTRACT_SHA256`].
````

### B0276 · l.170–171 · REWRITE (confirm) · rule 2.3 · 2 → 1

````text
    /// The repo root: three levels above this crate's manifest directory.
````

### B0280 · l.275–279 · REWRITE (amend) ·p · rule 1.7 · 5 → 4

````text
    /// [`assert_binary_matches_checkout`] returns `Ok(())` against the live
    /// checkout — the predicate behind `stages::deny_on_contract_skew`, run by
    /// every mutating door (`stages::run_stage`, `stages::run_stage_holding_lock`,
    /// `crate::fixes::apply`) before dispatch; Stop is ungated (round-1 finding A1-1).
````

## `sabrage/crates/sabrage-core/src/checks/mod.rs`

Deleted (nothing carried): B0282, B0298, B0320, B0340

### B0281 · l.1–39 · REWRITE (amend) ·p · rule 1.2 · 39 → 16

````text
//! The check engine: one contract-ordered registry, bound to evaluator functions.
//!
//! `contract/pipeline.toml` owns the slug list, its order, and the per-side gates;
//! nothing here may add, remove, or reorder checks. This crate owns check logic and
//! message/remedy prose — every string must match `scripts/demo/doctor.sh` verbatim
//! because docs/troubleshooting.md quotes those lines (the parity harness joins on
//! slug plus status, never prose).
//!
//! Evaluators are read-only probes: no filesystem mutation in check code. Auto-fixes
//! live in the fix registry and run from the preflight, never from doctor.
//!
//! Contract `group` values name their module except where folded or renamed:
//! `crossover` lives in [`system`], `bottle-bridge` in [`bridge`], `run-only` in
//! [`run_only`]; keep new evaluators in the module their group points at. The registry
//! binds by slug in contract order
//! (tests::registry_binds_in_contract_order_and_covers_every_slug).
````

### B0283 · l.68–74 · REWRITE (confirm) · rule 1.7 · 7 → 6

````text
/// Result of one check.
///
/// `Pass`/`Warn`/`Fail`/`Info`/`Skipped` are the five statuses the zsh tap
/// channel emits (`chk ok|warn|fail|info`, plus explicit `tap <slug> skipped`).
/// `NotImplemented` has no zsh counterpart and is reported to the tap as
/// `skipped` (see [`crate::tap`]).
````

### B0295 · l.176 · REWRITE (amend) · rule 1.7 · 1 → 2

````text
    /// A `NotImplemented` outcome for a slug with no bound evaluator — produced
    /// only by a lenient registry build ([`build_registry`] with `strict = false`).
````

### B0321 · l.334–340 · REWRITE (confirm) · rule 1.4 · 7 → 5

````text
/// A check evaluator: a synchronous, read-only probe.
///
/// Sync is deliberate: every doctor probe is a `stat`, a small read, a digest, or a
/// short subprocess. The async machinery of design-core §3 belongs to the stage
/// layer, where long-running children and cancellation exist.
````

### B0324 · l.348 · REWRITE (confirm) · rule 2.3 · 1 → 1

````text
    /// `None` only in a lenient registry build; [`registry`] binds every slug.
````

### B0329 · l.393–395 · REWRITE (confirm) · rule 1.5 · 3 → 3

````text
    /// The doctor-visible subset: contract order minus [`NO_DOCTOR_ROW_GROUP`].
    /// doctor.sh never emits these slugs, so neither may the native doctor
    /// (console, tap, or fail count) — tests::doctor_walks_only_doctor_visible_checks.
````

### B0337 · l.470–480 · REWRITE (confirm) ·p · rule 1.7 · 11 → 10

````text
/// Join the contract's ordered check list with the evaluator map.
///
/// `strict = true` is the release contract; `strict = false` tolerates missing
/// bindings, whose checks then report [`CheckStatus::NotImplemented`].
///
/// # Errors
///
/// Missing evaluator (strict only), unknown slug, or duplicate binding. This enforces
/// "adding a check to only one place must fail"
/// (sabrage-parity::tests::strict_registry_builds_and_covers_the_contract_in_order).
````

### B0338 · l.518–523 · REWRITE (confirm) · rule 1.3 · 6 → 3

````text
        // Run-only preflights are bound too: they have no doctor row
        // (NO_DOCTOR_ROW_GROUP keeps them out of Registry::doctor_checks), but the
        // launch preflight resolves their evaluators through this same registry.
````

### B0339 · l.537–541 · REWRITE (confirm) ·p · rule 1.2 · 5 → 7

````text
/// The registry, built strictly: every contract slug has a bound evaluator,
/// run-only preflights included ([`run_only`] binds those; [`Registry::doctor_checks`]
/// hides them from doctor, not an absent binding).
///
/// # Panics
///
/// A contract slug with no evaluator is a hard error by design.
````

## `sabrage/crates/sabrage-core/src/checks/network.rs`

Deleted (nothing carried): B0349, B0353, B0357, B0359, B0360

### B0348 · l.1–29 · REWRITE (amend) · rule 1.6 · 29 → 9

````text
//! Doctor evaluators for the `network` group: `net.ports` and
//! `net.adb-forwards`, bound in contract order by [`defs`]. Message and
//! remedy strings track `scripts/demo/doctor.sh` sections 16/16b verbatim
//! (see [`super`] for why). Reference: scripts/demo/doctor.sh.
//!
//! One declared divergence (A4-4 / A3b packet): a failed `adb forward --list`
//! probe Warns here and taps `ok` in zsh. PARITY.md § Declared by the
//! 2026-08-30 adversarial review (round 1 fixes), "**`net.adb-forwards` on a
//! failed probe.**"
````

### B0350 · l.43–46 · REWRITE (confirm) ·p · rule 1.2 · 4 → 3

````text
/// The exact literal ports `scripts/demo/doctor.sh` section 16 passes. Pinned
/// to doctor.sh's text rather than derived from `contract().ports.stream`
/// (`[9943, 9944]` today), because doctor.sh does not derive it either.
````

### B0351 · l.49–53 · REWRITE (amend) ·p · rule 1.2 · 5 → 4

````text
/// `COMMAND(PID)` for every `lsof` row on the streaming ports: deduplicated,
/// sorted, space-joined **with a trailing space** when non-empty (the caller
/// concatenates directly before doctor.sh's em dash). Empty when nothing is
/// listening or `lsof` cannot be spawned.
````

### B0352 · l.75–80 · REWRITE (confirm) · rule 1.6 · 6 → 3

````text
/// `net.ports`: Warn naming the busy listeners, Pass when the streaming
/// ports are free. Reference: scripts/demo/doctor.sh section 16.
/// tests::net_ports_matches_a_direct_lsof_probe
````

### B0354 · l.95–105 · REWRITE (confirm) ·p · rule 1.7 · 11 → 6

````text
/// The local side (`tcp:<port>`) of every `adb forward --list` row.
///
/// # Errors
/// `Err` when `adb` cannot be spawned or exits non-zero — distinct from
/// `Ok(vec![])` (no forwards). Callers must not fold the two (A4-4).
/// tests::adb_forward_local_specs_reports_nonzero_exit_as_err
````

### B0355 · l.125–133 · REWRITE (amend) · rule 1.6 · 9 → 5

````text
/// `net.adb-forwards`: Skipped when adb probes are disabled or no adb is
/// found, Warn when `tcp:9943`/`tcp:9944` are forwarded or the probe failed,
/// Pass otherwise — a row zsh does not print (PARITY.md § Doctor / checks,
/// "`net.adb-forwards` renders a green"). Reference:
/// scripts/demo/doctor.sh section 16b.
````

### B0356 · l.143–145 · REWRITE (amend) · rule 1.5 · 3 → 3

````text
        // A4-4 / A3b packet: a failed probe is not "no stale forwards"; stale
        // tcp:9943/9944 would silently break WiFi discovery, so never Pass here.
        // tests::net_adb_forwards_warns_not_passes_when_the_probe_cannot_spawn_adb
````

### B0364 · l.251 · REWRITE (confirm) ·p · rule 1.4 · 1 → 1

````text
    /// A4-4: failed adb probe must not read as "clean". tests::net_adb_forwards_warns_not_passes_when_the_probe_cannot_spawn_adb
````

## `sabrage/crates/sabrage-core/src/checks/overlay.rs`

Deleted (nothing carried): B0368

### B0367 · l.1–15 · REWRITE (amend) ·p · rule 1.6 · 15 → 6

````text
//! Group `overlay` — the global bridge overlay inside CrossOver.app: the DXMT
//! artifacts and the built `wineopenxr` binaries must match the copies under
//! `$CX/lib` (a CrossOver update silently reverts them).
//!
//! Owns `overlay.dxmt-d3d11`, `overlay.dxmt-winemetal`, `overlay.woxr-dll`,
//! and `overlay.woxr-so` in contract order. Mirrors doctor.sh section 10.
````

### B0369 · l.31–32 · REWRITE (amend) ·p · rule 1.2 · 2 → 5

````text
/// Compares one overlay source against its destination under `$CX/lib`.
///
/// Passes when byte-identical; fails with the `./demo.sh install` remedy
/// when they differ or either file is missing; skipped only when `dst` is
/// `None` (no CrossOver.app).
````

### B0370 · l.39–41 · REWRITE (confirm) ·p · rule 1.6 · 3 → 1

````text
    // Skip reason is ours — doctor.sh section 10 taps bare `… skipped` with no `info` line.
````

## `sabrage/crates/sabrage-core/src/checks/pinned.rs`

### B0380 · l.61–65 · REWRITE (amend) ·p · rule 1.4 · 5 → 2

````text
/// Hash this multi-megabyte dll once and reuse the digest for the warn detail.
/// Re-hashing to build it is the obvious edit; no test catches the added cost.
````

## `sabrage/crates/sabrage-core/src/checks/run_only.rs`

Deleted (nothing carried): B0385, B0387, B0389, B0400, B0403, B0407

### B0382 · l.1–34 · REWRITE (amend) ·p · rule 1.6 · 34 → 15

````text
//! Group `run-only` — preflights that exist only in the launch path, so
//! doctor prints no row for them ([`super::NO_DOCTOR_ROW_GROUP`]).
//!
//! Slugs owned here, in contract order: `run.wine-exec`, `run.bridge-built`
//! (one gate over the pair doctor splits across `build.oxr-dylib` and
//! `build.woxr-dll`), `run.wired-adb` (evaluated only for `--wired`, which
//! needs a connected device for the `tcp:9943`/`tcp:9944` forwards). Every
//! evaluator is a read-only `fn(&CheckCtx) -> CheckOutcome`.
//!
//! Reference: `scripts/demo/run.sh`, the `# preflight: run.*` tags. With no
//! doctor row, each `message` carries run.sh's whole `die` sentence and no
//! `remedy`; the launch preflight turns a FAIL into
//! [`crate::error::SabrageError::Fatal`] with that text. See
//! `checks::tests::registry_binds_in_contract_order_and_covers_every_slug` and
//! tests::wine_exec_fails_with_run_shs_verbatim_die_text_when_the_file_is_not_executable.
````

### B0386 · l.62–76 · REWRITE (amend) · rule 1.6 · 15 → 9

````text
/// `run.wine-exec`: passes when `ctx.paths.wine` exists and is executable;
/// otherwise fails with run.sh's `# preflight: run.wine-exec` die sentence,
/// except in the declared divergence below.
///
/// Declared divergence — no CrossOver.app: there is no path to interpolate
/// ([`crate::paths`] models that case as `wine: None` rather than reproducing
/// lib.sh's bogus `/Contents/SharedSupport/CrossOver/bin/wine`), so the
/// sentence names the missing app instead. See
/// tests::wine_exec_says_crossover_app_not_found_when_there_is_no_path_at_all.
````

### B0388 · l.99–104 · REWRITE (confirm) · rule 1.6 · 6 → 5

````text
/// `run.bridge-built`: passes when both bridge outputs exist, otherwise fails
/// with run.sh's `# preflight: run.bridge-built` die sentence. One gate over
/// the pair doctor splits across `build.oxr-dylib` and `build.woxr-dll`; the
/// `detail` names whichever half is missing, which the shell's single message
/// cannot. See tests::bridge_built_needs_both_halves.
````

### B0392 · l.142–158 · REWRITE (confirm) ·p · rule 1.5 · 17 → 10

````text
/// `"$ADB" devices` stdout, or empty when the binary is missing, fails to run,
/// or does not answer within `timeout`. Same probe `checks::headset` makes;
/// duplicated because that module is private.
///
/// The bound is load-bearing (A7-4): evaluators are synchronous, so a wedged
/// `adb` would hold the launch's operation lock past every cancellation
/// checkpoint. tests::the_devices_probe_gives_up_on_a_wedged_adb_instead_of_blocking_forever.
///
/// `timeout` is a parameter so that test can pin the deadline in milliseconds;
/// the one production call site passes [`ADB_PROBE_TIMEOUT`].
````

### B0395 · l.197–198 · REWRITE (confirm) · rule 1.6 · 2 → 3

````text
/// The first serial marked `device` in `adb devices` output, or `None` —
/// run.sh's `WIRED_SER` awk (`NR>1 && $2=="device"`) under
/// `# preflight: run.wired-adb`.
````

### B0396 · l.212–222 · REWRITE (confirm) · rule 1.6 · 11 → 6

````text
/// `run.wired-adb`: with `--wired`, fails with run.sh's two
/// `# preflight: run.wired-adb` die sentences when adb is absent or no device
/// answers. Without `--wired` the shell evaluates neither, so this reports
/// [`CheckStatus::Skipped`] — "not applicable", which the launch preflight
/// treats as a non-blocking row rather than as an unverifiable gate. See
/// tests::wired_adb_is_skipped_unless_wired.
````

### B0411 · l.477–481 · REWRITE (confirm) · rule 1.7 · 5 → 4

````text
    /// A7-4 regression: an unbounded probe let a wedged `adb` block inside the
    /// evaluator, and with it the launch preflight, which holds the operation
    /// lock and can only check for cancellation between evaluators. Pins that
    /// the deadline fires and that the timed-out child is not left running.
````

## `sabrage/crates/sabrage-core/src/checks/sources.rs`

Deleted (nothing carried): B0416

### B0414 · l.1–13 · REWRITE (amend) ·p · rule 1.6 · 13 → 5

````text
//! Group `sources` — doctor.sh section 6: submodule checkouts and the ALVR patch set.
//!
//! Slugs, in contract order: `src.oxrsys`, `src.wineopenxr`, `src.alvr`,
//! `src.alvr-patchset` (`is_streaming_nonblocking` pin sanity-check for
//! oxrsys-v20.14.1).
````

### B0415 · l.20–22 · REWRITE (confirm) ·p · rule 1.6 · 3 → 2

````text
/// True when the submodule has a `.git` entry — a file (gitlink) or a
/// directory (nested clone).
````

### B0418 · l.62–64 · REWRITE (confirm) ·p · rule 1.6 · 3 → 3

````text
/// Passes when ALVR's `alvr/server_core/src/connection.rs` contains `is_streaming_nonblocking`;
/// a missing or unreadable file fails (`tests::patchset_check_greps_connection_rs`). No regex
/// metacharacters in the needle, so this substring search and doctor.sh's `grep -q` agree.
````

## `sabrage/crates/sabrage-core/src/checks/system.rs`

Deleted (nothing carried): B0424, B0425, B0429, B0432, B0435

### B0423 · l.1–15 · REWRITE (amend) ·p · rule 1.6 · 15 → 11

````text
//! Group `system, crossover` — hardware, OS version, and the CrossOver install.
//!
//! Slugs owned here, in contract order: `sys.arch`, `sys.macos27`,
//! `cx.present` (silent when found), `cx.version`.
//!
//! `sys.macos27` is a hard FAIL below macOS 27 even on the native-arm64
//! encoder path, so the in-process fallback stays viable.
//!
//! Every evaluator is `fn(&CheckCtx) -> CheckOutcome`: a read-only probe.
//! Message and remedy strings must match scripts/demo/doctor.sh verbatim;
//! `cx.present` is silent in doctor.sh, so its message is Sabrage-only.
````

### B0426 · l.45–46 · REWRITE (confirm) · rule 1.2 · 2 → 2

````text
/// The CPU brand string, empty when the probe fails: doctor.sh discards stderr
/// and has no `|| echo` fallback here.
````

### B0427 · l.54 · REWRITE (confirm) · rule 1.2 · 1 → 2

````text
/// The macOS product version, or `"0"` when the probe fails, matching
/// doctor.sh's `|| echo 0` fallback.
````

### B0428 · l.62 · REWRITE (confirm) · rule 1.2 · 1 → 2

````text
/// `CFBundleShortVersionString` from `plist`, or `"0"` when the read fails,
/// matching doctor.sh's `|| echo 0` fallback.
````

### B0431 · l.95–103 · REWRITE (confirm) · rule 1.2 · 9 → 8

````text
/// True iff `a`'s dotted version is `>=` `b`'s, treating a missing trailing
/// component on either side as `0`; a single-component `b` (e.g. `"27"`)
/// therefore pins only the major version, as doctor.sh's `${OSVER%%.*}`
/// truncation does for `sys.macos27`.
///
/// design-core divergence 10: this is a numeric compare, not doctor.sh's
/// string ordering, and the two must agree on every observable version —
/// tests::dotted_ge_table.
````

## `sabrage/crates/sabrage-core/src/checks/toolchain.rs`

Deleted (nothing carried): B0447

### B0440 · l.1–15 · REWRITE (amend) ·p · rule 1.6 · 15 → 10

````text
//! Group `toolchain` — the `tool.*` and `rust.x64-target` doctor rows.
//!
//! Slug list and order live in `contract/pipeline.toml`; shell probes are in
//! `scripts/demo/doctor.sh` sections `4.` and `5.`.
//!
//! `rust.x64-target` requires a rustup toolchain because Homebrew's cargo
//! ships no std for `x86_64-apple-darwin`.
//!
//! Every evaluator is `fn(&CheckCtx) -> CheckOutcome`: a read-only probe.
//! Message and remedy strings must match `scripts/demo/doctor.sh` verbatim.
````

### B0442 · l.28–29 · REWRITE (confirm) · rule 1.2 · 2 → 2

````text
/// Outcome for `slug` from whether `bin` resolves on `PATH`, carrying the
/// shared toolchain remedy when it does not.
````

### B0443 · l.58–63 · REWRITE (confirm) ·p · rule 1.6 · 6 → 4

````text
/// Whether `rustup target list --installed` printed `x86_64-apple-darwin`.
///
/// `rustup`'s own exit status is ignored: the doctor.sh pipeline's status
/// is `grep -q`'s, so only stdout decides.
````

## `sabrage/crates/sabrage-core/src/config/mod.rs`

### B0448 · l.1–15 · REWRITE (amend) ·p · rule 1.7 · 15 → 13

````text
//! User-facing configuration: [`runtime_toml`], the typed, format-preserving
//! editor for `~/Library/Application Support/OXRSys/oxrsys-runtime.toml`,
//! written once by `demo.sh` and never touched again.
//!
//! Sabrage owns the six `EDITABLE_KEYS` values and nothing else: ordering, spacing,
//! unknown keys, the provenance header and comments survive byte-for-byte because the
//! file is shared with a human and a line-oriented C++ parser that is not a TOML
//! implementation (`runtime_toml::tests::a_real_edit_preserves_crlf_a_bom_and_a_missing_final_newline`).
//! The one exception: a same-line `#` comment is relocated above its key by [`runtime_toml`]
//! (`runtime_toml::tests::a_same_line_comment_moves_to_its_own_line_above_the_key`).
//! See `sabrage/docs/design/design-core.md` §4.1.
//!
//! Sabrage's own state — settings, the game library — lives in [`crate::store`].
````

## `sabrage/crates/sabrage-core/src/config/runtime_toml.rs`

Deleted (nothing carried): B0457, B0484, B0490, B0503, B0510, B0512, B0518, B0535, B0555, B0558, B0560, B0570, B0574, B0576, B0578, B0582, B0584, B0616, B0619, B0621, B0639, B0650, B0653, B0656

### B0449 · l.1–64 · REWRITE (confirm) ·p · rule 1.2 · 64 → 23

````text
//! `oxrsys-runtime.toml` — typed read, format-preserving patch, backed-up write.
//!
//! Sabrage edits the six streaming keys of a file `scripts/demo/setup.sh` writes
//! once and never touches again; design-core §4.1 makes that a narrow override:
//! create an absent file from [`crate::util::toml_template`] byte-for-byte, then
//! edit values in place with a rolling backup, never regenerate, never migrate.
//!
//! The consumer is not a TOML library. `ext/oxrsys/runtime/src/Config.cpp` reads
//! lines: `"`-aware `#` comments, `[table]` headers ignored so a key counts
//! wherever it sits, split on the first `=`, one pair of double quotes off a
//! string value, last *accepted* assignment wins. Values therefore always come
//! from [`read_lines_like_the_runtime`] and never from `toml_edit`, which answers
//! a different question — see PARITY.md § Declared by the 2026-08-30
//! adversarial review (round 1 fixes), "Config readers: doctor emulates `awk`"
//! and tests::{the_comment_stripper_matches_config_cpp,
//! effective_accepted_agrees_with_the_line_reader_on_the_shadowed_fixtures}.
//!
//! `toml_edit` decides whether the file can be rewritten safely and performs the
//! rewrite. Every other byte is preserved — the file is hand-maintained — and a
//! same-line `#` comment on a rewritten line is relocated above the key because
//! runtime builds before the 2026-08 parser fix mis-read trailing comments; see
//! tests::{a_key_inside_a_multiline_string_reads_live_and_refuses_the_write,
//! a_same_line_comment_moves_to_its_own_line_above_the_key}.
````

### B0487 · l.342–350 · REWRITE (confirm) · rule 1.2 · 9 → 7

````text
/// Why this file must not be rewritten, if it must not be — the value
/// [`RuntimeConfigView::parse_error`] carries and [`write`] refuses on.
///
/// `Some` when `toml_edit` cannot parse the text at all, or when the physical
/// lines and the parsed document disagree about an editable key
/// ([`line_document_mismatch`]); either way an edit would not land where the
/// runtime looks.
````

### B0488 · l.360–378 · REWRITE (confirm) · rule 1.2 · 19 → 10

````text
/// The second half of [`round_trip_error`], over an already-parsed document, so
/// [`apply_patch`] can consult it without parsing the file twice.
///
/// `Some` when an editable key's physical assignment count differs from the
/// parsed one, in either direction: a key inside a `"""…"""` block is live to
/// the runtime and invisible to `toml_edit`, and a key behind a byte-order mark
/// is the reverse. Both are refused rather than repaired — the BOM is a byte
/// Sabrage does not own. See
/// tests::{a_key_inside_a_multiline_string_reads_live_and_refuses_the_write,
/// a_bom_on_a_root_key_is_not_round_trippable}.
````

### B0502 · l.636–645 · REWRITE (confirm) · rule 2.3 · 10 → 8

````text
/// `Config.cpp`'s bounds check at `Config.cpp`'s precision: `std::stof` yields a
/// C++ `float`, so `1.00000001` *is* `1.0f` to the runtime and is accepted.
///
/// Comparing the `f64` Rust parsed would reject it, and the Settings screen would
/// then show the default for a file the runtime reads as `1.0`. The bounds are
/// exactly representable, so the two comparisons agree everywhere except within
/// one `f32` ulp of an endpoint. See
/// tests::the_scale_bounds_are_checked_at_the_runtimes_float_precision.
````

### B0504 · l.655–660 · REWRITE (confirm) · rule 1.6 · 6 → 5

````text
/// Every `key = value` the runtime's line reader sees, in physical order.
///
/// `ext/oxrsys/runtime/src/Config.cpp`'s `ParseConfigToml` loop, ported. Tables
/// are not tracked because the runtime does not track them — a key counts
/// wherever it sits (tests::the_line_reader_agrees_with_parse_config_toml).
````

### B0505 · l.672–682 · REWRITE (confirm) · rule 1.2 · 11 → 9

````text
/// What the runtime would use, read the way the runtime reads it — the
/// **primary** reader (see the module header), used whether or not `toml_edit`
/// can also parse the file.
///
/// Returns the last *accepted* assignment of each editable key, so a junk
/// assignment after a valid one leaves the valid value in force, plus every
/// rejected occurrence (`invalid`) and every key assigned more than once
/// (`shadowed`). See
/// tests::a_later_invalid_assignment_does_not_erase_an_earlier_valid_one.
````

### B0506 · l.713–725 · REWRITE (amend) ·p · rule 1.2 · 13 → 11

````text
/// The value the runtime would end up with for **any** key, as a plain string,
/// with no accepted-set filtering: the last assignment wins, whatever table it
/// sits in, with one pair of double quotes removed. `None` means the key is never
/// assigned, so the caller's own default applies (`${…:-auto}` in the shell).
///
/// Deliberately free of `toml_edit` — it answers what the runtime does, not what
/// TOML says — so the run preflight reads the file the way `Config.cpp` does
/// instead of carrying its own quote-blind, first-match split. The doctor checks
/// do *not* share it: PARITY.md § Declared by the 2026-08-30 adversarial
/// review (round 1 fixes), "Config readers: doctor emulates `awk`". See
/// tests::effective_string_is_last_wins_table_blind_and_double_quote_only.
````

### B0507 · l.733–752 · REWRITE (confirm) · rule 1.2 · 20 → 11

````text
/// The value the runtime would end up with for one of the **six modeled** keys:
/// the last assignment it would *accept*, in the key's canonical spelling.
///
/// [`effective_string`] answers "what does the last line say"; this answers
/// "what is the runtime holding", and the two differ where `Config.cpp` throws a
/// value away — `protocol = "alvr"` followed by `protocol = "banana"` leaves
/// ALVR in force, and reading that file with the raw helper would block a launch
/// the runtime would have run. `None` means no occurrence was accepted (including
/// an absent key and a key outside [`EDITABLE_KEYS`]), so the caller's own default
/// applies. See tests::{effective_accepted_keeps_the_last_value_the_runtime_would_accept,
/// effective_accepted_agrees_with_the_line_reader_on_the_shadowed_fixtures}.
````

### B0516 · l.830–835 · REWRITE (confirm) · rule 1.5 · 6 → 5

````text
/// Every assignment of `key` in the document, in physical order.
///
/// **Dotted tables are skipped on purpose**: `streaming.protocol = "alvr"` is one
/// physical line whose key text is `streaming.protocol`, which the runtime's line
/// reader does not see as `protocol` (tests::a_dotted_key_is_not_an_occurrence).
````

### B0517 · l.843–852 · REWRITE (confirm) · rule 1.5 · 10 → 8

````text
/// Whether the document's spelling of `key` is the bare one the runtime's reader
/// recognises. A key Sabrage synthesised has no repr yet; it renders bare.
///
/// The runtime splits the line on its first `=` and trims, so the key text of
/// `"protocol" = "oxrsys"` is literally `"protocol"` and matches nothing, while
/// `toml_edit` decodes it to `protocol` — without this check Sabrage would edit a
/// line the runtime ignores. See tests::{a_quoted_key_is_not_an_occurrence,
/// a_key_that_exists_only_as_a_quoted_key_is_refused}.
````

### B0520 · l.930–935 · REWRITE (confirm) · rule 1.3 · 6 → 3

````text
    // Enforced here and not only in [`RuntimeConfigView::parse_error`]: `write`,
    // `edit_protocol` and the golden tests all come through this function, and
    // reporting success for an edit the runtime never reads is worse than refusing.
````

### B0522 · l.971–981 · REWRITE (amend) · rule 1.5 · 11 → 3

````text
    // "Nothing changed" means no key changed, never "the re-render happens to match":
    // `doc.to_string()` normalises CRLF, drops a BOM and adds a final newline, which
    // [`ByteShape`] undoes for a real edit (tests::an_empty_patch_is_the_identity_on_every_input_shape).
````

### B0523 · l.995–1006 · REWRITE (confirm) · rule 1.2 · 12 → 7

````text
/// The three byte-level properties `toml_edit`'s renderer does not preserve: a
/// CRLF file, a leading BOM, and the absence of a final newline. Captured before
/// parsing and restored after rendering, because Sabrage owns six values and not
/// the file's shape.
///
/// Mixed line endings are the one shape not preserved — there is no "the file's"
/// ending to restore — so such a file is rendered LF.
````

### B0528 · l.1068–1074 · REWRITE (amend) · rule 2.3 · 7 → 8

````text
/// Replace the value of an existing key, keeping the key, its own decor and the
/// `=` spacing. Returns whether anything changed.
///
/// A key already carrying the wanted value is left **completely** untouched,
/// including a same-line comment: design-core §4.1 rule 2 relocates a comment on a
/// line this function rewrites, and reformatting a line the patch did not need to
/// touch would break "an unchanged patch writes nothing"
/// (tests::setting_every_key_to_its_current_value_changes_nothing).
````

### B0529 · l.1086–1087 · REWRITE (amend) · rule 2.3 · 2 → 3

````text
    // design-core §4.1 rule 2: a same-line comment on a line Sabrage rewrites moves above
    // the key, at the key's own indentation — runtime builds before the 2026-08 parser fix
    // mis-read trailing comments (tests::a_same_line_comment_moves_to_its_own_line_above_the_key).
````

### B0530 · l.1117–1128 · REWRITE (amend) · rule 1.2 · 12 → 9

````text
/// Whether the runtime would read the same value out of both, i.e. whether
/// rewriting the line would be a pure reformat.
///
/// Textual equality of the value's own bytes, and nothing looser: it is the
/// runtime, not `toml_edit`, that has to agree. `0x50` and `80` are one integer to
/// `toml_edit` and two different values to the runtime, and a `'alvr'` literal
/// string is valid TOML the runtime does not unquote. Every spelling the runtime
/// would misread therefore counts as a change, which is what makes saving one fix
/// it (tests::a_literal_quoted_string_is_invalid_and_gets_rewritten).
````

### B0536 · l.1202–1225 · REWRITE (confirm) ·p · rule 1.5 · 24 → 21

````text
/// Patch the file on disk, creating it from the shared template first when it is
/// absent and backing up the previous contents when it is not.
///
/// The write-once override of design-core §4.1: an absent file is created with
/// [`crate::util::toml_template`] byte-for-byte, never a rendering of the patch,
/// so both front-ends still agree on first-write bytes — PARITY.md § Invariants
/// that must NOT change (byte/behavior parity), "Write-once `oxrsys-runtime.toml`
/// creation". Every mutation goes through `executor`, so a
/// [`crate::DryRunExecutor`] plans the create, the backup, the prune and the write
/// without touching disk.
///
/// # Errors
///
/// Refuses a patch the runtime would ignore, a file that cannot be round-tripped,
/// a live session ([`blocking_session`]: the runtime re-reads this file every
/// 250 ms and rebuilds the encoder, so a save mid-stream is a live
/// reconfiguration) and a concurrent edit ([`still_safe_to_replace`]: the bytes on
/// disk must still be the ones the patch was computed against). See
/// tests::{write_creates_from_the_template_byte_identically_then_patches,
/// write_refuses_while_a_session_is_live_and_touches_nothing,
/// the_replacement_refuses_when_the_file_changed_underneath}.
````

### B0537 · l.1249–1255 · REWRITE (amend) · rule 1.5 · 7 → 3

````text
    // A10-1: serializes read-patch-backup-write across processes at the documented lock path
    // (tests::write_takes_the_cross_process_lock_at_the_documented_path); best-effort, since
    // [`still_safe_to_replace`]'s compare-and-swap is the net. A dry run has nothing to lock.
````

### B0538 · l.1262–1269 · REWRITE (amend) · rule 1.5 · 8 → 3

````text
    // A10-1: the probe and the create are two syscalls apart, so this branch uses
    // [`Executor::create_new`] (`O_EXCL`) and never `write_atomic`'s unconditional rename — a
    // file created in that window survives (tests::write_never_clobbers_a_file_created_in_the_toctou_window).
````

### B0539 · l.1284–1289 · REWRITE (confirm) · rule 1.3 · 6 → 3

````text
            // Lost the create race: read what the other writer put there rather
            // than overwriting it, and continue on the "file already existed"
            // path, whose compare-and-swap still guards against a third writer.
````

### B0540 · l.1305–1313 · REWRITE (confirm) · rule 1.7 · 9 → 3

````text
        // Nothing to write, so nothing to back up: an existing file is not even
        // opened, and the test is "no key changed", never `patched.text == base`
        // (tests::a_no_op_write_leaves_the_file_and_backups_untouched).
````

### B0542 · l.1337–1349 · REWRITE (amend) · rule 1.5 · 13 → 3

````text
    // A10-2: everything from here to the rename is undone on failure — the reservation is
    // unlinked, nothing older is dropped before the commit — and the compare-and-swap runs again
    // because the backup write is the widest window (tests::a_failed_write_prunes_nothing_and_leaves_no_reservation).
````

### B0544 · l.1368–1377 · REWRITE (amend) · rule 1.7 · 10 → 3

````text
    // A10-2: best-effort and deliberately not `?`. The commit already happened, so an `Err` here
    // would report a failed save over a file that holds the new bytes; a stray backup is
    // recoverable, a lie about the write is not (tests::an_unprunable_stale_backup_still_reports_a_committed_save).
````

### B0545 · l.1384–1401 · REWRITE (confirm) ·p · rule 1.7 · 18 → 13

````text
/// The session that must be stopped before this file may be rewritten, or `None`
/// when nothing is streaming.
///
/// Delegates to [`crate::session::session_block_at`] so that every "not while the
/// game is running" door asks the same question — PARITY.md § Declared by the
/// 2026-08-30 adversarial review (round 1 fixes), "External sessions".
///
/// The door exists because the runtime does not read `oxrsys-runtime.toml` once at
/// game start: `Config::GetValues()` refreshes it whenever the mtime moved, at
/// most every 250 ms, and `AlvrStreamingBackend::EnsureEncoder` retires the
/// encoder when `encoder_process`/`video_codec` drift — so a save mid-stream
/// rebuilds the encoder, and selecting `native` with no staged helper drops frames
/// for the rest of the session.
````

### B0547 · l.1430–1437 · REWRITE (confirm) · rule 1.3 · 8 → 6

````text
/// Where `session-state.json` sits, given the backups directory.
///
/// `backups_dir` is always `<sabrage_appsup>/backups` ([`crate::paths::Paths`]), so
/// its parent is the directory that record lives in. Deriving it keeps the guard
/// hermetic under test: a temp `backups_dir` yields a temp session path, never the
/// developer's real running session.
````

### B0550 · l.1453–1465 · REWRITE (confirm) · rule 1.2 · 13 → 11

````text
/// Takes the advisory lock at [`Paths::toml_lock_path`], derived from
/// `toml_path`'s parent because [`Paths::toml_path`] and [`Paths::toml_lock_path`]
/// always share one directory. Held by the returned `File`; dropping it releases
/// the `flock`.
///
/// A separate dotfile, not a lock on the config itself, so it survives
/// [`Executor::write_atomic`]'s rename. `None` on any failure, including a holder
/// that will not let go: this narrows the window [`still_safe_to_replace`] guards,
/// it does not replace it.
///
/// [`Paths`]: crate::paths::Paths
````

### B0553 · l.1545–1554 · REWRITE (confirm) · rule 1.5 · 10 → 9

````text
/// A10-1: reserves `<backups_dir>/oxrsys-runtime.toml.<secs>[-n]` and writes
/// `bytes` into it via [`Executor::create_new`] (`O_EXCL`).
///
/// [`next_backup_path`]'s `!exists()` probe is itself a check-then-create race
/// between two Sabrage processes backing up in the same second. A lost race
/// retries the whole probe rather than bumping the suffix locally, because the
/// directory listing has changed underneath. See
/// tests::{concurrent_backups_in_the_same_second_each_keep_their_own_bytes,
/// a_same_second_backup_collision_gets_a_numeric_suffix}.
````

### B0564 · l.1838–1846 · REWRITE (confirm) ·p · rule 1.7 · 9 → 8

````text
    /// A value spelled with **literal** quotes is one the runtime throws
    /// away: `ParseString` strips double quotes only, so `'alvr'` fails the
    /// whitelist and `streamingProtocol` keeps its `oxrsys` default.
    /// Re-saving rewrites the line in the spelling the runtime does read.
    ///
    /// The defect this pins: accepting either quote flavour in `unquote` and
    /// `same_to_the_runtime` shows ALVR with no warning and reports Save as
    /// a success while the dead line stays on disk.
````

### B0566 · l.1880–1885 · REWRITE (confirm) ·p · rule 1.7 · 6 → 6

````text
    /// A quoted key is a different key to the runtime's line reader (its key
    /// text keeps the quotes), so `toml_edit`'s decoded view must not
    /// disagree — same rule as the dotted key above. The quoted spelling is
    /// the LAST in the document: the defect this pins reports its value from
    /// `read` and edits its line from `apply_patch`, while the runtime only
    /// ever sees the bare one in `[streaming]`.
````

### B0607 · l.2400–2404 · REWRITE (confirm) · rule 1.7 · 5 → 5

````text
    /// A10-3/A10-4: the refusal belongs to [`apply_patch`], which every caller
    /// goes through, not to the *view* alone — a caller that does not render
    /// Settings first (`write`, `edit_protocol`, the CLI) would otherwise
    /// rewrite the outer line, report success, and leave the runtime obeying
    /// the line inside the string.
````

### B0609 · l.2441 · REWRITE (confirm) · rule 1.7 · 1 → 1

````text
        // …and so does the fix, which must not claim it set protocol.
````

### B0623 · l.2773–2777 · REWRITE (confirm) · rule 1.7 · 5 → 3

````text
    /// A10-1: the absent-file branch goes through [`Executor::create_new`]
    /// (`O_EXCL`), never an unconditional rename, so a file that appears
    /// between the probe and the create is read back instead of clobbered.
````

### B0645 · l.3316–3322 · REWRITE (amend) · rule 1.3 · 7 → 3

````text
        // A live `process_id` and a fresh stamp together: that is
        // `watcher::runtime_status_live`, the single predicate both this door and
        // the phase the Session screen renders go through (A10-8).
````

### B0654 · l.3435–3438 · REWRITE (confirm) · rule 1.7 · 4 → 4

````text
    /// A context whose `toml_path` and `sabrage_appsup` both live under a
    /// scratch directory: no test may touch the real ones, which a
    /// config-writing suite would otherwise overwrite (see the deployed
    /// fixture's own header).
````

## `sabrage/crates/sabrage-core/src/contract.rs`

Deleted (nothing carried): B0711

### B0658 · l.1–29 · REWRITE (confirm) ·p · rule 1.3 · 29 → 19

````text
//! The shared parity contract, compiled in.
//!
//! `contract/pipeline.toml` is the single source for pins, the depot triple,
//! the host-manifest path, the DXMT artifact set, the port lists, the
//! **ordered check registry**, and the launch-action registry. The zsh side
//! consumes it through the GENERATED `scripts/demo/contract.gen.sh`;
//! sabrage-core parses the TOML directly.
//!
//! The three contract files are baked in with `include_str!` rather than read
//! from `repo_root`: `Sabrage.app` is installed somewhere unrelated to the
//! repo and `repo_root` is user-configurable, so the check registry is part
//! of the binary's identity, not of machine state. Editing a contract file
//! retriggers a rebuild, which is the tripwire the parity design wants.
//! [`crate::util::contract_hash`] reads the three files from `repo_root` at
//! runtime because `meta.contract-sync` compares the *on-disk* contract
//! against the *on-disk* generated shell file.
//!
//! All three includes live in this one module so the repo-root depth is
//! stated exactly once.
````

### B0698 · l.202–204 · REWRITE (amend) ·p · rule 1.5 · 3 → 3

````text
    /// Ordered check registry; order is doctor.sh's and load-bearing (section 3
    /// resolves bottle context later checks consume; run-only preflights last).
    /// Pinned by `sabrage-parity::tests::slug_coverage::doctor_slug_coverage_matches_the_contract`.
````

### B0705 · l.242–245 · REWRITE (confirm) · rule 1.5 · 4 → 4

````text
    /// The `DepotDownloader …` remedy string doctor's `game.present` row prints.
    ///
    /// Byte-identical to lib.sh's `DEPOT_CMD` / doctor.sh's `$DEPOT_CMD`, including
    /// the quoting of `-dir`; pinned by `tests::depot_command_matches_lib_sh`.
````

### B0708 · l.269–290 · REWRITE (amend) ·p · rule 1.3 · 22 → 14

````text
/// The `contract-sha256` of the contract **this binary was compiled from** —
/// the same `cat pipeline.toml runtime-template host-template | shasum -a 256`
/// recipe [`crate::util::contract_hash`] recomputes from `repo_root` on disk,
/// and the same value `scripts/demo/contract.gen.sh` records in its
/// `# contract-sha256:` header.
///
/// The on-disk half of `meta.contract-sync` only proves a checkout is
/// self-consistent: a binary built from checkout X, pointed at checkout Y via
/// `repo_root`, still executes **X's** registry, pins, ports, and templates.
/// The parity harness cannot see this either because tier 2 rebuilds the CLI
/// from the checkout it diffs. `meta.contract-sync` compares this value
/// against `util::contract_hash(repo_root)` to detect the skew; different is
/// a Fail, pinned by
/// `checks::meta::tests::fails_when_the_binary_was_compiled_from_a_different_contract`.
````

### B0709 · l.305–306 · REWRITE (confirm) · rule 1.3 · 2 → 2

````text
        // The include path is proven by compiling; these assertions pin the
        // values the rest of the crate hard-codes.
````

### B0710 · l.357–358 · REWRITE (confirm) · rule 2.3 · 2 → 1

````text
    /// The repo root — the checkout this binary was compiled from.
````

## `sabrage/crates/sabrage-core/src/error.rs`

### B0712 · l.1–11 · NEEDS-TEST (amend) · rule 1.7 · 11 → 6

test first: `display_matches_lib_sh_die_text` in `sabrage/crates/sabrage-core/src/error.rs (mod tests)` — Formatting SabrageError::Download and SabrageError::HashMismatch yields exactly "download failed: <url>" and "sha256 mismatch for <label> (got <hash>)", the two lib.sh fetch_pinned die strings.

````text
//! Error taxonomy (design-core §8).
//!
//! `Download` and `HashMismatch` keep `Display` byte-identical to `lib.sh`'s
//! `fetch_pinned` die text, so a front-end can branch on `kind()` while the
//! console text still matches the shell and the docs that quote it
//! (tests::display_matches_lib_sh_die_text).
````

### B0729 · l.127–148 · REWRITE (amend) · rule 1.2 · 22 → 11

````text
    /// Has this error's prose already reached the user as a `Fatal` row?
    ///
    /// True for the variants whose raiser emits the row and then returns the
    /// error, its callers propagating rather than re-emitting
    /// ([`crate::stages::StageCtx::fatal`], `privilege::upgrade_write_error`,
    /// `privilege::upgrade_child_write_error`, `privilege::elevate_osascript`),
    /// and for [`SabrageError::Cancelled`], where the user's own Stop or Ctrl-C
    /// is the report and the exit code (130) carries the fact. A front-end
    /// reports the error only when this is false — the CLI's final `error:`
    /// line — and the rule lives in core so the GUI can share it
    /// (tests::already_reported_covers_the_variants_that_emit_their_own_row).
````

## `sabrage/crates/sabrage-core/src/events.rs`

Deleted (nothing carried): B0736, B0743, B0751, B0775, B0816, B0819

### B0737 · l.47–52 · REWRITE (amend) · rule 1.6 · 6 → 5

````text
/// The five mutating pipeline stages.
///
/// `all` is deliberately absent: it is a caller-level loop over fresh contexts,
/// one per stage of [`Stage::ALL_CHAIN`], not a sixth stage. Doctor is absent
/// too — it is read-only and lives in [`crate::checks`].
````

### B0762 · l.260–283 · REWRITE (amend) ·p · rule 1.6 · 24 → 10

````text
    /// A raw `print -r --` line, reproduced **verbatim** — leading spaces and
    /// all, and possibly empty (`print ""`).
    ///
    /// [`StageEvent::Line`] cannot carry these: they fall outside `lib.sh`'s
    /// `info`/`ok`/`warn`/`fail` vocabulary, so a renderer prints them with no
    /// marker, colour, or indent. Source: scripts/demo/run.sh
    /// `print`/`print -r --` lines; the `# launch-action: launch-wine` block
    /// is the largest. Its `-- launching Beat Saber through the bridge` line is
    /// *not* a [`StageEvent::Section`]: the indented lines under it belong to
    /// the same block and the CLI reproduces it byte-for-byte.
````

### B0795 · l.464–467 · REWRITE (amend) · rule 1.6 · 4 → 3

````text
    /// The ordered preflight block: every `# preflight:` / `# preflight-warn:` /
    /// `# preflight-autofix:` tagged check of `scripts/demo/run.sh`, in that
    /// script's order.
````

## `sabrage/crates/sabrage-core/src/executor.rs`

Deleted (nothing carried): B0824, B0853, B0876, B0904, B0935

### B0821 · l.1–27 · REWRITE (confirm) ·p · rule 1.2 · 27 → 17

````text
//! Every mutating primitive, behind one trait — so `--dry-run` is a real
//! preview rather than a second, drifting code path (design-core §6.3).
//!
//! * [`RealExecutor`] does the thing.
//! * [`DryRunExecutor`] records a [`PlannedAction`] and does not. **Read-only
//!   probes still execute** — the byte compare behind `copy_if_changed`, the
//!   sha256 behind `download` — so the plan says *unchanged* vs *installed*
//!   truthfully instead of guessing.
//!
//! `setup` execs `curl -fL --retry 3` and `tar -xzf` rather than linking
//! `reqwest`/`flate2`/`tar`: same tool, same flags, same failure modes as
//! `scripts/demo/setup.sh`. [`Executor::dir_copy`] shells out to `/bin/cp -R`
//! because CrossOver's `lib/dxmt` tree contains symlinks a naive recursive
//! walk would dereference.
//!
//! Methods return a [`BoxFuture`] because [`crate::stages::StageCtx`] holds an
//! `Arc<dyn Executor>` and `async fn` in a trait is not object-safe.
````

### B0825 · l.53–67 · REWRITE (confirm) ·p · rule 1.6 · 15 → 6

````text
/// `install_if_changed`'s two branches (scripts/demo/lib.sh).
///
/// The **caller** prints the row: `info "unchanged: <dst>"` for
/// [`Copied::Unchanged`], `ok "installed: <dst>"` for [`Copied::Copied`], with
/// `<dst>` the full destination path. Keeping the strings at the call site is
/// what lets the dry run render "would install: …" from the same outcome.
````

### B0846 · l.126–127 · REWRITE (confirm) · rule 2.3 · 2 → 2

````text
    /// A child process would be spawned **detached** — surviving this process,
    /// with its output going to a log file or to `/dev/null`.
````

### B0847 · l.132–138 · REWRITE (confirm) · rule 1.2 · 7 → 6

````text
    /// One human line: what would happen, and why.
    ///
    /// Paths are rendered exactly as the executor received them (absolute,
    /// unabbreviated). A stage's narrative rows say `"install: <path>"` either
    /// way; only this line distinguishes *would copy* from *would skip because
    /// the bytes already match*, which is why the plan is recorded at all.
````

### B0858 · l.239–241 · REWRITE (confirm) · rule 2.3 · 3 → 4

````text
    /// `install_if_changed`: copy `src` over `dst` when the bytes differ; when
    /// only the mode differs, `dst`'s mode is repaired and the result is still
    /// [`Copied::Copied`]. Parent directories are **not** created (`cp` does
    /// not, and the shell `mkdir -p`s explicitly where it needs to).
````

### B0860 · l.249–264 · REWRITE (confirm) ·p · rule 1.2 · 16 → 13

````text
    /// Create `path` with `bytes` **only if it does not exist**: `Ok(true)`
    /// when this call created it, `Ok(false)` when something else got there
    /// first (and the file was left untouched).
    ///
    /// Write-once documents (`oxrsys-runtime.toml` above all) go through this
    /// rather than [`Executor::write_atomic`] because `exists()`-then-write is
    /// a race whose loser silently replaces a hand-edited config; an exclusive
    /// publish makes "did I create it?" the kernel's answer
    /// (tests::create_new_never_clobbers_an_existing_file).
    ///
    /// The default implementation is the racy check-then-write, kept only so a
    /// decorating [`Executor`] (the test doubles) inherits sane behaviour; both
    /// executors in this module override it.
````

### B0867 · l.323–335 · REWRITE (confirm) · rule 1.5 · 13 → 13

````text
    /// `lib.sh`'s `fetch_pinned`: skip when `dest` already hashes to `sha256`,
    /// else `curl -fL --retry 3` to `<dest>.tmp`, verify, rename.
    ///
    /// Unlike the other primitives this one **emits its own rows**, because they
    /// interleave with the transfer:
    ///
    /// * `info "already present: <label>"`,
    /// * `info "downloading <label> ..."` then curl's progress on stderr,
    /// * `ok "fetched <label> (sha256 verified)"`.
    ///
    /// Divergence: the `.tmp` file is removed when the download or the hash
    /// check fails (PARITY.md § Setup, "A pinned download's `.tmp` file is
    /// removed when curl or the sha256 check fails").
````

### B0871 · l.357–376 · REWRITE (confirm) ·p · rule 1.5 · 20 → 15

````text
    /// Spawn a **detached** child: its own process group, no pipes this
    /// process pumps, and — the whole point — `kill_on_drop(false)`, so it
    /// outlives Sabrage.
    ///
    /// Exactly two callers, both in the run stage: wine launch ([`DetachedStdio::LogFile`])
    /// and ALVR dashboard ([`DetachedStdio::Null`]). Everything else uses [`Executor::run_child`].
    ///
    /// Never [`crate::process::spawn_streamed`] for those two: its
    /// `kill_on_drop(true)` SIGKILLs the CrossOver wine wrapper the moment
    /// Sabrage quits, orphaning wineserver and the game (design-core §3.3;
    /// PARITY.md § Run (launch), "The wine child is spawned in its **own
    /// process group**").
    ///
    /// Returns `Ok(None)` under [`DryRunExecutor`] — a dry run never spawns —
    /// so callers must treat "no child" as the planned case, not as failure.
````

### B0879 · l.432–438 · REWRITE (confirm) · rule 1.7 · 7 → 6

````text
    /// Refuse to mutate anything once the run is cancelled.
    ///
    /// Every filesystem primitive calls this first, so cancellation lands
    /// between two file copies and not only at the next child-spawn boundary
    /// (`process::spawn_streamed`'s `select!`): install's layers 1-3 are dozens
    /// of copies with no child in between.
````

### B0880 · l.471–486 · REWRITE (confirm) ·p · rule 2.3 · 16 → 8

````text
                // Bytes match — but a staged file that lost its execute bit is
                // *not* installed, and rebuilding cannot repair it (bytes never
                // change; checks/build.rs requires the bit, fixes/helper.rs
                // restages through this primitive). Repair the mode and report
                // as work done, where lib.sh's `install_if_changed` would print
                // `unchanged: <dst>` (PARITY.md § Install (the one privileged
                // write), "`copy_if_changed` repairs the destination's mode
                // when the bytes already match").
````

### B0887 · l.726–728 · REWRITE (confirm) · rule 1.5 · 3 → 4

````text
    // Own process group, like every other child — but NOT kill_on_drop: a
    // detached child must survive this process exiting (design-core §3.3;
    // PARITY.md § Run (launch), "The wine child is spawned in its **own
    // process group**").
````

### B0897 · l.894–898 · REWRITE (amend) · rule 1.5 · 5 → 4

````text
    // The rename is only as durable as the directory entry it created, so the
    // parent is synced too — and a failure there is *reported*: the audio guard
    // acts on "persisted" for `session-state.json` by switching the Mac's
    // output device (tests::a_parent_that_cannot_be_synced_is_reported_not_swallowed).
````

### B0901 · l.937–948 · REWRITE (confirm) · rule 1.7 · 12 → 12

````text
/// Create `path` with `bytes` only if it does not exist: `Ok(true)` when this
/// call created it, `Ok(false)` when something else got there first.
///
/// The exclusive create happens on a **sibling temp** that is written, chmodded
/// and `fsync`ed first, and the final name is then claimed with `link(2)` —
/// which, like `O_EXCL`, refuses to replace an existing name, so "did I create
/// it?" is the kernel's answer rather than a stale `exists()`. Claiming the
/// final name before the bytes are written would let a SIGKILL or a power loss
/// strand an empty file that every later call answers `Ok(false)` for — and an
/// empty `oxrsys-runtime.toml` is *valid TOML*, so setup would treat a
/// zero-byte config as hand-edited content it must not overwrite
/// (tests::create_new_publishes_finished_bytes_and_leaves_no_temp).
````

### B0927 · l.1469–1474 · REWRITE (confirm) ·p · rule 1.2 · 6 → 3

````text
    /// r2:A2-4 regression: the write-once config is published whole or not at
    /// all. A crash strands a temp, never a zero-length `oxrsys-runtime.toml`
    /// that later runs read as hand-edited content they must not replace.
````

### B0929 · l.1492–1493 · REWRITE (confirm) ·p · rule 1.7 · 2 → 1

````text
        // A file already at the final name is never replaced, including an empty one.
````

### B0931 · l.1521–1522 · REWRITE (confirm) · rule 2.3 · 2 → 2

````text
        // The link holds the bytes that were live at that instant, even after
        // the linked-from name is replaced by an atomic rename.
````

## `sabrage/crates/sabrage-core/src/fixes/adb.rs`

Deleted (nothing carried): B0956, B0957, B0965

### B0944 · l.1–36 · REWRITE (amend) · rule 1.6 · 36 → 17

````text
//! `fix.remove-adb-forwards` — drop the two `--wired` stream port forwards.
//!
//! Leftover `tcp:9943`/`tcp:9944` forwards persist across sessions and silently
//! break WiFi discovery ("searching for streamer"), so a normal (non-wired) run
//! clears exactly those, per serial (`adb -s <serial> forward --remove`), never
//! with `--remove-all` — distinct from the `adb reverse --remove-all` that
//! run.sh does use: PARITY.md § Invariants that must NOT change (byte/behavior
//! parity), "adb `forward --remove` per-serial". The port pair comes from the
//! contract (`ports.stream`), never a literal; an absent `paths.adb` is
//! "nothing to do", not an error. Reference: `scripts/demo/run.sh`.
//!
//! Where the shell is silent about a removal that failed, a [`FixReport`] warns
//! and names what may still be installed: PARITY.md § Declared by the
//! 2026-08-30 adversarial review (round 1 fixes), "`net.adb-forwards` on a
//! failed probe". Enforced by
//! tests::removes_exactly_the_two_stale_ports_per_serial_never_remove_all and
//! tests::a_failed_removal_is_never_reported_as_a_clean_table.
````

### B0945 · l.47–50 · REWRITE (confirm) · rule 1.3 · 4 → 2

````text
/// Bound on `adb forward --list`: a cold run starts adb's background server and
/// can block for seconds. A timeout is a query failure, never an empty table.
````

### B0946 · l.53–60 · REWRITE (confirm) · rule 1.5 · 8 → 5

````text
/// This fix's own step id, used when it runs as a fix (doctor's fix list or
/// `fixes::apply`). The launch path must not use it: it passes
/// [`crate::events::step::RUN_ADB_FORWARDS`] to [`remove_adb_forwards_at`] so
/// its rows sort and group with the run stage's
/// (tests::the_step_id_is_the_fixs_own_by_default_and_the_callers_with_at).
````

### B0948 · l.78–92 · REWRITE (confirm) · rule 1.3 · 15 → 8

````text
/// `adb forward --list` as `(serial, local)` rows. A read-only query, so it
/// bypasses the executor: dry-run gating only matters for mutations.
///
/// # Errors
/// `Err(reason)` when the query could not be answered — a spawn failure,
/// [`ADB_LIST_TIMEOUT`], or a non-zero `adb`. Never folded into an empty list:
/// "adb could not tell us" and "there is nothing to clear" are different facts
/// (tests::a_query_failure_is_reported_as_a_query_failure).
````

### B0949 · l.114–126 · REWRITE (amend) ·p · rule 1.6 · 13 → 8

````text
/// run.sh's `info` row for one cleared forward. `verb` is `"cleared"` for a
/// real removal and `"would clear"` for a dry run (Sabrage-only; the shell has
/// no dry run) (tests::the_cleared_forward_line_is_the_one_renderer).
///
/// `pub` for A1-3, so `sabrage-parity` can pin this live literal rather than a
/// fragment copied into the parity crate: CI's tier 1 runs `-p sabrage-parity
/// -p sabrage-contract-gen` only, so this module's own frozen-text test does
/// not gate a native literal edited without touching run.sh.
````

### B0950 · l.134–135 · REWRITE (confirm) · rule 1.5 · 2 → 2

````text
/// The `tcp:<port>` local-forward specs this fix targets, from the contract's
/// `[ports] stream` — never a literal `"tcp:9943"`.
````

### B0951 · l.145–158 · REWRITE (amend) · rule 1.2 · 14 → 10

````text
/// Remove the stale `tcp:9943`/`tcp:9944` forwards, per serial, as the
/// standalone fix — every row stamped [`STEP`].
///
/// # Errors
/// Refuses while a session is live — duplicating [`crate::fixes::apply`]'s
/// gate, because this function is also reachable directly: a `--wired` session
/// streams over these very forwards, and doctor offers this remedy without
/// knowing the launch's `wired` state. The launch path calls
/// [`remove_adb_forwards_at`], which is not gated
/// (tests::the_standalone_fix_refuses_during_a_live_session_but_the_launch_path_does_not).
````

### B0952 · l.175–182 · REWRITE (confirm) ·p · rule 1.2 · 8 → 4

````text
/// The same removal as [`remove_adb_forwards`], stamped with a caller-supplied
/// step id and without the live-session gate: the launch path clears leftovers
/// before the session exists ([`crate::stages::run::actions::adb_forward_hygiene`];
/// tests::the_step_id_is_the_fixs_own_by_default_and_the_callers_with_at).
````

### B0954 · l.224–228 · REWRITE (confirm) · rule 1.5 · 5 → 3

````text
        // Never `--remove-all` here. A non-zero exit comes back `Ok`, matching
        // the shell's tolerant `&&`, but the pair is remembered so the report
        // cannot claim a clean table (tests::a_failed_removal_is_never_reported_as_a_clean_table).
````

### B0961 · l.500–501 · REWRITE (confirm) · rule 1.3 · 2 → 1

````text
        // #16c: one implementation serves both step ids.
````

### B0962 · l.534–536 · REWRITE (confirm) · rule 1.7 · 3 → 2

````text
    /// A removal that failed must never be reported like a clean forwarding
    /// table while the WiFi-breaking forward is still installed.
````

### B0964 · l.624–634 · REWRITE (confirm) · rule 1.7 · 11 → 6

````text
    /// An `adb` that cannot be spawned at all is the other half of "adb could
    /// not tell us": exactly one `warn` naming the query failure and the two
    /// ports that may still be installed, plus an `unchanged` report carrying
    /// the same text. Doctor records only `fatal` events and repaints the row
    /// from a fresh check pass, so this warn does not reach the GUI (A4-5;
    /// the UI half is `ui/src/screens/Doctor.svelte`).
````

## `sabrage/crates/sabrage-core/src/fixes/backend.rs`

Deleted (nothing carried): B0980, B0988, B0996, B1000, B1006

### B0969 · l.1–16 · REWRITE (amend) ·p · rule 1.6 · 16 → 9

````text
//! `fix.set-graphics-backend` — force `CX_GRAPHICS_BACKEND` to `dxmt` in the
//! bottle's `cxbottle.conf`.
//!
//! The CrossOver GUI writes `""` (= auto), which does not select DXMT: the game
//! spins forever before D3D11 device creation, with no DXMT banner and no
//! streamer (`docs/troubleshooting.md`). The edit is a permanent mutation that
//! is never unwound (design-core §3.2).
//!
//! Reference: `scripts/demo/run.sh`.
````

### B0974 · l.45–46 · REWRITE (confirm) · rule 1.6 · 2 → 1

````text
/// Which of [`rewrite_graphics_backend`]'s three branches it took.
````

### B0975 · l.49–50 · REWRITE (confirm) · rule 1.6 · 2 → 1

````text
    /// An existing `"CX_GRAPHICS_BACKEND" = "..."` line was rewritten in place.
````

### B0978 · l.59–91 · REWRITE (amend) ·p · rule 1.6 · 33 → 16

````text
/// Introduce `TARGET_LINE` into a `cxbottle.conf` body; returns the new bytes
/// and which branch produced them. Callers have established the line is not
/// present verbatim — this only decides how to introduce it.
///
/// Rewrite and insert branches are line-oriented: every untouched line and the
/// trailing-newline state survive. Append is a raw concatenation.
///
/// Like `sed`, the rewrite branch only touches a `CX_GRAPHICS_BACKEND` line
/// shaped `"CX_GRAPHICS_BACKEND" = "..."`; any other shape is left untouched
/// and the caller does not re-verify, so the result can lack the target line
/// (tests::branch_rewrite_cases).
///
/// Not byte-parity with BSD `sed` in one cell: with `[EnvironmentVariables]`
/// as the last line and no trailing newline, sed's `a\` concatenates onto the
/// header; this joins them with a real line break and keeps the newline absent
/// (tests::branch_rewrite_cases).
````

### B0979 · l.121–122 · REWRITE (confirm) · rule 1.3 · 2 → 1

````text
    // A raw append, indifferent to whether `conf` already ended in a newline.
````

### B0983 · l.151–155 · REWRITE (confirm) ·p · rule 1.4 · 5 → 4

````text
/// Every live process whose resolved executable equals `wineserver_exe`,
/// canonicalized on both sides like [`crate::process::find_processes_by_exe`]
/// (not reused: it skips `environ`, and a second full scan would repeat the
/// same syscalls).
````

### B0984 · l.163–165 · REWRITE (confirm) ·p · rule 1.4 · 3 → 2

````text
    // `System::new()` loads nothing; `new_with_specifics` would walk the whole
    // process table once more before the explicit scan below.
````

### B0985 · l.187–198 · REWRITE (confirm) ·p · rule 1.7 · 12 → 7

````text
/// The [`bottle_wineserver_is_live`] decision as a pure function of the
/// `WINEPREFIX` values observed on live wineserver processes; `None` (absent or
/// unreadable environment) cannot be ruled out and counts as live.
///
/// Separate from [`scan_wineservers`] so the "when in doubt, refuse" rule has a
/// test independent of system-wide process state
/// (tests::wineservers_indicate_live_decides_by_wineprefix).
````

### B0986 · l.206–218 · REWRITE (confirm) · rule 1.2 · 13 → 8

````text
/// Whether a CrossOver wineserver appears to be alive **for `bottle_prefix`
/// specifically**, read from process state because `wineserver -w` would block
/// indefinitely against a live server: every process whose resolved executable
/// is `wineserver_exe`, matched on its `WINEPREFIX` environment variable.
///
/// A matching process whose `WINEPREFIX` cannot be read, or that lacks it, is
/// treated as live: a false "clear" would let this fix edit a file the CrossOver
/// GUI still has open (tests::wineservers_indicate_live_decides_by_wineprefix).
````

### B0993 · l.257–274 · REWRITE (confirm) ·p · rule 1.6 · 18 → 9

````text
/// [`set_graphics_backend`] **without** the live-wineserver refusal — the
/// launch preflight's variant. Same three-branch rewrite, same `FixReport`,
/// same console text; only the liveness gate differs.
///
/// The refusal protects an edit that must survive alongside a running CrossOver.
/// A launch's edit does not: run.sh's `wineserver-reset` kills that wineserver
/// before anything reads the file again, so refusing here would block
/// `./demo.sh run` after a crashed session
/// (tests::for_launch_edits_even_while_the_bottles_wineserver_is_live).
````

### B0995 · l.315–323 · REWRITE (amend) · rule 1.5 · 9 → 3

````text
    // The sed-faithful rewrite branch can return bytes without the target line, so
    // verify it before writing — else the fix claims a success doctor still fails
    // (tests::a_line_the_rewrite_cannot_canonicalize_is_a_failure_not_a_success).
````

### B0997 · l.364–377 · REWRITE (confirm) · rule 1.7 · 14 → 8

````text
    /// Every branch of `rewrite_graphics_backend` over literal `cxbottle.conf`
    /// bodies: (label, input, expected branch, expected bytes out, whether the
    /// result contains the line doctor greps for).
    ///
    /// Two rows pin measured shell behaviour: a key line not shaped
    /// `"CX_GRAPHICS_BACKEND" = "..."` is left alone as sed would, so the anchor
    /// column is `false`; and the header-with-no-trailing-newline row is a
    /// deliberate improvement over BSD sed's `a\` (review finding #10).
````

### B0998 · l.449–460 · REWRITE (amend) · rule 1.4 · 12 → 4

````text
    // `scan_wineservers`'s OS-process scan is deliberately not exercised by
    // spawning a stand-in child: system-wide process state is not a fixture this
    // suite can pin down (cargo test runs in parallel; a sandboxed runner has
    // SIGKILLed a copied-to-`/tmp` executable before it could be scanned).
````

### B1003 · l.603–607 · REWRITE (confirm) · rule 1.7 · 5 → 4

````text
    /// A `CX_GRAPHICS_BACKEND` line the anchored rewrite cannot touch (an
    /// unquoted value, unusual spacing) is a failure through both doors: the
    /// fix dies with run.sh's post-fix text instead of reporting "forced to
    /// dxmt" over bytes that still lack the target line.
````

### B1005 · l.698–705 · REWRITE (amend) ·p · rule 1.3 · 8 → 5

````text
        // Stand in for a live wineserver with this test binary's own process —
        // guaranteed alive, nothing to spawn (the trick
        // `process::tests::finds_this_test_binary_by_its_exe_path` uses). A
        // `cargo test` process normally lacks `WINEPREFIX`, hitting
        // `wineservers_indicate_live`'s "cannot rule this one out" branch.
````

## `sabrage/crates/sabrage-core/src/fixes/helper.rs`

Deleted (nothing carried): B1013, B1016, B1020

### B1008 · l.1–46 · REWRITE (confirm) ·p · rule 1.6 · 46 → 15

````text
//! `fix.restage-helper` — stage the native-arm64 encoder helper from
//! `build-helper-arm64` next to the runtime dylib in `build-x64`.
//!
//! Returns unchanged when the staged copy is already arm64, changed once
//! restaged, or fails when neither copy nor build output is arm64. The x86_64
//! runtime finds the helper beside its own dylib, so a swept staged copy
//! silently downgrades to in-process H.264.
//!
//! Only `build-helper-arm64` builds the helper; the staged copy is validated
//! at its destination (arm64, executable):
//! tests::a_byte_identical_but_non_executable_staged_helper_is_repaired.
//!
//! Reference: `scripts/demo/run.sh`'s `ensure_helper_staged`. The shell skips
//! that function for `encoder_process=inproc`; this fix always attempts the
//! restage regardless of launch context.
````

### B1009 · l.55–60 · REWRITE (amend) ·p · rule 1.2 · 6 → 2

````text
/// Step id for this fix's [`crate::events::StageEvent::Line`] rows; the copy
/// itself uses [`step::BUILD_HELPER`], since restaging is that step's purpose.
````

### B1010 · l.63–75 · REWRITE (amend) ·p · rule 1.2 · 13 → 8

````text
/// `$ENCODER_PROC` as the *runtime* resolves it, not as the shell's `awk`
/// recipe does.
///
/// [`crate::config::runtime_toml`]'s reader is table-blind and last-assignment-wins,
/// matching `ext/oxrsys/runtime/src/Config.cpp`, so the die text names the value
/// the runtime will actually use. Returns empty when no usable assignment exists,
/// including values the runtime would ignore
/// (tests::parse_encoder_process_follows_the_runtime_semantics).
````

### B1011 · l.85–93 · REWRITE (amend) ·p · rule 1.5 · 9 → 7

````text
/// `${ENCODER_PROC:-auto}` — an empty read (no `encoder_process` key, no file,
/// or a value the runtime would ignore) falls back to `"auto"`, like the shell
/// parameter expansion (tests::encoder_process_or_default_falls_back_to_auto).
///
/// This reader and the doctors' `awk` emulation disagree on unquoted values:
/// PARITY.md § Declared by the 2026-08-30 adversarial review (round 1 fixes),
/// "Config readers: doctor emulates `awk`, launch uses the runtime's semantics."
````

### B1014 · l.148–154 · REWRITE (amend) · rule 1.3 · 7 → 4

````text
        // Unreachable unless the destination changed since the arm64 probe:
        // `copy_if_changed` reports a mode-only repair as `Copied`, not as
        // unchanged (tests::a_byte_identical_but_non_executable_staged_helper_is_repaired).
        // The `unchanged:` row is `install_if_changed`'s, reproduced regardless.
````

### B1015 · l.174–178 · REWRITE (confirm) · rule 1.3 · 5 → 3

````text
    // A dry run never wrote the file, so re-validating it would always look
    // like a failed restage; the plan records the work instead, including a
    // mode-only repair (tests::a_dry_run_plans_the_mode_repair_without_performing_it).
````

### B1017 · l.233–235 · REWRITE (confirm) · rule 1.7 · 3 → 2

````text
        // The runtime is table-blind and last-assignment-wins, so a shadowed
        // earlier line is NOT the value the launched runtime uses.
````

### B1024 · l.378–382 · REWRITE (confirm) ·p · rule 1.7 · 5 → 4

````text
    /// A byte-identical staged copy that lost its execute bit is repaired:
    /// `copy_if_changed`'s byte compare cannot see the mode, so without the
    /// repair re-validation fails on every retry and neither this fix nor
    /// `./demo.sh build` can recover.
````

### B1025 · l.438–441 · REWRITE (confirm) · rule 2.3 · 4 → 3

````text
    /// The same state under a dry run: the staged file is untouched, the mode
    /// repair appears in the plan as a `Copy` rather than a skip, and the report
    /// says a restage *would* happen.
````

## `sabrage/crates/sabrage-core/src/fixes/mod.rs`

### B1027 · l.1–47 · REWRITE (confirm) ·p · rule 1.2 · 47 → 21

````text
//! The fix registry: the small set of mutations offered as remedies.
//!
//! A doctor row never mutates anything ([`crate::checks`]); when its remedy is
//! mechanically applicable the contract names a fix id, the GUI turns it into a
//! button, and the launch preflight applies it for `autofix`-gated checks.
//!
//! [`FixAction`]'s serde spelling is the contract id without the `fix.` prefix,
//! which lives in one constant ([`CONTRACT_FIX_PREFIX`]).
//! [`DEFERRED_CONTRACT_FIX_IDS`] are contract ids this crate never offers;
//! tests::every_contract_fix_id_is_modelled_or_explicitly_deferred pins that
//! set exactly, so a new contract fix cannot disappear silently.
//!
//! Every action runs behind [`crate::stages::OPERATION_LOCK`] and mutates state
//! a live session depends on. [`apply`] takes the lock and refuses a live
//! session both before and after the wait, using persistent identity rather than
//! this process's handle
//! (tests::a_queued_fix_is_refused_when_a_session_goes_live_during_the_wait).
//! [`apply_holding_lock`] is for callers that already hold the lock:
//! `tokio::sync::Mutex` is not reentrant (silent deadlock). It skips the
//! liveness check because the launch preflight edits while a stale wineserver
//! is still alive.
````

### B1029 · l.63–78 · REWRITE (amend) · rule 1.7 · 16 → 12

````text
/// Contract fix ids this crate does not offer as a button, sorted.
///
/// * `fix.create-z-drive` - creating `dosdevices/z:` in a bottle, reachable
///   only from `bottle.zdrive`, which no gate auto-fixes; no [`FixAction`]
///   variant models it.
/// * `fix.delete-session-json` - modelled ([`FixAction::DeleteSessionJson`])
///   but withheld: deleting the file leaves the client at an 800x900 black
///   screen ([`crate::fixes::session_json`]), and the working recovery is to
///   edit the pinned IP in place, which the Settings screen's config editor
///   does. Returning `None` here keeps the destructive button off the Doctor
///   row; the action stays reachable from `sabrage fix delete-session-json`
///   for a user who has read [`FixDef::consequence`].
````

### B1031 · l.85–86 · REWRITE (amend) · rule 2.3 · 2 → 2

````text
    /// Force `"CX_GRAPHICS_BACKEND" = "dxmt"` in the bottle's `cxbottle.conf`
    /// (`bottle.gfx-dxmt`). CrossOver's "auto" silently breaks DXMT.
````

### B1034 · l.94–100 · REWRITE (amend) ·p · rule 2.3 · 7 → 8

````text
    /// Delete ALVR's `session.json` to clear stale manual IP pins.
    ///
    /// **Known-bad remedy.** Deleting the file leaves the client at an 800x900
    /// black screen; editing the pinned IPs in place is the working recovery.
    /// Modelled because `cfg.session-pins` names it, marked `destructive` so
    /// it never runs unconfirmed
    /// (tests::the_known_bad_session_json_deletion_documents_its_outcome), and
    /// withheld from every Doctor button by [`DEFERRED_CONTRACT_FIX_IDS`].
````

### B1035 · l.102–109 · REWRITE (confirm) ·p · rule 1.2 · 8 → 5

````text
    /// Set `protocol = "alvr"` in `oxrsys-runtime.toml`
    /// (`cfg.protocol.supported` / `cfg.protocol.legacy-oxrsys`).
    ///
    /// The only fix that writes the runtime config; delegates to
    /// [`crate::config::runtime_toml::write`].
````

### B1043 · l.171–179 · REWRITE (confirm) ·p · rule 1.3 · 9 → 8

````text
    /// Is this action's contract id one of the deliberately withheld ones
    /// ([`DEFERRED_CONTRACT_FIX_IDS`])?
    ///
    /// The [`FixAction`]-shaped form of the withheld set: the Tauri `fix`
    /// command needs it to refuse an action the registry withholds from the
    /// GUI, however the frontend arrived at it (A4-2 - the TypeScript mirror
    /// of the fix table can offer a button [`FixAction::from_contract_id`]
    /// would not). Pinned by tests::is_deferred_is_exactly_the_withheld_set.
````

### B1060 · l.366–395 · REWRITE (amend) ·p · rule 1.2 · 30 → 21

````text
/// Apply one fix, taking [`crate::stages::OPERATION_LOCK`] for its duration.
///
/// The public door for the CLI and the Tauri `fix` command: a fix serializes
/// against stages and other Sabrage processes' operations. `sink` is a separate
/// parameter so a run preflight can stream fix rows into the run's event channel
/// while carrying the run's own [`StageCtx`].
///
/// The row carrying `ctx.run_id` is emitted before the cancellable wait, and
/// refusals are re-run once the lock is in hand: a `run` that won the lock race
/// publishes its live session and hands the lock back at its launch boundary.
/// See tests::{a_queued_fix_carries_its_run_id_and_cancels_out_of_the_wait,
/// a_queued_fix_is_refused_when_a_session_goes_live_during_the_wait}.
///
/// **A caller that already holds the lock must call [`apply_holding_lock`]** -
/// `tokio::sync::Mutex` is not reentrant.
///
/// # Errors
///
/// [`SabrageError::Cancelled`] when the wait is cancelled; a fatal error when a
/// session is live or the checkout is not the one this binary was built from;
/// otherwise whatever the action itself fails with.
````

### B1063 · l.429–437 · REWRITE (confirm) · rule 1.4 · 9 → 5

````text
/// Enforce [`FixDef::forbidden_while_session_live`] for the GUI/CLI door.
///
/// Checked **before** the lock, because the operation lock is deliberately free
/// for the whole of a live session (`stages`' "Lock policy for `run`"): waiting
/// on it would say nothing about whether a session is running.
````

### B1064 · l.457–469 · REWRITE (confirm) ·p · rule 1.2 · 13 → 12

````text
/// [`apply`] for a caller that already holds [`crate::stages::OPERATION_LOCK`] -
/// the shape a launch preflight needs, taking the lock once and auto-fixing what
/// its `autofix`-gated checks reported.
///
/// `tokio::sync::Mutex` is not reentrant, so whole-stage fixes delegate to
/// [`crate::stages::run_stage_holding_lock`] (not [`crate::stages::run_stage`],
/// which would deadlock); they still stream like a user-initiated stage. See
/// tests::apply_holding_lock_runs_a_stage_fix_under_a_lock_the_caller_holds.
///
/// The delegation is boxed so the future's size stays independent of the stage
/// layer and a fix->stage->fix cycle is a runtime recursion rather than an
/// unsized-future compile error.
````

### B1070 · l.544–547 · REWRITE (confirm) · rule 1.7 · 4 → 3

````text
    /// `apply` is the public entry point, so it must serialize against a stage:
    /// a Doctor "Fix" button cannot be allowed to rewrite `cxbottle.conf` or
    /// restage the helper while an install is halfway through.
````

### B1084 · l.811–817 · REWRITE (confirm) ·p · rule 1.7 · 7 → 6

````text
    /// The refusal is re-run with the operation lock in hand.
    ///
    /// The window: a fix admitted while idle waits behind another operation;
    /// a `run` acquires first, publishes its live session and hands the lock
    /// back at its launch boundary - leaving `remove-adb-forwards` free to
    /// delete the very forwards the `--wired` stream is running over.
````

## `sabrage/crates/sabrage-core/src/fixes/session_json.rs`

Deleted (nothing carried): B1105

### B1094 · l.1–38 · REWRITE (confirm) ·p · rule 1.2 · 38 → 20

````text
//! `fix.delete-session-json` — delete ALVR's `session.json` to clear stale
//! manual client IP pins.
//!
//! Known-bad remedy, marked `destructive`: deleting the file has been observed
//! to leave the client at an 800x900 black screen; editing the pinned IPs in
//! place is the recovery that works. Listed in
//! [`crate::fixes::DEFERRED_CONTRACT_FIX_IDS`], so no GUI Doctor row offers a
//! button for it.
//!
//! Backs [`crate::paths::Paths::alvr_session_json`] up under
//! `ctx.paths.sabrage_appsup`'s `backups/`, routes every write and the removal
//! through [`crate::executor::Executor`], treats an absent file as
//! [`FixReport::unchanged`], and refuses while any CrossOver wineserver is
//! alive — `session.json` is machine-global, so there is no per-bottle
//! narrowing as in [`crate::fixes::backend`] (`backend::any_wineserver_alive`
//! is this crate's one home for wineserver-liveness scanning). See
//! tests::{deletes_after_backing_up_and_reports_the_backup_location,
//! refuses_while_any_wineserver_is_alive}. No shell equivalent: `run.sh`
//! never deletes this file, and no message text here is a verbatim shell
//! string.
````

### B1095 · l.52–66 · REWRITE (amend) ·p · rule 1.2 · 15 → 13

````text
/// Remove ALVR's `session.json` (after backing it up).
///
/// Backup and removal both go through [`crate::executor::Executor`], so
/// mutation is decided by the executor, never by `ctx.opts.dry_run`: a preview
/// context built with [`StageCtx::with_executor`] plans both. See
/// tests::a_preview_executor_beats_opts_dry_run_false. The removal uses
/// `Executor::remove_file`, not `remove_dir_all` on `alvr/`, which would take
/// the trusted-client state with it. A dry run says what it *would* do and
/// still reports `changed`.
///
/// # Errors
/// Fails when `ctx.paths.wineserver` is known and any CrossOver wineserver is
/// alive, and on a read, backup, or removal I/O failure.
````

### B1097 · l.93–96 · REWRITE (amend) · rule 1.3 · 4 → 3

````text
    // `ctx.paths.sabrage_appsup`, not the global `sabrage_support_dir()`: the
    // field exists so a caller can redirect Sabrage's own store away from the
    // real `$HOME` without mutating the process environment.
````

### B1098 · l.130–142 · REWRITE (confirm) · rule 1.2 · 13 → 11

````text
/// Write the backup under a name **no existing backup owns**, and return it.
///
/// The `session.json.<secs>` suffix is whole seconds, so two deletions inside
/// one second collide; [`Executor::create_new`] (`O_EXCL`) makes the kernel
/// allocate the `-2`, `-3`, … name instead of a probe another writer can win,
/// so the earlier backup — the one that recovers this fix's known-bad outcome
/// — is never replaced. See
/// tests::a_second_deletion_in_the_same_second_does_not_overwrite_the_first_backup.
///
/// # Errors
/// Propagates the executor's write failures.
````

### B1100 · l.185–191 · REWRITE (confirm) · rule 1.3 · 7 → 2

````text
    /// A ctx whose OXRSys store **and** Sabrage store live under a scratch
    /// dir — never the real `~/Library/Application Support`.
````

### B1103 · l.313–317 · REWRITE (amend) · rule 1.7 · 5 → 3

````text
    /// Two deletions inside one wall-clock second: the second backup must not
    /// overwrite the first. Restoring that backup is the documented recovery
    /// from this fix's own known-bad outcome.
````

### B1106 · l.361–365 · REWRITE (confirm) · rule 1.7 · 5 → 3

````text
    /// The mutation decision belongs to the executor, never to `opts.dry_run`:
    /// with a preview executor and `dry_run: false` this fix deleted
    /// `session.json` for real while its backup was only planned.
````

### B1107 · l.419–423 · REWRITE (confirm) · rule 1.3 · 5 → 3

````text
        // Stand in for a live wineserver with this test binary's own running
        // process, as `fixes::backend`'s equivalent test does:
        // `any_wineserver_alive` ignores WINEPREFIX, so no spawn is needed.
````

## `sabrage/crates/sabrage-core/src/lib.rs`

### B1108 · l.1–64 · REWRITE (amend) ·p · rule 1.7 · 64 → 20

````text
//! `sabrage-core`: the UI-agnostic native pipeline engine behind Sabrage.
//!
//! An independent implementation of the zsh pipeline (`demo.sh` +
//! `scripts/demo/*.sh`, which stays the reference). They meet at two places:
//!
//! 1. `contract/pipeline.toml` — pins, depot triple, port lists, DXMT artifact
//!    set, and ordered check/launch-action registries. Parsed directly here
//!    ([`contract`]); the shell reaches it via generated
//!    `scripts/demo/contract.gen.sh`. Registry order and slug coverage pinned by
//!    `checks::tests::registry_binds_in_contract_order_and_covers_every_slug`.
//! 2. Byte-shared on-disk artifacts — the host OpenXR manifest and the
//!    `oxrsys-runtime.toml` first-write template, rendered from
//!    `contract/*.template` by both sides ([`util`]). Manifest bytes pinned by
//!    `stages::install::tests::layer_four_stages_the_host_manifest_file_form_byte_for_byte`.
//!
//! Deliberate divergences live in `sabrage/PARITY.md`.
//!
//! Checks are read-only. Every mutation goes through [`executor::Executor`], so
//! `--dry-run` is the same code path with one implementation swapped rather than
//! a second, drifting one.
````

## `sabrage/crates/sabrage-core/src/logs.rs`

Deleted (nothing carried): B1110, B1114, B1121, B1160, B1163, B1169, B1171, B1172, B1177, B1179

### B1109 · l.1–41 · REWRITE (confirm) ·p · rule 1.6 · 41 → 22

````text
//! Log files: naming the wine console log, tailing the three live sources, and
//! listing past runs.
//!
//! Reference: scripts/demo/run.sh, which names the log with `date
//! +%Y%m%d-%H%M%S` and pipes the child through `tee`.
//!
//! The name is local civil time, which is why this crate depends on `chrono`
//! at all: `std::time` has no calendar. Sabrage diverges twice — a same-second
//! name collision gets a `-2`, `-3`, ... suffix (detected by opening the file
//! `create_new` in [`crate::executor::Executor::spawn_detached`], never
//! assumed) and the child writes into the file descriptor directly instead of
//! through `tee`, which can lose its last buffer when the pipeline is torn
//! down (PARITY.md § Run (launch), "The wine console log is a plain file the
//! child's stdout/stderr are redirected into").
//!
//! [`Tailer`] is rotation-aware: a new inode, a size below the cursor, or a
//! prefix that mismatches the bytes last read from it (in-place
//! `truncate(true)` that grew back past the cursor between two polls, which is
//! how ALVR rewrites `session_log.txt`) each mean the file was replaced, and
//! the tailer reopens from the start and says so ([`LogBatch::rotated`]).
//! Splitting reuses [`crate::process::ChunkSplitter`] rather than a second
//! copy of the same `\n`/`\r`/`\r\n` rule.
````

### B1111 · l.55 · REWRITE (confirm) · rule 2.3 · 1 → 2

````text
/// Filename prefix shared with run.sh: every wine console log is
/// `beatsaber-<stamp>.log`.
````

### B1112 · l.58–70 · REWRITE (confirm) ·p · rule 1.3 · 13 → 10

````text
/// The candidate path for attempt `attempt` of this launch's console log,
/// given an already-formatted `YYYYmmdd-HHMMSS` stamp.
///
/// * `attempt == 0` -> `beatsaber-YYYYmmdd-HHMMSS.log`, byte-identical to the
///   shell's name for the same instant;
/// * `attempt == n >= 1` -> the same name with `-{n+1}` before `.log`, i.e.
///   `beatsaber-20260829-101112-2.log` for the first collision.
///
/// `stamp` is a plain string so no caller needs a date/time dependency just to
/// name a candidate (F16, tests::wine_log_candidate_delegates_to_the_stamped_form_byte_for_byte).
````

### B1124 · l.160–168 · REWRITE (confirm) · rule 1.2 · 9 → 8

````text
    /// **The lines in THIS batch begin a new incarnation of the file** — it
    /// was replaced (new inode), truncated, or rewritten in place. The UI
    /// clears its buffer on this and then appends `lines`.
    ///
    /// It is a property of the batch, not of the poll: a rotation detected
    /// while earlier lines are still queued is announced on the first batch
    /// that carries bytes from the reopened file (A8-4,
    /// tests::a_backlog_queued_before_a_vanish_survives_reappearance_uncut).
````

### B1129 · l.189–198 · REWRITE (confirm) ·p · rule 1.7 · 10 → 9

````text
/// Bound on the bytes ONE [`Tailer::poll`] reads, whatever has accumulated in
/// the file since the last one.
///
/// [`MAX_LINES_PER_POLL`] caps what a poll *delivers*, not what it reads:
/// without this bound, opening a large `--verbose` wine console log (or an
/// `oxrsys-runtime.log` at its 5 MiB rotation size) from offset 0 would
/// materialise the whole file as `String`s in one call. The cursor and
/// splitter survive to the next poll, so a backlog drains across polls
/// (tests::one_poll_reads_at_most_the_byte_budget_and_still_delivers_every_line).
````

### B1130 · l.201–202 · REWRITE (confirm) · rule 2.3 · 2 → 2

````text
/// One `read` inside [`Tailer::poll`]'s read loop. Small enough that the line
/// cap is noticed promptly, large enough that a 1 MiB budget is 16 syscalls.
````

### B1136 · l.235–236 · REWRITE (confirm) · rule 2.3 · 2 → 2

````text
    /// `\n`/`\r`/`\r\n`-tolerant splitter; its internal buffer *is* the
    /// partial final line that must never reach a caller as a complete one.
````

### B1139 · l.245–253 · REWRITE (confirm) · rule 1.5 · 9 → 7

````text
    /// How many of the leading entries in `pending` were read out of a
    /// **previous** incarnation of the path (A8-4).
    ///
    /// The backlog goes out under `rotated: false` and `drain_capped` never
    /// lets one batch straddle the boundary, so a consumer that clears on
    /// `rotated` cannot label the old file's last lines as the new one's first
    /// (tests::a_backlog_queued_before_a_vanish_survives_reappearance_uncut).
````

### B1145 · l.341–345 · REWRITE (confirm) · rule 1.3 · 5 → 3

````text
                    // `splitter` still holds any genuine trailing partial
                    // line — left in place so the very next `poll()` completes
                    // it, exactly like an ordinary mid-stream partial.
````

### B1146 · l.365–379 · REWRITE (amend) ·p · rule 1.2 · 15 → 16

````text
    /// Read whatever has arrived since the last poll.
    ///
    /// Only complete lines are delivered; a trailing partial line stays
    /// buffered until its newline arrives or it outgrows
    /// [`MAX_UNTERMINATED_LINE_BYTES`]. A file that was replaced, truncated,
    /// or rewritten in place is reopened from the start and the batch says so
    /// ([`LogBatch::rotated`]); a vanished file yields whatever was already
    /// queued and is picked up on the next poll.
    ///
    /// One call reads at most [`POLL_BYTE_BUDGET`] bytes and stops early once
    /// [`MAX_LINES_PER_POLL`] lines are queued; the remainder arrives on later
    /// calls.
    ///
    /// # Errors
    ///
    /// I/O failures other than the file being absent, which is not an error.
````

### B1149 · l.427–441 · REWRITE (amend) · rule 1.7 · 15 → 5

````text
            // Backlog from the previous incarnation goes out under `rotated:
            // false`; under `true` the consumer clears and then reads it as
            // the new file's first lines, misattributing a startup failure
            // (A8-4,
            // tests::a_backlog_queued_before_a_vanish_survives_reappearance_uncut).
````

### B1153 · l.516–524 · REWRITE (amend) · rule 1.4 · 9 → 4

````text
        // Bracket the read with the same witness: a truncate-and-regrow
        // landing between the precheck and here would read as a continuation
        // and skip the new file's whole prefix for good (A8-7,
        // tests::a_rewrite_landing_inside_the_read_window_is_not_read_as_a_continuation).
````

### B1154 · l.536–540 · REWRITE (confirm) ·p · rule 1.7 · 5 → 4

````text
            // Undo the read: those bytes belong to an incarnation this
            // tailer cannot place. Dropping `open` makes the next poll a fresh
            // open from byte 0, and the deferred marker (A8-4) makes that
            // batch the one that announces the rotation.
````

### B1155 · l.560–563 · REWRITE (confirm) · rule 1.3 · 4 → 3

````text
        // A rotation detected on an earlier poll is announced HERE: with
        // `carry` down to zero, this batch is the first whose lines really do
        // begin the new incarnation. The marker is deferred, never dropped.
````

### B1165 · l.694–697 · REWRITE (confirm) · rule 1.4 · 4 → 4

````text
    /// Called by [`Tailer::poll`] between its read loop and its post-read
    /// continuity check — the exact window a truncate-and-regrow has to land
    /// in to be read as a continuation (A8-7). Nothing outside `cfg(test)` can
    /// register anything, and the default is a no-op.
````

### B1170 · l.745–749 · REWRITE (confirm) · rule 1.3 · 5 → 4

````text
        // Both entry points, one expectation: the stamped form emits these
        // paths for a hand-built stamp with no chrono in sight (F16), and the
        // chrono wrapper delegates to it byte for byte, so neither can drift
        // alone.
````

### B1201 · l.1434–1435 · REWRITE (confirm) · rule 1.7 · 2 → 2

````text
        // The next poll reopens from byte 0 and delivers the new file
        // entire, prefix included.
````

## `sabrage/crates/sabrage-core/src/paths.rs`

Deleted (nothing carried): B1211, B1248, B1249

### B1202 · l.1–18 · REWRITE (amend) ·p · rule 1.6 · 18 → 10

````text
//! The typed port of `scripts/demo/lib.sh`'s derived-path block.
//!
//! Two shell traps are closed at the type level: `cx`/`wine`/`wineserver` are
//! `None` when CrossOver is absent, where lib.sh builds the bogus absolute
//! `/Contents/SharedSupport/CrossOver` from an unset `CX_APP`; `adb` is `None`,
//! where lib.sh leaves `ADB=""` and every call site guards with `[ -n "$ADB" ]`.
//!
//! Sabrage's binary can sit anywhere, so [`Paths`] takes `repo_root` as explicit
//! input (persisted in Sabrage settings). Changing it invalidates install state:
//! the host OpenXR manifest embeds the absolute dylib path under it.
````

### B1203 · l.25–35 · REWRITE (confirm) · rule 1.5 · 11 → 5

````text
/// `$HOME`, or `/` if the environment has none.
///
/// Read-only probes only: with an empty `HOME` every derived path comes out
/// *relative* to the working directory, so a stage that writes must go through
/// [`home_dir_checked`] (tests::home_is_required_to_be_absolute_and_non_empty).
````

### B1207 · l.78–86 · REWRITE (amend) · rule 1.1 · 9 → 6

````text
/// `~/Library/Application Support/Sabrage` — Sabrage's own store.
///
/// GUI-only state lives here and **never** in the repo or in OXRSys's
/// directory (CLAUDE.md, "Sabrage ⇄ demo.sh parity"), so nothing under it is
/// a parity artifact. [`crate::privilege::sabrage_support_dir`] is a thin
/// alias for this.
````

### B1214 · l.135–159 · REWRITE (confirm) ·p · rule 1.7 · 25 → 16

````text
/// Resolve the wine-vr checkout root: `override_root` when the caller has one
/// (CLI `--repo-root`, the GUI's persisted `settings.repo_root`), else a
/// non-empty `SABRAGE_REPO_ROOT`, else the first ancestor of
/// `std::env::current_exe()` holding both [`REPO_ROOT_MARKERS`].
///
/// The first two are normalized logically ([`logical_absolute`]) to match
/// demo.sh's `ROOT="$(cd "$(dirname "$0")" && pwd)"`: canonicalizing would make
/// the front-ends write different host-manifest bytes and thrash with sudo (A2-6;
/// tests::a_relative_root_becomes_absolute_without_resolving_symlinks).
///
/// An explicit root skips [`REPO_ROOT_MARKERS`] validation so scratch and fixture
/// trees work (tests::explicit_override_wins_and_is_canonicalized).
///
/// # Errors
///
/// Fatal when `current_exe()` fails, or no ancestor holds both markers and no override was given.
````

### B1215 · l.189–208 · REWRITE (amend) ·p · rule 2.3 · 20 → 6

````text
/// The working directory the way `pwd` prints it: `$PWD` when it really names
/// this process's cwd, `getcwd()` otherwise, `None` when neither is available.
///
/// `$PWD` is the *logical* cwd (`pwd -L`, symlinks intact) that demo.sh's
/// `ROOT` is built from; `getcwd()` is the physical one. A GUI-launched `.app`
/// or cargo test binary can inherit a stale `PWD`, hence the dev/ino check.
````

### B1220 · l.261–265 · REWRITE (amend) · rule 1.4 · 5 → 2

````text
/// First ancestor of `start` (excluding `start` itself, which is an executable
/// path, not a directory) that contains both [`REPO_ROOT_MARKERS`].
````

### B1254 · l.448–454 · REWRITE (amend) ·p · rule 1.6 · 7 → 4

````text
    /// `<root>/logs` — where `run` writes `beatsaber-<ts>.log`.
    ///
    /// Reference: `scripts/demo/run.sh`. Both front-ends write here, so the Logs
    /// screen lists the shell's past runs too. Gitignored on purpose.
````

### B1270 · l.582–589 · REWRITE (confirm) · rule 1.5 · 8 → 7

````text
/// Resolve the Beat Saber directory the way lib.sh/doctor.sh do:
/// `${WINEVR_BS_DIR:-$PREFIX/drive_c/Program Files (x86)/Steam/steamapps/common/<bs_dir_leaf>}`.
///
/// With no bottle and no override the shell's empty-`$PREFIX` quirk is
/// reproduced on purpose: the absolute-looking `/drive_c/Program Files
/// (x86)/...` string appears in remedy text and must match
/// (tests::bs_dir_override_wins_and_default_uses_the_contract_leaf).
````

## `sabrage/crates/sabrage-core/src/privilege.rs`

Deleted (nothing carried): B1283, B1295, B1310, B1316, B1322, B1333, B1340, B1349, B1361, B1366, B1370, B1371, B1376, B1379, B1391, B1394, B1396, B1399

### B1282 · l.1–81 · REWRITE (amend) · rule 1.2 · 81 → 31

````text
//! The privilege boundary — the pipeline's one privileged write.
//!
//! Install layer 4 (`/usr/local/share/openxr/1/active_runtime.x86_64.json`,
//! `root:wheel 0644`) is the only privileged write in the pipeline. Everything
//! else, including install layers 1-2 inside `CrossOver.app`, is a plain user
//! write; those need macOS App Management (TCC), which `sudo` cannot grant —
//! conflating the two is the classic mistake here, see [`classify_write_error`].
//!
//! The content written is [`crate::util::host_manifest_file_bytes`] verbatim,
//! and the write is skipped entirely when
//! [`crate::util::host_manifest_is_current`] already holds, which is what keeps
//! `./demo.sh install` and Sabrage from re-prompting after each other
//! (PARITY.md § Invariants that must NOT change (byte/behavior parity),
//! "host-manifest bytes + skip-when-current").
//! The JSON never rides on a command line: it is staged to a private temp file
//! that the elevated `install -m 0644 -o root -g wheel` copies into place.
//! The mechanism and the alternatives weighed against it: design-core
//! § 5. Privilege boundary.
//!
//! The staging file is the soft spot of that scheme, so it lives under
//! `~/Library/Application Support/Sabrage/tmp/` (mode `0700`) and never `/tmp`,
//! is created `O_CREAT|O_EXCL` mode `0600` under a random name, and is removed
//! on every exit path except a cancellation, which leaves it for
//! [`sweep_stale_staging`] ([`StagedTemp`]).
//!
//! [`AdminMethod::detect`] picks `sudo` whenever a controlling terminal is
//! reachable and never consults stdout, so `sabrage install | tee log` prompts
//! in the terminal exactly like `./demo.sh install | tee`; a GUI-launched `.app`
//! has no controlling terminal and takes the osascript path. The two `die`
//! strings on the `sudo` path are install.sh's, verbatim.
//! Reference: scripts/demo/install.sh.
````

### B1286 · l.109–111 · REWRITE (amend) · rule 2.3 · 3 → 3

````text
/// What [`StageEvent::NeedsAdmin`] says before the osascript dialog appears, so
/// the user can predict the dialog and knows it will not return on every install
/// (design-core § 5. Privilege boundary, implementation step 4).
````

### B1287 · l.115–123 · REWRITE (confirm) · rule 1.5 · 9 → 6

````text
/// The same announcement for the [`AdminMethod::Sudo`] path, which prompts on
/// the controlling terminal, not in a macOS dialog.
///
/// It says where the prompt is because a GUI started from a shell inherits that
/// terminal and the password prompt can sit behind the window the user is
/// looking at (tests::the_announcement_names_the_method_detect_actually_picked).
````

### B1289 · l.135–139 · REWRITE (amend) · rule 1.5 · 5 → 3

````text
/// The `--dry-run` stand-in for the prompt: a dry run must never emit
/// [`StageEvent::NeedsAdmin`], which the GUI renders as "macOS will ask for your
/// password" (PARITY.md § CLI / GUI, "A dry run's layer 4 prints").
````

### B1292 · l.150–157 · REWRITE (confirm) · rule 1.5 · 8 → 6

````text
/// How long a cancellation waits for the elevated child to actually exit before
/// giving up on it.
///
/// Bounded on purpose: past `sudo`'s exec the child's real uid is 0 and an
/// unprivileged `kill(2)` comes back `EPERM`, so an unbounded wait would never
/// finish (tests::a_cancelled_child_is_reaped_before_the_call_returns).
````

### B1304 · l.211–218 · REWRITE (confirm) · rule 1.5 · 8 → 6

````text
/// The decision, as a pure function of the two probes.
///
/// Deliberately not a function of stdout: `sabrage install | tee log` has a
/// non-tty stdout and a perfectly good terminal to prompt on, and either probe
/// alone is sufficient
/// (tests::admin_method_is_decided_by_the_controlling_terminal_never_by_stdout).
````

### B1305 · l.228–233 · REWRITE (confirm) · rule 1.3 · 6 → 4

````text
/// True when this process has a controlling terminal — the thing `sudo` prompts
/// on. Opening `/dev/tty` is the portable test: it is the controlling terminal
/// by definition, and the open fails with `ENXIO` when there is none (a
/// GUI-launched `.app`, a daemon, a session-leaderless child).
````

### B1311 · l.257–267 · REWRITE (confirm) · rule 1.5 · 11 → 10

````text
/// Escape a string for embedding inside an AppleScript string literal.
///
/// `do shell script "…"` nests two levels of quoting — AppleScript's own literal
/// wrapping a `/bin/sh` command line — and getting either wrong is command
/// injection as root, not a cosmetic bug
/// (tests::nasty_paths_round_trip_through_both_quoting_layers).
///
/// Newline, carriage return and tab are emitted as their AppleScript escapes
/// because a raw newline inside an AppleScript literal is a syntax error
/// (tests::applescript_escape_covers_every_special).
````

### B1313 · l.302–307 · REWRITE (confirm) · rule 1.2 · 6 → 4

````text
/// The `/bin/sh` command the authorization dialog runs: `mkdir -p` of the
/// destination's directory and `install -m 0644 -o root -g wheel` of the staging
/// file, joined with `&&` so there is exactly one prompt
/// (tests::the_command_creates_the_directory_and_installs_root_wheel_0644).
````

### B1315 · l.330–336 · REWRITE (confirm) · rule 1.2 · 7 → 7

````text
/// The exact argv of every child the elevation runs, in order.
///
/// One vector for [`AdminMethod::Osascript`], two for [`AdminMethod::Sudo`]
/// (`sudo mkdir -p …`, then `sudo install …`), mirroring install.sh's two sudo
/// calls and its two `die` strings
/// (tests::elevation_argv_is_one_osascript_or_install_shs_two_sudo_calls).
/// Reference: scripts/demo/install.sh.
````

### B1317 · l.370–377 · REWRITE (confirm) · rule 1.2 · 8 → 7

````text
/// The exact bytes install layer 4 stages and installs: the rendered manifest
/// plus the single trailing newline `print -- "$WANT"` appends
/// ([`crate::util::host_manifest_file_bytes`]).
///
/// Named so tests and `sabrage-parity`'s golden pin the byte source of the
/// privileged write directly, rather than the util helper the write path might
/// not call.
````

### B1318 · l.382–409 · REWRITE (amend) ·p · rule 1.6 · 28 → 28

````text
/// Write the host OpenXR manifest for `oxr_dylib`, prompting for authorization
/// only if needed.
///
/// Returns [`PrivilegedWrite::Skipped`] without prompting when `dest` is already
/// current ([`crate::util::host_manifest_is_current`], install.sh's own currency
/// test, so neither front-end re-prompts after the other ran),
/// [`PrivilegedWrite::Planned`] under `--dry-run`, and
/// [`PrivilegedWrite::Written`] only after `dest` has been re-read and
/// byte-compared. Reference: scripts/demo/install.sh.
///
/// The parameter is the dylib path, never pre-rendered content: the comparison
/// form ([`crate::util::render_host_manifest`]) and the file form differ by
/// exactly one byte, and a signature accepting either lets a caller write a
/// manifest that differs from `./demo.sh install`'s
/// (PARITY.md § Invariants that must NOT change (byte/behavior parity),
/// "the host manifest's bytes end in install.sh's trailing newline").
///
/// The elevated child is the one mutation that does not go through
/// [`crate::executor::Executor`]: the osascript branch needs its stderr captured
/// to tell "declined" from "failed" and the sudo branch needs the process's own
/// tty, neither of which `spawn_streamed` gives. A dry run never reaches it
/// (tests::a_dry_run_plans_the_staging_write_and_the_elevated_argv).
///
/// # Errors
///
/// [`SabrageError::AdminDeclined`] for a declined prompt (the UI offers the
/// paste-this-in-Terminal fallback), [`SabrageError::Cancelled`], and a fatal
/// error when the destination does not match the intended bytes.
````

### B1321 · l.433–437 · REWRITE (confirm) · rule 1.5 · 5 → 4

````text
    // Nothing below this line is undoable from our side: past `osascript`'s or
    // `sudo`'s exec we cannot signal the elevated child at all, so a run
    // cancelled during layer 3 must not surface an authorization prompt
    // (tests::a_cancelled_run_neither_announces_nor_stages_the_privileged_write).
````

### B1325 · l.463–467 · REWRITE (confirm) · rule 1.5 · 5 → 3

````text
        // A cancelled elevation is the one case where the privileged command
        // may still be reading this file, so it is kept rather than unlinked
        // (tests::a_defused_staging_file_outlives_its_drop).
````

### B1326 · l.479–485 · REWRITE (confirm) · rule 1.2 · 7 → 6

````text
/// The `--dry-run` half of [`write_host_manifest_privileged`]: record the
/// staging write and the elevated argv through the executor, mutate nothing.
///
/// Reported as [`PrivilegedWrite::Planned`], never `Written`: a preview that
/// prints the row a real install prints is indistinguishable from completed work
/// in the event log.
````

### B1327 · l.506–512 · REWRITE (amend) · rule 2.3 · 7 → 8

````text
/// One `osascript -e 'do shell script … with administrator privileges'`.
///
/// Exit status alone cannot tell a declined dialog from a failed command — both
/// are non-zero — so the `(-128)` marker in stderr is what separates
/// [`SabrageError::AdminDeclined`] from a real failure; everything else on
/// stderr is surfaced as [`StageEvent::Output`] rather than swallowed
/// (tests::declined_and_failed_authorization_are_told_apart_by_stderr;
/// design-core § 6, "No swallowed diagnostics").
````

### B1328 · l.546–556 · REWRITE (confirm) · rule 1.6 · 11 → 8

````text
/// install.sh's sudo path, structurally unchanged, with
/// `install -m 0644 -o root -g wheel` in place of `sudo tee` — same bytes, mode
/// and ownership, without the JSON ever crossing a pipe or a command line
/// (PARITY.md § Install (the one privileged write), "The elevated write is").
/// Reference: scripts/demo/install.sh.
///
/// Both children inherit this process's stdio so `sudo`'s own password prompt
/// works, which is why they cannot go through `spawn_streamed`.
````

### B1331 · l.595–611 · REWRITE (amend) ·p · rule 1.2 · 17 → 14

````text
/// Refuse a dylib path the host manifest cannot represent.
///
/// install.sh's two `${//}` substitutions (mirrored exactly by
/// [`crate::util::json_escape_string`]) escape `\` and `"` and nothing else, so
/// a path holding a control character renders a manifest that is not JSON —
/// installed `root:wheel` over the file the OpenXR loader reads
/// (tests::a_control_character_in_the_path_is_refused_before_any_prompt).
///
/// Native-only divergence: install.sh writes the invalid bytes; every path
/// without a control character passes through unchanged.
///
/// # Errors
///
/// Fatal when the path contains a control character.
````

### B1336 · l.658–663 · REWRITE (amend) · rule 1.5 · 6 → 4

````text
            // Bounded moment for the child to finish: returning the instant
            // Stop is pressed would tear the staging file down under a live
            // privileged `install` (CANCEL_REAP_GRACE; run_inheriting's twin
            // arm is pinned by tests::a_cancelled_child_is_reaped_before_the_call_returns).
````

### B1339 · l.690–695 · REWRITE (confirm) · rule 1.7 · 6 → 4

````text
                // Signal, then reap — bounded, and the kill is best effort:
                // after `sudo` execs, its real uid is 0 and this process cannot
                // signal it at all
                // (tests::a_cancelled_child_is_reaped_before_the_call_returns).
````

### B1343 · l.716–721 · REWRITE (amend) · rule 1.5 · 6 → 8

````text
    /// Create `dir` (mode `0700`) and a fresh randomly-named `0600` file in it
    /// holding `content`.
    ///
    /// `create_new` is the load-bearing flag: it fails rather than opening a
    /// path something else pre-created, so with the random name the bytes root
    /// installs can only have come from us
    /// (tests::staging_creates_the_directory_0700_and_never_reuses_a_name,
    /// tests::staged_temp_is_0600_and_deletes_itself).
````

### B1346 · l.759–765 · REWRITE (confirm) · rule 1.3 · 7 → 6

````text
/// Remove staging files a previous run left behind (see [`StagedTemp::defuse`]).
///
/// Only files older than [`STAGING_SWEEP_AGE`] go, so a concurrent sabrage's
/// live staging file is never taken out from under its own elevated write.
/// Best effort throughout: a file we cannot remove is a stale `0600` file in our
/// own `0700` directory, not a reason to fail an install.
````

### B1350 · l.801–810 · REWRITE (confirm) · rule 1.5 · 10 → 11

````text
/// Classify an `io::Error` from a write to `path`.
///
/// The `.app` test is on the path, not the error: telling the user to try `sudo`
/// under `…/CrossOver.app/…` sends them down a road with no end
/// (tests::write_errors_classify_by_errno_and_path).
///
/// [`WriteErrorKind::TccAppManagementLikely`] is a hypothesis, never a diagnosis
/// — macOS reports a TCC refusal as a plain `EPERM`, indistinguishable from a
/// genuine mode problem — so every string built from it says "likely" and offers
/// the Terminal fallback
/// (tests::the_app_management_strings_stay_a_hypothesis_with_a_way_out).
````

### B1351 · l.821–838 · REWRITE (confirm) · rule 1.2 · 18 → 11

````text
/// Turn a plain [`SabrageError::Io`] from install layers 1-2 into the App
/// Management error when the path and errno say that is the likely cause,
/// emitting the explanation as a [`StageEvent::Fatal`] on the way out; anything
/// else passes through untouched. Use it at the call site of a write inside
/// `CrossOver.app`.
///
/// The returned error keeps its own `Display` (`kind() == "tcc_denied"`, which
/// the GUI's permission panel branches on) and the prose the user reads travels
/// in the emitted event, so a caller that gets `TccDenied` back must propagate
/// it rather than emit a second `Fatal`
/// (tests::only_a_tcc_shaped_io_error_is_upgraded).
````

### B1352 · l.856–866 · REWRITE (confirm) · rule 1.2 · 11 → 9

````text
/// [`upgrade_write_error`] for a failure that arrives as a child's exit status
/// rather than an `io::Error` — install layer 1's `cp -R` of the stock DXMT
/// tree, the first write the pipeline makes into `CrossOver.app`.
///
/// There is no errno to classify, only the child's output tail, so the test is
/// "destination inside a `.app` and the tail says permission denied". Same
/// contract as [`upgrade_write_error`] otherwise: the prose is emitted here,
/// once, and the caller propagates the returned [`SabrageError::TccDenied`]
/// (tests::a_refused_cp_into_a_bundle_is_upgraded_like_a_refused_write).
````

### B1355 · l.904–909 · REWRITE (confirm) · rule 1.5 · 6 → 7

````text
/// The grant flow (deep link + the relaunch requirement) and the Terminal
/// fallback, on one `remedy:` line.
///
/// The relaunch note is load-bearing: macOS only applies a TCC grant to a
/// process started after it was given, so without it the user's experience is
/// "granted, still broken"
/// (tests::the_app_management_strings_stay_a_hypothesis_with_a_way_out).
````

### B1359 · l.942–948 · REWRITE (confirm) · rule 1.7 · 7 → 6

````text
/// Sabrage's own support directory, `~/Library/Application Support/Sabrage` —
/// where the temp file for the privileged write is staged (never `/tmp`: a
/// world-writable staging path for a root-installed file is a swap race).
///
/// One implementation, in [`crate::paths::sabrage_support_dir`]: two spellings
/// of one path is how the two front-ends of one store drift apart.
````

### B1360 · l.961–964 · REWRITE (confirm) · rule 1.5 · 4 → 3

````text
    // No test here may execute `osascript` or `sudo`: every test exercises a
    // pure function, stages a file in a fixture directory, or drives the
    // dry-run path, which records argv through the executor and spawns nothing.
````

### B1380 · l.1356–1361 · REWRITE (confirm) · rule 1.2 · 6 → 4

````text
    /// The cancel arm must not return while the child it just signalled is
    /// still running: the caller's next act is to drop the staging file, and a
    /// privileged `install` reading it would get an ENOENT half-way through the
    /// pipeline's only root write.
````

### B1381 · l.1374–1376 · REWRITE (amend) ·p · rule 1.4 · 3 → 3

````text
        // Cancelled *after* the child is up: an already-cancelled token never reaches
        // the spawn (see tests::an_already_cancelled_token_never_spawns_the_elevated_child),
        // so pre-cancelling here would prove nothing about the reap.
````

### B1386 · l.1469–1470 · REWRITE (confirm) · rule 1.2 · 2 → 1

````text
    /// How many `host-manifest-*.json` are staged right now. Read-only.
````

### B1387 · l.1486–1492 · REWRITE (amend) ·p · rule 1.2 · 7 → 5

````text
    /// install.sh's escaping is two substitutions, so a path with a raw control
    /// character renders non-JSON over the root:wheel file the OpenXR loader
    /// reads. The guard fails closed, before the currency test and before any
    /// prompt; `json_escape_string` stays those same two substitutions, so every
    /// accepted path renders byte-identically on both front-ends.
````

### B1388 · l.1511–1512 · REWRITE (confirm) · rule 1.7 · 2 → 1

````text
        // …and an ordinary path is untouched by the guard.
````

## `sabrage/crates/sabrage-core/src/process.rs`

Deleted (nothing carried): B1409, B1428, B1440, B1450, B1461, B1476, B1487, B1502, B1509

### B1406 · l.1–29 · REWRITE (confirm) ·p · rule 1.7 · 29 → 28

````text
//! Child processes: spawn, stream, cancel, and the reap primitive.
//!
//! # Scope warning - build tools only
//!
//! [`spawn_streamed`] sets `kill_on_drop(true)` and assigns its own process
//! group for tree-wide cancellation. That is correct for `git`, `cmake`,
//! `ninja`, `cargo`, `curl`, `tar`, `adb`, `wineserver` - and **wrong for the
//! wine launch**: cancelling `run` means `wineserver -k` plus a bounded `-w`
//! wait (never SIGKILL), and the log is a file fd, not a pipe. The run stage
//! therefore builds its own `tokio::process::Command` with `kill_on_drop(false)`
//! and a file-fd redirect ([`crate::executor::Executor::spawn_detached`]); it
//! must not call [`spawn_streamed`].
//!
//! # Output splitting
//!
//! Chunks are delimited by **both** `\n` and `\r`, with `\r\n` counted once, so
//! curl's progress bar and cargo's status line arrive as successive chunks
//! instead of one enormous line at EOF ([`ChunkSplitter`]).
//!
//! # Reaping
//!
//! [`find_processes_by_exe`] matches on the **resolved executable path**, not on
//! an argv substring. `lib.sh`'s `reap_stray` uses `pgrep -f "$path"` /
//! `pkill -f`, which matches any process that merely mentions the path on its
//! command line. Declared divergence - PARITY.md
//! § Stop, "Each reap (leftover encoder helper, leftover ALVR dashboard)
//! matches by"; design-core §10.7 - and the GUI shows what will be killed
//! before killing it.
````

### B1429 · l.187–199 · REWRITE (amend) ·p · rule 1.7 · 13 → 12

````text
/// How a chunk was terminated - the byte(s) a faithful passthrough has to put
/// back.
///
/// A progress writer's `\r` and a line's `\n` are not interchangeable: printing
/// a repaint with `println!` turns curl's one self-overwriting line into
/// hundreds of permanent ones, and appending a newline to the final
/// unterminated chunk invents output the child never wrote
/// (tests::chunks_carry_their_terminator).
///
/// `Lf` is [`Default`] so [`crate::events::StageEvent::Output`]'s
/// `#[serde(default)]` `end` field reads as a plain newline-terminated line
/// for consumers that omit it (A14-3).
````

### B1433 · l.212–223 · REWRITE (confirm) · rule 1.2 · 12 → 12

````text
/// Splits a byte stream into chunks on `\n` **and** `\r`, counting `\r\n` once.
///
/// Progress-bar writers (curl, cargo, git) repaint by emitting `\r`; a plain
/// line splitter would buffer the entire download into a single chunk delivered
/// at EOF. Empty chunks are preserved (a blank line in build output is real
/// output), except for the phantom one `\r\n` would otherwise produce.
///
/// [`ChunkSplitter::push_with`] additionally reports each chunk's [`ChunkEnd`].
/// A `\r`-terminated chunk is delivered only once the byte behind it - or
/// [`ChunkSplitter::finish`] - has settled bare-CR versus CRLF: delivery
/// timing differs, the chunk sequence does not
/// (tests::chunks_carry_their_terminator).
````

### B1443 · l.304–306 · REWRITE (confirm) · rule 1.1 · 3 → 3

````text
    // `false`: no caller of `spawn_streamed` reads the tail, so the pumps skip
    // cloning every chunk of a chatty build tool into a buffer nobody looks at
    // (tests::spawn_streamed_does_not_populate_a_tail_nobody_reads).
````

### B1452 · l.437–444 · REWRITE (confirm) · rule 1.5 · 8 → 3

````text
            // A descendant that ignored the SIGTERM and redirected its own
            // stdout/stderr survives both the leader's exit and pipe EOF, so
            // liveness is measured on the group (tests::cancellation_escalates_when_a_descendant_outlives_the_leader).
````

### B1453 · l.455–459 · REWRITE (confirm) · rule 1.5 · 5 → 3

````text
    // The pipes stay open while any descendant still holds them (wine's
    // `reg add` leaves wineserver behind), so waiting for EOF unconditionally
    // would hang the stage (tests::a_backgrounded_descendant_does_not_wedge_the_stage).
````

### B1454 · l.462–465 · REWRITE (confirm) · rule 1.5 · 4 → 3

````text
            // Belt and braces: the group looked empty (or unsignalable) while
            // something still holds the pipe. Escalate on the group, not the
            // leader (tests::cancellation_kills_a_descendant_that_ignored_term_and_released_the_pipes).
````

### B1455 · l.469–472 · REWRITE (amend) · rule 1.5 · 4 → 3

````text
        // Whatever still holds the pipe outlives us: stop reading rather than wedge
        // the operation lock. No SIGKILL on the uncancelled path - that survivor is
        // usually one the pipeline wanted, e.g. wineserver (tests::a_backgrounded_descendant_does_not_wedge_the_stage).
````

### B1460 · l.537–543 · REWRITE (confirm) · rule 1.5 · 7 → 3

````text
    // A14-3: `end` carries each chunk's terminator so a byte-faithful renderer
    // repaints a `\r` chunk in place instead of `println!`-ing curl's and
    // cargo's progress spam (tests::output_events_carry_their_chunk_terminator).
````

### B1462 · l.573–579 · REWRITE (confirm) · rule 1.5 · 7 → 8

````text
/// One matched process.
///
/// Serializable because it is the **process identity** persisted in
/// `session-state.json` ([`crate::session::state::SessionState`]): `pid` alone
/// cannot say whether the wine process running under that number is *the* one
/// this Sabrage launched, and signalling a recycled pid is the one
/// unrecoverable mistake the reconcile path can make
/// (tests::identity_rejects_a_recycled_pid_and_the_unobservable_fallback).
````

### B1467 · l.615–630 · REWRITE (confirm) ·p · rule 1.2 · 16 → 14

````text
    /// Is the process this identity names **still the same process**?
    ///
    /// True iff the pid is alive *and* reports the same `start_time` as when
    /// spawned. Pids wrap; start times do not.
    ///
    /// `exe` is deliberately **not** compared: CrossOver's `wine` launcher
    /// `exec`s into the real loader, so a live session's `exe` path changes a
    /// few hundred milliseconds after launch while pid and start time stay
    /// constant.
    ///
    /// A `start_time` of 0 is the "could not observe at spawn" fallback
    /// [`crate::executor::Executor::spawn_detached`] records; it never equals
    /// a real start time, so such an identity always reports `false`
    /// (tests::identity_rejects_a_recycled_pid_and_the_unobservable_fallback).
````

### B1469 · l.657–668 · REWRITE (confirm) · rule 1.7 · 12 → 8

````text
/// True when any element of `cmd` - or the whitespace-joined command line as a
/// whole, the shape `pgrep -f` scans - contains `needle`.
///
/// Wine puts the game's own path on the command line as a single `Z:\...`
/// Windows-path argument (e.g. `Z:\repo\…\Beat Saber.exe`), so per-element and
/// whole-line matching agree in the real case; both are checked so a
/// hypothetical split across two argv elements still matches
/// (tests::cmdline_matching_is_the_pgrep_f_shape).
````

### B1470 · l.673–679 · REWRITE (amend) ·p · rule 1.5 · 7 → 7

````text
/// Every running process whose command line matches [`cmdline_contains`] for
/// `needle`, pid-ordered - the argv-based match `pgrep -f` performs, unlike
/// [`find_processes_by_exe`]'s exact exe-path equality (used by the reap
/// steps). Opposite trade-offs on purpose - see this module's header and
/// PARITY.md § Stop, "The Beat Saber survivor probe scans live processes'
/// argv". A single-needle convenience over [`ProcessScan::scan`] - see
/// [`find_processes_by_exe`]'s doc for when a caller should scan once instead.
````

### B1471 · l.684–689 · REWRITE (confirm) · rule 1.7 · 6 → 5

````text
/// One full-table process scan, with both `exe` and `cmd` refreshed, kept
/// around so a caller that needs several different needles against the same
/// instant pays for exactly one process walk: `stop`'s survivor check, each of
/// its two reap steps, and its foreign-helper scan share one [`ProcessScan`]
/// instead of walking the process table four times.
````

### B1479 · l.791–823 · REWRITE (confirm) ·p · rule 1.7 · 33 → 28

````text
/// Run a **read-only probe** and capture both pipes.
///
/// For run-stage probes whose *output* is the point and whose effect is nil:
/// `adb devices`, `adb forward --list`, `SwitchAudioSource -a -t output`,
/// `SwitchAudioSource -c -t output`. Streaming those as [`StageEvent::Output`]
/// would print machine-readable noise the shell never prints.
///
/// "Effect is nil" has one asterisk: the first `adb` after a reboot forks the
/// adb *server*, so `--dry-run` can leave a server behind the plan does not
/// mention. `demo.sh` does the same, so this is parity.
///
/// **Mutations must never come through here.** `adb forward`,
/// `adb forward --remove`, `adb reverse --remove-all`,
/// `SwitchAudioSource -t output -s …`, `wineserver -k` and every other write go
/// through [`crate::executor::Executor::run_child`], which is what makes
/// `--dry-run` plan them instead of performing them. A probe routed through
/// this function is invisible to the plan - correct for a probe, a silent
/// dry-run hole for anything else.
///
/// stdin is `/dev/null`, and `spec.env_path` is applied so a Finder-launched
/// `.app` still finds `adb` (see [`default_child_path`]).
///
/// **Bounded.** A wedged `adb` would otherwise hold the operation lock with
/// Cancel unable to interrupt it, so a probe gets [`DEFAULT_PROBE_TIMEOUT`] and
/// its own process group; on expiry the group is killed and the probe fails like
/// a missing binary (`kind() == "io"`), which every caller already handles
/// (tests::a_probe_that_never_answers_times_out_instead_of_hanging). Use
/// [`capture_with`] to attach the operation's cancellation token.
````

### B1493 · l.1065–1071 · REWRITE (amend) · rule 1.7 · 7 → 4

````text
    /// A14-3 pins that `StageEvent::Output` carries each chunk's
    /// [`ChunkEnd`] rather than computing and discarding it: a `\r` progress
    /// repaint, a `\n`-terminated line and the final unterminated chunk
    /// ([`ChunkEnd::Eof`]) must stay distinguishable downstream.
````

### B1496 · l.1167–1170 · REWRITE (amend) · rule 1.7 · 4 → 4

````text
    /// The SIGKILL escalation watches the process *group*, not the leader: a
    /// descendant that ignores SIGTERM (an ignored disposition survives
    /// `exec`) keeps the pipes open long after the leader is reaped, so
    /// `spawn_streamed` must not block on that EOF.
````

### B1503 · l.1333–1336 · REWRITE (confirm) · rule 1.7 · 4 → 4

````text
    /// `find_processes_by_exe`/`find_processes_by_cmdline` are thin wrappers
    /// over one [`ProcessScan`]; a caller sharing a scan across several
    /// needles (`stages::stop`) must see exactly the same matches the
    /// single-needle convenience functions would.
````

### B1504 · l.1347–1355 · REWRITE (confirm) · rule 1.3 · 9 → 6

````text
        // The convenience functions run their **own** scan, so a difference is
        // either a filter disagreement (what this test is about) or the process
        // table changing between the two walks (what it is not): under `cargo
        // test` every child a concurrent test spawns is briefly a copy of this
        // exe, between fork and exec. So the comparison is only made across a
        // window two bracketing scans agree on, and there the filters must match.
````

## `sabrage/crates/sabrage-core/src/session/mod.rs`

Deleted (nothing carried): B1515, B1536, B1547, B1554, B1576, B1586, B1613

### B1514 · l.1–34 · REWRITE (amend) ·p · rule 1.2 · 34 → 10

````text
//! Session state has three surfaces: a status other screens read
//! ([`SessionStatus`], broadcast on `session://status`), an in-process handle
//! for stop and detach ([`LiveSessionHandle`]), and a record written to disk
//! before each guarded mutation ([`state::SessionState`]) so a `SIGKILL` or
//! power loss still leaves enough to restore the Mac's audio device.
//!
//! The handle's two tokens are independent on purpose: `cancel` is the
//! INT/TERM path (stop wine, release every guard); `detach` leaks the guards
//! so the session keeps running (PARITY.md § Session (detach / reconcile)).
//! See tests::the_two_tokens_are_independent.
````

### B1516 · l.52–58 · REWRITE (amend) · rule 1.2 · 7 → 5

````text
/// Where a session is in its life.
///
/// Derived from several signals at once, never from one
/// ([`watcher::SessionMonitor::snapshot`]): `runtime_status.json` **freshness** counts,
/// its mere existence does not — the file outlives the process.
````

### B1525 · l.81–91 · REWRITE (confirm) ·p · rule 1.2 · 11 → 7

````text
    /// A session **nothing in Sabrage started**: no live handle, no
    /// `session-state.json`, but `runtime_status.json` is fresh and the
    /// process it names is alive — a `demo.sh run` in another terminal.
    /// Reporting it as [`SessionPhase::Idle`] invites a second launch over a
    /// live game. Derived conservatively and never from freshness alone
    /// ([`watcher::SessionMonitor::snapshot`]); carries only the runtime's
    /// pid, because nothing else is knowable from here.
````

### B1526 · l.95–102 · REWRITE (confirm) · rule 1.7 · 8 → 7

````text
/// The encoder configuration one session actually negotiated.
///
/// Parsed from the oxrsys log line
/// `OXRSys/ALVR: encoder ready {w}x{h} @{hz}Hz {mbps}Mbps ({codec}, {path})`,
/// e.g. `… (HEVC, native helper)` or `… (H.264, in-process)`. The `path` half
/// is the operational one: `in-process` means the arm64 helper did not take
/// and the session downgraded to Rosetta H.264.
````

### B1544 · l.203–208 · REWRITE (confirm) ·p · rule 1.2 · 6 → 5

````text
/// The live session's run id, without cloning the rest of the handle.
///
/// For callers that only compare identities — a detach poll, a reconcile —
/// without cloning two [`CancellationToken`]s, a [`PathBuf`] and a [`String`].
/// See tests::the_live_slot_is_set_and_cleared_by_run_id.
````

### B1548 · l.236–251 · REWRITE (confirm) ·p · rule 1.2 · 16 → 11

````text
/// The run stage's own account of where a launch currently is, with the
/// identity that goes with it.
///
/// [`SessionPhase::Preflight`], [`SessionPhase::Launching`] and
/// [`SessionPhase::Stopping`] exist only in [`crate::stages::run`]'s head,
/// so publishing them here lets [`watcher::SessionMonitor::snapshot`] report
/// a launch in progress instead of "No session".
///
/// A phase always carries its `run_id` and `bottle` in one value: without
/// them it is a Stop button that cannot be wired up. See
/// tests::the_run_phase_slot_carries_identity_and_clears_only_for_its_own_run.
````

### B1550 · l.262–271 · REWRITE (confirm) ·p · rule 1.2 · 10 → 7

````text
/// The at-most-one phase the run stage is currently reporting.
///
/// `None` means the run stage has nothing to say: `snapshot()` falls back to
/// [`LIVE_SESSION`] and `session-state.json`. Not serialized — a launch that
/// outlives this process is described by `session-state.json` instead.
/// `std::sync::Mutex` for the same reason [`LIVE_SESSION`] is one: every
/// access is a short get/set, never held across an `.await`.
````

### B1555 · l.307–338 · REWRITE (amend) ·p · rule 1.6 · 32 → 12

````text
/// Why a mutating operation must not start right now, or `None` when nothing
/// on this machine looks like a live session.
///
/// Seven signals, cheapest first, and deliberately **not** just the in-process
/// [`live_session`] slot: the session a Settings save or a Doctor fix would
/// break may belong to the other front-end, to an earlier run of this process,
/// or to `./demo.sh run`, none of which publish anything here (A4-1). See
/// tests::ensure_idle_refuses_for_every_source_that_can_know_about_a_session.
///
/// A live CrossOver `wineserver` is deliberately **not** one of the seven —
/// it is alive for any CrossOver app; the two fixes whose file a CrossOver
/// process can clobber keep narrower probes.
````

### B1559 · l.358–366 · REWRITE (amend) · rule 1.7 · 9 → 9

````text
/// [`live_session_reason`] with the bottle kept.
///
/// **The** live-session predicate: every "not while the game is running" door
/// — [`crate::stages::live_session_block`] (stage refusals and every gated
/// Doctor fix), [`crate::config::blocking_session`] (the Settings writer) and
/// `store::goldberg`'s revert — goes through this one function, so a state
/// that blocks one of them blocks all of them. A door with its own weaker copy
/// is how a `./demo.sh run` session gets its `steam_api64.dll` replaced
/// underneath it (A13a-2).
````

### B1561 · l.438–444 · REWRITE (confirm) · rule 1.3 · 7 → 3

````text
        // A record that exists but will not parse may still be describing a
        // live session, and the question every caller is asking is "may I
        // overwrite something a running game has open" — so refuse.
````

### B1562 · l.460–463 · REWRITE (confirm) · rule 2.3 · 4 → 3

````text
            // Reuse [`watcher::runtime_status_live`] so this door and the
            // phase the Session screen renders cannot disagree — otherwise
            // Sabrage says "No session running" while Settings refuses (A10-8).
````

### B1565 · l.500–513 · REWRITE (confirm) · rule 1.2 · 14 → 11

````text
/// The pid of a running Beat Saber, if there is one.
///
/// The signal none of the others can replace: a `./demo.sh run` writes no
/// session record and publishes no handle, and its runtime does not write
/// `runtime_status.json` until *streaming* begins, so every file-based signal
/// reads idle for the minutes between the wine spawn and the first status
/// (A13a-2). See tests::a_running_game_is_a_live_session_even_with_nothing_on_disk.
///
/// Limitation: the window *before* the wine spawn — Goldberg is installed
/// several steps earlier — stays open; closing it needs a marker both
/// front-ends take (contract + `scripts/demo/run.sh` + here).
````

### B1566 · l.520–531 · REWRITE (amend) · rule 1.2 · 12 → 10

````text
/// Refuse `action` while any session is live — the single policy every "not
/// while the game is running" caller shares, over [`live_session_reason`]'s
/// seven signals. The error carries `stop.sh`'s own remedy, so a GUI caller
/// renders the same row the config and doctor refusals render.
///
/// This form reads the machine's real support directories: both paths it
/// needs derive from `$HOME`, so the empty repo root below is never
/// consulted. Anything that already has a [`crate::paths::Paths`] — and every
/// test — must call [`ensure_idle_in`] instead, or it consults the
/// developer's own session.
````

### B1579 · l.631–647 · REWRITE (confirm) · rule 1.2 · 17 → 10

````text
/// Which device to restore the Mac's output to when the **recorded** one is
/// gone — the AirPods that disconnected while the session was running, which
/// makes `SwitchAudioSource -t output -s "…AirPods Pro"` exit non-zero and
/// leaves the Mac on `BlackHole 2ch`, silent.
///
/// `devices` is `SwitchAudioSource -a -t output`, in its own order. Returns
/// the built-in output when the list has one, else the first device that is
/// not one of [`VIRTUAL_OUTPUT_MARKERS`], else `None` — and `None` obliges
/// the caller to print the remedy rather than switch to something that stays
/// silent. See tests::the_fallback_picks_the_built_in_output_then_any_real_one.
````

### B1583 · l.678–682 · REWRITE (confirm) · rule 1.7 · 5 → 5

````text
/// The row printed when there is nothing to fall back to either: what failed,
/// and the two commands that fix it by hand.
///
/// It names the recorded device because that is the only record left of what
/// to restore once the Mac is stranded on `BlackHole 2ch`.
````

### B1584 · l.691–710 · REWRITE (confirm) ·p · rule 1.5 · 20 → 12

````text
/// Serializes — and resets — [`RUN_PHASE`] for the tests that touch it.
///
/// The harness runs unit tests on several threads in one process, so **every**
/// test that reads or writes the run phase must hold this guard: a writer
/// that skips it is exactly what the readers are being protected from.
/// Acquiring empties the slot so a test starts from Idle whatever its
/// predecessor left — including the `Exited` a normal teardown ends on.
/// Poisoning is ignored: a panicking test has already failed.
///
/// [`LIVE_SESSION`] is deliberately **not** reset here: tests in other modules
/// set that slot without holding this guard, and blanking it on every
/// acquisition would pull it out from under them.
````

### B1585 · l.721–729 · REWRITE (confirm) · rule 1.4 · 9 → 7

````text
/// What [`lock_session_globals`] hands back.
///
/// A newtype around the `MutexGuard` because async tests hold it across
/// `.await` points and `clippy::await_holding_lock` flags a raw guard there.
/// The lint's concern cannot arise: `#[tokio::test]` drives one future to
/// completion on the test's own thread, so the contention is between the
/// harness's OS threads, which is what a std `Mutex` is for.
````

### B1594 · l.909–914 · REWRITE (confirm) · rule 1.3 · 6 → 4

````text
        // 5: a `demo.sh run` session — nothing of ours anywhere, but the
        // runtime is reporting in right now, naming a live process. Both
        // halves are asserted because a door that refused on freshness alone
        // said "a session is live" over a file the UI was calling Idle (A10-8).
````

### B1614 · l.1164–1169 · REWRITE (confirm) · rule 1.7 · 6 → 5

````text
    /// Both tiers of the fallback policy in one table: a built-in output
    /// wherever it sits in the list, else the first non-virtual device, and
    /// `None` when every candidate is virtual or there are none — `None` is
    /// what makes the caller print the remedy instead of switching the Mac to
    /// something that stays silent.
````

### B1615 · l.1174–1176 · REWRITE (confirm) · rule 1.7 · 3 → 2

````text
                // A live `SwitchAudioSource -a -t output` list, in its own
                // order: the recorded AirPods are simply not on it any more.
````

## `sabrage/crates/sabrage-core/src/session/reconcile.rs`

Deleted (nothing carried): B1645, B1648, B1656, B1666, B1693, B1701, B1708, B1719, B1722, B1741, B1748, B1750, B1752, B1757

### B1616 · l.1–122 · REWRITE (amend) ·p · rule 1.2 · 122 → 26

````text
//! Reconcile a session Sabrage does not supervise.
//!
//! Run at app start, when the Session screen opens, and — as
//! [`finish_stopped_session`] — from [`crate::stages::stop`].
//!
//! `Live` is adopted untouched; `Dead` gets the full restore (audio device,
//! ALVR dashboard, `--wired` forwards); a recycled pid (`IdentityMismatch`)
//! gets the pid-free restore and signals nothing. The record is cleared only
//! once every guard it recorded is released, and kept otherwise.
//! `Unverifiable`, newer-schema, live-foreign and in-flight records are
//! reported and left as they are. [`detach`] marks the record and leaves every
//! guard in place. The row texts live on this file's consts; the shell has no
//! counterpart — PARITY.md § Session (detach / reconcile), "A recorded
//! **Live** session".
//!
//! Every mutation goes through [`crate::executor::Executor`], so `--dry-run`
//! plans the recovery instead of performing it; the audio probe is read-only
//! and goes through [`crate::process::capture`].
//!
//! # Failure policy
//!
//! Landmine: [`finish_stopped_session`] propagates only
//! [`SabrageError::Cancelled`]; every other failure becomes rows plus `Ok(())`
//! with the record kept, so `stop` still reaches its ports and audio reports.
//! See tests::{a_cancelled_reconcile_still_reaches_the_caller,
//! a_failed_restore_is_reported_and_the_record_is_kept_for_the_next_stop}.
````

### B1627 · l.179–186 · REWRITE (amend) · rule 2.3 · 8 → 7

````text
    /// Is the recorded process alive as far as anything here can tell?
    ///
    /// [`Classification::Unverifiable`] counts: it is the alive-pid case whose
    /// identity cannot be checked, and every door treats it as running
    /// ([`crate::session::session_block_at`]'s third signal,
    /// [`crate::session::watcher`]'s phase). Rendering it as exited is how the
    /// Session screen offers Launch for a session the launch path then refuses.
````

### B1637 · l.246–255 · REWRITE (confirm) · rule 1.7 · 10 → 8

````text
    /// The reason a caller about to mutate the machine must **stop**, or
    /// `None` when this outcome licenses carrying on.
    ///
    /// A9-1: every `Busy` but this process's own in-flight record (`silent`)
    /// is a refusal. Carrying on would take preflight's auto-fixes, `adb
    /// forward --remove` and the bottle's `wineserver -k` into the very
    /// session the classification had just refused to touch. See
    /// tests::every_busy_but_our_own_in_flight_record_is_a_refusal.
````

### B1642 · l.293–301 · REWRITE (confirm) · rule 1.7 · 9 → 8

````text
/// [`classify`] over a bare identity, for the caller that has one without a
/// record around it: [`crate::session::watcher::SessionMonitor`]'s live-handle
/// branch, whose `ProcInfo` is the same spawn-time identity `wine` holds.
///
/// One predicate, so the phase the Session screen shows and the classification
/// the launch path refuses on cannot disagree over an alive pid carrying the
/// spawn fallback's `start_time == 0`. See
/// tests::the_spawn_fallback_start_time_can_never_match.
````

### B1655 · l.461–466 · REWRITE (amend) · rule 1.5 · 6 → 4

````text
    // Before the wine spawn the record has `wine: None` (which classifies as
    // `Dead`) and no live handle exists, so only the published phase can keep
    // a mid-launch reconcile from tearing down the launch's own guards.
    // See tests::a_record_belonging_to_the_launch_in_progress_is_never_touched.
````

### B1657 · l.498–520 · REWRITE (amend) · rule 1.2 · 23 → 16

````text
/// The reconcile pass [`crate::stages::stop`] runs after `wineserver -k` and
/// before its audio report, on records this process does not supervise.
///
/// A dead wine pid gets [`RestoreMode::Full`], a recycled pid
/// [`RestoreMode::SafeOnly`]; the record is then cleared, or kept when a guard
/// could not be released. A still-alive one gets one warn and the record is
/// kept for the next `stop`. No record, another bottle's record, and a session
/// this process supervises are skipped.
///
/// # Errors
///
/// Only [`SabrageError::Cancelled`]. Every other failure becomes two rows and
/// `Ok(())` so the stage still reaches its ports and audio reports. See
/// tests::{stop_restores_and_clears_a_session_it_did_not_start,
/// stop_ignores_a_record_belonging_to_another_bottle,
/// a_failed_restore_is_reported_and_the_record_is_kept_for_the_next_stop}.
````

### B1661 · l.552–559 · REWRITE (confirm) · rule 1.2 · 8 → 6

````text
/// Turn a reconcile failure into two rows and `Ok(())`, so the stage that
/// invoked it keeps going.
///
/// [`SabrageError::Cancelled`] passes straight through: a Cancel must fail the
/// stage with exit 130 rather than be reported as a partial restore. See
/// tests::a_cancelled_reconcile_still_reaches_the_caller.
````

### B1670 · l.643–663 · REWRITE (amend) ·p · rule 1.2 · 21 → 13

````text
/// `SwitchAudioSource -c -t output` and `-a -t output`, `$(…)`-trimmed — both
/// read-only, hence [`crate::process::capture`] rather than the executor. The
/// two are independent, so they run concurrently ([`tokio::join!`]), and only
/// for a record that still has an unreleased audio guard.
///
/// `Ok(None)` is "we could not look" (no binary, a failed or wedged `-c`) and
/// leaves the audio guard **pending** rather than flagged; a failed `-a` only
/// costs the fallback pool.
///
/// # Errors
///
/// [`SabrageError::Cancelled`] only — see `probe_capture` and
/// tests::a_missing_switchaudiosource_leaves_the_audio_guard_pending.
````

### B1672 · l.701–711 · REWRITE (confirm) · rule 1.4 · 11 → 11

````text
/// [`crate::process::capture_with`] under the *operation's* token and
/// [`PROBE_TIMEOUT`]. `None` for a spawn failure, for a probe that ran out of
/// time, and for a cancelled one — indistinguishable to every caller, because
/// all three mean "we could not look".
///
/// The token lets Cancel interrupt a wedged CoreAudio probe instead of waiting
/// out the deadline with the operation lock held, and `capture_with` kills the
/// probe's whole **process group**, so a `SwitchAudioSource` that forked cannot
/// outlive its dropped leader. Not a `tokio::time::timeout` around `capture`:
/// it fires before `capture`'s own deadline, so the group kill never runs — a
/// timing property, and the test for it was decided against.
````

### B1678 · l.779–789 · REWRITE (confirm) · rule 1.7 · 11 → 11

````text
/// Put the Mac's output device back — but only while it is *still* BlackHole:
/// a user who already switched it back by hand must not have their choice
/// overwritten by a recovery pass.
///
/// Three outcomes, in order of preference: the recorded device; the fallback
/// [`crate::session::fallback_output_device`] picks when the recorded one is no
/// longer connected (its switch exits non-zero with "Could not find an audio
/// device named … Nothing was changed." and would otherwise leave the Mac
/// silent on BlackHole); or a warn naming the device and the commands to fix it
/// by hand, with the guard left **pending** so the record survives for the next
/// try. See tests::a_recorded_device_that_is_gone_falls_back_to_the_built_in_output.
````

### B1683 · l.885–894 · REWRITE (amend) · rule 1.5 · 10 → 11

````text
/// Remove exactly the recorded `--wired` forwards, on exactly the recorded
/// serials; never `--remove-all`
/// (PARITY.md § Invariants that must NOT change (byte/behavior parity),
/// "adb `forward --remove` per-serial for exactly tcp:9943+9944").
///
/// Each removal that succeeds drops its port from the record; the guard is only
/// flagged once none are left. A failed removal is *indeterminate* — usually a
/// vanished device, but equally a transient adb failure over a still-installed
/// `tcp:9943`, which silently breaks the next WiFi discovery — so the record is
/// kept for the next launch or `stop`. See
/// tests::a_forward_that_could_not_be_removed_keeps_the_record.
````

### B1685 · l.925–931 · REWRITE (confirm) · rule 1.5 · 7 → 4

````text
        // Progress is written the moment it happens: a retry of an
        // already-absent forward exits non-zero, which this loop reads as
        // "still installed", so a crash mid-loop would strand a phantom port.
        // See tests::a_removal_that_took_is_on_disk_before_the_next_one_is_tried.
````

### B1689 · l.995–1002 · REWRITE (confirm) · rule 1.7 · 8 → 8

````text
/// Clear the record — or keep it, when a guard is still pending. `true` when
/// it was kept.
///
/// A record whose guards are not all released is worth more on disk than off
/// it: clearing it strands the user on `BlackHole 2ch` with no machine-readable
/// trace of what to restore, and the next launch or `stop` retries from the
/// kept record. See
/// tests::with_nothing_to_fall_back_to_the_record_is_kept_with_the_remedy.
````

### B1690 · l.1010–1013 · REWRITE (confirm) ·p · rule 1.5 · 4 → 3

````text
        // Run-id guarded: reconciliation can take seconds, and a launch that
        // started meanwhile has written its own record at this path. Only the
        // record carrying the run-id we read is ours to delete.
````

### B1702 · l.1112–1137 · REWRITE (confirm) ·p · rule 1.2 · 26 → 13

````text
/// Detach from a live session: mark the state file `detached`, fire the
/// handle's `detach` token, and leave every guard in place. Takes [`Paths`]
/// rather than a [`StageCtx`] because app-quit has no stage context.
///
/// The supervisor is the authority: firing `handle.detach` is what disarms both
/// guards, marks the record, and drops out of
/// [`crate::session::LIVE_SESSION`]. This waits up to [`DETACH_WAIT`] for that
/// slot to clear and only then sets `detached` itself as a safety net — never
/// on the timeout, never once `handle.cancel` has fired, and through
/// [`state::mark_detached`], which creates nothing. See tests::{
/// detach_does_not_relabel_a_session_stopped_during_the_wait,
/// detach_that_times_out_leaves_the_record_alone,
/// detach_creates_nothing_when_the_supervisor_already_cleared_the_record}.
````

### B1704 · l.1161–1172 · REWRITE (confirm) · rule 1.5 · 12 → 8

````text
    // Stop is terminal and detach is subordinate to it: both tokens feed one
    // unbiased `select!`, so a detach fired after a Stop could otherwise win
    // that race and leave wine running under a caller reporting success.
    // See tests::detach_does_nothing_once_stop_has_already_fired.
    //
    // This is also the return that absorbs `commands::resolve_quit`'s
    // stop-then-timeout arm: nothing is detached there, so that arm's own
    // message has to say so.
````

### B1707 · l.1223–1228 · REWRITE (amend) · rule 1.5 · 6 → 4

````text
    /// A pid no process can have on macOS (the default `kern.maxproc` ceiling
    /// is five digits), so the "dead" case is deterministic and free of a
    /// pid-reuse race. Not `u32::MAX`: that is `-1` as an `i32`, and
    /// `kill(-1, …)` addresses every process the user can signal.
````

### B1709 · l.1244–1249 · REWRITE (confirm) · rule 1.5 · 6 → 5

````text
    /// A context whose Sabrage store and adb both point **inside the fixture**.
    ///
    /// `sabrage_appsup` is the load-bearing override: without it `Paths::new`
    /// derives it from the real `$HOME`, and these tests read — and with a real
    /// executor write — the developer's own `session-state.json`.
````

### B1715 · l.1365–1366 · REWRITE (confirm) · rule 1.7 · 2 → 2

````text
    /// The recorded output device of the disconnected-device defect: connected
    /// at launch, gone by the time `stop` ran.
````

### B1717 · l.1379–1386 · REWRITE (amend) ·p · rule 1.2 · 8 → 7

````text
    /// A [`crate::executor::DryRunExecutor`] whose children come back
    /// **non-zero** whenever `device` is one of their arguments: the machine
    /// failure the audio fallback exists for, a disconnected recorded output
    /// device.
    ///
    /// Everything else delegates, and the inner executor still *sees* the
    /// child, so the plan records every attempt in order.
````

### B1720 · l.1566–1570 · REWRITE (confirm) · rule 1.3 · 5 → 4

````text
        // `spawn_detached` records start_time 0 when it cannot observe the pid;
        // that identity gets its own classification, which restores nothing.
        // `IdentityMismatch` would undo the guards of a possibly-running
        // session (A9-5).
````

### B1725 · l.1692–1697 · REWRITE (confirm) · rule 1.3 · 6 → 3

````text
        // Every flag hits disk before the next guard is touched: progress has
        // to be crash-durable (A9-4). See
        // tests::a_removal_that_took_is_on_disk_before_the_next_one_is_tried.
````

### B1727 · l.1795–1802 · REWRITE (confirm) · rule 1.7 · 8 → 6

````text
    /// A9-1. Between the first guard and the wine spawn the record exists with
    /// `wine` still `None`, so `classify` says `Dead` and only the run stage's
    /// published phase knows the launch is happening. Reconciling it would
    /// restore the audio device mid-launch, `SIGTERM` the dashboard this launch
    /// just spawned, pull its `--wired` forwards and delete its record, under a
    /// launch that keeps going.
````

### B1730 · l.1888–1890 · REWRITE (confirm) · rule 1.7 · 3 → 3

````text
    /// A9-3. The other front-end's record: `sabrage run` in a terminal next to
    /// an open Sabrage. `owner_pid` names the process running it, and reconcile
    /// must not touch its guards.
````

### B1732 · l.1954–1963 · REWRITE (confirm) · rule 1.2 · 10 → 5

````text
    /// A9-1, A9-5. Every `Busy` except this process's own in-flight record is a
    /// record somebody is still using, and the launch path has to be able to
    /// *tell*: otherwise it carries on into preflight's auto-fixes, `adb forward
    /// --remove` and the bottle's `wineserver -k`, taking down the session the
    /// classification just refused to touch.
````

### B1739 · l.2087–2093 · REWRITE (confirm) · rule 1.7 · 7 → 6

````text
    /// A9-4. The removals are not atomic with the record that describes them:
    /// a crash (or a Cancel, or a power loss) between two `adb forward
    /// --remove`s must leave a record naming only what is *still* installed. A
    /// record that keeps an already-removed port can never be completed,
    /// because the retry's `--remove` of an absent listener exits non-zero and
    /// this module reads that as "still installed".
````

### B1742 · l.2213–2215 · REWRITE (amend) · rule 1.7 · 3 → 4

````text
    /// The disconnected-device defect: the recorded output (AirPods) is gone,
    /// so the switch to it fails. Without a fallback the Mac is left on
    /// BlackHole — silent — with the record cleared as if it had been restored;
    /// the built-in speakers take over instead.
````

### B1744 · l.2267–2269 · REWRITE (amend) · rule 1.1 · 3 → 3

````text
    /// Nothing audible to fall back to: warn with the unrestorable-device row,
    /// keep the record so the next launch or stop can try again, and re-save it
    /// with the audio guard still pending.
````

### B1746 · l.2343–2345 · REWRITE (confirm) · rule 1.7 · 3 → 2

````text
    /// The same audio failure through `stop`, the other entry point into
    /// `restore_with`.
````

### B1751 · l.2583 · REWRITE (amend) · rule 1.4 · 1 → 1

````text
    // These camelCase wire shapes are what sabrage/ui/src/ipc.ts mirrors.
````

### B1760 · l.2896–2901 · REWRITE (confirm) · rule 1.7 · 6 → 6

````text
    /// A9-9. Stop can also win *during* detach's wait: it fires the terminal
    /// token and then releases the live slot, and its teardown legitimately
    /// **keeps** the record when a guard could not be released (a disconnected
    /// output device). Writing `detached: true` over that record makes the app
    /// tell the user a session it had just stopped "detached instead of
    /// stopping — it is still running, unsupervised".
````

## `sabrage/crates/sabrage-core/src/session/state.rs`

### B1764 · l.1–71 · REWRITE (amend) ·p · rule 1.2 · 71 → 31

````text
//! `session-state.json` — the crash-recovery record.
//!
//! `~/Library/Application Support/Sabrage/session-state.json`
//! ([`crate::paths::Paths::session_state_path`]), written by the launch path
//! and read by [`super::reconcile`]. `run.sh`'s guards are shell traps, so a
//! `SIGKILL`, a panic or a power loss skips them and leaves the Mac's output
//! device on `BlackHole 2ch`, an unattributable ALVR dashboard and (after
//! `--wired`) two adb forwards that break WiFi discovery on the next run,
//! with nothing on the machine describing them. This file lets Sabrage undo
//! them (design-core §4.2; PARITY.md
//! § Session (detach / reconcile), "A **Dead** or **IdentityMismatch**
//! recorded session").
//!
//! The record is saved *before* each guarded mutation and again after each
//! guard is released, so recovery is idempotent: a flag that is already `true`
//! means that guard is done, and a crash at any instant leaves a file
//! describing only work that still needs doing. Every optional field and flag
//! carries `#[serde(default)]`, so an older file still loads;
//! [`SESSION_STATE_VERSION`] covers what defaults cannot.
//!
//! One record path is shared by both front-ends (the app and the `sabrage`
//! CLI), and an atomic rename stops a torn read but not a lost update. [`save`],
//! [`clear`], [`clear_run`] and [`mark_detached`] therefore compare what is on
//! disk under [`lock_record`]: a record a live foreign process owns and a
//! record from a newer schema are refused, and [`clear_run`] and
//! [`mark_detached`] additionally leave another run's record exactly as it is
//! (`Ok`), the newer owner being responsible for it. Serializing a whole
//! [`super::reconcile`] pass across several such calls is the operation lock's
//! job. See tests::{a_save_for_a_different_run_does_not_clobber_a_live_owners_record,
//! a_newer_schema_record_is_never_overwritten_or_deleted,
//! clear_run_only_removes_the_record_it_names}.
````

### B1766 · l.86–90 · REWRITE (amend) · rule 1.5 · 5 → 6

````text
/// One `adb forward tcp:<port> tcp:<port>` created by a `--wired` launch.
///
/// Per-serial, because the removal must be too: `adb forward --remove` names
/// exactly these ports on exactly this device, never `--remove-all`
/// (PARITY.md § Invariants that must NOT change (byte/behavior parity), "adb `forward --remove`
/// per-serial for exactly tcp:9943+9944"; CLAUDE.md's `--wired` note).
````

### B1774 · l.131–141 · REWRITE (amend) · rule 1.7 · 11 → 10

````text
    /// The owner's start time (seconds since the epoch), when it could be
    /// observed at write time.
    ///
    /// Pids are recycled, so `owner_pid` alone keeps a record whose owner died
    /// in the pre-spawn window foreign-owned — neither overwritable nor
    /// clearable, wedging the next launch — as soon as anything else reuses
    /// that pid; paired with the pid this is the same guard
    /// [`ProcInfo::is_same_process`] gives `wine`. `None` in records written
    /// before this field existed, where the pid alone is all there is.
    /// See tests::a_live_foreign_owner_is_recognised_only_while_its_session_can_still_be_running.
````

### B1784 · l.215–221 · REWRITE (confirm) · rule 1.2 · 7 → 6

````text
    /// Was this record written by a Sabrage this one understands?
    ///
    /// `false` means a **newer** schema: the file may describe a guard this
    /// binary has never heard of, so the mutating paths report such a record
    /// and leave it exactly as it is
    /// (tests::a_newer_schema_record_is_never_overwritten_or_deleted).
````

### B1786 · l.238–247 · REWRITE (confirm) · rule 1.5 · 10 → 9

````text
/// Read the state file.
///
/// * absent → `Ok(None)` — the normal case, meaning "no session to reconcile";
/// * present but unreadable or malformed → `Err` ([`SabrageError::Io`]).
///
/// The second case is deliberately not folded into `None`: a corrupt file may
/// still describe a live session with a rerouted audio device, and reporting
/// "nothing to recover" for it leaves the user with no sound and no
/// explanation (tests::a_corrupt_file_is_an_error_never_a_silent_none).
````

### B1787 · l.262–284 · REWRITE (amend) · rule 1.2 · 23 → 17

````text
/// Does `state` describe a session **another live process** is responsible
/// for?
///
/// True only when all three hold: `owner_pid` is neither 0 (an older record
/// that never wrote one) nor this process; that pid is alive **and** still the
/// process that recorded it (a record written before
/// [`SessionState::owner_started_at`] existed can only trust the bare pid);
/// and the session has not visibly ended — the recorded wine child is still
/// that same process, or there is no wine child yet (the pre-spawn window,
/// where the record is the only description of guards already taken).
///
/// A record whose wine pid is gone is therefore never protected, whoever
/// wrote it: it is a leftover, and undoing its guards is the job. The one
/// residual false positive — a recycled `owner_pid` in a record written
/// before `owner_started_at` existed — costs a kept record and a reported
/// row, never a mutation. See
/// tests::a_live_foreign_owner_is_recognised_only_while_its_session_can_still_be_running.
````

### B1792 · l.355–363 · REWRITE (confirm) · rule 1.5 · 9 → 8

````text
/// The refusal both [`save`] and [`clear`] raise for a record written by a
/// **newer** Sabrage.
///
/// Enforced here and not only in [`super::reconcile::untouchable`], because a
/// path that never went through reconcile (a launch that carried on past a
/// `Reconciled::Busy`, a teardown, a guard flag flip) would otherwise rewrite
/// a v2 record through this v1 struct
/// (tests::a_newer_schema_record_is_never_overwritten_or_deleted).
````

### B1794 · l.391–404 · REWRITE (amend) · rule 1.2 · 14 → 15

````text
/// Write the state file atomically (pretty JSON plus a trailing newline).
///
/// Goes through the [`Executor`] like every other mutation, so `--dry-run`
/// plans the write instead of performing it. Pretty-printed because a human
/// reading this file is exactly the situation it exists for.
///
/// # Errors
///
/// Refuses a record on disk that describes a **different** run another live
/// process owns ([`has_live_foreign_owner`]) — the ordinary guard-by-guard
/// flag flip over one's own run is unaffected — and any record written by a
/// newer Sabrage, one's own run included. The check and the write happen
/// under [`lock_record`], which a dry run skips, having nothing to
/// serialize. See
/// tests::a_save_for_a_different_run_does_not_clobber_a_live_owners_record.
````

### B1796 · l.434–444 · NEEDS-TEST (confirm) · rule 1.5 · 11 → 8

test first: `mark_detached_never_recreates_a_cleared_record` in `sabrage/crates/sabrage-core/src/session/state.rs (mod tests)` — mark_detached returns false and creates no file when the record is absent, and leaves a record belonging to a different run byte-identical.

````text
/// Set `detached: true` on the record for `run_id` — and **only** if that is
/// still the record on disk. `true` when the flag was written.
///
/// [`super::reconcile::detach`]'s safety net. The load, the compare and the
/// write happen under one [`lock_record`], and an absent record is left
/// absent: re-creating it would resurrect a session the supervisor cleared
/// while this call was in flight
/// (tests::mark_detached_never_recreates_a_cleared_record).
````

### B1798 · l.482–490 · REWRITE (confirm) · rule 1.5 · 9 → 9

````text
/// [`clear`], but only if the record on disk is still `expected`'s.
///
/// The compare-and-swap the single shared record path needs: neither the
/// atomic rename nor [`lock_record`] stops a *late* teardown (or a reconcile
/// that started before a launch did) from deleting a **newer** run's record —
/// its audio device, dashboard and forwards description and all. A different
/// run on disk is not an error: the file is left exactly as it is and `Ok(())`
/// is returned, because the newer owner is responsible for it now. See
/// tests::clear_run_only_removes_the_record_it_names.
````

### B1805 · l.698–700 · REWRITE (confirm) · rule 1.7 · 3 → 3

````text
        // Same pid, a different process: a recycled `owner_pid` is not an
        // owner. Without the recorded start time such a record can be neither
        // overwritten nor cleared, and it wedges the next launch.
````

### B1809 · l.752–753 · REWRITE (confirm) · rule 1.1 · 2 → 1

````text
        // The owner's own writes still go through.
````

## `sabrage/crates/sabrage-core/src/session/watcher.rs`

Deleted (nothing carried): B1873, B1875, B1886

### B1814 · l.1–20 · REWRITE (amend) ·p · rule 1.2 · 20 → 9

````text
//! Session telemetry, derived entirely from files and polls.
//!
//! `runtime_status.json` outlives the runtime, so it evidences a live session
//! only while fresh and while the pid it names is alive. Its `state` is
//! oxrsys's vocabulary: only [`RUNTIME_STATE_STREAMING`] is compared, and only
//! to decide whether a missing heartbeat means anything; every other value is
//! carried through and displayed
//! (tests::runtime_status_live_is_freshness_and_a_live_pid_together,
//! tests::is_fresh_accepts_only_stamps_inside_both_budgets).
````

### B1815 · l.31–42 · REWRITE (confirm) ·p · rule 1.2 · 12 → 6

````text
/// `runtime_status.json` as oxrsys writes it.
///
/// Keys are snake_case on the wire — the one session-layer type not renamed to
/// camelCase, because oxrsys owns the file. Unknown fields are ignored so a
/// runtime that grows a field does not blank the status pill. Shape pinned by
/// tests::parse_runtime_status_accepts_the_observed_and_minimal_documents_and_rejects_a_half_written_one.
````

### B1816 · l.45 · REWRITE (amend) · rule 2.3 · 1 → 2

````text
    /// Free-form. Only [`RUNTIME_STATE_STREAMING`] is ever compared; every
    /// other value is carried through and displayed.
````

### B1817 · l.54–61 · REWRITE (amend) · rule 2.3 · 8 → 3

````text
/// The `state` oxrsys writes while a client is connected and frames are
/// flowing; the only state with a per-second heartbeat, hence the only one
/// whose staleness means anything (tests::snapshot_tests::snapshot_phase_transitions).
````

### B1822 · l.100–107 · REWRITE (confirm) ·p · rule 1.2 · 8 → 7

````text
/// How far into the future an `updated_at_unix_ms` stamp may sit and still be
/// believed: ordinary clock skew between the runtime and this process.
///
/// Without a bound, a stamp an hour ahead (clock correction or corruption)
/// reads as "written now" and suppresses [`SessionPhase::Stalled`] until wall
/// time catches up
/// (tests::is_fresh_accepts_only_stamps_inside_both_budgets).
````

### B1824 · l.122–136 · REWRITE (amend) · rule 1.7 · 15 → 14

````text
/// Does `rs` describe a runtime that is **running right now**?
///
/// [`is_fresh`] *and* a `process_id` that is still alive: the two halves of
/// PARITY.md § Declared by the 2026-08-30 adversarial review (round 1 fixes),
/// "**External sessions.** The session monitor reports a session started
/// outside this Sabrage process". Both readers call this one function --
/// [`SessionMonitor::snapshot`]'s [`SessionPhase::External`] derivation and
/// [`crate::session::session_block_at`]'s status signal, the door every
/// mutating operation goes through.
///
/// A status with no `process_id` is therefore not evidence of a live runtime:
/// oxrsys writes that field unconditionally (`RuntimeStatus.cpp`), so its
/// absence means a file this build cannot vouch for
/// (tests::runtime_status_live_is_freshness_and_a_live_pid_together).
````

### B1825 · l.142–155 · REWRITE (amend) ·p · rule 1.7 · 14 → 10

````text
/// The wall-clock time an oxrsys log line carries, in [`super::now_unix_ms`]'s
/// units, or `None` when the line does not start with one.
///
/// The stamp follows oxrsys's spdlog format (`Config.cpp`:
/// `[%Y-%m-%d %H:%M:%S.%e] [%l] %v`) and is **local** time, making it
/// comparable with `started_at_unix_ms`. It exists for one question: the
/// runtime log is a single appending sink shared by every session, so a line in
/// [`RUNTIME_LOG_PRELOAD_LINES`]'s backward window is this session's only if
/// written after this session started
/// (tests::snapshot_tests::an_adopted_session_only_inherits_lines_written_after_it_started).
````

### B1827 · l.172–187 · REWRITE (confirm) ·p · rule 1.2 · 16 → 10

````text
/// Pull an [`EncoderInfo`] out of one oxrsys log line, or `None`.
///
/// The marker `OXRSys/ALVR: encoder ready ` is searched anywhere in the input,
/// so a timestamped spdlog line parses identically to the bare message
/// (tests::parse_encoder_ready_works_on_the_bare_message_with_no_timestamp_prefix).
///
/// `(H.264, in-process)` is the silent-downgrade signature the Session screen
/// must surface: the native arm64 helper did not take and encoding fell back to
/// Rosetta H.264. Shape pinned by
/// tests::the_encoder_ready_format_string_pin_is_unchanged.
````

### B1830 · l.234–241 · REWRITE (amend) ·p · rule 1.2 · 8 → 6

````text
    /// The most recent `encoder ready` line parsed so far. Cleared on the edge
    /// into [`SessionPhase::Idle`] / [`SessionPhase::Exited`]
    /// (tests::snapshot_tests::snapshot_phase_transitions, F16) and whenever
    /// the run it belongs to differs from the run being reported, so a new
    /// session never inherits the previous one's chip as a false-healthy signal
    /// (tests::snapshot_tests::a_previous_sessions_encoder_line_is_never_published_for_a_new_run).
````

### B1832 · l.247–257 · REWRITE (confirm) ·p · rule 1.2 · 11 → 7

````text
    /// When this monitor was built, against [`super::now_unix_ms`]'s clock.
    ///
    /// Preloaded log lines ([`RUNTIME_LOG_PRELOAD_LINES`]) predate the monitor,
    /// so they belong to the current session only when that session predates the
    /// monitor too: Sabrage opened onto a game already running
    /// (tests::snapshot_tests::an_adopted_session_only_inherits_lines_written_after_it_started,
    /// tests::snapshot_tests::a_previous_sessions_encoder_line_is_never_published_for_a_new_run).
````

### B1837 · l.270–277 · REWRITE (confirm) ·p · rule 1.7 · 8 → 7

````text
    /// Which run `ever_fresh`/`last_fresh_unix_ms`/`runtime_status` describe.
    ///
    /// The monitor outlives every session it watches, so freshness history not
    /// reset on a run change decides the *next* session's `Stalled`. `None` is
    /// a real value (nothing identifiable is running), and history recorded
    /// under it must not survive into a run either
    /// (tests::snapshot_tests::freshness_history_never_crosses_from_one_session_to_the_next).
````

### B1839 · l.285–287 · REWRITE (amend) · rule 1.5 · 3 → 3

````text
/// Which derived source produced a snapshot's phase, before the run stage's
/// published phase is weighed against it. See [`SessionMonitor::snapshot`]'s
/// precedence rules.
````

### B1844 · l.323–371 · REWRITE (confirm) ·p · rule 1.2 · 49 → 11

````text
    /// One snapshot folding [`super::live_session`], the run stage's published
    /// [`super::RunPhaseInfo`], persisted [`super::state::SessionState`], wine
    /// child liveness, `runtime_status.json` freshness, an external
    /// `Beat Saber.exe`, and the newest `encoder ready` line into one
    /// [`SessionStatus`]. Never fails: an unreadable source degrades one field
    /// rather than the whole snapshot.
    ///
    /// Phase precedence, the wholesale identity takeover (#200), and the
    /// `owned_by_this_process` rule (#201) are pinned by
    /// tests::snapshot_tests::snapshot_phase_precedence_table and
    /// tests::snapshot_tests::snapshot_identity_and_exit_code_sources.
````

### B1845 · l.376–379 · REWRITE (confirm) · rule 1.4 · 4 → 3

````text
        // Which of the three derived branches produced `status.phase` — the
        // published-phase precedence below is a function of it, not just of
        // the phase value.
````

### B1846 · l.389–393 · REWRITE (confirm) · rule 1.5 · 5 → 5

````text
            // Through `classify`, not `is_same_process()`: an alive pid whose
            // recorded `start_time` is the spawn fallback's 0 is `Unverifiable`
            // — live as far as anything can tell, and every door treats it that
            // way
            // (tests::snapshot_tests::an_alive_pid_with_no_verifiable_start_time_is_never_reported_exited).
````

### B1849 · l.420–426 · REWRITE (confirm) · rule 1.4 · 7 → 5

````text
        // `ever_fresh`, `last_fresh_unix_ms` and the cached status are what the
        // stall rule below reasons over, and this monitor outlives every
        // session it watches: carrying session A's history into session B
        // classifies B as `Stalled` off A's timestamps
        // (tests::snapshot_tests::freshness_history_never_crosses_from_one_session_to_the_next).
````

### B1850 · l.434–437 · REWRITE (confirm) · rule 1.4 · 4 → 4

````text
        // The file is global and outlives every session, so a stamp written
        // *before* the session being reported started is the previous
        // session's — evidence about a runtime that is gone, not this one
        // (tests::snapshot_tests::a_status_written_before_this_session_started_is_not_fresh).
````

### B1851 · l.450–459 · REWRITE (confirm) ·p · rule 1.4 · 10 → 5

````text
                    // No handle, no record, but the runtime is reporting *now*
                    // and the process it names is alive — an external launch.
                    // Reporting idle here invites a second launch onto a live
                    // game. Never from freshness alone: the pid must answer too
                    // (tests::snapshot_tests::a_session_started_outside_sabrage_is_reported_not_called_idle).
````

### B1852 · l.470–482 · REWRITE (confirm) ·p · rule 1.4 · 13 → 8

````text
        // The seventh door signal ([`super::running_game_pid`]) rendered.
        // `runtime_status.json` appears only once streaming starts, minutes
        // after `./demo.sh run` spawned the game; every file source above reads
        // idle for that window while the doors already refuse (A13a-2)
        // (tests::snapshot_tests::a_running_game_with_nothing_on_disk_is_external_not_idle).
        //
        // Last, and only for `Base::None`: the one probe that costs a full
        // process-table walk; any branch above already knows more than a pid.
````

### B1853 · l.491–494 · REWRITE (confirm) · rule 1.4 · 4 → 3

````text
        // Parsed here (the cursor has to advance every poll) but *attributed*
        // at the end of this method, once the phase and the run it belongs to
        // are settled.
````

### B1855 · l.508–518 · REWRITE (confirm) ·p · rule 1.7 · 11 → 6

````text
        // oxrsys writes `runtime_status.json` on state changes and once per
        // second *only while streaming* — there is no idle heartbeat. Staleness
        // therefore means "the stream's heartbeat stopped" only when the last
        // state is `streaming`; an `idle` runtime is legitimately stale for as
        // long as the user takes to put the headset on
        // (tests::snapshot_tests::snapshot_phase_transitions).
````

### B1856 · l.538–542 · REWRITE (confirm) · rule 1.4 · 5 → 3

````text
        // `Preflight` / `Launching` / `Stopping` exist only in the run stage's
        // own head — there is no `LIVE_SESSION` yet, or teardown has already
        // started clearing it — so nothing derived above can produce them.
````

### B1857 · l.545 · REWRITE (confirm) · rule 2.3 · 1 → 1

````text
                // Teardown is underway even though the handle is still up.
````

### B1858 · l.547 · REWRITE (confirm) · rule 2.3 · 1 → 1

````text
                // A launch that has not published its handle yet.
````

### B1859 · l.549–551 · REWRITE (amend) · rule 2.3 · 3 → 5

````text
                // `Exited` — and defensively anything else the run stage
                // might one day publish — is the weakest signal there is: it
                // survives `run()` returning, so it fills `Idle` (the screen
                // can still say "Exited (code N)") and loses to anything on
                // disk, which is newer truth.
````

### B1860 · l.556–563 · REWRITE (confirm) · rule 1.5 · 8 → 5

````text
                // #201: `RUN_PHASE` is an in-process global, so a published
                // phase that wins names a session this Sabrage owns — including
                // the Preflight/Launching window where `LIVE_SESSION` is not
                // populated yet
                // (tests::snapshot_tests::snapshot_identity_and_exit_code_sources).
````

### B1861 · l.565–575 · REWRITE (confirm) · rule 1.5 · 11 → 9

````text
                // #200: a published phase outranking a *persisted* derivation
                // for a DIFFERENT run means a launch started over a stale
                // `session-state.json`; keeping the old identity under the new
                // phase points Stop at a run the `RunRegistry` never heard of,
                // so the publication's identity is taken wholesale
                // (tests::snapshot_tests::snapshot_identity_and_exit_code_sources).
                //
                // `Base::Live` needs no such branch: a published phase over a
                // live handle is always the same run (teardown's `Stopping`).
````

### B1863 · l.599–607 · REWRITE (confirm) · rule 1.3 · 9 → 4

````text
        // A session that has just settled into Idle or Exited has nothing left
        // to report a codec for. Edge-triggered on entry, so a monitor that has
        // never seen a session does not have a freshly parsed chip yanked out
        // from under it in the very poll that produced it.
````

### B1864 · l.615–619 · REWRITE (confirm) · rule 1.3 · 5 → 4

````text
        // …and a chip never crosses from one run to another: the edge above
        // cannot catch a monitor that reports Running from its very first poll,
        // which is exactly when the log still holds the previous session's line
        // (tests::snapshot_tests::a_previous_sessions_encoder_line_is_never_published_for_a_new_run).
````

### B1865 · l.625–637 · REWRITE (confirm) · rule 1.3 · 13 → 8

````text
            // A line the tail read *live* was written after this poll's
            // predecessor, so it belongs to the session being reported. A
            // preloaded line is history, and history is this session's only if
            // the session predates the monitor and the line's own timestamp is
            // not older than the session; the log is one appending sink shared
            // by every session, so a preloaded line with no readable timestamp
            // proves nothing and is dropped
            // (tests::snapshot_tests::an_adopted_session_only_inherits_lines_written_after_it_started).
````

### B1870 · l.778–781 · REWRITE (confirm) · rule 1.7 · 4 → 4

````text
    /// A10-8. One predicate, two readers: the `External` phase the Session
    /// screen shows and the door every mutating operation goes through. Two
    /// spellings of "is the runtime live" let the UI say Idle while Settings
    /// refused to save over the same file.
````

### B1876 · l.963–968 · REWRITE (confirm) ·p · rule 1.2 · 6 → 3

````text
        /// Best-effort reset of the process-global live-session and run-phase
        /// slots before a test that must start from Idle. Cannot rule out a
        /// concurrent test in another module touching the same globals.
````

### B1879 · l.1022–1028 · REWRITE (amend) · rule 1.2 · 7 → 5

````text
        /// A9-6. The runtime log is global and append-only across sessions and a
        /// new monitor preloads its last 200 lines, so an `encoder ready` line
        /// from an earlier session would show a healthy chip where "waiting for
        /// encoder…" belongs — and, since oxrsys emits that line once per
        /// session, hide an `(H.264, in-process)` downgrade for the whole run.
````

### B1885 · l.1115–1121 · REWRITE (confirm) · rule 1.2 · 7 → 5

````text
        /// A9-6, both halves. An adopted session — one that started before the
        /// monitor — believes a preloaded line stamped after it started and
        /// publishes the chip it names; a line stamped before it belongs to a
        /// previous session, and publishing that one shows a healthy `(HEVC,
        /// native helper)` chip where "waiting for encoder…" belongs.
````

### B1892 · l.1278–1283 · REWRITE (confirm) · rule 1.7 · 6 → 5

````text
        /// A13a-2, rendered. The door counts a running `Beat Saber.exe` as a live
        /// session for the window a `./demo.sh run` spends between its wine spawn
        /// and its first `runtime_status.json`. The phase carries it too:
        /// reporting `Idle` there leaves Launch and every Doctor Fix enabled, and
        /// each one then dies with the Fatal the door raises.
````

### B1900 · l.1396–1401 · REWRITE (confirm) · rule 1.7 · 6 → 5

````text
        /// A9-5. The spawn fallback records `start_time: 0` when the child could
        /// not be observed (`Executor::spawn_detached`), so `is_same_process()`
        /// is false for it forever after. Reconciliation calls that alive pid
        /// `Unverifiable` and every door treats it as live; reporting `Exited`
        /// would put a Launch button over a session `run` refuses.
````

### B1905 · l.1523–1532 · REWRITE (amend) · rule 1.7 · 10 → 3

````text
        /// #2/#100: the precedence table in [`SessionMonitor::snapshot`]'s doc
        /// comment — every row where two sources disagree, plus the live-only and
        /// persisted-only baselines those conflicts are measured against.
````

### B1915 · l.1794–1804 · REWRITE (amend) ·p · rule 1.4 · 11 → 4

````text
        /// The phase transitions, in strict sequence inside one test rather than
        /// separate `#[tokio::test]`s: they read the process-global
        /// `LIVE_SESSION` slot and would race on separate threads. The residual
        /// cross-file risk is the caveat `session::mod`'s tests carry.
````

### B1920 · l.1869–1874 · REWRITE (amend) ·p · rule 1.3 · 6 → 4

````text
            // F2: the run stage's published phase wins over whatever would
            // otherwise be derived — here plain Idle — for the three phases only
            // it can know about. #100: it must also publish the identity, or the
            // Session screen offers a Stop button with no bottle to stop.
````

### B1921 · l.1900–1905 · REWRITE (amend) · rule 1.7 · 6 → 4

````text
            // F16: the encoder chip must clear on the edge into Idle/Exited
            // rather than linger as a false-healthy chip for a session that no
            // longer exists. One monitor throughout, so `last_phase` really
            // transitions out of Running within one instance.
````

### B1924 · l.1955–1957 · REWRITE (confirm) · rule 1.3 · 3 → 2

````text
            // Encoder chip: picked up from a fresh line appended after the monitor
            // already exists — Idle phase throughout, no live session.
````

### B1928 · l.2064–2068 · REWRITE (confirm) · rule 1.7 · 5 → 3

````text
            // Idle runtime waiting for the headset: the file was written once
            // (`SetIdle`) and is now arbitrarily stale, past every grace — oxrsys
            // has no idle heartbeat, so this is Running, never Stalled.
````

## `sabrage/crates/sabrage-core/src/stages/build.rs`

Deleted (nothing carried): B1931, B1940, B1944, B1946, B1952, B1954, B1955, B1958, B1964, B1966, B1967, B1968, B1969, B1970, B1973, B1975, B1982, B1985, B1987, B1988, B1990, B1991, B1999

### B1930 · l.1–118 · REWRITE (amend) ·p · rule 1.6 · 118 → 21

````text
//! `demo.sh build` — build oxrsys (x86_64 + embedded ALVR core), the native-arm64
//! encoder helper, wineopenxr, and the ALVR dashboard. Idempotent (all four
//! build systems are incremental).
//!
//! Reference: `scripts/demo/build.sh`. Six steps, in order:
//! [`step::BUILD_TOOLS`], [`step::BUILD_OXRSYS`], [`step::BUILD_HELPER`],
//! [`step::BUILD_WINEOPENXR`], [`step::BUILD_DASHBOARD`], [`step::BUILD_OUTPUTS`].
//!
//! Children are spawned with [`crate::process::default_child_path`]; the tool gates
//! probe that same list, so a Finder-launched `.app` missing `~/.cargo/bin` never
//! reports a false "missing" for a tool the spawn finds.
//!
//! Under `--dry-run` nothing is compiled: the helper postconditions, destination-side
//! validation, and seven-artifact sweep are skipped, and no row claims a build
//! (tests::a_dry_run_stages_nothing_and_says_would_build,
//! tests::narrate_built_swaps_the_verb_and_the_severity_under_dry_run).
//!
//! The staged helper is validated at its *destination*, which build.sh never checks:
//! a staged copy with the right bytes but no execute bit still FAILs doctor's
//! `build.helper-arm64`, so [`stage_encoder_helper`] re-validates and re-copies
//! (tests::a_byte_identical_but_non_executable_staged_helper_is_repaired).
````

### B1941 · l.185–195 · REWRITE (amend) ·p · rule 1.5 · 11 → 5

````text
/// `cmake -S "$OXRSYS" -B "$OXR_BUILD" …` (build.sh), identical argument order.
///
/// `-DOXRSYS_BUILD_ENCODER_HELPER=OFF` (appended by [`oxrsys_x64_configure_args`])
/// repairs a `build-x64` cache stuck at `ON` — CMake `option()` cannot clear it, and it
/// re-fatals on the thin-arm64 gate (r1:A5-2, tests::the_x64_configure_spec_renders_the_helper_off_flag).
````

### B1947 · l.231 · REWRITE (amend) ·p · rule 2.2 · 1 → 2

````text
/// `command -v <name>` searched over `search_path` (e.g.
/// [`crate::process::default_child_path`]), not this process's inherited `PATH`.
````

### B1949 · l.244–247 · REWRITE (confirm) · rule 1.7 · 4 → 3

````text
/// Is `path` an existing file with any execute bit set? (`command -v`
/// semantics for one candidate path.) Deliberately a private copy: `paths.rs`
/// and `checks/build.rs` each keep their own.
````

### B1951 · l.264–275 · REWRITE (confirm) ·p · rule 1.5 · 12 → 7

````text
/// `rustup target list --installed 2>/dev/null | grep -q x86_64-apple-darwin`.
/// A missing `rustup` and a present-but-target-less one produce the same die text,
/// matching the shell (tests::rustup_gate_dies_unless_the_x86_64_target_is_installed).
///
/// Cancel-aware: races the child against `cancel.cancelled()` with
/// `kill_on_drop(true)`, so a Cancel during a cold `rustup` returns
/// [`SabrageError::Cancelled`] at once (tests::rustup_gate_is_cancel_aware_and_kills_the_child).
````

### B1956 · l.360–364 · REWRITE (confirm) · rule 1.2 · 5 → 5

````text
/// Run `spec` through `ctx.executor`, mapping a non-zero real exit to
/// [`SabrageError::ChildFailed`] with an empty tail: build.sh runs under a bare
/// `set -e` and has no bespoke `die` text for the `cmake` calls, and every line
/// the child printed already reached the event stream, so re-capturing a tail
/// buys nothing (tests::run_child_ok_maps_a_real_failure_to_child_failed_with_no_tail).
````

### B1957 · l.378–380 · REWRITE (amend) · rule 1.2 · 3 → 6

````text
/// [`run_child_ok`], plus best-effort [`StageEvent::Progress`] derived from
/// `spec`'s stdout. It needs its own sink because `Executor::run_child`'s is
/// fixed at construction, and it takes that path only on a real run so
/// `--dry-run` keeps `Executor::planned()`'s bookkeeping
/// (tests::run_ninja_build_ok_derives_progress_and_forwards_output_on_a_real_run,
/// tests::run_ninja_build_ok_never_spawns_under_dry_run_either).
````

### B1962 · l.449–452 · REWRITE (confirm) · rule 1.3 · 4 → 2

````text
        // Dry run takes `fixes/helper.rs`'s "would install" verb rather than
        // claiming a copy that did not happen.
````

### B1963 · l.464–467 · REWRITE (confirm) · rule 1.5 · 4 → 4

````text
    // `copy_if_changed` compares bytes, so `Unchanged` says nothing about the
    // destination's mode, and a staged helper without its execute bit FAILs
    // doctor's `build.helper-arm64` (tests::a_byte_identical_but_non_executable_staged_helper_is_repaired).
    // Remove first: a fresh `std::fs::copy` takes the source's mode.
````

### B1971 · l.640–645 · REWRITE (confirm) · rule 1.4 · 6 → 3

````text
    // "all build outputs present" is a hard factual claim a dry run cannot
    // honestly make, so unlike the narrative "built" rows this one is skipped
    // entirely rather than swapped to a future-tense verb.
````

## `sabrage/crates/sabrage-core/src/stages/install.rs`

Deleted (nothing carried): B2006, B2008, B2009, B2011, B2018, B2019, B2023, B2036, B2042, B2053, B2059, B2060, B2089, B2095

### B2001 · l.1–71 · REWRITE (amend) ·p · rule 1.6 · 71 → 22

````text
//! `demo.sh install` — install the bridge into CrossOver, the bottle, and the
//! host loader. Idempotent (hash-gated copies); the ONLY stage that can prompt
//! for administrator authorization.
//!
//! Reference: `scripts/demo/install.sh`. Preconditions first (`require_bottle`,
//! CrossOver present, the three build outputs, a complete DXMT artifact set),
//! then four layers — 1. the global DXMT overlay, 2. global wineopenxr, 3. the
//! bottle, 4. the host registration — each opening with a
//! [`crate::stages::StageCtx::section`] banner whose text matches the shell's
//! `print -r --` line verbatim.
//!
//! Layers 1–2 write inside `CrossOver.app` and need macOS App Management
//! (TCC), not root. [`crate::privilege::upgrade_write_error`] is called
//! uniformly — a safe no-op outside a `.app`, and no layer-3 destination is
//! inside one. Layer 4 is the pipeline's only privileged write, skipped when
//! the on-disk bytes already match: that comparison is literal, so one extra
//! byte makes the two front-ends rewrite the root-owned file after each other.
//!
//! Order, TCC classification and layer 4's file form are pinned by
//! tests::{run_dry_runs_all_four_layers_in_order_without_touching_the_machine,
//! a_permission_denied_inside_crossover_app_is_tcc_denied_with_a_remedy,
//! layer_four_stages_the_host_manifest_file_form_byte_for_byte}.
````

### B2002 · l.82–90 · REWRITE (confirm) ·p · rule 1.6 · 9 → 5

````text
/// How long layer 3 waits for wine to flush `system.reg` after a successful
/// `reg add`, before settling for the (never fatal) lazy-flush warning.
///
/// Native-only: PARITY.md § Declared by the 2026-08-30 adversarial review (round 1 fixes),
/// "**Registry flush re-probe after `reg add`.**".
````

### B2004 · l.96–101 · REWRITE (confirm) · rule 1.5 · 6 → 5

````text
/// Name prefix of an **uncommitted** stock-DXMT capture (layer 1).
///
/// `cp -R` is not atomic, so anything still carrying this prefix is a truncated
/// tree: never trusted as the backup, always swept
/// (tests::an_interrupted_backup_never_becomes_the_trusted_stock_backup).
````

### B2010 · l.144–146 · REWRITE (confirm) ·p · rule 1.5 · 3 → 3

````text
    // A preview must not read like completed work in the event log: mutation
    // rows say "would" when nothing was mutated (vocabulary shared with
    // build.rs / setup.rs; tests::no_dry_run_row_claims_a_completed_mutation).
````

### B2012 · l.155–159 · REWRITE (confirm) · rule 1.3 · 5 → 4

````text
    // Swept before the branch below, never inspected: a partial outlives the
    // run that created it (a cancelled `remove_dir_all`, a SIGKILL) and nothing
    // else ever collects it
    // (tests::a_leftover_partial_capture_is_swept_not_promoted).
````

### B2013 · l.163–170 · REWRITE (confirm) · rule 1.5 · 8 → 3

````text
            // Never re-copied: `lib/dxmt` may already hold the fork, so a
            // re-capture would destroy the only rollback there is
            // (tests::an_empty_stock_backup_is_warned_about_and_never_recaptured).
````

### B2014 · l.180–191 · REWRITE (amend) · rule 1.5 · 12 → 6

````text
        // A shelled `cp -R`, the pipeline's first write into `CrossOver.app`:
        // no `io::Error` to classify, so TCC upgrading goes through
        // `upgrade_child_write_error` (tests::a_refused_stock_backup_cp_is_tcc_denied_and_removes_the_partial_dir).
        // It copies to a sibling nothing trusts: a truncated tree under the
        // committed name would be indistinguishable from a finished backup
        // (tests::an_interrupted_backup_never_becomes_the_trusted_stock_backup).
````

### B2015 · l.200–208 · REWRITE (confirm) · rule 1.3 · 9 → 5

````text
        // The commit: `mv` within one directory is `rename(2)`, so
        // `dxmt.stock-backup` exists only for a `cp -R` that ran to completion,
        // which makes the `is_dir()` test above a completeness test rather than
        // a guess. Skipped when nothing was copied: a dry run planned the copy
        // instead of performing it, so there is no partial to rename.
````

### B2016 · l.215–218 · REWRITE (confirm) · rule 1.3 · 4 → 3

````text
            // The tail is empty (`run_child` streams rather than collects), so
            // `upgrade_child_write_error` cannot call this App Management —
            // right, since the `cp -R` into this directory just succeeded.
````

### B2017 · l.233–236 · REWRITE (confirm) · rule 1.3 · 4 → 3

````text
            // Another writer (an unserialized `./demo.sh install`, which takes
            // no operation lock) captured stock between the test above and now:
            // `mv` would move the partial inside it, so drop it instead.
````

### B2020 · l.326–329 · REWRITE (confirm) · rule 1.5 · 4 → 3

````text
        // Output is streamed into the run's event log rather than the shell's
        // `>/dev/null 2>&1` — PARITY.md § Install (the one privileged write),
        // "`wine … reg add`'s output is captured into the event stream".
````

### B2022 · l.341–350 · REWRITE (confirm) · rule 1.7 · 10 → 3

````text
            // Warn, never Fail: wine flushes system.reg lazily, so the probe
            // is retried for REGISTRY_FLUSH_TIMEOUT before warning. `?`: a Stop
            // during the wait ends the stage here, before layer 4.
````

### B2024 · l.367–372 · REWRITE (confirm) · rule 1.5 · 6 → 3

````text
    // Refuse before the currency test: install.sh's two `${//}` substitutions
    // cannot represent a control character, so the manifest would be invalid
    // JSON, written as root (tests::a_control_character_in_the_dylib_path_refuses_layer_four).
````

### B2025 · l.378–381 · REWRITE (confirm) ·p · rule 1.5 · 4 → 4

````text
    // The *comparison* form (no trailing newline), used only for install.sh's
    // `[ "$(cat "$HOST_XR_JSON")" = "$WANT" ]` currency test. The bytes that
    // land on disk are rendered inside the privileged write from the dylib path
    // (tests::layer_four_stages_the_host_manifest_file_form_byte_for_byte).
````

### B2026 · l.391–395 · REWRITE (confirm) · rule 1.3 · 5 → 3

````text
        // `write_host_manifest_privileged` emits StageEvent::NeedsAdmin itself
        // and re-runs the currency test under the prompt, so it can still come
        // back Skipped; saying "written" here would be a lie.
````

### B2029 · l.422–444 · REWRITE (confirm) ·p · rule 1.2 · 23 → 13

````text
/// `lib.sh`'s `install_if_changed`, split the way [`crate::executor::Executor`]
/// intends: the executor does the byte compare and the copy, the caller prints
/// the row — `info "unchanged: <dst>"` / `ok "installed: <dst>"`, verbatim,
/// `<dst>` the full destination path exactly as the shell prints `$2`.
///
/// # Errors
///
/// A `PermissionDenied` under a `.app` bundle is upgraded by
/// [`privilege::upgrade_write_error`] to [`SabrageError::TccDenied`], propagated
/// as-is. Every other copy failure emits the io cause as stderr-shaped output
/// and dies with `lib.sh`'s verbatim `copy failed: $1 -> $2`. See
/// tests::{a_permission_denied_inside_crossover_app_is_tcc_denied_with_a_remedy,
/// a_non_tcc_copy_failure_dies_with_lib_shs_copy_failed_text}.
````

### B2032 · l.473–478 · REWRITE (confirm) · rule 1.5 · 6 → 5

````text
            // Everything else: the io cause as stderr-shaped output ahead of
            // lib.sh's verbatim die text, so a plain PermissionDenied, a
            // read-only volume and ENOSPC stay distinguishable. PARITY.md §
            // Install (the one privileged write), "A copy failure prints the OS
            // error as one stderr-shaped output line".
````

### B2033 · l.496–503 · REWRITE (confirm) ·p · rule 1.7 · 8 → 5

````text
/// `grep -q 'ActiveRuntime.*openxr.*wineopenxr64.json' "$PREFIX/system.reg"`:
/// true when one line carries all three literals in that order. Duplicated from
/// `checks::bridge`'s private `registry_has_active_runtime` because this stage
/// needs the read before deciding whether to run `reg add`, not only afterward
/// (tests::registry_current_requires_all_three_literals_in_order_on_one_line).
````

### B2034 · l.522–528 · REWRITE (confirm) · rule 1.7 · 7 → 4

````text
/// install.sh's bare post-write re-probe, `grep -q 'ActiveRuntime'`: looser
/// than [`registry_current`], because it calls a bottle still holding a *stale*
/// `ActiveRuntime` value registered. Test-only pin on the shell semantics the
/// strict predicate is contrasted against.
````

### B2035 · l.536–557 · REWRITE (amend) ·p · rule 1.7 · 22 → 19

````text
/// Wait for wine's lazy `system.reg` flush; `Ok(false)` when it never landed.
///
/// The wait is on [`registry_current`], the same three-literal test the launch
/// preflight blocks on (`bottle.registry`); the timeout arm never falls back to
/// install.sh's looser `ActiveRuntime` grep, so a stale value neither ends the
/// poll early nor earns an `OK` row the next launch rejects. A timeout is
/// always a warn — one more than the shell prints for a stale-value bottle.
///
/// # Errors
///
/// Cancellation is [`SabrageError::Cancelled`], never `Ok(false)`: the caller's
/// next steps are an `OK` row and the pipeline's one privileged write, so a
/// Stop must not be reported as a completed registration.
///
/// tests::{a_stale_active_runtime_value_does_not_end_the_flush_wait,
/// a_flush_that_never_lands_warns_even_with_a_stale_active_runtime,
/// a_cancel_during_the_registry_wait_stops_before_the_privileged_layer};
/// PARITY.md § Declared by the 2026-08-30 adversarial review (round 1 fixes),
/// "**Registry flush re-probe after `reg add`.**".
````

### B2037 · l.573–576 · REWRITE (confirm) · rule 1.7 · 4 → 3

````text
/// True when `dir` holds no entries at all — the shape an interrupted `cp -R`
/// leaves behind. A backup this stage committed is complete by construction;
/// this catches the ones it did not.
````

### B2038 · l.581–591 · REWRITE (confirm) ·p · rule 1.4 · 11 → 7

````text
/// Delete an uncommitted stock-DXMT capture, cancellation included.
///
/// The executor is tried first (so a dry run records the removal); every
/// [`crate::executor::RealExecutor`] mutation refuses once cancelled, so the
/// fallback goes straight to the filesystem — safe only because the path
/// carries [`PARTIAL_BACKUP_PREFIX`] and a uuid this run minted. A removal that
/// still fails is a `warn`: the leftover is inert and the next install sweeps it.
````

### B2040 · l.623–627 · REWRITE (confirm) · rule 1.5 · 5 → 5

````text
/// [`SabrageError::Cancelled`] once Stop has been pressed.
///
/// Layer 4 is the pipeline's only privileged write: a Stop that arrived during
/// layer 3 must end the stage before the authorization prompt, not after
/// (tests::a_cancel_during_the_registry_wait_stops_before_the_privileged_layer).
````

### B2041 · l.646–651 · REWRITE (confirm) · rule 1.2 · 6 → 3

````text
    /// A fresh scratch directory, unique per call. Several tests share
    /// `scratch("full")` through `full_fixture` and `cargo test` runs them
    /// concurrently, so a shared path would race their fixture trees.
````

### B2044 · l.676–686 · REWRITE (amend) ·p · rule 1.2 · 11 → 6

````text
    /// [`DryRunExecutor`] with two test affordances: every `write_atomic` is
    /// kept **with its bytes** (the plan records only a byte count, and the
    /// host manifest is defined by its bytes), and `copy_if_changed` can fail
    /// with `PermissionDenied` under a path prefix — the shape a macOS App
    /// Management refusal arrives in. Everything else delegates, so `run()`
    /// behaves as under a plain dry run and still touches nothing.
````

### B2046 · l.694–698 · REWRITE (confirm) · rule 1.2 · 5 → 4

````text
        /// `dir_copy` fails the way an **interrupted** `cp -R` does: the
        /// destination is really created and really left half-populated, then
        /// the copy reports failure. `remove_dir_all` becomes real too, so the
        /// on-disk end state of the failure path is observable.
````

### B2055 · l.920 · REWRITE (amend) · rule 2.3 · 1 → 1

````text
        // On-disk with the shell's single trailing newline (`print -r -- "$WANT"`).
````

### B2056 · l.924 · REWRITE (confirm) · rule 1.3 · 1 → 1

````text
        // No trailing newline: `$(cat …)` has nothing to strip, still current.
````

### B2063 · l.988 · REWRITE (confirm) · rule 2.3 · 1 → 1

````text
        // DXMT artifacts: every file the contract lists.
````

### B2067 · l.1053 · REWRITE (confirm) · rule 1.1 · 1 → 1

````text
        // Layer 1: one DirCopy (the backup), then one Copy/Skip per dxmt.files entry.
````

### B2079 · l.1218–1224 · REWRITE (amend) ·p · rule 1.7 · 7 → 5

````text
    /// install.sh writes `print -r -- "$WANT"`, so the live
    /// `/usr/local/share/openxr/1/active_runtime.x86_64.json` ends `7d 0a 7d
    /// 0a`. Layer 4 stages exactly those bytes, not the newline-less
    /// comparison form. Driven through the real [`run`] in dry-run against a
    /// fixture destination.
````

### B2085 · l.1323–1330 · REWRITE (confirm) · rule 1.7 · 8 → 5

````text
    /// The other half of the arm above: a copy failure that is **not** TCC
    /// (layer 3's destinations live in the bottle, never inside a `.app`, so
    /// `classify_write_error` can never call them App Management) still reaches
    /// the run log as one `Fatal` carrying `lib.sh`'s own
    /// `die "copy failed: $1 -> $2"`, with the io cause emitted before it.
````

### B2086 · l.1359 · REWRITE (confirm) · rule 1.6 · 1 → 1

````text
        // lib.sh's copy helper: `cp "$1" "$2" || die "copy failed: $1 -> $2"`.
````

### B2090 · l.1406–1409 · REWRITE (confirm) · rule 1.7 · 4 → 3

````text
    /// A `cp -R` that dies right after creating the destination leaves an empty
    /// `dxmt.stock-backup`. It is warned about and deliberately left alone:
    /// re-copying after an install has landed would capture the fork, not stock.
````

### B2104 · l.1746–1749 · REWRITE (confirm) · rule 1.7 · 4 → 4

````text
    /// Stop pressed while layer 3's `reg add` runs. Cancellation is distinct
    /// from a timed-out flush, so the stage neither warns nor claims the
    /// registration and never enters layer 4 — the pipeline's only privileged
    /// write.
````

### B2106 · l.1858–1863 · REWRITE (confirm) ·p · rule 1.7 · 6 → 4

````text
    /// The timeout arm of r1:A6-4's wait: when the flush never lands the stage
    /// warns, even for a bottle whose stale `ActiveRuntime` value satisfies
    /// the shell's `grep -q ActiveRuntime`. One warn more than the shell
    /// prints for this state, in the honest direction.
````

## `sabrage/crates/sabrage-core/src/stages/mod.rs`

Deleted (nothing carried): B2113, B2123, B2134, B2145, B2147, B2150, B2170, B2174, B2180

### B2110 · l.1–86 · REWRITE (amend) ·p · rule 1.6 · 86 → 29

````text
//! Stage orchestration: the context every stage runs in, the operation lock, and
//! the dispatcher. One stage is one `./demo.sh <verb>`, a plain `async fn`
//! readable next to the script it mirrors.
//!
//! [`OPERATION_LOCK`] plus an advisory file lock ([`OPERATION_LOCK_FILE_NAME`])
//! admit one mutating operation at a time across every Sabrage process; doctor is
//! read-only and never takes it, only annotates with [`operation_in_progress`].
//! demo.sh does not participate (PARITY.md § Declared by the 2026-08-30
//! adversarial review (round 1 fixes), "Cross-process operation lock.").
//!
//! `setup`/`build`/`install` refuse while [`live_session_block`] sees a session,
//! and every stage but `stop` refuses on contract skew
//! ([`deny_on_contract_skew`]); both refusals run before the lock wait and again
//! with the lock held. `all` is a caller-level loop over [`Stage::ALL_CHAIN`]
//! with a fresh [`StageCtx`] per stage, not a sixth stage.
//! See tests::{run_stage_refuses_setup_build_and_install_while_a_session_is_live,
//! a_queued_stage_is_refused_when_a_session_goes_live_during_the_wait,
//! every_mutating_stage_refuses_a_checkout_the_binary_was_not_built_from,
//! a_queued_stage_announces_itself_and_cancels_out_of_the_wait}.
//!
//! # Lock policy for `run`
//!
//! `run` releases the lock once the wine child is up, so `stop` and every fix
//! stay reachable during a session: [`run::run`] takes the guard by value and
//! drops it at the launch boundary; [`run_stage_holding_lock`] passes `None`.
//!
//! `tokio::sync::Mutex` is not reentrant: a caller already holding the lock must
//! use [`run_stage_holding_lock`] / [`crate::fixes::apply_holding_lock`], or it
//! deadlocks in silence.
````

### B2111 · l.112–122 · REWRITE (confirm) · rule 1.2 · 11 → 9

````text
/// Where a stage's events go.
///
/// A plain callback rather than an `mpsc::Sender`: every producer is synchronous
/// at the point of emission (a check resolving, a line being printed, a pump
/// forwarding a chunk), so a channel would force either an `.await` in those
/// places or a `try_send` that can silently drop a row.
///
/// Sinks are called from arbitrary tasks (both output pumps of every child), so
/// they must be `Send + Sync` and cheap.
````

### B2114 · l.132–138 · REWRITE (confirm) · rule 1.5 · 7 → 6

````text
/// The stage-relevant slice of the `WINEVR_*` mirror — all six flags demo.sh
/// accepts, plus Sabrage's own `dry_run`.
///
/// `no_audio` / `no_dashboard` / `wired` are read only by [`Stage::Run`] and by
/// the `run.wired-adb` preflight, which is why [`StageCtx::check_ctx`] forwards
/// them. See tests::check_ctx_forwards_every_launch_flag.
````

### B2132 · l.261–274 · REWRITE (amend) · rule 1.2 · 14 → 10

````text
/// A self-contained fixture context: [`null_sink`], a fresh
/// [`CancellationToken`], and always a [`DryRunExecutor`] — `opts.dry_run` is
/// forced true regardless of the caller, so a fixture can never mutate the
/// machine.
///
/// Exists for a downstream crate that needs only *a* `StageCtx` to drive a
/// text-rendering function (`sabrage-parity`'s A1-3 pins over
/// `stages::run::actions::banner_events`, `bs_win_path`,
/// `preflight::block_die`, `preflight::post_fix_die`, …) and should not have to
/// depend on `tokio_util` to build one.
````

### B2146 · l.393–411 · REWRITE (confirm) ·p · rule 1.6 · 19 → 11

````text
/// `lib.sh`'s `require_bottle`, message text verbatim.
///
/// # Errors
///
/// Two `die` strings. The missing-name one is two lines and ends with the bottle
/// list, which keeps the shell's trailing space (`tr '\n' ' '` appends one per
/// name), so an empty list renders as `Existing bottles: ` with nothing after it;
/// the not-found one is a single line. Deliberately **not** the text of doctor's
/// `bottle.exists` row ([`Bottle::resolve`]), which splits message and remedy — a
/// stage aborting must read exactly like the shell aborting.
/// See tests::require_bottle_reproduces_lib_sh_die_text.
````

### B2148 · l.442–458 · REWRITE (amend) · rule 1.6 · 17 → 8

````text
/// `run`'s wineserver-reset budget: **5 s, fatal on timeout**.
///
/// Reference: scripts/demo/run.sh, the poll loop over the backgrounded
/// `wineserver -w` that warns "wineserver still alive after 5s" and then dies.
///
/// Deliberately distinct from [`STOP_WINESERVER_WAIT`] — 5 s fatal here, 4 s
/// soft there. Never unify them; collapsing the two constants silently changes
/// one of the behaviours. See tests::the_two_wineserver_budgets_stay_distinct.
````

### B2149 · l.461–467 · REWRITE (confirm) · rule 1.7 · 7 → 5

````text
/// `stop`'s wineserver-wait budget: **4 s, never fatal**.
///
/// Reference: scripts/demo/lib.sh, `stop_wine` — it polls, then gives up
/// (`kill $_wp 2>/dev/null || true`). See [`RUN_WINESERVER_WAIT`] for why the two
/// budgets stay apart. Re-exported as `stages::stop::STOP_WINESERVER_WAIT`.
````

### B2152 · l.477–481 · REWRITE (confirm) ·p · rule 1.5 · 5 → 6

````text
/// The cross-process half's file name, under Sabrage's own support directory.
///
/// Sabrage-only: `demo.sh` does not take it, so this serializes the GUI against
/// the `sabrage` CLI and against a second GUI instance, not against the shell
/// pipeline (PARITY.md § Declared by the 2026-08-30 adversarial review
/// (round 1 fixes), "Cross-process operation lock.").
````

### B2158 · l.526–533 · REWRITE (confirm) · rule 1.7 · 8 → 7

````text
/// [`acquire_operation_lock`], abandoning the wait when `cancel` fires.
///
/// `None` means "cancelled while waiting" — the caller must not proceed. Both
/// halves are cancellable: the in-process mutex (another stage in this process)
/// and the advisory file lock (a `sabrage` CLI build in another process, which
/// can hold it for minutes), so the user's Stop can reach a queued stage.
/// See tests::the_file_lock_wait_gives_up_when_the_token_fires.
````

### B2171 · l.671–684 · REWRITE (confirm) · rule 1.7 · 14 → 9

````text
/// Why a mutating operation must not start right now, or `None` when nothing on
/// this machine looks like a live session.
///
/// A thin alias for [`crate::session::live_session_reason`]: the session layer
/// owns the policy, and this is the name every stage and fix refusal reads
/// ([`deny_stage_while_session_live`], [`crate::fixes`]'s `deny_if_session_live`,
/// [`crate::fixes::adb::remove_adb_forwards`]). Do not reintroduce a weaker local
/// copy: the doors that mutate must be guarded by no less than the doors that
/// only refuse. See [`crate::session::session_block_at`] for A4-1's seven signals.
````

### B2186 · l.839–869 · REWRITE (confirm) ·p · rule 1.7 · 31 → 19

````text
/// Run one stage, taking [`OPERATION_LOCK`] for its duration.
///
/// Emits [`StageEvent::StageStarted`] first and [`StageEvent::StageFinished`]
/// last — on the failure path too — so a UI never sees an unfinished stage.
///
/// `StageStarted` is emitted **before** the lock: that event is the front-end's
/// only source of the run id, making the wait behind another process's build
/// visible and cancellable. A cancelled wait ends as [`SabrageError::Cancelled`]
/// (exit 130) with nothing touched. The live-session refusal stays *before* the
/// event, matching demo.sh which dies before printing a stage banner.
///
/// [`deny_before_dispatch`] runs **again** once the lock is in hand: the wait is
/// unbounded, and a `run` admitted meanwhile publishes its live session and then
/// releases the lock at the launch boundary, so an install could otherwise
/// replace the artifacts of a streaming game. The second refusal goes through
/// [`finish_stage`], `StageStarted` having already been emitted.
/// See tests::{run_stage_brackets_the_stage_with_events_even_when_it_fails,
/// a_queued_stage_announces_itself_and_cancels_out_of_the_wait,
/// a_queued_stage_is_refused_when_a_session_goes_live_during_the_wait}.
````

### B2187 · l.871–873 · REWRITE (confirm) · rule 2.3 · 3 → 2

````text
    // Before the lock, not after: waiting minutes for another process's build
    // only to refuse is worse than refusing straight away.
````

### B2189 · l.891–894 · REWRITE (confirm) · rule 2.3 · 4 → 3

````text
        // The one stage that gives the lock back early: the guard is *moved
        // into* the stage, which drops it once the wine child is up. `run_stage`
        // must not keep one of its own, or the release would be a no-op.
````

### B2199 · l.1069 · REWRITE (amend) · rule 1.5 · 1 → 3

````text
        // PARITY.md § Invariants that must NOT change (byte/behavior parity),
        // "wineserver budgets (5 s fatal / 4 s soft)": 5 s fatal (run) vs
        // 4 s soft (stop). Never unify.
````

### B2202 · l.1103–1109 · REWRITE (amend) ·p · rule 1.2 · 7 → 3

````text
    /// The advisory file lock, not `OPERATION_LOCK`, excludes a second Sabrage
    /// process. `flock` is per open file description, so a second `File` on the
    /// same path in this process sees exactly what another process sees.
````

### B2219 · l.1351–1357 · REWRITE (confirm) ·p · rule 1.7 · 7 → 5

````text
    /// The live-session refusal is checked before the operation lock **and
    /// again after it**: a stage admitted while idle can wait minutes behind
    /// another process's build, and a `run` that wins the lock race publishes
    /// its session and releases the lock at launch — so the queued stage must
    /// be re-refused or it replaces the artifacts of a streaming game.
````

## `sabrage/crates/sabrage-core/src/stages/run/actions.rs`

Deleted (nothing carried): B2234, B2241, B2242, B2247, B2250, B2253, B2257, B2270, B2279, B2283, B2293, B2296, B2300, B2304, B2324, B2338, B2340, B2341, B2342

### B2230 · l.1–21 · REWRITE (amend) ·p · rule 1.6 · 21 → 11

````text
//! The seven contract-ordered launch actions ([`LAUNCH_ACTION_IDS`]) —
//! unconditional preparation steps, not checks: no pass/fail, no remedy, no gate.
//! `audio-route` and `dashboard` are acquired in [`super::guards`] instead,
//! because acquiring them is the same act as arming their undo. See
//! tests::{the_guarded_actions_are_listed_and_launch_is_last,
//! one_step_id_per_action_plus_the_three_run_only_phases}.
//!
//! Reference: scripts/demo/run.sh, the `# launch-action:` tags.
//!
//! Every mutation goes through [`crate::executor::Executor`], so `--dry-run` plans
//! it; read-only probes run in both modes, keeping the plan accurate.
````

### B2235 · l.66–70 · REWRITE (confirm) · rule 1.2 · 5 → 6

````text
/// The first serial in `adb devices`' stdout whose state is exactly `device`.
///
/// Mirrors run.sh's `awk 'NR>1 && $2=="device"{print $1; exit}'`: the header row
/// is skipped, and so is any row that is blank or in another state
/// (`unauthorized`, `offline`). See
/// tests::first_device_serial_matches_the_awk_program.
````

### B2236 · l.80–87 · REWRITE (confirm) ·p · rule 1.2 · 8 → 7

````text
/// The first `device`-state serial from `adb devices`, or `None`.
///
/// A read-only probe: bypasses the executor and runs under `--dry-run` too.
/// Carries the launch's cancellation token because a wedged `adb` here would
/// otherwise hold the operation lock with Cancel unable to interrupt it. A
/// cancelled or timed-out probe returns `None`, matching run.sh's empty
/// `$WIRED_SER`. See tests::a_cancel_during_the_device_probe_is_a_cancellation.
````

### B2237 · l.99–100 · REWRITE (amend) ·p · rule 1.5 · 2 → 3

````text
/// `tcp:<port>` for each of the contract's `ports.stream` — never a literal
/// `"tcp:9943"` (PARITY.md § Invariants that must NOT change (byte/behavior
/// parity), "adb `forward --remove` per-serial").
````

### B2238 · l.110–131 · REWRITE (amend) · rule 1.7 · 22 → 14

````text
/// Remove both stream forwards, never aborting on a failed removal, then bring
/// the persisted record back in line per port: a removal that succeeded drops its
/// record, a removal that failed keeps it.
///
/// A failed `--remove` is indeterminate — usually the device is gone and the
/// forward with it, but it may equally be a transient adb failure over a
/// still-installed `tcp:9943` — and the record is the only thing that would ever
/// retry it; [`crate::session::reconcile`]'s `restore_forwards` makes the same
/// distinction. The removals run on a fresh, non-cancelled executor
/// ([`super::teardown_ctx`]) because the usual reason to be rolling back is a
/// cancelled launch, whose executor refuses every child and every write. The save
/// is best-effort. See
/// tests::{a_rollback_whose_removal_fails_keeps_the_forward_on_record,
/// a_cancellation_mid_loop_still_rolls_the_first_forward_back}.
````

### B2239 · l.159–175 · REWRITE (amend) ·p · rule 1.6 · 17 → 14

````text
/// `launch-action: adb-forward-hygiene`. Reference: scripts/demo/run.sh.
///
/// `--wired`: create `tcp:9943` and `tcp:9944` on the first device whose state is
/// exactly `device`; if either fails, remove both and die. Otherwise remove exactly
/// those two local ports per serial — never `--remove-all`, which would delete
/// forwards this pipeline knows nothing about (PARITY.md § Invariants that must NOT
/// change (byte/behavior parity), "adb `forward --remove` per-serial").
///
/// Each intended forward is persisted to `state_path` before the `adb forward` that
/// creates it: an over-recorded forward costs one harmless `--remove` that finds
/// nothing, where an under-recorded one is the stale forward that silently breaks
/// the next WiFi run. See
/// tests::{wired_plans_both_forwards_and_reports_them,
/// a_failed_forward_removes_both_ports_and_dies_with_run_shs_text}.
````

### B2240 · l.185–191 · REWRITE (confirm) · rule 1.3 · 7 → 3

````text
        // `_at`, not the plain fix: these rows must be attributed to the run
        // stage's own step, not to the doctor fix list's `fix.remove-adb-forwards`.
        // See tests::the_non_wired_forward_cleanup_is_stamped_with_the_run_stages_step.
````

### B2246 · l.245–250 · REWRITE (confirm) · rule 1.3 · 6 → 3

````text
        // Every non-success leaves through the same door, a failed *exec*
        // included: a cancellation between the two ports would otherwise leave
        // the first forward on the device with nothing on disk naming it.
````

### B2251 · l.277–279 · REWRITE (confirm) · rule 1.6 · 3 → 3

````text
/// The die run.sh prints when one port's `adb forward` fails on one device.
/// `pub` (A1-3) so `sabrage-parity` can pin it against run.sh by calling the
/// real renderer instead of copying the sentence.
````

### B2252 · l.287–289 · REWRITE (amend) · rule 1.6 · 3 → 3

````text
/// The `wired mode: adb forward … up` info line; the caller passes the contract's
/// `ports.stream` specs, so the shell string's `tcp:9943/tcp:9944` cannot drift.
/// `pub` (A1-3), same reason as [`wired_forward_failed_die`].
````

### B2254 · l.299–305 · REWRITE (amend) · rule 1.5 · 7 → 7

````text
/// `"<pid> <exe-basename>"` per process, space-joined with a trailing space —
/// the shape of `pgrep -lf wineserver | tr '\n' ' '`, and the same rendering
/// `stop`'s private `format_survivors` produces.
///
/// `fallback` names a process whose exe path has no file name. (PARITY.md § Stop,
/// "The survivor warning lists".) See
/// tests::survivors_render_as_pid_basename_pairs_with_a_trailing_space.
````

### B2255 · l.322–333 · REWRITE (confirm) ·p · rule 1.2 · 12 → 8

````text
/// `launch-action: wineserver-reset`. Reference: scripts/demo/run.sh.
///
/// `wineserver -k` (failure ignored), then a bounded `-w` wait of
/// [`crate::stages::RUN_WINESERVER_WAIT`] — fatal on timeout, unlike `stop`'s 4 s
/// advisory wait. The timeout path warns with the survivor list
/// ([`wineserver_still_alive_warn`] over
/// [`crate::process::find_processes_by_cmdline`]) before dying. See
/// tests::wineserver_reset_plans_k_then_w_and_reports_down.
````

### B2256 · l.338–341 · REWRITE (amend) · rule 1.3 · 4 → 4

````text
    // No CrossOver means `$WINESERVER` is empty: the shell "runs" it, swallows
    // the command-not-found and reaches `ok` anyway. The `run.wine-exec`
    // preflight has already died by then, so this branch is reproduced rather
    // than special-cased. See tests::wineserver_reset_without_crossover_still_reports_down.
````

### B2262 · l.427–429 · REWRITE (confirm) · rule 1.6 · 3 → 3

````text
/// `$BS_DIR/Beat Saber_Data/Plugins/x86_64/steam_api64.dll`, else
/// `$BS_DIR/steam_api64.dll` — the second is returned even when it is absent, so
/// the caller produces [`steam_api_missing_die`] for it.
````

### B2264 · l.446–447 · REWRITE (confirm) · rule 1.6 · 2 → 3

````text
/// The `wineserver still alive after <n>s` warn; the survivor list is
/// [`format_survivors`]' rendering.
/// `pub` (A1-3), same reason as [`wired_forward_failed_die`].
````

### B2269 · l.474–488 · REWRITE (confirm) ·p · rule 1.6 · 15 → 13

````text
/// `launch-action: goldberg-stage`. Reference: scripts/demo/run.sh.
///
/// Stages four byte-exact artifacts: `steam_api64.dll` (under
/// `Beat Saber_Data/Plugins/x86_64/` or at the game root); `<api>.orig-steam`,
/// created once and never overwritten (the only copy of the real Steam dll on the
/// machine); the Goldberg dll copied over the live dll when bytes differ, hash
/// mismatch tolerated unlike in setup (parity decision 20); and `steam_appid.txt`
/// (appid digits, no trailing newline) plus the three `steam_settings/` flag files.
///
/// Fatal when no `steam_api64.dll` exists, when `.orig-steam` is not a regular
/// file, or when any copy/write fails. See
/// tests::{goldberg_backs_up_once_installs_and_writes_the_exact_artifacts,
/// goldberg_refuses_when_the_backup_name_is_not_a_regular_file}.
````

### B2271 · l.503–505 · REWRITE (amend) · rule 1.6 · 3 → 3

````text
    // Made once, never refreshed: overwriting it with an already-Goldberged dll
    // would destroy the only copy of the real Steam library on this machine. See
    // tests::goldberg_installs_the_dll_and_flag_files_and_never_refreshes_the_backup.
````

### B2273 · l.512–520 · REWRITE (amend) ·p · rule 1.5 · 9 → 6

````text
        // Sabrage-only refusal: run.sh's `-f` fails on non-regular names too,
        // so `[ ! -f "$API.orig-steam" ]` fires, `cp` writes into the directory
        // or through the symlink, and Goldberg goes over the live dll with no
        // usable backup (PARITY.md § Declared by the 2026-08-30 adversarial
        // review (round 1 fixes), "A non-regular `.orig-steam` refuses the
        // launch"). See tests::goldberg_refuses_when_the_backup_name_is_not_a_regular_file.
````

### B2274 · l.531–536 · REWRITE (confirm) · rule 1.3 · 6 → 4

````text
        // Sabrage-only row (run.sh is silent here): the live dll is ALREADY the
        // Goldberg build, so the backup this line mints holds Goldberg, not
        // Steam — and `store::goldberg::revert_original_steam_dll` would
        // otherwise copy these bytes back and call it a restore.
````

### B2275 · l.556–557 · REWRITE (amend) · rule 1.6 · 2 → 2

````text
    // `cmp -s`, not a hash: run tolerates a Goldberg dll that does not match
    // setup's pin (parity decision 20).
````

### B2276 · l.572 · REWRITE (amend) · rule 1.6 · 1 → 1

````text
    // `printf '%s' "$BS_APPID"`: the digits, no trailing newline.
````

### B2277 · l.587–589 · REWRITE (confirm) ·p · rule 1.6 · 3 → 3

````text
    // `mkdir -p` then three `: >` truncate-creates. The shell has no `|| die` on
    // either; a failure here is propagated rather than swallowed
    // (docs/design/design-core.md §6.6: no silent aborts, no silent successes).
````

### B2278 · l.598–605 · REWRITE (confirm) · rule 1.5 · 8 → 8

````text
/// `die "<text>"` for a failed executor primitive, with the io cause surfaced
/// first as a stderr-shaped [`StageEvent::Output`] line.
///
/// The shell shows `cp`'s own stderr and then dies, and Sabrage has no `cp` child
/// to borrow stderr from, so a plain `PermissionDenied`, a read-only volume and
/// `ENOSPC` stay distinguishable instead of collapsing into one die string
/// (PARITY.md § Install (the one privileged write), "A copy failure prints the OS
/// error").
````

### B2280 · l.624–636 · REWRITE (confirm) ·p · rule 1.6 · 13 → 9

````text
/// `launch-action: adb-reverse-cleanup`. Reference: scripts/demo/run.sh.
///
/// Under `protocol = "alvr"`, runs `adb reverse --remove-all` on the connected
/// device if there is one: oxrsys-era reverse tunnels squat the ALVR client's
/// stream port (`EADDRINUSE`). The legacy `protocol = "oxrsys"` branch is
/// deliberately absent: the contract gates `cfg.protocol.legacy-oxrsys` as `block`
/// natively, so the preflight has already died (PARITY.md § Run preflight (encoded
/// in the contract's per-side gates), "Launch refuses `protocol=oxrsys` outright").
/// See tests::adb_reverse_cleanup_is_silent_without_adb_or_on_the_legacy_protocol.
````

### B2281 · l.647–649 · REWRITE (amend) · rule 1.5 · 3 → 4

````text
    // `reverse --remove-all` IS correct here, unlike `forward --remove-all`:
    // different namespaces, and the ALVR client owns every reverse tunnel it
    // needs (PARITY.md § Invariants that must NOT change (byte/behavior parity),
    // "adb `forward --remove` per-serial").
````

### B2284 · l.670–680 · REWRITE (confirm) ·p · rule 1.2 · 11 → 10

````text
/// The wine child's spec: program, argv, cwd, env. Pure — builds and spawns
/// nothing, so the "copy the equivalent command" affordance and tests can both
/// read it.
///
/// argv matches run.sh exactly:
/// `"$WINE" --bottle "$WINEVR_BOTTLE" --no-update --cx-app "$BS_WIN"`, where
/// `BS_WIN` is [`bs_win_path`]. `paths.wine` is `Option` (no CrossOver); the bare
/// name `wine` stands in, matching the shell's empty `"$WINE"`, and the
/// `run.wine-exec` preflight blocks long before this. See
/// tests::wine_spec_is_run_shs_argv.
````

### B2287 · l.714–728 · REWRITE (amend) ·p · rule 1.2 · 15 → 9

````text
/// The launch environment: `XR_RUNTIME_JSON`, `CX_GRAPHICS_BACKEND=dxmt`,
/// `WINEDEBUG`, and `SteamAppId`/`SteamGameId`. Pure and table-testable.
/// Reference: scripts/demo/run.sh's `# launch-action: launch-wine` block.
///
/// The load-bearing detail is `WINEDEBUG`: the caller's preset wins in both
/// branches (`${WINEDEBUG:-…}`), so `WINEDEBUG=+d3d11` survives `--verbose`
/// (PARITY.md § Invariants that must NOT change (byte/behavior parity),
/// "`WINEDEBUG` caller-precedence"). `inherited_winedebug` is that preset; `None`
/// and `Some("")` both take the branch default (zsh's `:-` treats unset and empty alike).
````

### B2288 · l.753–761 · REWRITE (amend) ·p · rule 1.6 · 9 → 10

````text
/// The six-line launch banner, verbatim, as the exact event sequence the CLI
/// reproduces byte-for-byte (PARITY.md § Invariants that must NOT change
/// (byte/behavior parity), "the six-line launch banner text").
///
/// The `-- launching …` line is a [`StageEvent::Section`]; the rest are
/// [`StageEvent::Text`], leading spaces and empty lines included. See
/// tests::the_banner_is_one_section_with_every_text_row_on_the_launch_step.
///
/// `pub` (A1-3) so `sabrage-parity` can pin this banner against `run.sh` by
/// calling the real renderer instead of copying a substring per line.
````

### B2291 · l.814–829 · REWRITE (confirm) ·p · rule 1.6 · 16 → 12

````text
/// `launch-action: launch-wine`. Reference: scripts/demo/run.sh.
///
/// Emits the banner ([`banner_events`]) before the spawn, picks a non-colliding
/// log name ([`crate::logs::wine_log_candidate`]), and spawns **detached**
/// ([`crate::executor::Executor::spawn_detached`] — never
/// [`crate::process::spawn_streamed`], whose `kill_on_drop(true)` would SIGKILL the
/// game when Sabrage quits). Returns the child and log path, or `Ok(None)` under a
/// dry run. See tests::the_banner_is_one_section_with_every_text_row_on_the_launch_step.
///
/// If the spawn loses the `create_new` race, the next candidate is taken and a
/// corrected `   log: <path>` line is emitted (PARITY.md § Run (launch), "The wine
/// console log is a plain file").
````

### B2307 · l.1092–1099 · REWRITE (amend) ·p · rule 1.2 · 8 → 6

````text
    /// A `/bin/sh` script standing in for `adb`, so the adb branches run
    /// without an Android SDK, a device, or any real forward.
    ///
    /// `forward_exit` controls the exit status of `adb forward` calls;
    /// `--remove` always succeeds. For a rollback whose removal also fails,
    /// use [`every_call_fails_adb`].
````

### B2312 · l.1251 · REWRITE (confirm) ·p · rule 1.6 · 1 → 2

````text
    /// A nonzero `adb forward` removes both ports before dying, matching
    /// scripts/demo/run.sh # launch-action: adb-forward-hygiene.
````

### B2314 · l.1327 · REWRITE (confirm) ·p · rule 1.6 · 1 → 1

````text
        // Both removals still attempted (scripts/demo/run.sh # launch-action: adb-forward-hygiene).
````

### B2317 · l.1375–1378 · REWRITE (amend) ·p · rule 1.4 · 4 → 3

````text
        // Cancel once the fake adb is *inside* the second port's `forward` (it
        // drops a marker before sleeping): a fixed timer either fires before the
        // first forward exists or wastes real seconds.
````

### B2318 · l.1415–1420 · REWRITE (confirm) · rule 1.2 · 6 → 3

````text
    /// A failed write-before-mutate save for the SECOND port leaves through the
    /// same door as a failed `adb forward`: the first forward comes back down
    /// and the in-memory record says so.
````

### B2343 · l.2108 · REWRITE (confirm) ·p · rule 1.6 · 1 → 1

````text
        // Severity is `info`, matching scripts/demo/run.sh # launch-action: adb-reverse-cleanup.
````

## `sabrage/crates/sabrage-core/src/stages/run/guards.rs`

Deleted (nothing carried): B2345, B2368, B2370, B2387, B2401, B2403, B2411, B2415, B2428

### B2344 · l.1–51 · REWRITE (amend) ·p · rule 1.6 · 51 → 11

````text
//! The two guarded launch actions — audio routing and the ALVR dashboard —
//! the only mutations `run` undoes. Reference: scripts/demo/run.sh
//! (`launch-action: audio-route`, `launch-action: dashboard`); everything the
//! script does before them is permanent (parity decision 17).
//!
//! Each guard persists its record into [`SessionState`] before it mutates
//! (see [`crate::session::state`]). `release` undoes the mutation, sets the
//! flag and saves; `disarm` forgets without undoing (detach — `session-state.json`
//! still describes the device and dashboard for a later Sabrage); `Drop` is a
//! synchronous best-effort fallback for panics and early returns, inert once
//! released, disarmed, or under `--dry-run`.
````

### B2346 · l.69 · REWRITE (confirm) · rule 1.6 · 1 → 1

````text
/// Which of run.sh's audio branches applies (`launch-action: audio-route`).
````

### B2350 · l.81–87 · REWRITE (confirm) · rule 1.6 · 7 → 2

````text
/// run.sh's audio if/elif chain as a pure decision
/// (`launch-action: audio-route`).
````

### B2353 · l.112 · REWRITE (amend) · rule 1.6 · 1 → 1

````text
/// run.sh's `--no-audio` info line, verbatim.
````

### B2354 · l.116 · REWRITE (amend) · rule 1.6 · 1 → 1

````text
/// run.sh's BlackHole-not-present warn line, verbatim.
````

### B2356 · l.129–141 · REWRITE (amend) ·p · rule 1.2 · 13 → 6

````text
/// `launch-action: audio-route` — Reference: scripts/demo/run.sh.
///
/// Routes the Mac's default output to `BlackHole 2ch` for the session and
/// restores it on release. Inert for `--no-audio` (an `info` row), for
/// `protocol != "alvr"` or a missing `SwitchAudioSource` (silent), and for a
/// machine with no `BlackHole 2ch` output device (a `warn`).
````

### B2360 · l.157 · NEEDS-TEST (confirm) · rule 1.5 · 1 → 2

test first: `a_dry_runs_guard_restores_nothing_when_dropped` in `sabrage/crates/sabrage-core/src/stages/run/guards.rs` — Dropping an armed AudioGuard built over a dry-run context runs no child and emits no restore row.

````text
    /// A dry run never mutates, so its `Drop` must not either
    /// (tests::a_dry_runs_guard_restores_nothing_when_dropped).
````

### B2367 · l.261–273 · REWRITE (confirm) · rule 1.7 · 13 → 8

````text
    /// Take the guard, persisting `previous_output` into `state` **before**
    /// switching the device — but **without** switching it.
    ///
    /// The split from [`AudioGuard::apply_switch`] lets the caller install the
    /// armed guard in its held set before the one call that can come back
    /// `Cancelled` (A8-3), so a cancelled switch unwinds through the ordinary
    /// teardown — the only path that can set `guards.audio_restored` and save
    /// (tests::a_cancelled_switch_leaves_the_guard_armed_for_the_teardown).
````

### B2372 · l.328–334 · REWRITE (confirm) · rule 1.5 · 7 → 4

````text
        // A device carried forward from an earlier session's unfinished restore
        // (`stages::run::unfinished_audio_restore`) outranks the current
        // reading: in exactly that case the reading IS `BlackHole 2ch`, and
        // recording it would lose the real device for good. Sabrage-only.
````

### B2374 · l.346–353 · REWRITE (confirm) · rule 1.5 · 8 → 3

````text
        // Armed BEFORE the switch: `run_child` can report `Cancelled` for a child
        // that already applied the CoreAudio change, and returning that through `?`
        // here would drop the guard (A8-3; tests::a_cancelled_switch_leaves_the_guard_armed_for_the_teardown).
````

### B2376 · l.387–390 · REWRITE (confirm) ·p · rule 1.6 · 4 → 4

````text
            // BlackHole applies the macOS device volume to loopback samples, so
            // anything under 100% reaches the headset attenuated. Volume is
            // per-device (speakers untouched), and failure is swallowed as
            // run.sh's `|| true` does.
````

### B2377 · l.398–399 · REWRITE (confirm) · rule 1.6 · 2 → 2

````text
            // run.sh's failed-switch branch: warn, and clear the remembered
            // device again so the teardown restores nothing.
````

### B2379 · l.413–426 · REWRITE (amend) ·p · rule 1.2 · 14 → 11

````text
    /// Restore the device, set `guards.audio_restored`, and save.
    ///
    /// A guard that never switched writes nothing — no state file for a run
    /// that never touched audio.
    ///
    /// When the recorded device is gone, falls back to the built-in output
    /// ([`crate::session::fallback_output_device`]) and says so, or prints the
    /// remedy and leaves `guards.audio_restored` false so the record survives.
    /// Either outcome is rows only, never a failed stage
    /// (tests::a_recorded_device_that_vanished_falls_back_to_the_built_in_output,
    /// tests::an_unrestorable_device_prints_the_remedy_and_leaves_the_guard_pending).
````

### B2388 · l.546 · REWRITE (confirm) · rule 1.6 · 1 → 1

````text
/// Which of run.sh's dashboard branches applies (`launch-action: dashboard`).
````

### B2393 · l.559 · REWRITE (confirm) · rule 1.6 · 1 → 2

````text
/// run.sh's dashboard if/elif chain as a pure decision
/// (`launch-action: dashboard`).
````

### B2394 · l.576–580 · REWRITE (confirm) · rule 1.5 · 5 → 5

````text
/// `[ -x "$ALVR_DASHBOARD_BIN" ]`.
///
/// Duplicates `paths::is_executable`, which is private to its module; both
/// follow PARITY.md § Doctor / checks, "`is_executable` tests mode bits
/// `0o111`, not effective access".
````

### B2395 · l.588–606 · REWRITE (confirm) ·p · rule 1.2 · 19 → 11

````text
/// `launch-action: dashboard` — Reference: scripts/demo/run.sh.
///
/// Spawns the ALVR dashboard detached with pipes on `/dev/null`
/// ([`crate::executor::DetachedStdio::Null`]) and closes it on release. Inert
/// for `--no-dashboard` (an `info` row), for `protocol != "alvr"` (silent),
/// and for an unbuilt `alvr_dashboard` (a warn). Safe to launch before the
/// game: it polls `127.0.0.1:8082` until the embedded server appears.
///
/// The guard keeps the child's identity, not the child: the child is moved
/// into a task that `wait()`s on it, so a user-closed dashboard is reaped
/// instead of becoming a zombie (`spawn_detached` sets `kill_on_drop(false)`).
````

### B2398 · l.622 · REWRITE (confirm) · rule 1.6 · 1 → 1

````text
/// run.sh's `--no-dashboard` info line, verbatim.
````

### B2399 · l.624 · REWRITE (confirm) · rule 1.6 · 1 → 1

````text
/// run.sh's unbuilt-dashboard warn line, verbatim.
````

### B2402 · l.660 · REWRITE (confirm) · rule 1.6 · 1 → 1

````text
            // The shell's bare `:` — nothing is printed at all.
````

### B2406 · l.699–703 · REWRITE (confirm) · rule 1.5 · 5 → 5

````text
    /// Close the dashboard, set `guards.dashboard_closed`, and save.
    ///
    /// The kill is guarded by [`ProcInfo::is_same_process`] — pid **and** start
    /// time, where the shell only has `kill -0` — so a recycled pid is never
    /// signalled (tests::release_never_signals_an_identity_that_no_longer_matches).
````

### B2422 · l.1177–1178 · REWRITE (confirm) · rule 1.7 · 2 → 1

````text
    /// A device recorded at launch and disconnected before teardown.
````

### B2423 · l.1181 · REWRITE (confirm) · rule 1.7 · 1 → 1

````text
    /// One machine's `SwitchAudioSource -a -t output`, verbatim and in order.
````

### B2426 · l.1253–1255 · REWRITE (confirm) · rule 2.3 · 3 → 3

````text
        // What `arm` recorded before the switch: the record and the guard name
        // the same device, which is what makes the pending flag below mean
        // anything to `teardown`.
````

## `sabrage/crates/sabrage-core/src/stages/run/mod.rs`

Deleted (nothing carried): B2445, B2461, B2465, B2478, B2479, B2480, B2492, B2517, B2525, B2529, B2532, B2553, B2560, B2564, B2569, B2570

### B2432 · l.1–64 · REWRITE (amend) · rule 1.6 · 64 → 30

````text
//! `demo.sh run` — launch Beat Saber through the bridge.
//!
//! Reference: `scripts/demo/run.sh` (270 lines). Unlike the other four stages
//! this one is a state machine: preflight and prepare, then the guarded region
//! ([`guarded`]) that the shell's traps cover, then exactly one [`teardown`].
//!
//! Preflight and Prepare mutations are permanent and never unwound — the
//! `cxbottle.conf` backend fix, the helper restage, the adb forward
//! create/clear, the Goldberg swap — because run.sh installs its traps after
//! all of them (parity decision 17). Only the audio device and the dashboard
//! are guarded. A normal exit leaves the bottle's wineserver alive, as run.sh's
//! EXIT trap does; only the INT/TERM path calls `stop_wine`.
//!
//! [`run`] takes `Option<OperationGuard>` and drops it as soon as the wine
//! child is up (see [`crate::stages`]'s "Lock policy for `run`"); `None` means
//! the caller owns the lock. Every teardown runs against [`teardown_ctx`], a
//! fresh token and a fresh executor, because [`crate::executor::RealExecutor`]
//! refuses to mutate once its token has fired and the cancellation teardown
//! still has to run `wineserver -k`, restore audio and close the dashboard.
//!
//! run.sh keeps no session record at all, so everything built on
//! `session-state.json` is Sabrage-only: the live-session refusal is declared
//! in PARITY.md § Session (detach / reconcile), "A recorded **Live** session",
//! and detach in that same section's "Cmd-Q on a live session" row. The
//! teardown token and the early lock release are Sabrage-only concurrency
//! constructs with no ledger row of their own.
//!
//! See tests::{a_normal_exit_prints_the_blank_line_then_the_status,
//! the_cancelled_path_announces_itself_stops_wine_and_exits_130,
//! detaching_marks_the_state_leaves_the_guards_and_keeps_the_file}.
````

### B2433 · l.87–93 · REWRITE (confirm) · rule 1.6 · 7 → 5

````text
/// How long the cancellation teardown waits for the wine child after
/// `wineserver -k` before giving up on reaping it.
///
/// The game dies *with* its wineserver, so this is a generous bound on an
/// event that normally lands in well under a second.
````

### B2438 · l.121 · REWRITE (confirm) · rule 1.6 · 1 → 1

````text
    // Before anything else, and with lib.sh's die text.
````

### B2439 · l.124–129 · REWRITE (amend) ·p · rule 1.3 · 6 → 5

````text
    // Nothing else can report `Preflight`/`Launching`/`Stopping` this early —
    // there is no live handle yet and no state file — and the RAII guard makes
    // every early return end in a cleared slot rather than a phase that outlives
    // its launch.
    // tests::run_publishes_preflight_and_clears_it_when_the_preflight_fails
````

### B2440 · l.132–142 · REWRITE (confirm) ·p · rule 1.5 · 11 → 4

````text
    // The live-session gate (A8-1): asked before this launch’s own `Preflight`
    // (cannot self-block through `session_block_at`’s run-phase arm) and before
    // `reconcile`, which cannot see the `runtime_status.json` only `./demo.sh run` produces.
    // tests::a_shell_started_session_refuses_the_launch_before_anything_permanent
````

### B2441 · l.153–162 · REWRITE (confirm) · rule 1.5 · 10 → 4

````text
    // Sabrage-only, and deliberately *before* anything permanent — the
    // preflight's two auto-fixes included — because PARITY.md § Session
    // (detach / reconcile), "A recorded **Live** session" promises that a
    // launch refused for a live session changed nothing.
````

### B2442 · l.168–178 · REWRITE (confirm) · rule 1.5 · 11 → 6

````text
    // A record that is not ours to *touch* is not ours to launch over either
    // (A9-1 / A9-8): falling through reaches `wineserver_reset`, which takes
    // down the very session the classification exists to protect. `silent:
    // false` is the whole of "somebody else's" — the one silent shape is this
    // process's own in-flight launch.
    // tests::a_record_another_live_front_end_owns_refuses_the_launch
````

### B2446 · l.210–211 · REWRITE (confirm) · rule 1.3 · 2 → 2

````text
    // Everything from here to the guards is permanent: run.sh installs no
    // trap until after all of it.
````

### B2449 · l.239–243 · REWRITE (confirm) · rule 1.3 · 5 → 4

````text
    // Everything above was `Preflight` by [`SessionPhase::Preflight`]'s own
    // definition ("checks, wineserver reset, Goldberg"); the guards and the
    // spawn are `Launching`. Once the wine child is up, `guarded` clears the
    // slot and the live handle carries the phase from there.
````

### B2451 · l.281–293 · REWRITE (confirm) ·p · rule 1.7 · 13 → 7

````text
/// The output device a reconciled record still has pending, if any.
///
/// [`session::reconcile::finish_record`](crate::session::reconcile) keeps
/// `session-state.json` when a guard could not be released, and carrying the
/// name forward lets the retry that record was kept for actually happen
/// (A9-2; tests::a_teardown_with_an_unrestorable_guard_keeps_the_record).
/// A record whose restore succeeded has `audio_restored` set and carries nothing.
````

### B2462 · l.378–399 · REWRITE (amend) ·p · rule 1.2 · 22 → 16

````text
/// Publishes [`session::RunPhaseInfo`] for the three phases only this stage
/// can know about, and guarantees the slot is emptied however [`run`] leaves.
///
/// Without it [`crate::session::watcher::SessionMonitor::snapshot`] has nothing
/// to report between the stage starting and the wine child being published, so
/// a launch reads as “No session” for its entire preflight. Every publication
/// names its `run_id` and bottle — without them the Session screen offers a
/// Stop that takes the operation lock then dies on “bottle name required” —
/// and `Drop` clears the slot only while it belongs to this run, since `run`
/// releases the lock at the launch boundary and a detached or cancelled run
/// can still be unwinding while the next launch publishes its own `Preflight`.
/// [`finalize_exited`](Self::finalize_exited) is the one publication meant to
/// outlive `run`, so the Session screen can say “Exited (code N)”.
///
/// See tests::{the_scope_publishes_identity_and_drop_clears_only_its_own_run,
/// a_normal_teardown_reports_stopping_then_a_surviving_exited_code}.
````

### B2476 · l.516–520 · REWRITE (confirm) · rule 1.6 · 5 → 6

````text
/// The undoable half of run.sh's launch: from its trap installation to
/// `wait $WINE_PID`.
///
/// Everything this function does is undoable, and everything above it is not.
/// It never tears anything down itself — [`teardown`] owns every exit path, so
/// there is exactly one place the guards come off.
````

### B2477 · l.531–538 · REWRITE (confirm) · rule 1.5 · 8 → 5

````text
    // In two halves on purpose (A8-3): the guard is installed HERE, before
    // `apply_switch` runs the child that can come back `Cancelled` with
    // CoreAudio already changed — so the switch unwinds through `teardown`,
    // which sets `guards.audio_restored` and saves, rather than through `Drop`,
    // which restores the device but can record neither.
````

### B2482 · l.568–573 · REWRITE (confirm) · rule 1.5 · 6 → 5

````text
    // The game becomes reachable HERE: `spawn_detached` sets
    // `kill_on_drop(false)`, so from the spawn until this line a running Beat
    // Saber is reachable by *nothing* — no live handle, no record on disk.
    // Publishing the handle is infallible and in-process, so it goes first and
    // everything fallible follows it.
````

### B2487 · l.604–607 · REWRITE (confirm) · rule 1.3 · 4 → 3

````text
    // A session lasts hours; holding the operation lock through it would block
    // `stop` exactly when the user reaches for it. See `crate::stages`'s "Lock
    // policy for `run`". `None` means the caller owns it and it is not ours.
````

### B2488 · l.610 · REWRITE (confirm) · rule 1.6 · 1 → 1

````text
    // run.sh's `wait $WINE_PID`.
````

### B2489 · l.612–617 · REWRITE (amend) · rule 1.4 · 6 → 5

````text
    // `biased` on purpose: an unbiased `select!` picks at random among ready
    // branches, and a Stop losing that coin toss to a Detach would disarm the
    // guards and leave the game running while `stop_session` reports success.
    // Stop is terminal here — cancel is checked first and the detach arm
    // re-checks the token below — so a Stop that fired at ANY point wins.
````

### B2491 · l.643–656 · REWRITE (confirm) · rule 1.2 · 14 → 9

````text
/// Write `session-state.json` for a launch that is **already running and
/// already published** — deliberately best effort.
///
/// The file exists for a *later* process: a reconcile after a crash, a second
/// Sabrage, `./demo.sh stop`. Propagating a write failure would unwind out of
/// [`guarded`] with Beat Saber running and no way to stop it from Sabrage, so
/// this warns and carries on supervising.
///
/// See tests::a_failed_state_write_warns_instead_of_orphaning_the_running_game.
````

### B2496 · l.692–700 · REWRITE (confirm) · rule 1.3 · 9 → 6

````text
    // Teardown is a phase of its own and has to be visible *before*
    // `clear_live_session` runs, which is why published `Stopping` outranks a
    // live handle in `snapshot()`'s precedence table. Not `Detached` (that
    // phase is derived from the state file the arm leaves behind) and not
    // `DryRun`, where nothing ran.
    // tests::a_normal_teardown_reports_stopping_then_a_surviving_exited_code
````

### B2497 · l.710–720 · REWRITE (confirm) · rule 1.5 · 11 → 7

````text
            // Leak the guards on purpose: the dashboard stays open, the audio
            // device stays on BlackHole, and `session-state.json` keeps
            // describing both so a later reconcile can finish the job. The
            // record goes down FIRST — a write that fails keeps the guards
            // armed, because disarming before the write left the device on
            // BlackHole with nothing on disk naming it.
            // tests::a_detach_that_cannot_write_its_record_keeps_the_guards_armed
````

### B2498 · l.730–734 · REWRITE (confirm) · rule 1.3 · 5 → 4

````text
            // Detach is the one way supervision ends without a teardown, so
            // the announcement belongs to `step::RUN_SUPERVISE`, the step that
            // was running, not to a guard release that never happens here.
            // tests::the_detach_row_belongs_to_the_supervise_step
````

### B2500 · l.748–752 · REWRITE (confirm) · rule 1.6 · 5 → 5

````text
            // The two closing prints come FIRST: run.sh's EXIT trap does not
            // fire until `exit $rc`, below both, so `audio: restored output ->
            // …` and `dashboard: closed` land *after* the status line, never
            // before it.
            // tests::a_normal_exits_guards_come_off_after_the_status_line_not_before_it
````

### B2501 · l.758–762 · REWRITE (confirm) · rule 1.3 · 5 → 3

````text
            // This publication is the one that OUTLIVES `run` (see
            // [`RunPhaseScope::finalize_exited`]), so the Session screen can
            // show "Exited (code N)" until the next launch publishes over it.
````

### B2502 · l.769–780 · REWRITE (confirm) · rule 1.5 · 12 → 9

````text
            // That EXIT trap: no `stop_wine`. The bottle's wineserver stays up
            // on a clean quit — `./demo.sh stop` is what kills it.
            //
            // #202: both calls are best effort. A `?` would skip
            // `clear_live_session`, leaking the handle so the next
            // `stop_session` burns its 30 s timeout on an already-fired token,
            // and would turn a clean quit into exit 1 — but wine has already
            // exited with `rc`, and that is the number to report.
            // tests::a_normal_exit_survives_a_failed_state_save
````

### B2503 · l.799 · REWRITE (confirm) · rule 1.6 · 1 → 1

````text
            // run.sh's INT trap, verbatim and in order.
````

### B2504 · l.807–813 · REWRITE (confirm) · rule 1.5 · 7 → 6

````text
            // Best effort, exactly as the `Normal` arm above is (#202): the
            // shell's INT trap runs every one of its commands and only then
            // re-signals itself, so a failed `session-state.json` write must
            // not skip the reap, the record or the live handle — leaking the
            // handle costs the next `stop_session` its whole 30 s timeout.
            // tests::a_cancelled_teardown_survives_a_failed_state_save
````

### B2510 · l.878–893 · REWRITE (amend) · rule 1.6 · 16 → 7

````text
/// `lib.sh`'s `stop_wine`, on the 4 s advisory budget
/// ([`crate::stages::STOP_WINESERVER_WAIT`] — deliberately *not* the 5 s fatal
/// one this stage's own reset uses).
///
/// Reference: `scripts/demo/lib.sh`. Duplicates `stages::stop`'s function of
/// the same name: both are module-private, and the two differ in step id and
/// in return type (that one propagates, this one is best effort).
````

### B2511 · l.913–928 · REWRITE (confirm) · rule 1.6 · 16 → 9

````text
/// run.sh's `stop_helper`, a safety net only — the runtime spawns and owns the
/// helper, which dies with the game — so the reaped line is printed exactly
/// when something was found.
///
/// Matches on the **resolved executable path**
/// ([`crate::process::find_processes_by_exe`]), never `pkill -f`'s argv
/// substring (PARITY.md § Stop, "Each reap (leftover encoder helper"), and
/// kills through the executor as `/bin/kill -TERM <pid>`, once per matched
/// process.
````

### B2513 · l.956–962 · REWRITE (amend) ·p · rule 1.5 · 7 → 7

````text
    // …and only when it is still OUR record (A9-3): a teardown landing after
    // the next launch wrote its own would delete a live session’s only
    // description. `state::clear` compares the *owner*, which on this machine
    // is this process, so run identity is the only discriminant. An unreadable
    // record is still removed; it describes nothing anyone can act on, and
    // leaving it blocks the next reconcile.
    // tests::a_late_teardown_never_clears_a_newer_runs_record
````

### B2514 · l.975–982 · REWRITE (confirm) · rule 1.2 · 8 → 7

````text
/// Is a guard *this teardown was responsible for* still pending?
///
/// Deliberately **not** [`SessionState::has_pending_guards`]: that one also
/// counts `wired_forwards`, which `run` records as it creates them and never
/// removes, so it would keep a stale record after every `--wired` launch.
///
/// See tests::a_wired_run_whose_guards_came_off_still_clears_the_record.
````

### B2516 · l.993–1001 · REWRITE (confirm) · rule 1.5 · 9 → 8

````text
/// Clear `session-state.json` — or keep it, when a guard is still pending.
///
/// The teardown counterpart of `session::reconcile::finish_record`, and the
/// same guarantee PARITY.md § Session (detach / reconcile), "A **Dead** or
/// **IdentityMismatch** recorded session" states for `AudioGuard::release`:
/// the record is only cleared once every recorded guard is released.
///
/// See tests::a_teardown_with_an_unrestorable_guard_keeps_the_record.
````

### B2518 · l.1013–1016 · REWRITE (confirm) · rule 1.6 · 4 → 4

````text
/// The line run.sh's `stop_helper` prints when it reaps a leftover helper.
///
/// `pub` (A1-3) so `sabrage-parity` can pin it against `run.sh` by calling the
/// real constant rather than copying a substring.
````

### B2519 · l.1019–1021 · REWRITE (confirm) · rule 1.6 · 3 → 3

````text
/// run.sh's INT trap prints this first, before `stop_wine` runs.
///
/// `pub` (A1-3), same reason as [`HELPER_REAPED_LINE`].
````

### B2520 · l.1024–1026 · REWRITE (confirm) · rule 1.6 · 3 → 3

````text
/// run.sh's `print -r -- "wine exited with status $rc (log: $LOG)"`.
///
/// `pub` (A1-3), same reason as [`HELPER_REAPED_LINE`].
````

### B2522 · l.1042–1044 · REWRITE (confirm) · rule 1.5 · 3 → 4

````text
/// The refusal when [`session::reconcile`] finds a session that is still
/// running. Sabrage-only (PARITY.md § Session (detach / reconcile), "A
/// recorded **Live** session"): run.sh has no session record and would simply
/// reset wineserver under the running game.
````

### B2526 · l.1149 · REWRITE (confirm) · rule 2.3 · 1 → 1

````text
        // wine's own status is propagated unchanged.
````

### B2534 · l.1266–1272 · REWRITE (amend) ·p · rule 1.6 · 7 → 3

````text
        // run.sh's EXIT trap fires only after the status line, so the restore
        // row is the LAST line of a clean quit, never the first.
        // Reference: scripts/demo/run.sh; trap order is on `Guards::release`.
````

### B2537 · l.1430–1435 · REWRITE (confirm) · rule 1.7 · 6 → 5

````text
    /// #202: a clean quit whose `session-state.json` save fails stays a clean
    /// quit. If `held.release(…)?` propagated, `clear_live_session` would never
    /// run: the handle leaks for the app's lifetime, the next `stop_session`
    /// spends its whole 30 s timeout on an already-fired token, and `run`
    /// returns exit 1 for a wine process that exited 0.
````

### B2539 · l.1505–1508 · REWRITE (amend) · rule 1.5 · 4 → 5

````text
    /// A8-1: the record is only cleared once every guard it recorded is
    /// released (PARITY.md § Session (detach / reconcile), "A **Dead** or
    /// **IdentityMismatch** recorded session"). A device that could not be
    /// switched back leaves `audio_restored` false — deleting the record then
    /// leaves the Mac on `BlackHole 2ch` with nothing on disk to restore from.
````

### B2545 · l.1737–1740 · REWRITE (confirm) · rule 1.5 · 4 → 5

````text
    /// A7-1: a launch refused for a live session must change nothing
    /// (PARITY.md § Session (detach / reconcile), "A recorded **Live**
    /// session"). Reconciliation therefore runs BEFORE `preflight::run`,
    /// whose two auto-fixes (the `cxbottle.conf` backend line, the helper
    /// restage) are permanent and never unwound.
````

### B2547 · l.1789–1792 · REWRITE (amend) · rule 1.7 · 4 → 4

````text
    /// A8-1: `reconcile` reads `session-state.json`, and a `./demo.sh run`
    /// session writes no such file — the only trace it leaves on this machine
    /// is a fresh `runtime_status.json`. A launch that ignored that trace would
    /// walk into `wineserver_reset` and take the running game down.
````

### B2549 · l.1848–1851 · REWRITE (amend) · rule 1.7 · 4 → 4

````text
    /// A9-1 / A9-8: `reconcile` classifies a record it may not touch as
    /// `Busy` and leaves the file alone. A launch that read that as "nothing
    /// to carry" would keep going — through the bottle-scoped `wineserver -k`
    /// that kills the very session the classification protects.
````

### B2561 · l.2146–2149 · REWRITE (confirm) · rule 1.1 · 4 → 2

````text
        // Both guards are inert here, so this pins consumption and idempotence
        // only — not `Guards::release`'s trap order.
````

## `sabrage/crates/sabrage-core/src/stages/run/preflight.rs`

Deleted (nothing carried): B2585, B2598, B2633, B2634, B2637, B2638, B2639, B2640, B2643, B2652, B2654, B2658, B2663, B2664, B2665, B2682, B2683, B2685, B2687, B2690, B2695, B2697, B2698, B2702, B2703, B2705, B2719, B2720

### B2582 · l.1–58 · REWRITE (amend) ·p · rule 1.6 · 58 → 23

````text
//! Contract-ordered launch gates. Reference: scripts/demo/run.sh — its
//! `# preflight:` / `# preflight-autofix:` tags name the slugs, and the
//! contract's `native_gate` column decides `block` / `warn` / `autofix` /
//! `none` per slug, so a per-side divergence is recorded in
//! `contract/pipeline.toml` rather than discovered by reading two
//! implementations.
//!
//! Each evaluated slug emits exactly one [`crate::events::StageEvent::Check`]
//! carrying its final outcome; for an `autofix` slug that is the re-check's,
//! preceded by the [`crate::events::StageEvent::AutoFixed`] describing what
//! changed.
//!
//! The walk follows contract order, which is doctor's, not run.sh's, and in
//! which the bottle context resolves before anything consumes it. Both sides
//! evaluate the same set and abort on the same conditions, so only which die
//! wins can differ. A check the shell would not have evaluated either is a
//! `Skipped` row that never blocks; an applicable check that reached no
//! verdict is Fatal, never a pass — launching on an unverified gate is how a
//! black window happens. Pinned by
//! tests::{the_slug_list_is_unique_gating_only_and_includes_the_run_only_slugs,
//! an_unverifiable_applicable_check_is_fatal_not_a_pass}; the order and gate
//! divergences are declared in PARITY.md § Run preflight (encoded in the
//! contract's per-side gates).
````

### B2583 · l.72–74 · REWRITE (confirm) · rule 1.6 · 3 → 3

````text
/// The two `preflight-autofix`-gated helper slugs, in contract order. Both map
/// to `fix.restage-helper` and both are skipped under
/// `encoder_process = "inproc"`.
````

### B2584 · l.77–82 · REWRITE (amend) · rule 1.5 · 6 → 8

````text
/// The launch-preflight slugs this side evaluates, in contract order.
///
/// Exactly `contract().native_preflight()` — every check whose `native_gate`
/// is gating
/// (tests::the_slug_list_is_unique_gating_only_and_includes_the_run_only_slugs).
/// Derived, never hand-written: the parity crate joins run.sh's
/// `# preflight:` tags against this list, and a hand-maintained second list is
/// how the two drift.
````

### B2586 · l.93–95 · REWRITE (confirm) · rule 1.6 · 3 → 3

````text
/// `oxrsys-runtime.toml` read once, before the checks that branch on it — the
/// same two facts run.sh captures with `awk`, resolved the way the runtime
/// resolves them ([`read_toml_facts`]).
````

### B2589 · l.104–105 · REWRITE (confirm) · rule 1.6 · 2 → 2

````text
    /// `${ENCODER_PROC:-auto}` — already defaulted, exactly like the shell's
    /// own parameter expansion.
````

### B2590 · l.109–120 · REWRITE (confirm) · rule 1.2 · 12 → 9

````text
/// One key's **raw** last assignment via
/// [`crate::config::runtime_toml::effective_string`], with no accepted-set
/// filtering; an unassigned key is the empty string, which the callers below
/// already treat as "unset".
///
/// This is the right reader for a key whose accepted set Sabrage does not
/// model, and the fallback [`read_toml_facts`] uses when no occurrence at all
/// is one the runtime would accept: that is the value the die text has to
/// quote back to the user.
````

### B2591 · l.125–156 · REWRITE (confirm) ·p · rule 1.7 · 32 → 21

````text
/// The two config facts a launch branches on, in one read of the file,
/// resolved the way the **runtime** resolves them rather than the way `awk`
/// does.
///
/// `protocol` and `encoder_process` go through
/// [`crate::config::runtime_toml::read_lines_like_the_runtime`]: the value the
/// launched runtime uses is the last assignment it would **accept**, across
/// `[table]` boundaries, so `protocol = "alvr"` followed by
/// `protocol = "banana"` still runs ALVR. When nothing is acceptable each key
/// falls back to its raw last assignment, so run.sh's die still quotes the
/// value back; an absent `encoder_process` reads as `auto`. The Settings
/// screen reads the same file through the same function, so the two cannot
/// name different backends.
///
/// One declared DIVERGENCE from run.sh, in this side's favour: an **unquoted**
/// value (`protocol = alvr`) is accepted here and reads as empty through
/// `awk -F'"'`. PARITY.md § Declared by the 2026-08-30 adversarial review
/// (round 1 fixes), "Config readers: doctor emulates `awk`, launch uses the
/// runtime's semantics."; pinned by
/// tests::{the_shadowed_invalid_last_fixture_launches_on_its_valid_values,
/// a_trailing_invalid_assignment_leaves_the_previous_valid_one_in_force}.
````

### B2593 · l.186–187 · REWRITE (confirm) · rule 1.6 · 2 → 2

````text
/// run.sh's `case "$ENCODER_PROC"`: does this configuration need the staged
/// arm64 helper, and does the shell print a line about it first?
````

### B2600 · l.219–220 · REWRITE (confirm) · rule 1.6 · 2 → 2

````text
        // run.sh evaluates the whole `--wired` block only inside
        // `if [ -n "${WINEVR_WIRED:-}" ]`.
````

### B2601 · l.222 · REWRITE (confirm) · rule 1.6 · 1 → 1

````text
        // `inproc` never reaches run.sh's `ensure_helper_staged`.
````

### B2603 · l.235–238 · REWRITE (amend) · rule 1.6 · 4 → 5

````text
    // Mirrors run.sh's own `require_bottle`, just above its tagged preflight
    // block: this call is what enforces `bottle.named` + `bottle.exists`.
    // Their registry rows are still emitted below (they cannot fail once this
    // has passed), because a GUI preflight list with two silently-absent rows
    // reads like a bug.
````

### B2604 · l.245–252 · REWRITE (confirm) · rule 1.3 · 8 → 5

````text
    // The preflight checks the bottle **this stage resolved**, not one the
    // check layer re-derives from `$HOME`: a second, independent resolution is
    // a way for the two to disagree. It is also what lets a test point the
    // whole preflight at a fixture bottle instead of the machine's real
    // `~/Library/Application Support/CrossOver/Bottles`.
````

### B2608 · l.323–335 · REWRITE (confirm) · rule 1.7 · 13 → 11

````text
/// Evaluate one check without pinning the launch to a blocking probe.
///
/// Evaluators are synchronous `fn(&CheckCtx)` (the doctor's shape), so a slow
/// one runs *inside* this future and the walk's cancellation checkpoint —
/// which sits between evaluators — cannot interrupt it. The stage holds
/// [`crate::stages::OPERATION_LOCK`] throughout, so the child-probe slugs run
/// on a blocking thread raced against the launch's token and Stop stays
/// responsive; the probe's own deadline bounds the orphaned thread. Everything
/// else is evaluated inline — a `stat` is not worth a thread hop, and doctor
/// keeps calling the same evaluators directly
/// (tests::a_cancel_during_the_wired_adb_probe_stops_the_walk_promptly).
````

### B2610 · l.360–362 · REWRITE (confirm) · rule 1.6 · 3 → 3

````text
/// run.sh's `inproc` `info` line, verbatim.
/// `pub` (A1-3) so `sabrage-parity` can pin it against `run.sh` by calling the
/// real renderer instead of copying the sentence.
````

### B2612 · l.374–375 · REWRITE (amend) · rule 1.6 · 2 → 2

````text
/// run.sh's `warn` — the one `warn`-gated row's text, which is not
/// doctor's. `pub` (A1-3), same reason as [`INPROC_NOTICE`].
````

### B2613 · l.380 · REWRITE (confirm) · rule 1.6 · 1 → 1

````text
/// run.sh's `inproc` / unrecognized-encoder lines, verbatim.
````

### B2617 · l.423–429 · REWRITE (confirm) · rule 1.4 · 7 → 6

````text
    // run.sh's goldberg gate is `sha256_ok "$GBE_DLL" … || [ -f "$GBE_DLL" ] ||
    // die`: the launch dies only when the dll is *gone*; a hash that differs
    // from the pinned build is tolerated (a user-supplied Goldberg build is a
    // legitimate setup). Doctor is stricter on purpose, so its verdict is
    // reported as-is and simply not gated when the file exists
    // (tests::goldberg_hash_mismatch_does_not_block_the_launch).
````

### B2619 · l.446–450 · REWRITE (confirm) · rule 1.4 · 5 → 3

````text
    // Both protocol rows are decided from the single `$PROTOCOL` read rather
    // than from the evaluator's own re-read, so they can never disagree with
    // each other or with `PreflightFacts`.
````

### B2620 · l.462–466 · REWRITE (confirm) · rule 1.6 · 5 → 5

````text
            // `game.version` is the only `warn`-gated row, and its text is
            // *not* doctor's; every other Warn reaching a `block` gate is one
            // the shell's coarser test would have passed (`host.manifest`
            // pointing somewhere unexpected but present), so it is reported
            // and does not block.
````

### B2622 · l.491–497 · REWRITE (confirm) · rule 1.1 · 7 → 6

````text
/// "Applicable but unverifiable" — the row is emitted, and then the launch
/// stops (design-core §10's S11).
///
/// The reason the check gave is carried into the die text, with its remedy
/// appended when it has one, so the user reads why it could not be checked
/// rather than a bare slug.
````

### B2623 · l.510 · REWRITE (confirm) · rule 1.6 · 1 → 1

````text
/// run.sh's protocol gate, decided from the single `$PROTOCOL` capture.
````

### B2624 · l.520–529 · REWRITE (confirm) · rule 1.5 · 10 → 8

````text
    // Every branch below emits this same row and then decides: the row reports
    // the *doctor* evaluator's verdict (`awk`, last raw assignment), the gate
    // reports this side's runtime-semantics fact. When the two disagree and
    // the launch proceeds anyway, the row would otherwise read as a red check
    // the launch silently ignored, so it says why instead. PARITY.md
    // § Declared by the 2026-08-30 adversarial review (round 1 fixes),
    // "Config readers: doctor emulates `awk`, launch uses the runtime's
    // semantics."
````

### B2628 · l.563–567 · REWRITE (confirm) ·p · rule 1.5 · 5 → 6

````text
        // DECLARED DIVERGENCE (contract: shell_gate = warn, native_gate =
        // block) — PARITY.md § Run preflight (encoded in the contract's
        // per-side gates), "Launch refuses `protocol=oxrsys` outright". The
        // first line is run.sh's warn text verbatim; the second says what this
        // side does instead
        // (tests::an_oxrsys_protocol_blocks_the_launch_with_both_lines).
````

### B2629 · l.579–581 · REWRITE (confirm) · rule 1.6 · 3 → 3

````text
        // Anything else: run.sh's two-line die, attributed to the supported-set
        // row. The legacy row is `tap … skipped` in the shell and never reached
        // here (the die above aborts first).
````

### B2631 · l.601–609 · REWRITE (confirm) ·p · rule 1.5 · 9 → 10

````text
/// The `block`-gate die text for one slug — run.sh's `die` string verbatim,
/// with its interpolations.
///
/// Three have no shell counterpart at all (`overlay.dxmt-winemetal`,
/// `overlay.woxr-dll`, `overlay.woxr-so`) and reuse the sentence shape of the
/// `d3d11` die they extend: PARITY.md § Run preflight (encoded in the
/// contract's per-side gates), "Native preflight blocks on ALL four overlay
/// files".
/// `pub` (A1-3) so `sabrage-parity` can pin these against `run.sh` by calling
/// the real renderer instead of copying a substring per slug.
````

### B2635 · l.654–655 · REWRITE (confirm) · rule 1.6 · 2 → 1

````text
        // `dxmt-winemetal` is the same overlay, so it reuses the same sentence.
````

### B2641 · l.688–690 · REWRITE (confirm) · rule 1.6 · 3 → 3

````text
        // `checks::run_only` already carries each of these die strings whole,
        // because those slugs have no doctor row whose prose could compete with
        // run.sh's.
````

### B2644 · l.703–708 · REWRITE (confirm) · rule 1.6 · 6 → 5

````text
/// The `autofix` gate: apply the mapped fix, re-evaluate, and only then decide.
///
/// run.sh's two auto-fixing preflights are the `cxbottle.conf` backend rewrite
/// and `ensure_helper_staged`. Both are **permanent** mutations, never
/// unwound — see [`super`]'s "permanent vs guarded".
````

### B2647 · l.762–777 · REWRITE (confirm) · rule 1.7 · 16 → 12

````text
/// The autofix itself failed — the fix's own error, turned back into the one
/// `Check` + one `Fatal` this module promises, so an event-only consumer never
/// sees a failed stage with an unresolved row.
///
/// * `Cancelled` — Stop, not a failure. Propagated untouched: no row, no die.
/// * `Fatal` — the fix already emitted its own (`helper::restage_helper`'s
///   "neither the staged copy nor the build output is arm64"). Its text is
///   run.sh's; only the missing `Check` is added.
/// * anything else — the io cause is surfaced as a stderr-shaped `Output` line
///   and the die is run.sh's post-fix text, the same shape
///   `actions::die_with_cause` uses
///   (tests::a_backend_autofix_that_cannot_write_still_emits_its_check_and_dies_run_shs_way).
````

### B2651 · l.855–857 · REWRITE (confirm) · rule 1.6 · 3 → 2

````text
/// run.sh's die text for "the auto-fix ran and the condition is still there".
/// `pub` (A1-3), same reason as [`block_die`].
````

### B2660 · l.978–982 · REWRITE (confirm) · rule 1.7 · 5 → 3

````text
        // A3b-1/A7-1: a *valid* assignment shadowed by a later INVALID one.
        // Config.cpp assigns only inside its whitelist, so the runtime keeps
        // alvr/inproc rather than the raw last value.
````

### B2667 · l.1065–1075 · REWRITE (amend) · rule 1.2 · 11 → 7

````text
    /// Copy the checkout's `contract/` and its generated shell mirror into a
    /// scratch root, so the walk's **first** slug — `meta.contract-sync`,
    /// `block`-gated on this side — passes there and every test below reaches
    /// the row it is actually about instead of dying on row zero.
    ///
    /// The live files, not synthesised ones: the evaluator also compares the
    /// contract compiled into THIS binary, which came from the same checkout.
````

### B2688 · l.1360–1361 · REWRITE (confirm) ·p · rule 1.7 · 2 → 1

````text
        // Asserted separately: this gate forces the fixture to seed a whole scratch checkout.
````

### B2691 · l.1389–1395 · REWRITE (confirm) · rule 1.7 · 7 → 7

````text
    /// `meta.contract-sync` is `native_gate = "block"`: a checkout whose
    /// `contract.gen.sh` header does not match its `contract/` refuses to launch
    /// on the contract's very first slug, before the preflight has probed — or
    /// auto-fixed — anything else.
    ///
    /// The slug has no arm in [`block_die`], so the die text is the evaluator's
    /// own message and remedy, through the fallback arm.
````

### B2709 · l.1786–1790 · REWRITE (confirm) · rule 2.3 · 5 → 5

````text
    /// The repository's own "every key assigned twice, the second one junk"
    /// fixture, driven through the whole preflight: the launch and the Settings
    /// view must read the same `protocol` and `encoder_process`, and neither of
    /// them the junk. (`oxrsys-runtime.shadowed-invalid-last.toml` is the file
    /// `config::runtime_toml`'s tests pin the reader against.)
````

### B2711 · l.1822–1826 · REWRITE (confirm) · rule 1.7 · 5 → 5

````text
    /// A7-1/A3b-1: a valid assignment shadowed by a later INVALID one — the
    /// mirror of `the_shadowed_invalid_last_fixture_launches_on_its_valid_values`.
    /// The runtime keeps the *valid* value, so the launch must too; the raw-last
    /// reading died on `protocol='banana'` and demanded the arm64 helper for a
    /// runtime that encodes in-process.
````

### B2713 · l.1855–1856 · REWRITE (confirm) · rule 1.5 · 2 → 3

````text
        // The doctor evaluator still reports its own `awk` verdict (PARITY.md §
        // Declared by the 2026-08-30 adversarial review (round 1 fixes),
        // "Config readers: doctor emulates `awk`") — the row says why launch went on.
````

### B2718 · l.1918–1920 · REWRITE (confirm) · rule 1.4 · 3 → 2

````text
        // Dies on the missing helper: the later `native` is the value the runtime
        // uses, so both helper rows stay applicable.
````

## `sabrage/crates/sabrage-core/src/stages/setup.rs`

Deleted (nothing carried): B2727, B2732, B2734, B2738, B2739, B2741, B2744, B2750, B2752, B2755, B2757, B2758, B2762, B2765, B2773

### B2726 · l.1–67 · REWRITE (amend) ·p · rule 1.6 · 67 → 21

````text
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
````

### B2735 · l.136–137 · REWRITE (confirm) · rule 1.6 · 2 → 2

````text
    // blob:none keeps the wine-mirror clone to tens of MB instead of full
    // history.
````

### B2736 · l.147 · REWRITE (confirm) · rule 1.6 · 1 → 1

````text
    // openvr is an alvr_session build dependency.
````

### B2743 · l.257–261 · REWRITE (confirm) · rule 1.5 · 5 → 3

````text
        // The `ok` row claims the files *and* the marker; reaching this line
        // under a dry run means the marker is missing or stale and nothing
        // wrote it — tests::a_dry_run_over_an_already_set_up_checkout_still_reports_the_ok_rows.
````

### B2745 · l.289–290 · REWRITE (confirm) · rule 1.5 · 2 → 3

````text
        // `O_EXCL`, not `exists()`-then-rename: whoever created the file
        // between the probe above and this line keeps it —
        // tests::a_config_created_by_another_writer_is_reported_not_replaced.
````

### B2746 · l.295–298 · REWRITE (confirm) · rule 1.3 · 4 → 2

````text
            // Someone else won the race; setup may never replace a config it
            // did not write, so report it exactly as the branch above does.
````

### B2748 · l.321–327 · REWRITE (amend) · rule 1.6 · 7 → 6

````text
/// Emits the row for a config this run did not write: `info` when its
/// `protocol` is already `alvr`, otherwise the `warn` that reproduces
/// setup.sh's "not overwriting" text verbatim —
/// tests::config_warns_verbatim_when_protocol_is_not_alvr.
///
/// Reference: `scripts/demo/setup.sh`.
````

### B2749 · l.344–349 · REWRITE (amend) · rule 1.2 · 6 → 7

````text
/// The `protocol` value from an `oxrsys-runtime.toml`: the first matching
/// line wins, and an absent or unquoted value yields the empty string —
/// tests::parse_protocol_awk_matches_the_shell_recipe.
///
/// Reference: `scripts/demo/setup.sh`. [`crate::checks::config`]'s
/// `parse_protocol` mirrors doctor.sh's last-match semantics instead, so the
/// two are not one helper.
````

### B2751 · l.376–380 · REWRITE (confirm) · rule 1.6 · 5 → 3

````text
    // A given bottle name must still die the require_bottle way on a missing
    // bottle; with only --bs-dir, ctx.bs_dir already holds it —
    // tests::game_check_dies_the_require_bottle_way_for_a_missing_bottle.
````

### B2753 · l.399–409 · REWRITE (confirm) · rule 1.2 · 11 → 7

````text
/// Runs `spec` through the stage's executor.
///
/// # Errors
///
/// [`SabrageError::ChildFailed`] with an empty tail on a non-zero exit: every
/// line the child printed already reached the event stream as it ran. A dry
/// run never spawns and always reports success, so this returns `Ok` there.
````

### B2759 · l.658–664 · REWRITE (confirm) ·p · rule 1.2 · 7 → 4

````text
    /// A [`crate::executor::Executor`] that loses the `O_EXCL` create race
    /// A5-3 names: `create_new` writes the other writer's bytes itself and
    /// then returns `Ok(false)`, the caller's bytes unwritten. Every other
    /// method forwards to the inner executor.
````

### B2761 · l.785–787 · REWRITE (confirm) · rule 1.7 · 3 → 2

````text
        // Nothing on disk when the stage probes: the other writer's file
        // appears only inside the create call.
````

### B2770 · l.987–990 · REWRITE (confirm) · rule 1.5 · 4 → 3

````text
        // A5-1: the extraction row claims the marker too, and the marker is
        // exactly what this fixture lacks — so the row must stay future-tense
        // and the file must still be absent afterwards.
````

## `sabrage/crates/sabrage-core/src/stages/stop.rs`

Deleted (nothing carried): B2802, B2806, B2814, B2820, B2829, B2834, B2842, B2845, B2847, B2855, B2865, B2877

### B2786 · l.1–144 · REWRITE (amend) ·p · rule 1.6 · 144 → 27

````text
//! `demo.sh stop` — cleanly stop the game and the bottle's wine processes.
//!
//! Reference: `scripts/demo/stop.sh`. One step id
//! ([`step::STOP_WINESERVER`]) covers both the kill and the survivor probe;
//! [`step::STOP_PORTS`], [`step::STOP_REAP`] and [`step::STOP_AUDIO`] are one
//! step each. Between the reaps and the audio row,
//! [`crate::session::reconcile::finish_stopped_session`] restores the previous
//! session's guards; it reports its own failures, so the only error it hands
//! back here is [`SabrageError::Cancelled`].
//!
//! Mutations go through [`crate::stages::StageCtx::child`] +
//! [`crate::executor::Executor::run_child`], so `--dry-run` records them
//! instead of touching a live process. Read-only probes (`lsof`,
//! `SwitchAudioSource`) go through [`crate::process::capture_with`] with
//! `ctx.cancel` and a deadline: a wedged probe must not hold this stage — and
//! with it the process-wide operation lock — indefinitely. Cancellation is
//! checked between every step, so a cancelled `stop` returns
//! [`SabrageError::Cancelled`] rather than `StageFinished { ok: true }`.
//! See tests::{a_pre_cancelled_run_yields_cancelled_and_never_reports_stage_finished_ok,
//! cancellation_during_the_reporting_steps_still_fails_the_stage,
//! a_wedged_lsof_warns_instead_of_reporting_free_ports}.
//!
//! Declared divergences: PARITY.md § Stop, "Each reap (leftover encoder helper,
//! leftover ALVR dashboard)"; PARITY.md § Declared by the 2026-08-30
//! adversarial review (round 1 fixes), "**Stop reports probe failures.**";
//! PARITY.md § Session (detach / reconcile), "A **Dead** or
//! **IdentityMismatch** recorded session".
````

### B2787 · l.156–161 · REWRITE (confirm) · rule 1.7 · 6 → 5

````text
/// How long to wait for `wineserver -w` to return before giving up on it —
/// 4 s, never fatal.
///
/// Defined in [`crate::stages`] next to its deliberately distinct sibling
/// [`crate::stages::RUN_WINESERVER_WAIT`] (5 s, fatal); never unify the two.
````

### B2788 · l.164–167 · REWRITE (confirm) · rule 1.5 · 4 → 5

````text
/// The substring `pgrep -f 'Beat Saber.exe'` matches on argv — matched the
/// same way here by [`crate::process::find_processes_by_cmdline`]
/// (PARITY.md § Stop, "The Beat Saber survivor probe scans live processes'").
/// Also [`format_survivors`]'s fallback text when a survivor's exe path has no
/// file name.
````

### B2792 · l.182–186 · REWRITE (confirm) · rule 1.1 · 5 → 3

````text
/// How long [`reap`] waits for a signalled process to actually exit before
/// reporting it as a survivor, and how often it re-checks. Deliberately short:
/// this is a report, not a hard guarantee, and `stop` must stay snappy.
````

### B2793 · l.190–193 · REWRITE (confirm) · rule 1.2 · 4 → 3

````text
/// A reap's three "there was one" rows: the confirmed kill, the process that
/// outlived SIGTERM (plus the surviving `pid name` pairs), and the `--dry-run`
/// row that claims only a plan.
````

### B2798 · l.234–236 · REWRITE (amend) · rule 1.5 · 3 → 3

````text
    // The helper's not-found row is emitted by `report_foreign_helpers` rather
    // than by `reap`: "no leftover encoder helper" may only be said after
    // looking beyond *this* checkout's staged path.
````

### B2799 · l.245–251 · REWRITE (confirm) · rule 1.7 · 7 → 3

````text
    // Unconditional and report-only: a helper staged under another checkout is
    // invisible to the exact-path reap above, so gating this scan on a local
    // match reopens A5-2 (tests::a_foreign_helper_is_reported_whatever_the_local_reap_did).
````

### B2801 · l.280–291 · REWRITE (confirm) · rule 1.7 · 12 → 7

````text
/// `Err(`[`SabrageError::Cancelled`]`)` the moment `ctx.cancel` has fired.
///
/// Called between every step of [`run`], not only around the mutating
/// children: the reporting steps and the early returns in [`stop_wine`] and
/// [`reap`] reach no check of their own, so these calls are what make a
/// cancelled stop fail instead of reporting `StageFinished { ok: true }`.
/// See tests::cancellation_during_the_reporting_steps_still_fails_the_stage.
````

### B2803 · l.301–320 · REWRITE (amend) ·p · rule 1.6 · 20 → 16

````text
/// `wineserver -k`, then a bounded `wineserver -w`, per `lib.sh`'s `stop_wine`.
///
/// Emits nothing when `ctx.paths.wineserver` is `None`: lib.sh swallows the
/// command-not-found failure, so neither side prints anything.
///
/// # Errors
///
/// [`SabrageError::Cancelled`] if `ctx.cancel` fires during either child. A
/// plain non-zero or failed child is swallowed, matching the shell's `|| true`.
/// See tests::{dry_run_stop_wine_plans_the_wineserver_pair_only_when_crossover_is_present,
/// stop_wine_propagates_a_pre_cancelled_token_instead_of_swallowing_it}.
///
/// The `-w` bound is a [`tokio::time::timeout`]: dropping the timed-out future
/// ends the child via [`crate::process::spawn_streamed`]'s `kill_on_drop(true)`.
/// Nothing tests it (a timing property); breaking it leaks a `wineserver -w`
/// per stop.
````

### B2805 · l.361–368 · REWRITE (confirm) · rule 1.6 · 8 → 3

````text
/// `warn` naming the survivors when any Beat Saber process is still up, else
/// `ok "game and wineserver down"` — stop.sh's `pgrep -f 'Beat Saber.exe'`
/// branch. See tests::report_survivors_matches_a_direct_probe.
````

### B2807 · l.383–392 · REWRITE (confirm) · rule 1.1 · 10 → 8

````text
/// Did a probe hit its deadline, as opposed to failing the way a missing
/// binary fails? [`process::capture_with`] reports the deadline as an
/// [`SabrageError::Io`] of kind [`std::io::ErrorKind::TimedOut`].
///
/// Only a deadline may print a Sabrage-only row: every other failure is what
/// stop.sh's `2>/dev/null` folds into an empty `$STALE`/`$CUR`, and must keep
/// producing the shell's row.
/// See tests::a_missing_lsof_still_reports_the_shells_free_ports_row.
````

### B2808 · l.397–417 · REWRITE (confirm) ·p · rule 1.6 · 21 → 17

````text
/// `COMMAND(PID)` per listener on the streaming ports, deduplicated, sorted,
/// space-joined with a trailing space — the shape stop.sh's
/// `lsof ... | awk ... | sort -u | tr` pipeline produces. Duplicates
/// `checks::network`'s private equivalent, not reachable from here.
///
/// `Ok(None)` means the probe blew `deadline`. `lsof` and `deadline` are
/// parameters so a test can point at a stub that never answers; production
/// passes `lsof` and [`process::DEFAULT_PROBE_TIMEOUT`].
///
/// # Errors
///
/// [`SabrageError::Cancelled`] only. The probe runs through
/// [`process::capture_with`], which SIGKILLs its process group on the token
/// or the deadline: a wedged `lsof` must not hold this stage — and with it
/// the process-wide operation lock — past the reaps and the guard restore.
/// See tests::{stale_listeners_is_well_formed_cmd_pid_pairs_with_a_trailing_space,
/// a_wedged_lsof_warns_instead_of_reporting_free_ports}.
````

### B2811 · l.460–465 · REWRITE (amend) · rule 1.6 · 6 → 4

````text
/// `warn "streaming ports still held by: <stale>"` when [`stale_listeners`]
/// found any, else `ok "streaming ports free"` — stop.sh's `$STALE` branch. A
/// probe that blew its deadline gets [`ports_unreadable_warn`] instead; a
/// cancelled one emits nothing.
````

### B2815 · l.488–533 · REWRITE (amend) · rule 1.6 · 46 → 26

````text
/// One `/bin/kill -TERM <pid>` child per already-scanned match, routed through
/// the executor — `lib.sh`'s `reap_stray` specialised to exact-exe-path
/// matching (PARITY.md § Stop, "Each reap (leftover encoder helper, leftover
/// ALVR dashboard)"). The message fires at most once regardless of match
/// count, matching the shell's single `ok` (PARITY.md § Stop, "Each reap sends
/// `/bin/kill -TERM <pid>` once per matched process"); `procs` is the caller's
/// scan, this function never walks the process table itself.
///
/// Returns whether anything was signalled (under `--dry-run`: whether anything
/// matched), so a caller can own the not-found case — the helper's
/// cross-checkout scan does.
///
/// A pid whose `(pid, start_time)` identity no longer matches is not
/// signalled, and the `killed` row is emitted only once every signalled
/// identity is really gone ([`wait_for_exit`], bounded by [`REAP_EXIT_WAIT`]);
/// a process that outlives SIGTERM gets a `warn` naming it instead. Under
/// `--dry-run` nothing is signalled and the row swaps to [`ReapMsg::would`].
/// See tests::{dry_run_reap_plans_a_kill_per_match_and_reports_once,
/// a_real_reap_reports_the_kill_only_once_the_process_is_really_gone,
/// a_term_ignoring_process_gets_a_warn_row_not_a_green_killed_row,
/// reap_never_signals_a_pid_whose_identity_no_longer_matches}.
///
/// # Errors
///
/// [`SabrageError::Cancelled`] the moment `ctx.cancel` fires after any one
/// kill, skipping the remaining kills and the closing message.
````

### B2819 · l.607–632 · REWRITE (confirm) · rule 1.6 · 26 → 18

````text
/// Sabrage-only and deliberately report-only: a helper staged under another
/// checkout runs from a different absolute path, so neither [`reap`]'s
/// exact-path match nor the shell's `pkill -f "$OXR_HELPER_BIN"` can see it.
/// Nothing is signalled — a mutating kill may not rely on an argv match
/// (PARITY.md § Stop, "Each reap (leftover encoder helper, leftover ALVR
/// dashboard)") and another checkout's helper is that checkout's `stop` to run.
///
/// `matches` is the caller's [`HELPER_BASENAME`] cmdline scan; this function
/// narrows it to processes whose resolved executable is *named*
/// [`HELPER_BASENAME`] and lies outside `root`, so an editor or a `tail -f`
/// that merely mentions the path cannot match.
///
/// `local_matched` gates nothing but the [`NO_LEFTOVER_HELPER`] row, which may
/// print only when neither scan found anything — never the scan itself, since
/// a local and a foreign helper coexist routinely in a multi-worktree checkout
/// (A5-2).
/// See tests::{a_foreign_helper_is_reported_whatever_the_local_reap_did,
/// the_not_found_row_prints_only_when_nothing_foreign_and_no_local_match}.
````

### B2824 · l.692–704 · REWRITE (amend) · rule 1.6 · 13 → 6

````text
/// stop.sh's audio branch: `warn` plus the restore hint when the Mac's output
/// is still `BlackHole 2ch`, else `ok "audio output: <cur>"`. A probe that blew
/// its deadline gets [`audio_unreadable_warn`] instead; a cancelled one emits
/// nothing. Entirely silent — no step, no rows — when `SwitchAudioSource` is
/// not on `PATH`.
/// See tests::audio_branch_text_is_verbatim.
````

### B2828 · l.758–760 · REWRITE (confirm) · rule 1.7 · 3 → 3

````text
/// Sabrage-only, [`ports_unreadable_warn`]'s twin: a probe that never answered
/// must not print `ok "audio output: "` naming no device at all.
/// See tests::a_wedged_switchaudiosource_warns_instead_of_naming_an_empty_device.
````

### B2837 · l.867–869 · REWRITE (confirm) · rule 1.7 · 3 → 2

````text
    /// A cancelled token stops the `lsof` probe instead of holding `stop` — and
    /// the process-wide operation lock — until the wedged probe answers.
````

### B2838 · l.885–886 · REWRITE (confirm) · rule 1.2 · 2 → 1

````text
    /// A cancelled ports probe emits no row at all, green or otherwise.
````

### B2839 · l.902 · REWRITE (amend) · rule 1.4 · 1 → 2

````text
    /// r2:A5-5 regression: a probe that blows its deadline warns instead of
    /// claiming free ports.
````

### B2844 · l.1015 · REWRITE (amend) · rule 1.4 · 1 → 1

````text
    /// Finding #8: survivors are matched by argv, not by exe path.
````

### B2848 · l.1083–1086 · REWRITE (confirm) · rule 1.5 · 4 → 3

````text
        // Load-bearing: `Paths::new` derives `session-state.json` from the real
        // `$HOME`, so without this override a stop test would read — and with a
        // real executor delete — the developer's own live session record.
````

### B2866 · l.1383–1385 · REWRITE (confirm) · rule 1.7 · 3 → 2

````text
    /// r1:A5-7 regression: a helper left over in another checkout is reported
    /// whether or not this checkout's own reap matched.
````

### B2872 · l.1476 · REWRITE (amend) · rule 1.4 · 1 → 2

````text
    /// Finding #2: cancellation propagates out of the stop helpers instead of
    /// being swallowed.
````

### B2875 · l.1522–1527 · REWRITE (confirm) · rule 1.7 · 6 → 4

````text
    /// Both wineserver shapes, because they exercise different code: with a
    /// `wineserver` path [`stop_wine`] spawns and hits its own post-`run_child`
    /// check; with `None` it returns `Ok(())` immediately and only [`run`]'s
    /// between-step [`checkpoint`] can catch the cancellation.
````

### B2879 · l.1578–1589 · REWRITE (confirm) ·p · rule 1.7 · 12 → 8

````text
    /// Finding #6, at the stage level: the reconcile pass between steps 3 and 4
    /// is *additive*, so a failure inside it is reported rather than aborting the
    /// stage before the audio row — `stop.sh` has no step that can end the script.
    ///
    /// Deterministic and machine-independent: the record carries a `--wired`
    /// forward and `adb` points at a nonexistent path, so `forward --remove`
    /// fails at `spawn` with `ENOENT`. Nothing is spawned, signalled, or written
    /// on the machine.
````

### B2881 · l.1665–1667 · REWRITE (confirm) · rule 1.7 · 3 → 2

````text
            // One row normally; two when this Mac's output happens to be sitting
            // on BlackHole 2ch (warn + restore hint). Either proves step 4 ran.
````

### B2882 · l.1675–1682 · REWRITE (confirm) ·p · rule 1.7 · 8 → 5

````text
    /// The narrower window finding #3 named: cancellation after the wineserver
    /// kill, in the *reporting* half where no executor child is spawned and
    /// nothing else observes the token. Deterministic: cancel **from the event
    /// sink** the instant the first row (`report_survivors`'s) is emitted —
    /// [`run`]'s next call is its own [`checkpoint`].
````

## `sabrage/crates/sabrage-core/src/store/goldberg.rs`

Deleted (nothing carried): B2907, B2909

### B2885 · l.1–16 · REWRITE (amend) · rule 1.6 · 16 → 11

````text
//! The revert-original-`steam_api64.dll` action.
//!
//! Sabrage-only, so a user can launch through real Steam once without hunting
//! down `.orig-steam` by hand: run.sh installs Goldberg and never restores the
//! backup (PARITY.md § Planned for later phases (declared now),
//! "Revert-original-`steam_api64.dll` action").
//!
//! The revert swaps the dll back and leaves `.orig-steam`, `steam_appid.txt`
//! and `steam_settings/` in place, because the next launch through
//! [`crate::stages::run::actions::goldberg_stage`] reinstalls Goldberg
//! unconditionally.
````

### B2886 · l.30–33 · REWRITE (confirm) · rule 1.2 · 4 → 3

````text
/// The argv substring that means "Beat Saber is running", kept byte-identical
/// to [`crate::stages::stop`]'s private const of the same string so the two
/// doors agree on what a live game looks like.
````

### B2888 · l.40–45 · REWRITE (amend) · rule 1.2 · 6 → 4

````text
    /// `true` iff a `.orig-steam` backup existed, was not itself a Goldberg
    /// dll (see [`revert_original_steam_dll`] for what that is tested
    /// against), and was copied back over the live dll; `false` means nothing
    /// was reverted and `message` says which case applied.
````

### B2891 · l.53–58 · REWRITE (confirm) · rule 1.4 · 6 → 5

````text
/// `"$API.orig-steam"` — the backup path
/// [`crate::stages::run::actions::goldberg_stage`] writes.
///
/// `pub(crate)` so [`super::library::validate`] can compute the same path for
/// its `origSteamPresent` probe.
````

### B2892 · l.65–106 · REWRITE (amend) ·p · rule 1.6 · 42 → 29

````text
/// Restore the Steam `steam_api64.dll` from its `.orig-steam` backup.
///
/// Refuses while a session is live ([`session::live_session_reason`], with the
/// operation lock held across the check and the copy), while any Beat Saber
/// process is on argv, and when the backup is itself a Goldberg dll — tested
/// against the contract pin, this checkout's `Paths::gbe_dll` payload, and the
/// provenance a launch records
/// ([`crate::stages::run::actions::goldberg_backup_is_goldberg`]), because the
/// launch tolerates an unpinned payload (PARITY.md § Invariants that must NOT
/// change (byte/behavior parity), "Goldberg hash-tolerance at run").
///
/// The argv scan is not scoped to `bs_dir`: wine puts a `Z:\` Windows path on
/// the command line, which no unix prefix test can match, so any running Beat
/// Saber refuses this dll swap (A13a-2).
///
/// Nothing here proves a backup is the real Steam dll, only that it is not a
/// Goldberg one this pipeline knows; the success message says "the .orig-steam
/// backup", never "the original". See
/// tests::{refuses_when_the_backup_is_itself_the_pinned_goldberg_dll,
/// refuses_when_the_backup_is_an_unpinned_goldberg_build,
/// refuses_when_a_launch_recorded_the_backup_as_goldberg,
/// refuses_while_a_matching_game_process_is_running,
/// the_success_message_never_claims_the_original_was_restored}.
///
/// # Errors
///
/// Fatal when a session is live or a matching game process is running, and
/// whatever the copy returns; the no-backup and backup-is-Goldberg cases are
/// `Ok` with `restored: false`.
````

### B2893 · l.111–117 · REWRITE (amend) · rule 1.3 · 7 → 7

````text
    // Built here rather than taken as a parameter so the one call site (the
    // Tauri command layer) stays two-argument: the persisted
    // `settings.repo_root` through `resolve_repo_root`, degrading to the empty
    // root exactly as `SettingsPathsCache::snapshot` does when either step
    // fails. Degrading is harmless for the liveness predicate
    // (`sabrage_appsup`/`oxr_appsup` are `$HOME`-derived either way) but not
    // free: the is-Goldberg check reads `paths.gbe_dll`.
````

### B2894 · l.127–129 · REWRITE (confirm) · rule 1.2 · 3 → 3

````text
/// [`revert_original_steam_dll`] with the Goldberg pin passed in — the
/// testability seam, because no test can fabricate bytes hashing to the real
/// contract pin.
````

### B2895 · l.146–150 · REWRITE (confirm) · rule 1.2 · 5 → 3

````text
/// [`revert_with_pin`] with the running-game argv needle passed in, so a test
/// can hand in a needle that matches nothing or one that matches its own
/// command line (tests::refuses_while_a_matching_game_process_is_running).
````

### B2896 · l.158–161 · REWRITE (confirm) · rule 1.5 · 4 → 4

````text
    // Held through the liveness re-check and the copy: a run holds this lock
    // from before its Goldberg step until after it publishes the live session,
    // so it closes the check-then-copy window `live_session_reason` alone
    // leaves open (tests::waits_for_the_operation_lock_then_proceeds).
````

### B2897 · l.164–169 · REWRITE (amend) · rule 1.7 · 6 → 5

````text
    // The machine-wide predicate, not a local copy: a `./demo.sh run` session
    // writes neither an in-process handle nor `session-state.json`, and only
    // `live_session_reason` sees it, through its fresh `runtime_status.json`
    // (tests::refuses_while_only_the_runtime_reports_a_live_session). Its
    // rule that an unverifiable record counts as live applies here too.
````

### B2898 · l.177–182 · REWRITE (confirm) · rule 1.3 · 6 → 6

````text
    // The window `live_session_reason` structurally cannot see: a `./demo.sh
    // run` publishes no handle, no run phase and no `session-state.json`, and
    // its `runtime_status.json` appears only once the runtime streams, long
    // after the Goldberg install. A running game with this dll mapped is the
    // one thing observable throughout, so it is probed directly
    // (tests::refuses_while_a_matching_game_process_is_running).
````

### B2899 · l.211–218 · REWRITE (confirm) ·p · rule 1.3 · 8 → 6

````text
    // Any kind of Goldberg: the contract pin, the payload this checkout installs,
    // or a launch's recorded provenance — only the third recognises a backup from
    // a Goldberg build `gbe_dll` does not match (PARITY.md § Invariants that must
    // NOT change (byte/behavior parity), "Goldberg hash-tolerance at run"), the
    // exact backup a bytes-only test would restore
    // (tests::refuses_when_a_launch_recorded_the_backup_as_goldberg).
````

### B2910 · l.405–406 · REWRITE (confirm) · rule 1.6 · 2 → 2

````text
        // The install arrived already Goldberg'd: the first launch snapshotted
        // Goldberg's own bytes into `.orig-steam`.
````

### B2918 · l.637–642 · REWRITE (confirm) · rule 1.7 · 6 → 4

````text
        // Both halves of `watcher::runtime_status_live`: a fresh stamp *and* a
        // `process_id` that is still alive (this test process stands in for the
        // runtime). A status naming no live process is a file neither this door
        // nor the Session screen's `External` phase will vouch for.
````

### B2920 · l.716 · REWRITE (amend) · rule 1.3 · 1 → 1

````text
        // Give the spawned revert a chance to reach the lock and block there.
````

### B2921 · l.735–747 · REWRITE (amend) · rule 1.4 · 13 → 9

````text
    // Every test above holds `session::lock_session_globals()`: a
    // `RunPhaseScope` alive on another harness thread otherwise makes these
    // reverts fail with "a launch for bottle 'Steam' is in progress". The
    // guard deliberately does not reset `LIVE_SESSION` (other modules set it
    // without holding the guard), so no `LiveSessionHandle` is faked here;
    // tests::refuses_while_a_persisted_session_records_a_live_wine_child
    // exercises the same refusal branch of `live_session_reason`, and
    // tests::waits_for_the_operation_lock_then_proceeds the window that check
    // alone cannot close.
````

## `sabrage/crates/sabrage-core/src/store/library.rs`

Deleted (nothing carried): B2923, B2935, B2941, B2945, B2960, B2963, B2969, B2972, B2973, B2978, B2981

### B2925 · l.50–52 · REWRITE (confirm) · rule 1.7 · 3 → 2

````text
/// The most recent launch of one game, written by the Tauri launch command
/// after a `run` stage returns (via [`Library::record_last_session`]).
````

### B2932 · l.126–140 · REWRITE (amend) ·p · rule 1.5 · 15 → 8

````text
/// [`Library::upsert`] for the **Edit-game form**: every editable field comes
/// from `incoming`, while `last_session`, `added_at_unix_ms` and `appid` are
/// kept from the stored entry. An unknown id is a plain insert (Add-game
/// wizard's first save).
///
/// The form submits a whole [`GameEntry`] cloned minutes earlier, so a plain
/// `upsert` would delete a session recorded while it was open
/// (tests::an_edit_racing_a_recorded_session_keeps_both).
````

### B2934 · l.164–172 · REWRITE (confirm) · rule 1.5 · 9 → 7

````text
/// `settings ⊕ this game's overrides`, or `None` when the library has no
/// entry with `game_id`.
///
/// The one entry point for "what flags does *this* game launch with": the
/// Tauri launch command resolves the merge here rather than letting the
/// front-end keep a second copy of the precedence rule
/// (tests::launch_options_for_resolves_the_merge_by_id_and_is_none_for_a_stranger).
````

### B2937 · l.185–199 · REWRITE (amend) · rule 1.2 · 15 → 13

````text
/// Load `library.json`.
///
/// An absent file is `Ok(Library::default())` (version 1, no games): first run.
///
/// # Errors
///
/// A present but unparseable file — an `Err`, never a silent reset
/// (tests::a_corrupt_file_is_an_error_never_a_silent_reset) — and a `version`
/// newer than [`LIBRARY_VERSION`], refused **before** a caller can mutate and
/// re-save it, because [`Library`] is a closed serde struct and the re-save
/// would drop the newer build's fields while keeping its `version`. Any schema
/// addition here must bump [`LIBRARY_VERSION`]
/// (tests::a_newer_schema_version_is_refused_not_silently_rewritten).
````

### B2938 · l.227–240 · REWRITE (amend) · rule 1.5 · 14 → 12

````text
/// Serializes every [`transact`] against every other one in this process.
///
/// [`save`] is atomic per *write*, but a library edit is a
/// load → mutate → save *transaction*: two interleaving means the second
/// one's `save` writes a snapshot taken before the first one's
/// (tests::interleaved_transactions_do_not_resurrect_a_removed_game).
///
/// A lock of its own rather than [`crate::stages::OPERATION_LOCK`]: the
/// library is written from inside a run (the post-launch
/// `record_last_session`) as well as from the Library screen, so borrowing
/// the operation lock would deadlock the run that already holds it, or block
/// every library edit for the length of a session.
````

### B2939 · l.243–253 · REWRITE (confirm) · rule 1.2 · 11 → 10

````text
/// Run one complete `library.json` read-modify-write transaction under
/// [`LIBRARY_LOCK`], returning whatever `f` returns.
///
/// `f` sees the freshly loaded library and mutates it in place; the file is
/// rewritten only if `f` actually changed something, so a no-op removal
/// never mints a `library.json` that did not exist
/// (tests::transact_writes_only_when_the_library_actually_changed).
///
/// **Every** writer must go through this: a bare [`load`]/[`save`] pair
/// around the same file re-opens exactly the window this closes.
````

### B2946 · l.350–361 · REWRITE (confirm) ·p · rule 1.7 · 12 → 12

````text
/// Where the installed `steam_api64.dll` stands relative to the Goldberg
/// dll and its `.orig-steam` backup.
///
/// "Is Goldberg" means **either** the contract-pinned build
/// (`gbe_dll_sha256`) **or** the payload this checkout would install
/// (`Paths::gbe_dll`) byte for byte: `run` installs whatever is at that
/// path and only warns on a pin mismatch
/// (PARITY.md § Invariants that must NOT change (byte/behavior parity),
/// "Goldberg hash-tolerance at run"), and a pin bump orphans a dll
/// installed before it. A pin-only test calls both `Original`, and the
/// revert door offers to "restore" Goldberg's own bytes
/// (tests::goldberg_state_covers_all_five_variants).
````

### B2952 · l.389–395 · REWRITE (amend) · rule 1.5 · 7 → 8

````text
/// Where a [`GameEntry`] stands, for the Library screen's status tag.
///
/// Invariant: **`Ready` means every hard gate the launch itself enforces is
/// already satisfied** — exe, 1.29.4, bottle, `z:` when outside `drive_c`,
/// and `steam_api64.dll`. Anything `run.sh`/[`crate::stages::run`] would
/// `die` on must show as something other than `Ready`, or the badge
/// contradicts the button next to it
/// (tests::healthy_game_without_steam_dll_is_not_ready).
````

### B2955 · l.439–449 · REWRITE (confirm) · rule 1.2 · 11 → 9

````text
/// Read-only probes over one `(bs_dir, bottle)` pair. Never touches the
/// machine beyond `stat`/`read` — same contract as every `checks::*`
/// evaluator.
///
/// `paths` is accepted whole (like `CheckCtx`'s) because callers already
/// have one; its `gbe_dll` is the Goldberg payload *this checkout* installs,
/// read by the classification alongside the contract pin.
///
/// Thin wrapper around [`validate_with_bottle`].
````

### B2956 · l.455–467 · REWRITE (confirm) · rule 1.2 · 13 → 9

````text
/// [`validate`]'s actual logic, taking an already-resolved [`Bottle`] rather
/// than building one from `bottle_name` itself.
///
/// The split exists **entirely for testability**: [`Bottle::unvalidated`]
/// resolves against the real `$HOME`-derived bottles root
/// (`paths::bottles_root` is not `Paths`-derived), so only a caller-supplied
/// `Bottle` — whose `prefix` may point anywhere — lets a test make
/// `bottle_exists` true from a fixture. [`validate`] is the only caller that
/// derives one from `$HOME`.
````

### B2957 · l.483–489 · REWRITE (confirm) · rule 1.2 · 7 → 6

````text
/// [`validate_with_bottle`] with the Goldberg pin passed in.
///
/// Second testability seam: no test can fabricate bytes hashing to the real
/// contract pin, so a test exercising the pin-matched branches hands in the
/// digest of its own fixture. [`validate_with_bottle`] is the only caller
/// that reads the contract pin.
````

### B2958 · l.516–520 · REWRITE (amend) · rule 1.6 · 5 → 3

````text
    // The same prefix test `checks::bottle::bs_dir_outside_drive_c` makes,
    // computed unconditionally: this module has no "bottle resolved" gate,
    // and an unresolved bottle's prefix is still a meaningful string.
````

### B2959 · l.527–531 · REWRITE (confirm) · rule 1.3 · 5 → 4

````text
    // The dll's own bytes are the only positive evidence available here:
    // deriving "original" from a missing backup labels an already-Goldberg'd
    // install untouched, then backs it up and "restores" it — see
    // `super::goldberg`'s refusal.
````

### B2961 · l.578–580 · REWRITE (confirm) · rule 1.6 · 3 → 3

````text
        // run.sh's `# launch-action: goldberg-stage` dies here in these same
        // words (`stages::run::actions::goldberg_stage`); a game that cannot
        // launch is never `Ready`.
````

### B2962 · l.587–589 · REWRITE (confirm) · rule 1.1 · 3 → 2

````text
    // Most-specific first; `Ready` is the invariant documented on
    // `GameStatus`.
````

### B2965 · l.776–779 · REWRITE (confirm) · rule 1.7 · 4 → 4

````text
        // The shape this pins: the Library screen removes a game while the
        // post-launch task records that same game's last session. Without
        // `transact`, each loads its own snapshot and saves the whole file
        // back, and whichever renames last wins outright.
````

### B2979 · l.1194–1195 · REWRITE (confirm) · rule 1.6 · 2 → 3

````text
        // Ready requires the dll run.sh's `# launch-action: goldberg-stage`
        // would otherwise die on — see
        // `healthy_game_without_steam_dll_is_not_ready`.
````

### B2980 · l.1218–1221 · REWRITE (amend) · rule 1.7 · 4 → 3

````text
        // Template and backend are detail-row facts, not launch gates: a
        // wrong value surfaces as a problem but never moves `status` off
        // Ready.
````

### B2994 · l.1323 · REWRITE (confirm) · rule 1.6 · 1 → 2

````text
        // Everything run.sh checks except the dll its
        // `# launch-action: goldberg-stage` block dies on.
````

## `sabrage/crates/sabrage-core/src/store/mod.rs`

### B2995 · l.1–28 · REWRITE (confirm) ·p · rule 1.7 · 28 → 16

````text
//! Sabrage's own persistent store under `~/Library/Application Support/Sabrage/`:
//! GUI-only state, never a parity artifact and never read by the shell pipeline
//! (CLAUDE.md, "Sabrage ⇄ demo.sh parity").
//!
//! A missing file loads as the type's default, a corrupt or newer-schema file is a
//! hard [`crate::error::SabrageError`] rather than a silent reset, and every write
//! goes through the [`crate::executor::Executor`] so `--dry-run` plans instead of
//! mutating — the same convention as [`crate::session::state`]. Pinned by
//! settings::tests::a_corrupt_file_is_an_error_never_a_silent_reset,
//! library::tests::a_newer_schema_version_is_refused_not_silently_rewritten, and
//! settings::tests::a_dry_run_executor_plans_the_write_instead_of_performing_it.
//!
//! [`goldberg::revert_original_steam_dll`] refuses rather than claim to have restored
//! an "original" it cannot authenticate — Sabrage's one deliberate divergence from
//! `run.sh`'s Goldberg step (PARITY.md § Planned for later phases (declared now),
//! "Revert-original-`steam_api64.dll` action").
````

## `sabrage/crates/sabrage-core/src/store/settings.rs`

### B2996 · l.1–30 · REWRITE (confirm) · rule 1.2 · 30 → 13

````text
//! `settings.json` — Sabrage's own global preferences, at
//! `~/Library/Application Support/Sabrage/settings.json`
//! ([`crate::paths::sabrage_support_dir`], [`settings_path`]). GUI-only state
//! with no demo.sh counterpart (design-core §4.2); written through the
//! [`Executor`] like every other mutation, read with plain `std::fs`.
//!
//! Unknown keys — top-level and inside `launch` — are preserved verbatim so an
//! older build's autosave cannot delete what a newer one wrote, and [`load`]
//! refuses a file whose `version` is newer than [`SETTINGS_VERSION`] rather
//! than reading it half-way. See
//! tests::{unknown_fields_survive_a_load_save_round_trip,
//! unknown_nested_launch_keys_survive_a_load_save_round_trip,
//! a_newer_version_is_refused_and_its_bytes_left_alone}.
````

### B2998 · l.53–63 · REWRITE (amend) ·p · rule 1.7 · 11 → 6

````text
    /// Keys of this object a newer Sabrage wrote and this one has no field
    /// for, kept exactly as read and written straight back out. The outer
    /// flattened map on [`Settings`] collects top-level keys only, so without
    /// this a `launch.someFlag` is dropped during deserialization and deleted
    /// by the next autosave — the UI hands the whole object back on save
    /// (tests::unknown_nested_launch_keys_survive_a_load_save_round_trip).
````

### B2999 · l.68–72 · REWRITE (confirm) · rule 1.5 · 5 → 5

````text
/// Schema version this Sabrage writes into `settings.json`. Bump it only for
/// a change the [`Settings::extra`]/[`LaunchDefaults::extra`] verbatim round
/// trip cannot absorb, and always for such a change: [`load`] refusing a newer
/// file is the only guard against an older build's autosave
/// (tests::a_newer_version_is_refused_and_its_bytes_left_alone).
````

### B3001 · l.85–87 · REWRITE (confirm) · rule 1.7 · 3 → 2

````text
    /// [`SETTINGS_VERSION`] at write time. A file without the key reads as the
    /// current version: its shape *is* the current shape.
````

### B3006 · l.99–104 · REWRITE (confirm) · rule 1.3 · 6 → 4

````text
    /// Whether doctor/preflight probes may shell out to `adb` (which starts its
    /// daemon as a side effect). Defaults **true**, matching
    /// [`crate::checks::CheckOptions::new`]'s doctor-parity default, so an
    /// absent or freshly-created settings file behaves exactly like doctor.
````

### B3007 · l.106–111 · REWRITE (confirm) · rule 1.7 · 6 → 5

````text
    /// One-time acknowledgement of the runtime-config write-once override
    /// (design-core §4.1 rule 2): the Settings screen shows a confirmation
    /// panel the first time it writes `oxrsys-runtime.toml`, and this flag
    /// suppresses it afterward. Defaults **false**, so a file without the key
    /// still shows the panel once.
````

### B3008 · l.113–123 · REWRITE (confirm) · rule 1.7 · 11 → 10

````text
    /// Every top-level key this binary does not have a field for, kept exactly
    /// as read and written straight back out.
    ///
    /// The UI autosaves a complete `Settings` object on every control change,
    /// so a key not represented here would be deleted the first time an older
    /// build touched one toggle. Flattened, so the keys sit at the top level
    /// where they were found; skipped when empty, so an ordinary file's bytes
    /// are unchanged (tests::an_ordinary_settings_file_carries_no_extra_keys).
    ///
    /// [`Settings`] is `PartialEq` but not `Eq`, because [`Value`] is not.
````

### B3011 · l.165–183 · REWRITE (amend) ·p · rule 1.2 · 19 → 11

````text
/// Load `settings.json`.
///
/// * absent → `Ok(Settings::default())`, the ordinary first-run case;
/// * present but unparseable → `Err`, never a silent reset
///   (tests::a_corrupt_file_is_an_error_never_a_silent_reset);
/// * `version` newer than [`SETTINGS_VERSION`] → `Err` with the bytes left
///   untouched, the same refusal [`super::library::load`] makes: the two
///   `extra` maps preserve unknown *keys*, so a version bump is reserved for a
///   change they cannot express, and reading such a file would let the next
///   autosave persist the loss
///   (tests::a_newer_version_is_refused_and_its_bytes_left_alone).
````

### B3015 · l.302–304 · REWRITE (confirm) · rule 1.7 · 3 → 3

````text
    /// A13a-3 / A13b-4 regression: a key nested inside `launch` survives a
    /// load/save round trip on a downgrade (the version half of the pair is
    /// pinned by tests::a_newer_version_is_refused_and_its_bytes_left_alone).
````

## `sabrage/crates/sabrage-core/src/tap.rs`

### B3017 · l.1–17 · REWRITE (confirm) ·p · rule 1.6 · 17 → 10

````text
//! The parity tap channel: `"<slug> <status>"`, one line per check.
//!
//! The channel carries slug and status only (see scripts/demo/lib.sh, `tap()`),
//! never prose, which is why check message text can stay implementation-owned;
//! the tier-2 live differ (`scripts/dev/parity.sh`) compares it between a zsh
//! doctor run and a native one.
//!
//! Status vocabulary is fixed by the zsh side: `ok`, `warn`, `fail`, `info`,
//! `skipped`. Nothing else may appear on this channel
//! (tests::words_match_the_zsh_vocabulary).
````

### B3018 · l.24–31 · REWRITE (confirm) · rule 1.7 · 8 → 5

````text
/// The tap word for a status.
///
/// [`CheckStatus::NotImplemented`] maps to `skipped` so an unbound slug reads as
/// a row that did not run and the differ reports a real mismatch against a zsh
/// `ok`/`fail` instead of silently agreeing (tests::words_match_the_zsh_vocabulary).
````

## `sabrage/crates/sabrage-core/src/util/mod.rs`

Deleted (nothing carried): B3031, B3036, B3042, B3046

### B3027 · l.1–3 · REWRITE (confirm) ·p · rule 1.6 · 3 → 2

````text
//! Primitives shared by checks, fixes, and stages: byte-level ports of the
//! shell pipeline's idioms.
````

### B3028 · l.11–18 · REWRITE (confirm) · rule 1.5 · 8 → 3

````text
/// Re-exported from [`crate::checks::build`] so the fix and stage layers share the
/// one implementation: the `arm64e`-must-not-satisfy rule is a parity invariant
/// (PARITY.md § Doctor / checks, "`helper_is_arm64` currently shells out to `lipo`").
````

### B3029 · l.27–33 · REWRITE (confirm) ·p · rule 1.2 · 7 → 4

````text
/// `cmp -s "$1" "$2"`: true iff both files exist, are readable, and are
/// byte-identical. Any error (missing, permission, directory) returns `false`,
/// so it never reports "equal" for a file it could not read.
/// See tests::cmp_files_matches_cmp_s_semantics.
````

### B3032 · l.93–112 · REWRITE (amend) ·p · rule 1.6 · 20 → 7

````text
/// Best-effort Beat Saber version for `bs_dir`, reproducing lib.sh's
/// `bs_version()` quirks: the marker file wins even when empty; otherwise every
/// stamp on the first matching line of `globalgamemanagers`, newline-joined, else
/// `?`. Trailing newlines are stripped (doctor captures via `$(bs_version)`).
///
/// See tests::bs_version_falls_back_to_question_mark and
/// tests::version_stamp_scan_matches_grep.
````

### B3037 · l.202–222 · REWRITE (amend) ·p · rule 1.6 · 21 → 8

````text
/// Render the host OpenXR manifest for `dylib_path` in its *comparison* form:
/// the template with the placeholder replaced by the JSON-escaped path, minus the
/// template's trailing newline (install.sh reads with `$(<file)`, which strips it).
///
/// Use [`host_manifest_file_bytes`] for what lands on disk — one extra byte and the
/// two front-ends thrash each other with sudo prompts. See
/// sabrage-parity tests::artifact_goldens::render_host_manifest_matches_the_on_disk_template,
/// sabrage-parity tests::artifact_goldens::render_host_manifest_json_escapes_the_dylib_path.
````

### B3038 · l.230–248 · REWRITE (amend) ·p · rule 1.5 · 19 → 9

````text
/// Escape `s` for embedding in a JSON string literal, byte-for-byte the way
/// install.sh does: backslash first (so introduced escapes are not re-escaped),
/// then double quote, nothing else. The escaped path lands in the root-owned host
/// manifest (PARITY.md § Declared by the 2026-08-30 adversarial review (round 1 fixes),
/// "Control characters in the checkout path.").
///
/// Not a full JSON encoder: control characters stay unescaped, as on the zsh side,
/// so widening this must land on both sides in the same commit. See
/// tests::json_escape_string_is_install_shs_two_substitutions.
````

### B3041 · l.273–278 · REWRITE (amend) · rule 1.5 · 6 → 6

````text
/// The `oxrsys-runtime.toml` first-write template, byte-for-byte: exactly the
/// bytes setup writes, including the trailing newline and every comment (comments
/// are load-bearing — the pre-2026-08 runtime parser choked on a same-line `#`).
///
/// **Write-once**: never regenerate, never migrate. See
/// sabrage-parity tests::artifact_goldens::toml_template_matches_the_on_disk_contract_file.
````

### B3043 · l.285–293 · REWRITE (confirm) ·p · rule 1.6 · 9 → 4

````text
/// True when every `[dxmt] files` entry is present under `ext/dxmt-artifacts/` —
/// ALL of them, never a subset. A partial overlay black-windows the game with no
/// error of its own, so `install` refuses to half-apply it.
/// See tests::dxmt_helpers_need_every_file_and_a_current_marker.
````

### B3044 · l.302–311 · REWRITE (confirm) ·p · rule 1.6 · 10 → 5

````text
/// True when the `.sha256` provenance marker matches the contract pin **and**
/// every `[dxmt] files` entry is present. Trailing newlines are irrelevant
/// (command-substitution semantics), so a marker written by either front-end
/// reads as current to the other. See [`contract_marker_bytes`] for the write
/// side and tests::dxmt_helpers_need_every_file_and_a_current_marker.
````

### B3045 · l.317–326 · REWRITE (confirm) · rule 1.5 · 10 → 6

````text
/// The exact bytes of the `.sha256` provenance marker `setup` writes: the pin
/// plus **one** trailing newline.
///
/// Zero or two would still *read* as current (command substitution eats them) but
/// would make the two front-ends write different bytes for the same state; see
/// tests::marker_bytes_are_the_pin_plus_exactly_one_newline.
````

### B3047 · l.333–340 · REWRITE (amend) ·p · rule 1.2 · 8 → 6

````text
/// The `meta.contract-sync` hash over contract bytes already in memory:
/// `cat <parts…> | shasum -a 256`, in the order given.
///
/// Same recipe as [`contract_hash`], so the compiled-in identity
/// ([`crate::contract::COMPILED_CONTRACT_SHA256`]) and the on-disk recompute
/// can differ only in *what* they hash, which is the skew they exist to expose.
````

### B3048 · l.350–364 · REWRITE (amend) ·p · rule 1.6 · 15 → 10

````text
/// The `meta.contract-sync` hash, recomputed from the contract files **on disk**
/// under `repo_root`, concatenated in `CONTRACT_FILES` order.
///
/// # Errors
/// Any contract file that cannot be opened or read.
///
/// Pinned by doctor.sh section 0 and the `# contract-sha256:` header of the
/// generated shell file. Runtime reads, not [`include_str!`]: this compares
/// the *checkout* against its own generated file, so a stale compiled-in copy
/// would defeat the tripwire. See tests::contract_hash_matches_the_generated_header.
````

### B3050 · l.397 · REWRITE (confirm) · rule 2.3 · 1 → 1

````text
    /// The repo root, three directories above this crate's manifest directory.
````

## `sabrage/crates/sabrage-core/src/util/winpath.rs`

### B3063 · l.1–25 · REWRITE (amend) ·p · rule 1.6 · 25 → 7

````text
//! `win_path()` — the unix→Windows path rule the run stage hands to wine
//! (reference: `scripts/demo/lib.sh`).
//!
//! Load-bearing (design-core §10 parity decision 22), pinned by
//! `sabrage-parity::tests::artifact_goldens::win_path_table`: `C:` needs the trailing slash of
//! `<prefix>/drive_c/` (bare `drive_c` falls through); `Z:` prefixes the whole unix path and only
//! translates separators.
````

## `sabrage/crates/sabrage-parity/src/lib.rs`

Deleted (nothing carried): B3068, B3074, B3079, B3098, B3101, B3115, B3116, B3123, B3127, B3131, B3133, B3136, B3140, B3144, B3146, B3148, B3166

### B3067 · l.26–29 · REWRITE (confirm) ·p · rule 1.2 · 4 → 5

````text
    /// The repo root, resolved from this crate's manifest dir.
    ///
    /// `sabrage-contract-gen` sits at the same `crates/<name>` depth, so both
    /// resolve the same directory — pinned by
    /// `tests::contract_gen_parity::check_reports_in_sync_against_the_live_checkout`.
````

### B3070 · l.49–52 · REWRITE (confirm) · rule 1.3 · 4 → 3

````text
            // include_str! resolves relative to this file, so the comparison is
            // against the bytes checked in on disk, never a value re-derived
            // from contract/.
````

### B3072 · l.72–82 · REWRITE (amend) · rule 1.2 · 11 → 6

````text
        /// The three contract scalars `sabrage-contract-gen` emits must be
        /// sourced by the shell, never re-typed: editing one in `pipeline.toml`
        /// and regenerating moves only the generated file's `# contract-sha256:`
        /// header, so `--check`, `--regen` and doctor's `meta.contract-sync` all
        /// report "in sync" while native setup fetches one asset and setup.sh
        /// another (or the two resolve BS_DIR to different directories).
````

### B3075 · l.170–177 · REWRITE (amend) · rule 1.5 · 8 → 6

````text
    /// `sabrage/parity/shell.fingerprint` pins the sha256 of `demo.sh` plus
    /// every `scripts/demo/*.sh` (`contract.gen.sh` excluded — `contract_gen_parity`
    /// covers it). Any edit to a tracked shell file — its content OR the tracked
    /// file set itself — turns this test red until `scripts/dev/parity.sh --bless`
    /// re-signs it, which per docs/design/design-parity.md §4 tier 1.2 only
    /// happens after the rest of the suite passes.
````

### B3082 · l.324–331 · REWRITE (amend) ·p · rule 1.5 · 8 → 8

````text
        /// The executable part of a shell line: everything before the first
        /// **comment** `#` — one that starts a word (line start, or after
        /// whitespace) and is not inside a single- or double-quoted string, so
        /// `${_f#$ROOT/}`, `$#` and a quoted `#` all survive.
        ///
        /// Without this the scanner credits a commented-out `chk` line as a live
        /// emission, so a deleted check keeps its coverage; see
        /// `tests::slug_coverage::a_hash_starts_a_comment_only_when_it_begins_an_unquoted_word`.
````

### B3088 · l.383–392 · REWRITE (amend) ·p · rule 1.3 · 10 → 8

````text
        /// Does the loop body starting at `rest` actually emit for `var`?
        ///
        /// The two halves must be *joined*, not merely both present: a `chk`/`tap`
        /// call counts only when it carries the loop item's slug, inline
        /// (`${_x%%:*}`) or through the assigned variable. Independent counting
        /// credits every header slug once the body contains any unrelated emission
        /// — the shape a deleted per-item check leaves behind (r1:A1-7). See
        /// `tests::slug_coverage::a_loop_credits_its_header_slugs_only_when_the_body_emits_for_them`.
````

### B3104 · l.741–744 · REWRITE (confirm) · rule 1.7 · 4 → 2

````text
        /// All slugs on `# <tag> <slug> [<slug> …]` lines, in file order; a line
        /// naming several slugs expands to one entry per slug.
````

### B3105 · l.756–762 · REWRITE (confirm) · rule 1.7 · 7 → 6

````text
        /// slug -> the gate its tag claims, for every tagged (or documented
        /// untagged) preflight in run.sh.
        ///
        /// Per gate, not one "is gating at all" bag: a bag cannot tell `warn` from
        /// `block` in either direction, so turning `game.version`'s `warn` into a
        /// `die` would leave the comparison unchanged.
````

### B3112 · l.945–947 · REWRITE (confirm) · rule 1.7 · 3 → 3

````text
        /// Swapping the two verbs of the mixed protocol group in memory — the
        /// change a group-level verb bag cannot see — must be reported once per
        /// half.
````

### B3117 · l.1104–1114 · REWRITE (confirm) · rule 1.2 · 11 → 7

````text
    /// Byte-exact rendering checks, each built from the template/contract file
    /// read fresh off disk rather than sabrage-core's compiled-in copy — plus
    /// the pure pins that live here per 3.5 (`win_path_table` and the
    /// write-signature shim), which read nothing. sabrage-core pins the
    /// compiled-in templates and their digests; this module is where a
    /// *rendered* artifact is compared against the on-disk bytes, so a stale
    /// `include_str!` shows up here as a byte diff on the artifact itself.
````

### B3118 · l.1137–1147 · REWRITE (confirm) · rule 1.7 · 11 → 6

````text
        /// r1:A1-8 regression: the dylib path is JSON-escaped, so a path
        /// containing `"` or `\` decodes back to itself.
        ///
        /// Both front-ends escape before the `@OXR_DYLIB@` substitution
        /// (`util::json_escape_string` here, parameter expansion in install.sh); a
        /// raw replace writes an invalid or misdirected root-owned manifest.
````

### B3119 · l.1172–1192 · REWRITE (confirm) · rule 1.7 · 21 → 9

````text
        /// The bytes install layer 4 actually **writes**: the file form, which is
        /// `render_host_manifest` plus one trailing newline (install.sh's
        /// `print -- "$WANT"`), never the newline-less comparison form. The two
        /// are one byte apart on the most drift-sensitive artifact in the
        /// pipeline, and `write_host_manifest_privileged` takes the dylib path
        /// rather than pre-rendered content so the mistake is unexpressible (the
        /// compile-time tripwire below). Driving layer 4 end to end needs an async
        /// runtime this crate does not depend on; that half is sabrage-core's
        /// `stages::install::tests::layer_four_stages_the_host_manifest_file_form_byte_for_byte`.
````

### B3122 · l.1313–1323 · REWRITE (confirm) · rule 1.3 · 11 → 5

````text
            // run.sh writes this file with `printf '%s' "$BS_APPID"`, so the
            // contract value must render with no trailing newline. The write
            // itself is covered by sabrage-core's `goldberg_stage` tests; this
            // pins the value and its shape, so a contract change that would alter
            // the on-disk bytes fails here first.
````

### B3126 · l.1396–1409 · REWRITE (confirm) · rule 1.7 · 14 → 7

````text
        /// Hermetic mirror of sabrage-core's registry invariants so CI (which runs
        /// only this crate + contract-gen on ubuntu) gates them: the strict
        /// registry must build, cover the contract in order, and leave **no** slug
        /// unbound — run-only preflights included, because `checks::run_only`
        /// binds a real evaluator for each even though they have no doctor row. A
        /// mis-registered evaluator would otherwise ship CI-green and panic at
        /// runtime in the CLI and app.
````

### B3132 · l.1493–1504 · REWRITE (amend) · rule 1.7 · 12 → 8

````text
        /// A dependency-free `block_on`.
        ///
        /// `sabrage-parity` carries no async runtime: every `Cargo.toml` entry is
        /// a dev-dependency and none is tokio. Every `DryRunExecutor` method that
        /// [`sabrage_core::stages::run::actions::goldberg_stage`] drives resolves
        /// on the first poll — none of them actually awaits — so a hand-rolled
        /// loop over [`std::task::Waker::noop`] drives it to completion with no
        /// dependency.
````

### B3134 · l.1534–1542 · REWRITE (confirm) · rule 1.6 · 9 → 6

````text
        /// run.sh's `printf '%s' "$BS_APPID" > "$APIDIR/steam_appid.txt"`
        /// (`# launch-action: goldberg-stage`): the appid digits, and nothing else
        /// — no trailing newline. Driven through the real `actions::goldberg_stage`
        /// under `--dry-run`, never a copy of its recipe, so a call-site regression
        /// turns this red: the plan's `Write` action records `"<n> bytes"`, and `n`
        /// must equal the appid string's own length.
````

### B3137 · l.1592–1605 · REWRITE (confirm) ·p · rule 1.7 · 14 → 9

````text
        /// A synchronous [`Executor`] that really writes, into the test's own
        /// scratch tree.
        ///
        /// A dry-run plan carries no content — `write_atomic` records only
        /// `"<n> bytes"` — so the golden above can pin the payload's length but not
        /// its bytes. [`sabrage_core::executor::RealExecutor`] cannot be driven from
        /// this crate (`tokio::fs` primitives, no async runtime), so `std::fs`
        /// behind the same trait gives the real on-disk bytes; every primitive the
        /// run stage does not use panics rather than pretending to succeed.
````

### B3138 · l.1731–1736 · REWRITE (confirm) · rule 1.6 · 6 → 4

````text
        /// The same `printf '%s' "$BS_APPID"` write (run.sh, `# launch-action:
        /// goldberg-stage`), read back off disk: the appid digits and **nothing
        /// else**. The dry-run golden above sees only the payload's length; this
        /// one goes red for a six-byte impostor (`999999`, `62098\n`) as well.
````

### B3141 · l.1783–1786 · REWRITE (confirm) · rule 1.6 · 4 → 5

````text
        /// run.sh's five wine exports, table form (`# launch-action:
        /// launch-wine`). The load-bearing branch is `WINEDEBUG`: the caller's
        /// preset wins in **both** the verbose and non-verbose arms
        /// (`${WINEDEBUG:-…}`), and an inherited empty string is treated like
        /// unset (zsh's `:-`, not `-`).
````

### B3147 · l.1891–1892 · REWRITE (confirm) · rule 1.5 · 2 → 3

````text
        /// `date +%Y%m%d-%H%M%S` for attempt 0; Sabrage's own `-{n+1}` suffix on a
        /// collision — a declared divergence (PARITY.md § Run (launch), "The wine
        /// console log is a plain file").
````

### B3162 · l.2295–2296 · REWRITE (confirm) · rule 1.6 · 2 → 2

````text
        /// run.sh's nine-line launch banner (`# launch-action: launch-wine`) —
        /// every line, in order, including the two blank lines that frame it.
````

### B3167 · l.2379–2393 · REWRITE (confirm) · rule 1.7 · 15 → 11

````text
    /// Both front-ends embed the repo root as an absolute string inside the
    /// root-owned host manifest and compare those bytes **literally**
    /// (`install.sh`'s `[ "$(cat "$HOST_XR_JSON")" = "$WANT" ]`,
    /// `util::host_manifest_is_current` here), so two spellings of one checkout
    /// mean two manifests and a sudo prompt per alternation (r1:A2-6).
    ///
    /// The shared contract is zsh's **logical** `pwd`: absolute, `.`/`..` folded
    /// textually, symlinks preserved as the user spelled them — demo.sh's
    /// `ROOT="$(cd "$(dirname "$0")" && pwd)"` and `paths::resolve_repo_root`'s
    /// `logical_absolute`, deliberately not `canonicalize`. This module pins both
    /// halves.
````

## `sabrage/src-tauri/src/commands.rs`

Deleted (nothing carried): B3173, B3182, B3245, B3252, B3257, B3282, B3286, B3290, B3305, B3314, B3349, B3352

### B3171 · l.1–58 · REWRITE (amend) · rule 1.7 · 58 → 20

````text
//! Tauri commands over `sabrage-core`: doctor, pipeline stages, fixes,
//! sessions, logs, settings, library, and runtime config.
//!
//! `sabrage_core::StageEvent` is forwarded to the frontend verbatim — it is
//! already the wire shape design-core §3.1 specifies (internally tagged on
//! `kind`, camelCase fields), so there is no second event type to keep in
//! sync. Streaming commands take an IPC [`Channel`]; the settings, library and
//! config commands do not stream and mutate through a bare [`RealExecutor`]
//! rather than a [`StageCtx`].
//!
//! [`launch`]'s promise does not resolve until the session ends, which can be
//! hours — every command's promise is a secondary confirmation, never the
//! liveness signal a screen renders off of.
//!
//! [`detach_session`]/[`resolve_quit`] are the app-quit-while-a-session-is-live
//! answer: `lib.rs`'s `ExitRequested`/`CloseRequested` handlers open the dialog
//! they resolve, and that comment cites this one for why they exist.
//!
//! `ui/src/ipc.ts` hand-mirrors every serde shape here 1:1 — keep both sides in
//! sync when either changes.
````

### B3174 · l.89–106 · REWRITE (confirm) · rule 1.7 · 18 → 10

````text
/// One streamed doctor row: a `CheckOutcome` plus the `group` the contract
/// attaches to its `slug` and, when the contract names one, the `fix` id
/// (`CheckOutcome` itself carries neither).
///
/// `fix` is the bare contract id (`"fix.set-graphics-backend"`), projected
/// through [`offered_fix_id`], so an id this build withholds
/// ([`sabrage_core::fixes::DEFERRED_CONTRACT_FIX_IDS`]) reaches no client at
/// all: `ipc.ts`'s own fix table is a hand-maintained mirror and cannot be
/// trusted to withhold it. See
/// tests::a_withheld_fix_reaches_no_doctor_row_and_no_fix_call.
````

### B3175 · l.119–131 · REWRITE (confirm) · rule 1.2 · 13 → 6

````text
/// The fix id a [`DoctorEvent`] may carry, given the one the contract names
/// for that check — `None` when it names none, and `None` for an id this build
/// models but withholds ([`sabrage_core::fixes::DEFERRED_CONTRACT_FIX_IDS`]).
/// [`FixAction::from_contract_id`] is the single source of truth for both, and
/// the id is round-tripped back out of the parsed action, so what the client
/// receives is by construction a spelling that function accepts.
````

### B3177 · l.146–152 · REWRITE (confirm) · rule 1.7 · 7 → 6

````text
/// Sidebar footer snapshot.
///
/// `default_bottle`/`default_bs_dir` are `settings.json`'s stored defaults,
/// straight through, so the Sidebar/Session screens prefill without a second
/// `get_settings` round trip. `None` means "nothing configured yet", same as on
/// [`sabrage_core::store::settings::Settings`] itself.
````

### B3179 · l.177–179 · REWRITE (confirm) · rule 1.7 · 3 → 2

````text
    // A corrupt or missing settings file degrades to defaults inside
    // `load_settings` rather than failing doctor outright.
````

### B3181 · l.202–206 · REWRITE (confirm) · rule 1.7 · 5 → 4

````text
    // Precedence, highest first: explicit GUI args > `WINEVR_*` env (parity
    // with the CLI and demo.sh) > the persisted `settings.json` defaults — a
    // Finder-launched .app has no environment at all, so the last tier is the
    // only one that can supply what the Settings screen configured.
````

### B3184 · l.266–268 · REWRITE (confirm) · rule 1.7 · 3 → 3

````text
/// Sidebar footer snapshot: repo root (if resolvable), bottles present on this
/// machine, the pinned ALVR client version, and `settings.json`'s default
/// bottle/Beat Saber dir.
````

### B3185 · l.283–304 · REWRITE (confirm) · rule 1.4 · 22 → 5

````text
// `tokio_util::sync::CancellationToken` and `uuid::Uuid` are reached without
// either crate being a direct dependency of `sabrage-app`: `Default::default()`
// builds the token `StageCtx::new` wants, and the run id is threaded in its
// `.to_string()` form (the same bytes `StageEvent`'s `runId` serializes as), so
// [`RunRegistry`] stores an opaque canceller rather than the concrete token.
````

### B3188 · l.326–334 · REWRITE (confirm) · rule 1.7 · 9 → 8

````text
/// Options for [`launch`] — [`StageRunOpts`]'s bottle/bs-dir/dry-run plus the
/// four flags `run.sh` reads only inside itself
/// (`WINEVR_NO_AUDIO`/`_NO_DASHBOARD`/`_WIRED`/`_VERBOSE`), which is why they
/// have no home on [`StageRunOpts`] — every other stage ignores them.
///
/// `game_id` is the library entry this launch came from, if any: the Library
/// screen's "Run through bridge" sets it, an ad hoc launch leaves it `None`.
/// See [`last_session_to_record`] for what it unlocks.
````

### B3192 · l.401–406 · REWRITE (confirm) · rule 1.7 · 6 → 4

````text
/// [`channel_sink`], plus a `tap` called with every event before it is
/// forwarded — [`launch`]'s way of observing whether `StageEvent::Launched`
/// fired for this run ([`last_session_to_record`]) without a second event type
/// or a second subscription.
````

### B3194 · l.445–458 · REWRITE (confirm) ·p · rule 1.7 · 14 → 7

````text
/// Merge GUI-supplied stage options onto the `WINEVR_*` environment — the same
/// precedence [`run_doctor`] gives [`CheckOptions`]: start from
/// [`StageOptions::from_env`], then let `bottle`/`bs_dir` override
/// field-by-field only when the caller supplied one. `dry_run` is the one
/// field this does not set — [`execute_stage`] and [`fix`] each decide it
/// afterwards, since a fix is never a dry run regardless of the environment.
/// See tests::stage_options_from_env_and_gui_honours_winevr_bottle_when_the_gui_passes_none.
````

### B3195 · l.470–476 · REWRITE (confirm) · rule 1.7 · 7 → 8

````text
/// The lowest precedence tier of [`stage_options_from_env_and_gui`] (and of
/// [`run_doctor`]'s `CheckOptions`): `settings.json`'s `default_bottle` /
/// `default_bs_dir` fill a bottle or Beat Saber dir that neither the
/// environment nor the caller supplied — a Finder-launched .app has no
/// `WINEVR_*` environment, so this tier is what makes the Settings screen's
/// "Paths" card reach setup/build/install/doctor/stop at all. Pure (settings
/// passed in) so it is testable without touching `$HOME`. See
/// tests::settings_defaults_fill_only_what_env_and_gui_left_unset.
````

### B3198 · l.526–546 · REWRITE (confirm) ·p · rule 1.7 · 21 → 12

````text
/// Announce a run that is about to queue behind another operation, so the
/// frontend can name it (and offer Cancel) during the wait.
///
/// Emitted as an app event rather than on the stage channel, because
/// `StageEvent::StageStarted` is always the first event of a run and a queue
/// notice is not part of the run's own stream. The probe is
/// [`sabrage_core::stages::operation_in_progress_anywhere`], not the in-process
/// half alone: the wait is most often behind a `sabrage` CLI build in another
/// process, which the in-process mutex cannot see. Best-effort in both
/// directions — the probe is racy, and a failed emit is dropped like a failed
/// channel send. See
/// stages::tests::a_queued_stage_announces_itself_and_cancels_out_of_the_wait.
````

### B3199 · l.560–570 · REWRITE (confirm) · rule 1.7 · 11 → 9

````text
/// [`execute_stage`]/[`launch`]'s shared body, taking an already-built
/// [`EventSink`] rather than a raw `Channel` — the seam [`launch`] uses to pass
/// a tapped sink ([`channel_sink_tee`]) instead of a plain forwarding one, so
/// it can observe `StageEvent::Launched` without a second subscription.
///
/// Resolves the repo root through [`resolve_repo_root_via_settings`], builds a
/// [`StageCtx`] from an already-merged [`StageOptions`], registers its
/// cancellation handle, runs the stage, and unregisters on the way out,
/// success or failure alike.
````

### B3201 · l.618–635 · REWRITE (confirm) ·p · rule 1.7 · 18 → 12

````text
/// Trailing "plan (dry run)" rows: which copies would happen and which would be
/// skipped because the bytes already match — a distinction the narrative rows
/// do not draw.
///
/// Emitted as a [`StageEvent::Section`] plus one `info` row per action, using
/// [`sabrage_core::dry_run_plan_body`] — the same text the CLI prints under its
/// own `-- plan (dry run)` header, so the two front-ends say the same thing
/// word for word, after `StageFinished` and on the failure path too
/// (`(nothing planned)` when the stage died before its first mutating step).
/// Keyed on `executor.is_dry_run()` rather than `opts.dry_run`, so a real run's
/// event stream is untouched. See
/// tests::a_dry_run_emits_the_shared_plan_rows_and_a_real_run_emits_none.
````

### B3202 · l.646–662 · REWRITE (confirm) · rule 1.7 · 17 → 12

````text
/// Run one pipeline stage (`setup`/`build`/`install`/`stop`/`run`), streaming
/// every [`StageEvent`] to `on_event` as it happens.
///
/// `run` behaves identically to [`launch`] with the same options — both reach
/// [`sabrage_core::run_stage`] through [`execute_stage_with_sink`] — but
/// [`launch`] is the intended UI entry point for it: it is named for what it
/// does, and its doc comment warns that the call can take hours.
///
/// The returned promise does not resolve until the stage finishes (or fails);
/// callers drive their UI off the event stream (in particular `StageFinished`)
/// and treat the resolved/rejected promise as a secondary confirmation — the
/// same shape [`run_doctor`] already uses.
````

### B3204 · l.694–697 · REWRITE (confirm) · rule 1.7 · 4 → 4

````text
    // Tee the channel so a `StageEvent::Launched` for *this* run is observed,
    // win or lose — `last_session_to_record` below is the pure decision of
    // whether that (plus `game_id`, plus a settled outcome) is worth writing
    // into `library.json`.
````

### B3207 · l.746–753 · REWRITE (confirm) · rule 1.7 · 8 → 8

````text
/// What [`stop_live_session_and_wait`]'s bounded wait actually observed.
///
/// The distinction is the whole point: a teardown that hung past
/// [`LIVE_SESSION_STOP_TIMEOUT`] must never be reported as one that finished,
/// or [`stop_session`] answers `ok: true` and [`resolve_quit`]'s `Stop` arm
/// approves the quit and exits, skipping the detach fallback in `lib.rs`'s
/// `RunEvent::Exit` arm and leaving the game and its guards unsupervised. See
/// tests::the_stop_and_quit_refusal_claims_only_what_happened.
````

### B3212 · l.770–789 · REWRITE (confirm) · rule 1.7 · 20 → 16

````text
/// What [`resolve_quit`]'s `Stop` arm must tell the user about a teardown that
/// did not simply finish — `None` when it did.
///
/// Pure, and deliberately says only what happened: it may not claim a detach,
/// because detaching is a no-op in both refusal arms — a `TimedOut` stop has
/// already fired the session's `cancel` token, and
/// [`sabrage_core::session::reconcile::detach`] then returns `Ok(())` without
/// disarming a guard or marking the record, while a `Detached` slot is already
/// clear so [`detach_live_session`] finds no live handle at all.
///
/// Not detaching is also the better outcome: `detached: true` means "leave
/// every guard in place, and no later reconcile may undo that"
/// (`session::reconcile`'s module doc), whereas a record left `detached: false`
/// with a dead owner is exactly the shape `reconcile::classify` recovers on the
/// next Sabrage start — restoring audio and removing the `--wired` forwards.
/// See tests::the_stop_and_quit_refusal_claims_only_what_happened.
````

### B3215 · l.827–839 · REWRITE (confirm) · rule 1.7 · 13 → 12

````text
/// Fire the live session's `cancel` token — the INT path: stop wine, then
/// restore every guard (see [`sabrage_core::session`]'s module doc) — and wait,
/// bounded at [`LIVE_SESSION_STOP_TIMEOUT`], for
/// [`sabrage_core::live_session`] naming that same run to go back to `None`.
/// [`TeardownWait::NothingLive`] when nothing is live.
///
/// Shared by [`stop_session`]'s live-session branch and [`resolve_quit`]'s
/// `Stop` arm, so the two can never disagree on what "stop the session" means —
/// including on what *failing* to stop it means. The wait runs inside
/// [`tauri::async_runtime::spawn_blocking`] because no `tokio::time` is
/// reachable from this crate to `.await` a sleep with; only `tokio::sync` items
/// arrive, via [`tauri::async_runtime`].
````

### B3217 · l.863–873 · REWRITE (confirm) · rule 1.7 · 11 → 10

````text
/// Did the run whose slot just cleared *detach* instead of tearing down?
///
/// `session-state.json` outlives the handle: a detach marks the record
/// `detached` and leaves it in place, while a completed teardown clears it.
/// Both tokens are independent (`session/mod.rs`), so a Stop and a detach can
/// race; this is what stops [`stop_session`] from reporting `ok: true` for a
/// session that is in fact still running.
///
/// Best-effort: an unresolved repo root or an unreadable record answers "no",
/// and the caller then reports the ordinary stop.
````

### B3218 · l.885–898 · REWRITE (confirm) · rule 2.3 · 14 → 10

````text
/// Apply the "keep running" answer to a quit nobody could be asked about
/// (AppKit `terminate:` — Dock-menu Quit, logout, AppleScript `quit` — which
/// tao cannot intercept): when a session this process supervises is still live
/// and no dialog answer approved this exit, detach synchronously —
/// [`detach_live_session`] fires the handle's detach token and waits (bounded,
/// inside `reconcile::detach`) for the supervise loop to disarm its guards and
/// mark `session-state.json` `detached`.
///
/// Best-effort: an error here must not stop the process from exiting, and there
/// is nobody left to show it to.
````

### B3243 · l.1179–1198 · REWRITE (confirm) · rule 1.7 · 20 → 17

````text
/// Apply one fix ([`FixAction`]). A destructive fix ([`FixAction::def`]) needs
/// `confirmed: true`; the frontend shows its own in-app confirm dialog first
/// (never `window.confirm`, which blocks the webview) and this check is the
/// backend's half of that contract, not a substitute for it.
///
/// A fix whose [`FixAction::as_stage`] is `Some` (`RunSetup`/`RunBuild`/
/// `RunInstall`) works through this command too — it delegates to
/// [`sabrage_core::run_stage`] internally — but the intended UI path for those
/// three is [`run_stage`], so the GateModal they open is the one a plain stage
/// run uses.
///
/// An action this build withholds ([`FixAction::is_deferred`]) is refused here
/// whatever the frontend believed, because the GUI's fix table is a
/// hand-maintained mirror; the `sabrage fix <id>` CLI path is deliberately not
/// gated the same way, for a user who has read
/// [`sabrage_core::fixes::FixDef::consequence`]. See
/// tests::a_withheld_fix_reaches_no_doctor_row_and_no_fix_call.
````

### B3244 · l.1210–1220 · REWRITE (confirm) · rule 1.3 · 11 → 5

````text
    // A fix waits on the same operation lock a stage does, and its whole-stage
    // forms run for minutes, so it needs a stage's cancellation handle:
    // `fixes::apply` emits a row carrying `ctx.run_id` *before* it waits for
    // the lock, so a queued fix can be named — and cancelled — and the `forget`
    // below therefore covers the cancelled path too.
````

### B3251 · l.1286–1303 · REWRITE (confirm) · rule 1.7 · 18 → 15

````text
/// Started once from `lib.rs`'s `.setup()`: snapshot the session every second
/// and broadcast it on `session://status` **when it changed**. Runs for the
/// app's lifetime; there is nothing to stop it with.
///
/// The dedup is here rather than in the frontend store, which assigns every
/// payload straight through (`stores/session.svelte.ts`); the first snapshot
/// after startup is always emitted, and [`get_session_status`] is the poll
/// fallback for a listener that attached late.
///
/// The 1 s sleep runs on a blocking task with a synchronous `block_on` of the
/// lock+snapshot, for the same reason [`stop_live_session_and_wait`]'s wait
/// does: no `tokio::time` is reachable from this crate, and
/// [`tauri::async_runtime::spawn_blocking`] plus
/// [`tauri::async_runtime::block_on`] is the primitive that is. See
/// tests::the_status_broadcast_skips_repeats_but_never_the_first_one.
````

### B3261 · l.1431–1440 · REWRITE (confirm) · rule 1.7 · 10 → 11

````text
/// Tracks in-flight [`start_log_tail`] pollers by an opaque id. Stopping one
/// flips its [`AtomicBool`] rather than aborting the task outright: the task
/// notices on its own next wake (at most [`LOG_TAIL_POLL_INTERVAL`] later) and
/// exits between polls instead of being cut off mid-read.
///
/// Every registration is paired with a [`TailGuard`] moved into the polling
/// task, so *every* way that task can end — the stop flag, a send error, an
/// unreadable file — removes its entry; without it the map keeps ids whose task
/// is already dead and [`stop_log_tail`] answers `true` for them, contradicting
/// its own "no longer tracked" contract. See
/// tests::a_tail_unregisters_itself_when_its_task_ends.
````

### B3263 · l.1494–1499 · REWRITE (confirm) · rule 1.5 · 6 → 6

````text
    /// Stop every tracked tail. `lib.rs` calls this from the builder's
    /// `on_page_load` hook: a webview reload runs no Svelte `onDestroy`, and a
    /// `Channel::send` on macOS is a `webview.eval` that keeps succeeding after
    /// a reload, so a tail cannot notice on its own and would poll its file
    /// every 250 ms for the rest of the app's life. See
    /// tests::stop_all_stops_every_tracked_tail.
````

### B3266 · l.1551–1556 · REWRITE (confirm) · rule 1.5 · 6 → 3

````text
                    // A send error means the channel is gone for good; it is
                    // NOT a reload signal (on macOS `Channel::send` is a
                    // `webview.eval`) — `TailRegistry::stop_all` covers those.
````

### B3271 · l.1597–1616 · REWRITE (amend) · rule 1.7 · 20 → 16

````text
// The commands below back the Settings/Library/Edit-game screens. None of them
// streams over an IPC `Channel`, so every mutation goes through a bare
// `RealExecutor` ([`real_executor`]) rather than a `StageCtx`: there is no
// multi-step stage here, nothing to cancel, and no live listener for a single
// small JSON/TOML write to stream to. Every mutation still goes through the
// `Executor` trait (the crate-wide rule); it just never needs the
// dry-run/stage machinery layered on top.
//
// `settings.json`/`library.json`/`oxrsys-runtime.toml` all live under paths
// derived from `$HOME` alone — [`sabrage_core::paths::sabrage_support_dir`] and
// `Paths::oxr_appsup`/`toml_path`, never from the repo root — so
// [`SettingsPathsCache`] tolerates an unresolved repo root (falling back to an
// empty one) rather than erroring out: these screens must keep working before a
// wine-vr checkout is even configured. Of the commands in this section only
// [`get_repo_info`] reports whether the repo root itself actually resolved;
// doctor and stage execution above already fail loudly on it.
````

### B3273 · l.1640–1646 · REWRITE (amend) · rule 1.7 · 7 → 5

````text
/// [`resolve_repo_root`], honoring `settings.json`'s persisted override — what
/// every call site that has no override of its own resolves through. Built on
/// [`load_settings`], so a corrupt settings file degrades to `None` (the env
/// and executable-walk precedence tiers still apply underneath) rather than
/// turning every command that resolves a repo root into a hard failure.
````

### B3274 · l.1651–1673 · REWRITE (confirm) · rule 1.7 · 23 → 18

````text
/// Cached `settings.json` + the [`Paths`] derived from it, for the settings,
/// library and config commands (and [`launch`]'s last-session recording) that
/// only ever read `settings.json` and the `$HOME`-derived halves of [`Paths`]
/// (`sabrage_appsup`, `oxr_appsup`/`toml_path`) — never the machine-probed
/// halves (`cx_app`, `wine`, `adb`, …), which stay live-probed by
/// [`run_doctor`]/[`get_app_state`], because a doctor row or the sidebar footer
/// must reflect a CrossOver reinstall or a freshly plugged `adb` without a
/// restart.
///
/// [`Paths::new`] re-probes the machine (three `stat`s and a `$PATH` walk) and
/// `settings::load` re-reads and re-parses `settings.json` on every call;
/// E-C3-settings-paths-cache measured a single Settings-screen mount paying
/// that cost four times over. Empty until first use, filled on demand, and
/// dropped by [`save_settings`] — the only writer of `settings.json` through
/// this app — so an edit from outside (or a bottle added on disk) can leave it
/// stale until the next save. See
/// tests::{cache_hit_returns_the_stored_pair_without_reloading,
/// invalidate_drops_the_cached_pair}.
````

### B3281 · l.1742–1749 · REWRITE (confirm) · rule 1.7 · 8 → 6

````text
/// A [`RealExecutor`] for the small, non-stage mutations this section adds —
/// see the module note above for why no [`StageCtx`] applies. `run_id` and the
/// cancellation token are `Default::default()`, the same way [`RunRegistry`]'s
/// section reaches `Uuid`/`CancellationToken` without either crate being a
/// direct dependency of `sabrage-app`; the sink is [`null_sink`], since none of
/// these commands stream events.
````

### B3284 · l.1766–1798 · REWRITE (confirm) · rule 1.7 · 33 → 28

````text
/// Patch `oxrsys-runtime.toml`'s six editable keys, creating it from the shared
/// template first if it does not exist yet ([`runtime_config::write`]'s
/// write-once-on-create rule).
///
/// Refuses **before** calling [`runtime_config::write`] when the file is one
/// Sabrage must not rewrite ([`runtime_config::RuntimeConfigView::parse_error`]):
/// either `toml_edit` cannot parse it at all, or its physical lines and the
/// parsed document disagree about where an editable key is assigned (a
/// `"""…"""` block containing `protocol = …`, a BOM in front of a key), so an
/// edit would land somewhere the runtime's own line reader does not look.
/// `apply_patch` refuses on both counts too — checking here only buys the
/// message the view already computed — and the two refusals share
/// `line_document_mismatch` precisely so they cannot drift.
///
/// A live session refuses the write ([`sabrage_core::session::ensure_idle_in`],
/// the policy `runtime_config::write` also enforces): `Config.cpp` re-reads the
/// file every 250 ms and the ALVR frame path rebuilds the encoder when
/// `encoder_process`/`video_codec` move, so a save mid-stream is a live
/// reconfiguration. Checked here as well so the error text comes from one place
/// and nothing — not even the backup — is written on the way to the refusal.
///
/// The operation lock ([`sabrage_core::stages::acquire_operation_lock`]) is
/// held across the whole read-modify-write: `stages::setup` writes the template
/// while holding the same lock, so without it a concurrent `setup` could
/// overwrite a patch that had just reported success. It also serializes this
/// against the `edit-protocol` fix, which re-enters the same writer. See
/// config::runtime_toml::tests::{a_key_inside_a_multiline_string_reads_live_and_refuses_the_write,
/// write_refuses_while_a_session_is_live_and_touches_nothing}.
````

### B3288 · l.1838–1839 · REWRITE (confirm) · rule 1.7 · 2 → 2

````text
/// Persist `settings.json`, returning it back as-saved — nothing on this side
/// is derived beyond what was sent.
````

### B3299 · l.1931–1934 · REWRITE (confirm) · rule 1.7 · 4 → 3

````text
    /// The nearest **existing** directory at or above `current` (when given),
    /// else at or above `derived`, else `$HOME` — an NSOpenPanel handed a path
    /// that does not exist just opens wherever it last was.
````

### B3318 · l.2276–2282 · REWRITE (confirm) · rule 1.2 · 7 → 5

````text
    /// Serializes every test in this module that touches a `WINEVR_*`
    /// variable: `std::env::set_var`/`remove_var` are process-global. Named
    /// for `WINEVR_BOTTLE` but not scoped to it —
    /// [`launch_stage_options_layers_the_launch_flags_with_gui_precedence`]
    /// holds it for the four launch flags too.
````

### B3326 · l.2357–2362 · REWRITE (confirm) · rule 1.7 · 6 → 3

````text
        // Finding #4: the env base is read before the GUI's own args, so a
        // GUI `None` still picks up `WINEVR_BOTTLE`; without it a Session
        // screen's `stop_session(None)` dies with "bottle name required".
````

### B3334 · l.2440–2441 · REWRITE (confirm) · rule 1.7 · 2 → 2

````text
        // Finding #8: the live branch is scoped by bottle, so stopping bottle
        // B can never tear down a live session on bottle A.
````

### B3335 · l.2455–2457 · REWRITE (confirm) · rule 1.7 · 3 → 3

````text
        // A11-1: `TeardownWait` separates a finished teardown from one still
        // running at the deadline, so `stop_session` cannot report both as
        // success.
````

### B3336 · l.2479–2483 · REWRITE (confirm) · rule 1.7 · 5 → 3

````text
        // A11-1 (round 2): the stop has already fired the session's cancel
        // token and `reconcile::detach` returns `Ok(())` once it is set, so
        // the `TimedOut` message may not claim a detach it cannot back.
````

### B3337 · l.2508–2511 · REWRITE (confirm) · rule 1.7 · 4 → 3

````text
        // A4-2 / A12-1: `DEFERRED_CONTRACT_FIX_IDS` is enforced at both IPC
        // doors, not only in `FixAction::from_contract_id`, so a row such as
        // `cfg.session-pins` cannot render a Fix button for a withheld id.
````

### B3340 · l.2555–2558 · REWRITE (amend) · rule 1.5 · 4 → 3

````text
        // A11-4: the dialog has exactly one responder (the webview's
        // `app://quit-requested` listener), so without the deadline a webview
        // that never answers leaves every Cmd-Q and window close prevented.
````

### B3341 · l.2608–2610 · REWRITE (confirm) ·p · rule 1.7 · 3 → 3

````text
        // A11-5: a tail unregisters itself when its task ends, not only via
        // `stop_log_tail`, so `stop_log_tail` cannot answer `true` for a
        // dead task.
````

### B3344 · l.2656–2657 · REWRITE (confirm) · rule 1.7 · 2 → 2

````text
        // E-A11: the 1 Hz broadcast drops repeats in the backend; the
        // frontend store is not relied on to dedup.
````

### B3345 · l.2679–2682 · REWRITE (confirm) · rule 1.7 · 4 → 3

````text
        // Finding #13 (GUI half): `planned()` rows reach the GUI so a dry run
        // can tell "would copy" from "would skip (bytes already match)", the
        // distinction the plan exists for.
````

### B3350 · l.2766–2769 · REWRITE (amend) · rule 1.7 · 4 → 3

````text
        // `resolve_repo_root`'s explicit-override and env tiers cannot
        // themselves fail (`paths.rs`), so an explicit setting wins whether or
        // not the walk would also have succeeded, and env likewise over the walk.
````

### B3351 · l.2798–2802 · REWRITE (confirm) · rule 1.3 · 5 → 3

````text
        // Validity is recomputed, never trusted from the stored JSON: an
        // entry whose bs_dir and bottle do not exist comes back `NotFound`
        // whatever the entry itself claims.
````

### B3353 · l.2820–2824 · REWRITE (amend) · rule 1.3 · 5 → 3

````text
        // E-C3-settings-paths-cache: seeds the cache directly rather than
        // through `snapshot`'s load-on-miss path, which would touch the real
        // `settings.json`; a populated cache must serve exactly what was stored.
````

## `sabrage/src-tauri/src/lib.rs`

Deleted (nothing carried): B3355, B3358, B3360, B3368

### B3354 · l.10–15 · REWRITE (confirm) · rule 2.3 · 6 → 4

````text
/// Builds the native menu bar: App / Edit / Pipeline / Window.
///
/// The Edit submenu's predefined clipboard items are load-bearing: without
/// them Cmd-C / Cmd-V do not work in webview text inputs on macOS.
````

### B3356 · l.20 · REWRITE (amend) · rule 1.7 · 1 → 2

````text
    // Disabled: `run()`'s `on_menu_event` has no `app_settings` arm, so the
    // item would open nothing.
````

### B3357 · l.32–41 · REWRITE (amend) ·p · rule 1.7 · 10 → 3

````text
    // Not `PredefinedMenuItem::quit`: it sends AppKit's `terminate:`, which
    // tao does not intercept, so `ExitRequested` never fires. A custom item
    // calling `app.exit(0)` takes the interceptable `RequestExit` path.
````

### B3359 · l.88–89 · REWRITE (confirm) · rule 1.7 · 2 → 2

````text
    // Pipeline submenu: `run()`'s `on_menu_event` turns these ids into
    // `menu://…` events the frontend acts on.
````

### B3361 · l.120–129 · REWRITE (confirm) ·p · rule 1.2 · 10 → 4

````text
/// What to do with one `ExitRequested`/`CloseRequested`, per
/// [`commands::quit_intercept_decision`]. A dialog nobody answers cannot make
/// the app unquittable; see
/// `commands::tests::quit_is_intercepted_once_and_given_up_on_when_nobody_answers`.
````

### B3363 · l.147–166 · REWRITE (amend) ·p · rule 1.2 · 20 → 6

````text
/// Builds and runs the app.
///
/// Uses `run`'s callback form so `ExitRequested` and the main window's
/// `CloseRequested` can be intercepted: one window means closing it is
/// app-quit, and both arms share [`quit_decision`]. See
/// `commands::tests::quit_is_intercepted_once_and_given_up_on_when_nobody_answers`.
````

### B3365 · l.211–217 · REWRITE (confirm) · rule 1.1 · 7 → 3

````text
        // A webview reload runs no Svelte `onDestroy`, and this app has
        // exactly one window, so every tail registered before a page load
        // belongs to the page being replaced. See `TailRegistry::stop_all`.
````

### B3369 · l.279–287 · REWRITE (amend) · rule 1.5 · 9 → 4

````text
        // AppKit's `terminate:` (Dock-menu Quit, logout) cannot be
        // intercepted but still reaches here as `Exit`: a session nobody was
        // asked about is detached ([`commands::detach_on_terminate`]) so the
        // guards' `Drop` fallbacks do not yank a running game's audio device.
````

## `sabrage/ui/src/App.svelte`

### NEW-1 · before l.2 · ADD (amend) ·p · rule 1.2 · 0 → 10

````text
  /**
   * Application shell: owns the active `screen`, the Library entry EditGame is
   * open for, and the two menu-request counters. Owns no store — reads
   * `doctorStore` for the sidebar badge, calls `sessionStore.stop()` for Stop,
   * and passes `stageStore` to the stages panel and gate modal.
   *
   * Invariant: a menu-triggered Launch or Run Doctor only navigates and bumps
   * a counter here; the owning screen performs the action, giving one launch
   * path and one doctor path.
   */
````

### B3370 · l.21–23 · REWRITE (confirm) · rule 1.2 · 3 → 2

````text
  /** The Library entry `"edit"` is open for; `null` means "Add game" (a new
   * entry). Only meaningful while `screen === "edit"`; stale otherwise. */
````

### B3371 · l.26–30 · REWRITE (confirm) · rule 1.2 · 5 → 3

````text
  /** Bumped every time the Pipeline ▸ Launch menu item (⌘R) fires; Session
   * watches the prop and calls its own `doLaunch(false)` once its bottle and
   * options have loaded, so the menu and the Launch button share one path. */
````

### B3372 · l.33–37 · REWRITE (confirm) · rule 1.2 · 5 → 3

````text
  /** Bumped every time the Pipeline ▸ Run Doctor menu item (⌘D) fires; Doctor
   * watches the prop and forces a fresh pass even when it is already the open
   * screen (plain navigation is then a no-op) or its cached result is fresh. */
````

### B3375 · l.56–57 · REWRITE (confirm) · rule 1.2 · 2 → 2

````text
  /** EditGame's Save/Cancel — both return to Library; on Save the entry has
   * already been persisted before this runs. */
````

### B3376 · l.62–65 · REWRITE (confirm) ·p · rule 1.3 · 4 → 3

````text
  // Menu ids this shell does not handle (Setup/Build/Install, Open Logs or
  // Config Folder) are deliberate no-ops — they belong to another screen or
  // the opener plugin.
````

## `sabrage/ui/src/components/BottleSelect.svelte`

### B3377 · l.2–5 · REWRITE (confirm) ·p · rule  · 4 → 1

````text
  // Presentational only: callers own `bottles`/`bottlesLoaded` and pass them in; no owned state.
````

## `sabrage/ui/src/components/CheckRow.svelte`

### B3381 · l.12–14 · REWRITE (confirm) · rule 1.3 · 3 → 2

````text
    /** Non-null disables the Fix button and explains why via its `title`
     * (e.g. a live session, where a fix would be refused server-side anyway). */
````

### B3383 · l.45–48 · REWRITE (amend) ·p · rule 1.7 · 4 → 3

````text
      <!-- The check's own diagnostic (e.g. "read error"/"JSON parse error"): remedy is fixed per slug
           and can misblame a cause the native evaluator never hit (A3b-3,
           checks::config::tests::malformed_json_warns, checks::config::tests::unreadable_session_json_warns), so detail is truthful. -->
````

## `sabrage/ui/src/components/GateModal.svelte`

Deleted (nothing carried): B3388, B3400

### NEW-1 · before l.2 · ADD (amend) ·p · rule 1.2 · 0 → 10

````text
  /**
   * The single gate modal, mounted once at the app root. Owns the transcript
   * for one pipeline operation (rows, console, progress, outcome, fix state)
   * and drives `stageStore`: reads `.gate` via the `request` prop and holds
   * `.setRunning` for setup/build/install/stop runs, which it starts via
   * `runStage`. In run mode it starts nothing — `sessionStore.launch(...)` was
   * already called by the opener, and this component only renders
   * `sessionStore.launchRows` (the same store Session.svelte reads, so the
   * transcript survives closing the modal and mid-launch navigation).
   */
````

### B3386 · l.31–33 · REWRITE (confirm) · rule 1.3 · 3 → 2

````text
  // Extracted from the imported union so the payload shape cannot drift from
  // events.rs by hand.
````

### B3387 · l.46–50 · REWRITE (confirm) · rule 1.2 · 5 → 3

````text
  /** One `StageEvent` -> the row it renders as, or `null` for the four kinds
   * that drive other UI (progress bar, console pane, runId, finished banner)
   * instead of a row of their own. */
````

### B3389 · l.77–83 · REWRITE (confirm) · rule 1.7 · 7 → 3

````text
  /** This request's runId announced by `stage://queued` - arrives (if at all)
   * while the run is still waiting on `OPERATION_LOCK`, before its own
   * `stageStarted` row exists, so Cancel has a target during that wait. */
````

### B3390 · l.92–96 · REWRITE (confirm) · rule 1.5 · 5 → 3

````text
  // Set when a `fatal` event already rendered the condition as a row: the
  // rejected promise that follows carries the same text verbatim, so showing
  // it again duplicates it. Invoke-layer rejections never set it.
````

### B3391 · l.100–104 · REWRITE (amend) ·p · rule 1.2 · 5 → 3

````text
  /** The in-flight fix's run id, from `applyFix`'s first `StageEvent`.
   * Emitted before the operation lock, so a queued fix has a `cancelStage`
   * target. Distinct from `runId` above, the stage's own run. */
````

### B3393 · l.115–125 · REWRITE (confirm) ·p · rule 1.7 · 11 → 7

````text
  /**
   * The request actually driving `rows`/`runId`/`sessionStore.launchRows`.
   * The `request` prop reflects `stageStore.gate`, which a new `openGate(...)`
   * can replace while this one is still running and merely hidden. The template
   * renders `displayRequest`, never `request`, so a queued replacement cannot
   * relabel the in-flight operation's title, rows, or Cancel target.
   */
````

### B3395 · l.133–139 · REWRITE (confirm) ·p · rule 1.3 · 7 → 4

````text
  // Fresh `openGate(...)` calls always hand in a new object, so identity
  // comparison detects "a new run was requested". Run mode never calls
  // `start()`: `sessionStore.launch(...)` already ran and this component only
  // observes. A replacement arriving while `running` is deliberately deferred.
````

### B3396 · l.165–169 · REWRITE (amend) · rule 1.3 · 5 → 4

````text
  // One modal instance, mounted once at the app root, and every opener
  // (Doctor, StagesPanel, Session) routes through it: with the process-wide
  // operation lock allowing at most one stage running or waiting, a
  // `stage://queued` event always belongs to whichever request is open.
````

### B3397 · l.215–218 · REWRITE (confirm) · rule 1.3 · 4 → 3

````text
    // Mirrored on the store so other openers (StagesPanel's Run/Dry-run,
    // Doctor's whole-stage Fix) disable themselves instead of racing a second
    // `openGate(...)` against this in-flight one.
````

### B3399 · l.299–302 · REWRITE (confirm) · rule 1.1 · 4 → 2

````text
          // The first event of this fix's own stream carries its run id;
          // capture it once so Cancel targets the fix, not the stage's `runId`.
````

### B3401 · l.345–349 · REWRITE (confirm) · rule 1.7 · 5 → 3

````text
  // `sessionStore.launchedEv`/`fatalEv`/`startedEv` are O(1) fields the store
  // captures in its own `launch()` callback, so these `$derived`s cost no scan
  // over `launchRows`. See that store's doc comments.
````

### B3402 · l.352–360 · REWRITE (amend) · rule 1.5 · 9 → 4

````text
  // The launch's own `runId`, the id `cancelStage` takes.
  // Never `sessionStore.stop()` in this window: run holds `OPERATION_LOCK`
  // from `stageStarted` through `launched` (sabrage-core `stages`, "Lock
  // policy for `run`"), so a stop would block until this launch finishes.
````

### B3404 · l.371–374 · REWRITE (confirm) · rule 1.3 · 4 → 3

````text
        // Unreachable while the template swaps Cancel for "Open Session" past
        // `launched`; kept as the correct fallback, since past Launched the run
        // itself is live, not a stage waiting on the lock.
````

### B3405 · l.379–382 · REWRITE (confirm) · rule 1.1 · 4 → 2

````text
        // Queued behind another operation: no `stageStarted` row exists yet,
        // but `stage://queued` already announced this run's id.
````

### B3406 · l.385–388 · REWRITE (confirm) · rule 1.3 · 4 → 2

````text
      // else: no run id has arrived and the run hasn't launched - nothing safe
      // to cancel; the button's `disabled` guard keeps this unreachable.
````

### B3407 · l.465–469 · REWRITE (amend) ·p · rule 1.3 · 5 → 4

````text
      <!-- `reason` (privilege.rs's `needs_admin_reason`) already names the
           mechanism picked and why - macOS authorization dialog or sudo in the
           launching terminal - so no static prompt text here: under sudo,
           reachable from `cargo tauri dev`, it would be wrong. -->
````

## `sabrage/ui/src/components/QuitDialog.svelte`

### B3408 · l.2–8 · REWRITE (amend) ·p · rule 1.2 · 7 → 6

````text
  // Three-way app-quit gate for a live session: stop and quit, keep running
  // and quit, or cancel. Mounted once at the app root (App.svelte); shown
  // when sessionStore.quitRequested (Rust side intercepts ExitRequested/
  // CloseRequested to prevent a live session dying with the process). Buttons
  // answer via sessionStore.resolveQuit; unanswered, ipc.ts gives up after
  // 20 s. Exit policy: sabrage/docs/design/critique.md.
````

## `sabrage/ui/src/components/Sidebar.svelte`

### NEW-1 · before l.2 · ADD (amend) ·p · rule 1.2 · 0 → 12

````text
  /*
    Navigation chrome: item list, active-screen highlight, and footer session
    line.  The parent owns `screen`; this component only reports clicks via
    `onNavigate`.

    Loads `bottlesStore` on mount because the sidebar stays mounted while
    screens swap, keeping the list warm before any bottle-showing screen
    appears.  Other screens still call `bottlesStore.load()` themselves.

    `PHASE_DOT` and `footerLabel` are exhaustive over `SessionStatus["phase"]`
    by type, so adding a phase in `ipc.ts` breaks this file at compile time.
  */
````

### B3409 · l.17 · REWRITE (confirm) · rule 1.7 · 1 → 1

````text
    /** Shows the attention dot on the Doctor nav item. */
````

## `sabrage/ui/src/components/StagesPanel.svelte`

### NEW-1 · before l.2 · ADD (amend) ·p · rule 1.2 · 0 → 4

````text
  // Owns selectedBottle and copiedStage; drives `stageStore` by opening a gate
  // (dry run or real) and reads `bottlesStore` for the picker, `sessionStore`
  // for liveness. Shares one GateModal and one disable rule with Doctor's
  // whole-stage Fix: `blocksMutation(session phase)` or `stageStore.running`.
````

### B3411 · l.51–56 · REWRITE (confirm) ·p · rule 1.5 · 6 → 2

````text
  /** `deny_stage_while_session_live` refuses Setup/Build/Install — dry runs
   * included — while a session is live, so both buttons are disabled here. */
````

### B3412 · l.58–65 · REWRITE (confirm) · rule 1.5 · 8 → 3

````text
  /** A stage started elsewhere keeps running after its gate dialog is Hidden:
   * `stageStore.gate` returns to `null` but the `runStage` invocation and its
   * lock hold do not, so the buttons stay disabled on `running`, not on `gate`. */
````

### B3413 · l.75–80 · REWRITE (confirm) ·p · rule 1.3 · 6 → 3

````text
    // Every demo.sh stage accepts `--bottle`, so the flag is shown whenever a
    // bottle is selected; `needsBottle` gates the Run button and the picker, not
    // the flag. A required-but-unselected bottle shows `<name>` so the command reads as a template.
````

### B3415 · l.103–107 · REWRITE (confirm) · rule 1.3 · 5 → 3

````text
    // The bottle selector belongs to the whole panel, so the selection is passed
    // for every card regardless of `card.needsBottle`: `setup` with a bottle runs
    // its Beat Saber presence check against that bottle.
````

## `sabrage/ui/src/components/StatusIcon.svelte`

Deleted (nothing carried): B3416

### B3417 · l.11–12 · REWRITE (amend) ·p · rule 1.7 · 2 → 1

````text
    /** Size in px for ok/warn/fail SVGs only; lock is fixed at 15px, empty/spinner/info are CSS-sized. */
````

## `sabrage/ui/src/ipc.ts`

Deleted (nothing carried): B3427, B3434, B3450, B3464, B3474, B3482, B3485, B3535

### B3419 · l.1–3 · REWRITE (confirm) · rule 1.2 · 3 → 3

````text
// Hand-mirrored IPC boundary between the Svelte frontend and sabrage-app's
// Tauri commands (`src-tauri/src/commands.rs`). No codegen: a shape change on
// either side has to be made by hand on both.
````

### B3422 · l.20–24 · REWRITE (confirm) · rule 1.7 · 5 → 5

````text
  /** Bare contract fix id (e.g. `"fix.set-graphics-backend"`), or `null` when
   * this check's remedy has none. Resolve to a `FixAction` with
   * `contractFixIdToAction` before offering a Fix button — `fix.create-z-drive`
   * is the one contract id deliberately left unmodelled and resolves to
   * `null`. */
````

### B3424 · l.35–37 · REWRITE (confirm) · rule 1.7 · 3 → 3

````text
/** Sidebar footer snapshot — mirrors `commands::AppState`. `defaultBottle`/
 * `defaultBsDir` come from `Settings`, so the Sidebar and Session screen can
 * prefill without a second `getSettings()` call. */
````

### B3428 · l.84–87 · REWRITE (confirm) · rule 1.7 · 4 → 4

````text
/** Mirrors `sabrage_core::Stage` (serde lowercase). `run_stage(stage: "run")`
 * works, but its promise does not resolve until the session ends — see
 * `launch()`'s own doc comment; the Session screen calls `launch`, not
 * `runStage`, for that reason. */
````

### B3442 · l.216–228 · REWRITE (amend) · rule 1.5 · 13 → 10

````text
/** Static metadata about each fix — hand-mirrors `sabrage_core::fixes::fix_defs()`
 * plus `FixAction::as_stage()`. Only `runInstall` needs admin; only
 * `deleteSessionJson` is destructive (and carries a `consequence` the confirm
 * dialog must show); `stage` names the whole-stage fixes whose intended UI
 * path is `runStage` directly (see `contractFixIdToAction`'s doc comment)
 * rather than `applyFix`.
 *
 * `fixes::apply` refuses every fix while a session is live, so disable the
 * buttons with `blocksMutation(status.phase)`
 * (`sabrage_core::fixes::tests::apply_refuses_every_session_forbidden_fix_while_a_session_is_live`). */
````

### B3444 · l.306–324 · REWRITE (confirm) · rule 1.7 · 19 → 15

````text
/** Contract fix ids Sabrage deliberately **withholds** — mirrors
 * `sabrage_core::fixes::DEFERRED_CONTRACT_FIX_IDS`, and the reason
 * `contractFixIdToAction` cannot just ask whether `FIX_META` has a key:
 *
 * - `fix.create-z-drive` — no `FixAction` models it at all (it is absent from
 *   `FIX_META` too, so this entry is belt-and-braces);
 * - `fix.delete-session-json` — modelled *and* documented here (the confirm
 *   dialog needs its title and `consequence` for the CLI-initiated path), but
 *   never offered as a button: deleting that file is known to leave the client
 *   at an 800x900 black screen, and editing the pinned IP on the Settings
 *   screen is the recovery that works.
 *
 * This set is the third of three doors: `run_doctor` also projects every row's
 * fix id through `FixAction::from_contract_id`, and the `fix` command refuses a
 * withheld action outright. */
````

### B3445 · l.330–334 · REWRITE (confirm) · rule 1.7 · 5 → 4

````text
/** `"fix.set-graphics-backend"` -> `"set-graphics-backend"`; `null` for a
 * contract id this table does not model **and** for one it models but does not
 * offer (`DEFERRED_FIX_IDS`) — mirrors `FixAction::from_contract_id` exactly.
 * A `null` result means "render no Fix button", not an error. */
````

### B3448 · l.372–391 · REWRITE (confirm) · rule 1.7 · 20 → 17

````text
/**
 * Stop the session — two cases (see `commands::stop_session`'s doc comment):
 *
 * - a session this Sabrage process is supervising (`launch()` still pending)
 *   is stopped by firing its own cancel token: `bottle` is ignored, `onEvent`
 *   receives nothing new (the pending `launch()` call's own `onEvent` carries
 *   every teardown row), and the resolved `StageOutcome` is synthetic
 *   (`{ stage: "run", ok: true, exitCodeEquiv: 130 }`, INT parity);
 * - otherwise the `stop` stage runs for `bottle`.
 *
 * `sessionStore.stop()` is the intended call site for both — it supplies
 * `bottle` from `sessionStore.status.bottle`.
 *
 * The live-session branch **rejects** when teardown did not actually finish
 * (it timed out, or the run detached instead of stopping); show that message,
 * or "Stopped" appears over a still-running game.
 */
````

### B3449 · l.401–417 · REWRITE (confirm) · rule 1.7 · 17 → 14

````text
/**
 * Apply one fix. Destructive fixes (`FIX_META[action].destructive`) must not
 * be called with `confirmed: false` — the backend refuses them (see
 * `commands::fix`'s doc comment); show an in-app confirm dialog first, never
 * `window.confirm` (it blocks the webview). An action `contractFixIdToAction`
 * withholds (`DEFERRED_FIX_IDS`) is refused outright, confirmed or not.
 *
 * **The first `onEvent` carries the run id.** `fixes::apply` emits its
 * `"applying fix '<action>'"` line before it waits for the operation lock, so
 * `ev.runId` from that first event is a live `cancelStage(runId)` handle for
 * the whole wait — which can last minutes behind another Sabrage operation and
 * ends with nothing touched. Capture it and enable a Cancel affordance; a call
 * site that ignores it leaves the user with no way out of a queued mutation.
 */
````

### B3452 · l.461–466 · REWRITE (confirm) · rule 1.7 · 6 → 6

````text
/** Is something running (or about to be) that a mutating action must not
 * disturb? The one definition of "a session is live" for the whole UI; no
 * screen derives its own phase set.
 *
 * `"exited"` is not live (the wine child is gone, the row is just the last
 * session's epitaph) and neither is `"idle"`. Everything else is. */
````

### B3460 · l.558–563 · REWRITE (confirm) · rule 1.7 · 6 → 6

````text
/** Mirrors `commands::LaunchOpts` (serde camelCase). Every field but the
 * bottle/bs-dir pair has no `demo.sh` counterpart at all outside `run.sh`
 * itself — see that struct's own doc comment. `gameId` is the library entry
 * this launch belongs to, when it came from the Library screen's Run button —
 * the backend uses it to record a `LastSession` after the run settles; omit it
 * for the plain Session screen's own launches. */
````

### B3463 · l.613–615 · REWRITE (amend) · rule 2.3 · 3 → 3

````text
/** Detach from the live session — the app-quit "leave it running" answer
 * (`sabrage/docs/design/critique.md`, "app-quit semantics for a live
 * session"). A no-op when nothing is live. */
````

### B3473 · l.688–693 · REWRITE (confirm) · rule 2.3 · 6 → 5

````text
/**
 * Reconcile whatever `session-state.json` says on disk against what is
 * actually running. Call at startup and again before showing the Launch
 * button. Request/response, not a stream — there is no `onEvent` here.
 */
````

### B3484 · l.764–775 · REWRITE (confirm) · rule 1.5 · 12 → 9

````text
/** Resolve the pending quit-while-live dialog (`onQuitRequested`). `"stop"`/
 * `"keep"` exit the app from the Rust side — only `"cancel"` reliably returns
 * control to the caller.
 *
 * `"stop"` rejects (while still exiting) when teardown did not finish inside
 * its 30 s budget: Sabrage detaches instead — guards disarmed, the record
 * marked detached, the game left running. An unanswered dialog is given up on
 * after 20 s and the next quit request exits through the keep-running path, so
 * a webview that died before subscribing cannot make Sabrage unquittable. */
````

### B3488 · l.794–809 · REWRITE (confirm) · rule 1.7 · 16 → 14

````text
/**
 * Subscribe to `stage://queued` — fired when a `runStage`/`launch`/
 * `stopSession` call finds another operation already in flight and has to
 * wait for it.
 *
 * Core emits `stageStarted` before it takes the operation lock, so a queued
 * run already has an id Cancel can use; this event additionally names the wait
 * *as* a wait (the common case is queueing behind a `sabrage` CLI build in
 * another process, which nothing on the stage channel would mention).
 * Cancelling on this id works — the lock wait itself is cancellable, and the
 * executor fails every filesystem primitive once the token fires. Treat it
 * exactly like `stageStarted`'s `runId` (and expect `stageStarted` for the
 * same run).
 */
````

### B3491 · l.836–841 · REWRITE (amend) · rule 1.4 · 6 → 3

````text
// Sabrage edits `~/Library/Application Support/OXRSys/oxrsys-runtime.toml` in
// place — the deliberate divergence from demo.sh's write-once treatment of that
// file (`sabrage/docs/design/design-app.md` §4, "Settings write policy").
````

### B3505 · l.919–935 · REWRITE (confirm) ·p · rule 1.5 · 17 → 15

````text
/**
 * Apply `patch` to `oxrsys-runtime.toml` — creates the file byte-identical to
 * the shared template if it doesn't exist, otherwise snapshots a backup and
 * edits in place, preserving every other byte. Rejects when the file has a
 * `parseError` (fix by hand first), on a validation/IPC failure, and **while
 * a session is live** (`./demo.sh stop --bottle <name>`). A patch that changes
 * nothing writes no backup and no file.
 *
 * The runtime re-reads this file every 250 ms and rebuilds the encoder when
 * `encoderProcess`/`videoCodec` move, so a save mid-stream is a live
 * reconfiguration — disable Save while
 * `blocksMutation(sessionStore.status.phase)`. Also serializes against
 * setup/build/install and the `edit-protocol` fix (the process-wide operation
 * lock), so a save can block briefly.
 */
````

### B3506 · l.940–943 · REWRITE (confirm) · rule 1.4 · 4 → 1

````text
// Settings are persisted at `<Sabrage appsup>/settings.json`.
````

### B3507 · l.945–955 · REWRITE (confirm) · rule 1.7 · 11 → 9

````text
/** Mirrors `store::settings::LaunchDefaults` (serde camelCase) — the four
 * `run.sh`-only flags (see `LaunchOpts`'s doc comment), as app-wide defaults
 * rather than one launch's overrides.
 *
 * The index signature is the wire half of `LaunchDefaults`'s own
 * `#[serde(flatten)] extra` map, for the same reason `Settings` has one a
 * level up. Spread the loaded object (`{ ...settings.launch, wired: true }`)
 * rather than rebuilding it field by field, or a newer Sabrage's keys are
 * stripped before they reach the backend. */
````

### B3509 · l.966–973 · REWRITE (confirm) · rule 1.7 · 8 → 7

````text
/** Mirrors `store::settings::Settings` (serde camelCase).
 *
 * The index signature is the wire half of the store's `#[serde(flatten)]
 * extra` map: a newer Sabrage's keys are read and written back verbatim. Keep
 * spreading the loaded object (`{ ...settings, ...patch }`) rather than
 * rebuilding one field by field, or the extras are dropped before they reach
 * the backend. */
````

### B3515 · l.998–1000 · REWRITE (confirm) · rule 1.7 · 3 → 3

````text
/** Persist `settings` and return it back as saved (same shape, including any
 * unknown top-level keys). Rejects when `$HOME` is missing/empty/non-absolute
 * rather than writing the store somewhere else. */
````

### B3520 · l.1034–1037 · REWRITE (confirm) · rule 1.4 · 4 → 1

````text
// The library is persisted at `<Sabrage appsup>/library.json`.
````

### B3534 · l.1157–1172 · REWRITE (amend) ·p · rule 1.5 · 16 → 16

````text
/**
 * Restore `steam_api64.dll.orig-steam` back over `steam_api64.dll` for the
 * library entry named `gameId`. A no-op (`restored: false`, explanatory
 * `message`) when there is no `.orig-steam` to restore from, or when the
 * backup is itself the pinned Goldberg dll — the message says "the .orig-steam
 * backup", never "the original", because nothing can prove it is. The next
 * launch re-applies Goldberg regardless, and `message` says so. Restoring at
 * all is a divergence — run.sh never does (PARITY.md § Planned for later
 * phases (declared now), "Revert-original-`steam_api64.dll` action"). Rejects
 * while a session is live.
 *
 * `expectedBsDir` is the Beat Saber directory the screen showed and validated.
 * The command mutates the entry's **saved** `bsDir`, so an unsaved path edit
 * would target a different installation; pass it and the backend fails closed
 * on a mismatch. Optional for callers that render no path.
 */
````

### B3536 · l.1185–1191 · REWRITE (amend) · rule 2.2 · 7 → 5

````text
/**
 * Open a native "choose a folder" dialog (`@tauri-apps/plugin-dialog`, the
 * `dialog:allow-open` capability in `capabilities/main.json`). Resolves `null`
 * on cancel.
 */
````

## `sabrage/ui/src/lib/demo.ts`

### B3539 · l.1–6 · REWRITE (amend) ·p · rule 1.7 · 6 → 4

````text
// `./demo.sh …` command lines for display. `demoRunCommand` builds the `run`
// equivalent of a `LaunchOpts` — the "equivalent demo.sh command" footer on the
// Session and Settings screens (design-app.md §4); both call it, so identical
// options yield byte-identical text. `shQuote` is also used by StagesPanel.
````

### B3540 · l.10–23 · REWRITE (confirm) ·p · rule 1.2 · 14 → 6

````text
/**
 * Quote `v` for use in a copy-pasted zsh command line. Single-quotes the value
 * with embedded apostrophes escaped, so no shell metacharacter is interpreted;
 * values already safe bare and the literal `<name>` placeholder are returned
 * unquoted.
 */
````

## `sabrage/ui/src/lib/text.ts`

### B3541 · l.1–4 · REWRITE (amend) · rule 1.7 · 4 → 1

````text
// Small text-formatting helpers shared across screens, components, and stores.
````

### B3544 · l.19–20 · REWRITE (confirm) · rule 1.2 · 2 → 1

````text
/** The message text for a caught value that may or may not be an `Error`. */
````

## `sabrage/ui/src/screens/About.svelte`

### B3545 · l.10 · REWRITE (confirm) ·p · rule 1.4 · 1 → 1

````text
  // Chip labels copied verbatim from pipeChips in sabrage/docs/design/mockup/Sabrage.dc.html.
````

### B3549 · l.29–31 · REWRITE (amend) ·p · rule 1.3 · 3 → 3

````text
  // Credits upstream projects; dingyifei forks appear only in descriptions. Authorship
  // and licenses follow sabrage/docs/design/mockup/Sabrage.dc.html, except oxrsys's
  // MPL-2.0, which comes from CLAUDE.md's ext/oxrsys rules.
````

## `sabrage/ui/src/screens/Doctor.svelte`

Deleted (nothing carried): B3556, B3565, B3566

### NEW-1 · before l.2 · ADD (amend) ·p · rule 1.2 · 0 → 4

````text
  // The Doctor screen — drives `doctorStore` (every check pass goes through
  // `runChecks()`) and owns bottle selection plus Fix-in-flight state.
  // Whole-stage fixes share StagesPanel's single GateModal
  // (`stageStore.openGate`); `sessionStore` blocks every mutation while live.
````

### B3551 · l.23–29 · REWRITE (confirm) · rule 1.2 · 7 → 4

````text
    /** Bumped by App.svelte on every Pipeline ▸ Run Doctor (⌘D) firing;
     * mirrors Session's `launchRequest`. Each new value forces a fresh pass,
     * even when Doctor is already the open screen or its last run is still
     * within `AUTORUN_STALE_MS`. */
````

### B3553 · l.51–62 · REWRITE (confirm) ·p · rule 1.2 · 12 → 5

````text
  /**
   * Per-slug notice from the last fix run. Keyed so it survives the
   * `runChecks()` repaint `runFix` fires in its `finally`: a fix can emit a
   * `warn` or resolve `changed: false` while the check still reports the same
   * status. Cleared when a new run starts (`dismissFixNotice`) or on dismiss. */
````

### B3554 · l.65–69 · REWRITE (confirm) · rule 1.2 · 5 → 4

````text
  /** Drops slug's notice, if any. Called by the row's Dismiss control and by
   * `runFix` before a new attempt, so a notice from a previous attempt never
   * survives into a fresh one; it re-appears only if the new run warns or
   * resolves unchanged. */
````

### B3555 · l.78–83 · REWRITE (confirm) · rule 1.2 · 6 → 4

````text
  /** True once this mount's `onMount` has made its freshness-based autorun
   * decision. The request-replay effect below waits on it so it cannot read
   * `doctorStore.running` or start a pass before that decision is made
   * (mirrors Session's `bottlePrefillDone`). */
````

### B3557 · l.102–105 · REWRITE (confirm) · rule 1.3 · 4 → 3

````text
    // Set only after the decision above: `runChecks()`, if it fired, already
    // set `doctorStore.running` synchronously before its first await, so the
    // request-replay effect can never start a second concurrent pass.
````

### B3558 · l.109–114 · REWRITE (confirm) · rule 1.3 · 6 → 3

````text
  // A menu-triggered Run Doctor (⌘D) always forces a fresh pass, bypassing
  // `AUTORUN_STALE_MS` — including when Doctor is already the open screen, where
  // nothing remounts to re-run `onMount`'s pass. Waits on `doctorAutorunDecided`.
````

### B3559 · l.128–132 · REWRITE (confirm) · rule 1.2 · 5 → 3

````text
  /** May Doctor mutate the machine right now? The backend refuses every Fix
   * while a session is live (`fixes::apply` -> `deny_if_session_live`); this
   * only disables the button early so the user learns why before clicking. */
````

### B3560 · l.135–140 · REWRITE (amend) ·p · rule 1.2 · 6 → 4

````text
  /** Whether a pipeline stage is already running — `stageStore.running`, not
   * `stageStore.gate`, which reads `null` once the modal is Hidden. Whole-stage
   * fixes share that one GateModal (`stageStore.openGate`), so a second would
   * queue behind the first and display mislabelled when the modal reopens. */
````

### B3561 · l.147–150 · REWRITE (confirm) · rule 1.2 · 4 → 2

````text
  /** Handles a CheckRow's Fix button: resolves the contract fix id to a
   * `FixAction` and dispatches it. An unrecognised id is ignored. */
````

### B3562 · l.157–160 · REWRITE (confirm) · rule 1.2 · 4 → 3

````text
  /** Dispatches an already-resolved `FixAction` — shared with the error
   * banner's retry button, whose action comes off a `Fatal` event rather than
   * a contract id string. */
````

### B3563 · l.209–213 · REWRITE (confirm) ·p · rule 1.3 · 5 → 3

````text
    // A failing fix emits a `Fatal` event on this stream (message + remedy +
    // follow-up fix id) before `applyFix` rejects; the catch below reports
    // that, not the bare rejection message.
````

### B3564 · l.215–218 · REWRITE (confirm) · rule 1.3 · 4 → 3

````text
    // A fix can warn without failing outright, and the check it is meant to
    // fix may keep reporting the same status, so these warnings are the only
    // signal that something is still wrong.
````

### B3567 · l.242–247 · REWRITE (confirm) · rule 1.3 · 6 → 3

````text
      // TypeScript narrows `fatal` to the initializer's `null` here — it cannot
      // prove the callback above ran — so the cast restores the declared union;
      // the truthiness check below still does the branching.
````

### B3568 · l.274–277 · REWRITE (confirm) · rule 1.3 · 4 → 3

````text
    // A rerun that rejected before reporting every slug must not keep reading
    // "Running checks…": that text otherwise survives once `running` flips
    // false, with nothing else in the header saying doctor failed.
````

### B3569 · l.466–469 · REWRITE (confirm) · rule 1.3 · 4 → 3

````text
  /* Advisory tone, deliberately distinct from `.fix-error-banner`'s Fatal
     treatment below: a fix notice is not a failed fix, so it matches the
     `warn` StatusIcon rather than the error banner's accent color. */
````

## `sabrage/ui/src/screens/EditGame.svelte`

Deleted (nothing carried): B3572, B3575, B3579, B3581, B3585

### B3570 · l.2–8 · REWRITE (amend) ·p · rule 1.2 · 7 → 7

````text
  // Add/edit one library entry: Identity & paths (editable, debounced
  // `validateGame`), Patches and Streaming (read-only — global runtime values
  // live in Settings), and per-flag launch overrides. Owns a deep-cloned draft
  // and writes `libraryStore` only on Save; `gameId === null` is the "Add game"
  // path (from `newGameTemplate()`), otherwise edits a saved entry in place.
  // `isLivePhase` (shared with Session.svelte) gates Revert; Save is never
  // phase-gated.
````

### B3571 · l.33–36 · REWRITE (confirm) · rule 1.2 · 4 → 2

````text
    /** `null` = Add game (unsaved, from `newGameTemplate()`); otherwise the
     * id of the saved entry to edit. */
````

### B3574 · l.58–62 · REWRITE (confirm) · rule 1.2 · 5 → 3

````text
  /** `entry.bsDir` as persisted — Revert targets the *saved* row, not this
   * unsaved draft (see `doRevert`'s `expectedBsDir`, which the backend fails
   * closed against). `null` on the Add-game path, where Revert never renders. */
````

### B3577 · l.124–126 · REWRITE (confirm) · rule 1.5 · 3 → 3

````text
    // Disarm on teardown: Cancel/Save/"Open Settings" within 300 ms of the
    // last keystroke would otherwise validate for a destroyed component and
    // write `$state` on it (timing-only; the UI has no test harness).
````

### B3578 · l.136–140 · REWRITE (amend) ·p · rule 2.3 · 5 → 5

````text
      // `pickFolder` rejects on a missing dialog capability or failed panel
      // (a cancel resolves `null`, absorbed by the `if (picked)` below), so
      // an unhandled rejection would leave Browse… as a dead button.
      // Start in the field's own dir, else the bottle's derived Beat Saber
      // path (nearest existing ancestor), never wherever macOS last was.
````

### B3580 · l.164–168 · REWRITE (amend) · rule 1.4 · 5 → 3

````text
  // Seed the four selects once, the moment `entry` first resolves. The
  // `seeded` latch also gates the write-back effect below, which would
  // otherwise stamp four `"global"` defaults over `entry.launchOverrides`.
````

### B3582 · l.190–195 · REWRITE (amend) · rule 1.5 · 6 → 3

````text
  // `phase` stays `"exited"` after a session ends until the next launch, so a
  // `!== "idle"` gate would disable Revert forever after the very run that
  // creates the `.orig-steam` backup it restores; `isLivePhase` excludes it.
````

### B3584 · l.214–218 · REWRITE (confirm) · rule 1.3 · 5 → 3

````text
      // Pass the draft path this form validated and displayed as
      // `expectedBsDir`: the backend fails closed when it differs from the
      // persisted row's `bsDir`, rather than reverting an install not on screen.
````

## `sabrage/ui/src/screens/Library.svelte`

Deleted (nothing carried): B3588, B3594

### B3587 · l.31–38 · REWRITE (amend) ·p · rule 2.3 · 8 → 6

````text
  // Goldberg is staged during run, well before `sessionStore.launch` settles
  // (that promise resolves only when the session ends), so a row's
  // `validity.goldberg`/status would stay at their pre-launch snapshot for the
  // whole session. `launchedAt` (session.svelte.ts) is the launch-local
  // "game is up" signal, set after Goldberg staging; this refresh does not
  // replace `runGame`'s settlement refresh, which updates `lastSession`.
````

### B3589 · l.76–80 · REWRITE (confirm) · rule 1.4 · 5 → 3

````text
  // `effectiveLaunchOpts` mirrors `store::library::effective_options` on the
  // Rust side (per field, `override ?? global default`); the client, not the
  // backend, is the source of truth for the `LaunchOpts` flags sent over IPC.
````

### B3590 · l.82–100 · REWRITE (amend) · rule 1.7 · 19 → 11

````text
  // `isLivePhase` excludes `"exited"`: the backend leaves that phase published
  // until the next launch, so a bare `phase !== "idle"` would hold every Run
  // button disabled for the rest of the app's life after one session ended.
  //
  // `!settingsStore.loadOk` holds Run disabled until settings.json loads: the
  // `?? false` fallback in `effectiveLaunchOpts` is right only for the backend's
  // "no settings.json yet" default — a corrupt file would launch wrong flags.
  //
  // `stageStore.running` covers a Hidden stage (the guard StagesPanel and
  // Doctor carry too): Hide clears `gate`, not the `runStage` behind it, and
  // GateModal won't adopt a second `openGate` — nothing is left to cancel it.
````

### B3591 · l.125–134 · REWRITE (amend) ·p · rule 1.3 · 10 → 6

````text
    // Both calls fire together, same as Session.svelte's `doLaunch`: the store
    // owns the launch (and the rows GateModal reads), the gate is this launch's
    // window and renders any failure.
    // `launch` settles only after the backend records this run's `lastSession`
    // (when there is one), so refreshing on either settlement is the earliest
    // the row can show the current run in Last session.
````

### B3592 · l.143–145 · REWRITE (amend) · rule 1.3 · 3 → 3

````text
    // `"exited"` stays published until the next launch, so a bare `!== "idle"`
    // here would leave every row sharing the bottle reading "Running" long
    // after the game closed.
````

### B3593 · l.147–151 · REWRITE (confirm) · rule 1.3 · 5 → 4

````text
    // A session this process launched remembers its Library entry, so two
    // entries sharing a bottle don't both read "Running". `SessionStatus`
    // carries no gameId, so a session Sabrage did not start (external or
    // re-attached) falls back to the bottle match.
````

## `sabrage/ui/src/screens/Logs.svelte`

### NEW-1 · before l.2 · ADD (confirm) ·p · rule 1.2 · 0 → 5

````text
  /** Owns the Logs screen: selected tab, live tail buffer (`lines` and its
   * lowercased twin `lowerLines`), filter, and past-runs listing. Holds the
   * tail id and talks to the backend over log-tail IPC directly, so every
   * navigation and unmount must stop the current tail before starting
   * another. */
````

### B3595 · l.24–27 · REWRITE (amend) · rule 1.3 · 4 → 3

````text
  /** Hysteresis over `MAX_LINES`: a trim re-slices both `lines` and
   * `lowerLines`, so it runs about once per 1000 lines instead of on every
   * batch once the buffer sits at cap. */
````

### B3596 · l.29–32 · REWRITE (amend) · rule 1.7 · 4 → 3

````text
  /** Debounce before `filterQuery` — and so `filteredLines` — follows a
   * keystroke: a non-empty query rescans the whole buffer, a cost every
   * incoming batch already pays again while a filter is set. */
````

### B3597 · l.36–40 · REWRITE (amend) · rule 1.5 · 5 → 3

````text
  /** The last tab a navigation was *requested* for. `switchTab`'s
   * "already there, no-op" guard reads this and not `tab`, which updates only
   * once the in-flight `stopTail()` resolves; a rapid A -> B -> A would land on B. */
````

### B3601 · l.57–61 · REWRITE (confirm) · rule 1.5 · 5 → 3

````text
  /** Guards `switchTab`/`openPastRun`/`backToPastRuns`: a `stopTail()` await
   * that settles after a later navigation superseded it must not assign
   * `tab`/`openedPastRun` or start a tail for the navigation already left. */
````

### B3602 · l.63–73 · REWRITE (amend) · rule 1.5 · 11 → 3

````text
  /** Guards a leaked tail: `startTail` captures this at its start and skips
   * every shared-state write it would make, `tailId` included, once a later
   * `startTail`/`stopTail` bumped it. Only the id in `tailId` is ever stopped. */
````

### B3603 · l.123–126 · REWRITE (confirm) · rule 1.3 · 4 → 3

````text
    // Generation-check this call's batches too: one landing between
    // `startLogTail` resolving and the stale-case `stopLogTail` below would
    // otherwise mix into whatever tab is current by then.
````

### B3605 · l.155–158 · REWRITE (confirm) · rule 1.3 · 4 → 3

````text
    // Bump first, unconditionally: this invalidates a `startTail` still in
    // flight even before it reaches `startLogTail`, and even when no
    // replacement `startTail` follows (the tail-less "Past runs" tab).
````

### B3607 · l.181–187 · REWRITE (confirm) · rule 1.1 · 7 → 2

````text
    // Past runs is the one tab worth re-clicking: it refreshes the listing.
    // Compares `pendingTab`, not `tab`, which lags (see `pendingTab`'s doc).
````

### B3608 · l.192–194 · REWRITE (confirm) · rule 1.1 · 3 → 2

````text
    // A later navigation superseded this one while `stopTail()` was in
    // flight; let its own continuation own `tab`/`openedPastRun` and the tail.
````

## `sabrage/ui/src/screens/Session.svelte`

Deleted (nothing carried): B3610, B3622, B3624, B3625, B3629, B3630

### NEW-1 · before l.2 · ADD (amend) ·p · rule 1.2 · 0 → 6

````text
  // Session screen. Owns launch form state (bottle, Beat Saber directory, four
  // demo.sh toggles — prefilled at mount from `AppState` and `settingsStore`)
  // and drives `sessionStore`: launch, stop, detach, and the mount-time
  // reconcile whose rows become the banner. The stage-gate dialog belongs to
  // `stageStore`; "is a session live" is `isLivePhase` / `canStop` from
  // `ipc.ts`, shared with Library.svelte.
````

### B3609 · l.21–27 · REWRITE (amend) ·p · rule 1.2 · 7 → 5

````text
    /** Bumped by App.svelte on Pipeline ▸ Launch (⌘R). A change from the
     * last-handled value calls `doLaunch(false)` — the same function the
     * Launch button calls — once this screen's mount-time prefill has settled;
     * if a launch or stage is already running or no bottle is selected, a
     * notice is shown instead. `0` never triggers. */
````

### B3611 · l.44–51 · REWRITE (amend) ·p · rule 1.7 · 8 → 4

````text
  /** True when the launch form was prefilled at mount: a bottle or Beat Saber
   * directory from `AppState`, or at least one `settingsStore` toggle on.
   * Drives the "defaults from Settings" hint, hidden on a fresh install where
   * only hardcoded fallbacks apply. */
````

### B3612 · l.53–61 · REWRITE (amend) · rule 1.7 · 9 → 5

````text
  /** True once THIS mount has finished prefilling `selectedBottle`. The
   * menu-launch effect gates on this, never on the module-global
   * `bottlesStore.bottlesLoaded` — already `true` when ⌘R arrives from
   * another screen, so it would read `selectedBottle` before this mount
   * assigns it and show a false "No bottle selected". */
````

### B3618 · l.125–139 · REWRITE (amend) ·p · rule 1.7 · 15 → 11

````text
  // `isLivePhase` (ipc.ts) defines "session is live" for the whole UI, shared
  // with Library.svelte: every phase but `idle` and `exited`, `external`
  // included — a session with no Sabrage-owned handle, only a fresh
  // runtime_status.json naming a live pid. No second Launch may run over them.
  //
  // `stageStore.running` is the other half of `gate !== null`: Hide closes the
  // dialog without stopping its stage, so a Launch over a hidden-but-running
  // install would queue a second `openGate` that GateModal refuses to adopt —
  // the modal reopens on the install's title, rows and Cancel target while
  // this launch runs behind the operation lock with no feedback. Refuse up
  // front, the same rule StagesPanel and Doctor apply.
````

### B3619 · l.144–146 · REWRITE (amend) · rule 1.7 · 3 → 3

````text
  /** The `./demo.sh run …` command equivalent to the options selected here,
   * built by `demoRunCommand` (lib/demo.ts) so every screen that shows this
   * line renders the byte-identical string for the same options. */
````

### B3626 · l.235–241 · REWRITE (amend) · rule 1.2 · 7 → 5

````text
  /** The Stop stage's own lines, shown as a small local list next to the
   * button. Only populated for a session this process does *not* supervise,
   * where `stop_session` runs the real `Stop` stage; a supervised session's
   * teardown rows arrive on `sessionStore.launchRows` instead, rendered by
   * `sessionLogTail` below. */
````

### B3628 · l.259–266 · REWRITE (confirm) · rule 1.7 · 8 → 4

````text
  /** Trailing lines of the launch invocation's own rows, for a session this
   * process supervises: `commands::stop_session` only fires the cancel token
   * there and streams nothing on `on_event`, because the still-pending
   * `launch()` call emits every teardown row onto `sessionStore.launchRows`. */
````

### B3631 · l.358–362 · REWRITE (amend) · rule 1.7 · 5 → 4

````text
  // `canStop` (ipc.ts) is `isLivePhase`, the same predicate `busy` uses above:
  // `external` counts — a session Sabrage did not start, which the recordless
  // `stop` stage can still stop. Sharing the predicate is what keeps this
  // screen in sync with a new `SessionPhase`.
````

## `sabrage/ui/src/screens/Settings.svelte`

Deleted (nothing carried): B3634, B3635, B3642, B3646, B3648, B3650

### B3632 · l.2–16 · REWRITE (confirm) ·p · rule 1.2 · 15 → 12

````text
  // Two independent persistence stores back this screen:
  //  - `configStore` (oxrsys-runtime.toml, via config/runtime_toml.rs) — Streaming
  //    card only. A local `draft` is diffed against the loaded view so Save writes
  //    only changed keys (`null` = untouched, never sent — see `buildPatch`);
  //    explicit Save/Revert, gated behind a one-time inline acknowledgement panel
  //    (never `window.confirm` — it freezes the webview).
  //  - `settingsStore` (settings.json) — Audio & launch, Paths, adb-probe toggle.
  //    Every field autosaves on change/blur (Session.svelte's
  //    local-`$state`-then-store convention) with a transient "Saved" flash
  //    instead of a form-wide save button.
  // Reference: sabrage/docs/design/design-app.md, "Settings write policy for
  // `oxrsys-runtime.toml`".
````

### B3633 · l.43–48 · REWRITE (confirm) · rule 1.4 · 6 → 4

````text
  // `write_runtime_config` fails closed while a session is live rather than
  // deferring to "next launch" (see that IPC fn's doc comment), so Save is
  // disabled proactively with an honest reason instead of surfacing the
  // backend's refusal only after a click.
````

### B3637 · l.98–105 · REWRITE (amend) · rule 1.3 · 8 → 4

````text
    // Only the last write queued for a burst reseeds the controls, on success
    // as well as on failure: an earlier write settling while a newer one is
    // still queued would reseed from a store that does not yet reflect it (see
    // settingsStore.update). `writeSeq` is bumped synchronously at queue time.
````

### B3639 · l.121–124 · REWRITE (confirm) · rule 1.3 · 4 → 3

````text
    // A function, not a snapshot: it resolves inside settingsStore.update's
    // queued step, so it composes with whatever `settings.launch` is then
    // instead of carrying an earlier queued write's rolled-back value.
````

## `sabrage/ui/src/stores/bottles.svelte.ts`

### B3651 · l.1–12 · REWRITE (amend) ·p · rule 1.7 · 12 → 7

````text
// `AppState` from `get_app_state` (one Tauri round trip), shared by Session,
// Settings, EditGame, StagesPanel, and Sidebar (for `alvrVersion`). Doctor is
// not a consumer — doctorStore fetches its own copy. Module-scoped Svelte 5
// rune store, same shape as doctor.svelte.ts and session.svelte.ts.
//
// `load()` re-fetches every call (not a cross-screen cache); concurrent callers
// within the same tick share one in-flight request.
````

## `sabrage/ui/src/stores/config.svelte.ts`

### B3655 · l.20–22 · REWRITE (amend) ·p · rule 2.3 · 3 → 3

````text
  /** Fetch the current `RuntimeConfigView` into `view`; never rejects.
   * A missing `oxrsys-runtime.toml` yields `view.exists` `false` with every value `null`.
   * An IPC-layer failure leaves `view` untouched and sets `error`. */
````

### B3656 · l.35–44 · REWRITE (amend) ·p · rule 1.2 · 10 → 7

````text
  /**
   * Apply `patch`, re-load `view`, and return the backend's `WriteReport`.
   * Re-loading keeps the backend the source of truth: a write can create the
   * file from template, resolve `shadowed` occurrences, or change `modifiedUnixMs`.
   * Rejects and sets `error` on a `parseError`, failed validation, or a live
   * session; the Settings screen validates client-side first, so this is the backstop.
   */
````

## `sabrage/ui/src/stores/doctor.svelte.ts`

### B3658 · l.1–6 · REWRITE (confirm) ·p · rule 1.7 · 6 → 2

````text
// Doctor screen state: written by Doctor.svelte, read by the App shell for the
// sidebar failure badge. Module-scoped `$state`, no event bus, no persistence.
````

### B3660 · l.13–20 · REWRITE (confirm) ·p · rule 1.3 · 8 → 6

````text
  /**
   * `"waiting"` — seen on a previous run but not yet reported by the current
   * one; the first waiting row stands in for "currently running" because the
   * backend streams resolved outcomes with no per-check start event.
   * `"done"` — the current run reported this slug.
   */
````

### B3662 · l.36–37 · REWRITE (confirm) · rule 1.7 · 2 → 2

````text
  /** `settings.json`'s `defaultBottle` as reported by `get_app_state` — the
   * Doctor screen's first choice before its hardcoded "Steam" fallback. */
````

### B3665 · l.84–88 · REWRITE (confirm) · rule 1.3 · 5 → 3

````text
      // A rerun that rejects before reporting every slug must not leave the
      // previous run's rows dim forever with no explanation; rows this run did
      // report before rejecting stay.
````

### B3666 · l.125 · REWRITE (amend) · rule 2.3 · 1 → 2

````text
    /** FAIL count of the current summary — 0 whenever there is none (before the
     * first run, during a run, and after a rejected one). Drives the sidebar badge. */
````

## `sabrage/ui/src/stores/library.svelte.ts`

### B3670 · l.36–42 · REWRITE (confirm) ·p · rule 1.2 · 7 → 5

````text
  /**
   * Upsert `entry` by `entry.id` and return the stored row; an existing row
   * stays in place, a new one is appended. Leaves `loading`/`error`
   * untouched — EditGame renders its own inline failure.
   */
````

## `sabrage/ui/src/stores/session.svelte.ts`

Deleted (nothing carried): B3681, B3687

### B3675 · l.33–38 · REWRITE (confirm) ·p · rule 1.4 · 6 → 4

````text
  // Launch state lives here because both GateModal and the Session screen
  // observe the same in-flight launch: GateModal renders `launchRows` live,
  // the Session screen reads `launching`, and Retry calls `launch` from
  // either component.
````

### B3676 · l.42–47 · REWRITE (amend) · rule 1.2 · 6 → 4

````text
  /** Wall-clock timestamp of this launch's `"launched"` event; null from the
   * start of `launch()` until one arrives. The launch-local twin of
   * `status.startedAtUnixMs`, available before the `session://status`
   * broadcast catches up. */
````

### B3677 · l.49–54 · REWRITE (confirm) · rule 1.3 · 6 → 3

````text
  /** The `"launched"`/`"fatal"`/`"stageStarted"` rows of this launch, captured
   * as they arrive so consumers read an O(1) field instead of re-scanning
   * `launchRows` on every event. */
````

### B3678 · l.59–64 · REWRITE (confirm) · rule 1.2 · 6 → 4

````text
  /** Set only on an `invoke()` rejection — an IPC-layer failure rather than a
   * reported `"fatal"` row, which appears in `launchRows`. Consumers that
   * render `launchRows` check for a `"fatal"` row first and use this as the
   * fallback. */
````

### B3679 · l.66–74 · REWRITE (confirm) ·p · rule 1.2 · 9 → 4

````text
  /** The `gameId` given to this store's most recent `launch()`, or null when
   * omitted. `SessionStatus` has no `gameId`, so this is the only way to
   * match a session to a Library entry without bottle-name equality, which
   * conflates entries sharing a bottle. Not cleared when the session ends. */
````

### B3680 · l.77–81 · REWRITE (confirm) · rule 1.2 · 5 → 4

````text
  /** The one place `status` is assigned. Clears `launchedGameId` when a
   * session turns live with no `launch()` of ours in flight: that session was
   * started elsewhere, so a stale id would pin "Running" on the wrong Library
   * row. */
````

### B3682 · l.123–148 · REWRITE (amend) ·p · rule 1.5 · 26 → 7

````text
  /**
   * Stop whatever session `status` is showing.
   *
   * During `"preflight"`/`"launching"` with a `runId`, `stopSession` would
   * block on `run_stage`'s `OPERATION_LOCK`; `cancelStage(runId)` bypasses it.
   * See `sabrage_core::session::tests::stop_plan_decides_from_the_status_alone`.
   */
````

### B3683 · l.157–163 · REWRITE (confirm) · rule 1.2 · 7 → 3

````text
  /** Detach from the live session, leaving it running. Refreshes `status`
   * before returning so the next render shows `SessionPhase::Detached`
   * without waiting for the 1 Hz `session://status` tick. */
````

### B3684 · l.169–171 · REWRITE (confirm) · rule 1.2 · 3 → 2

````text
  /** Re-poll `status` outside the 1 Hz broadcast. Best-effort: a failure
   * leaves the previous `status` in place. */
````

### B3685 · l.176 · REWRITE (confirm) · rule 1.3 · 1 → 1

````text
      // best-effort: keep the previous status.
````

### B3686 · l.180–182 · REWRITE (confirm) · rule 1.4 · 3 → 1

````text
  // `report.kind` is not kept: only `rows` (the banner text) has a consumer.
````

### B3688 · l.200–202 · REWRITE (amend) · rule 1.3 · 3 → 2

````text
      // Matters only for "cancel"; for "stop"/"keep" the Rust side normally
      // exits the app before this line runs.
````

### B3689 · l.207–211 · REWRITE (confirm) · rule 1.4 · 5 → 3

````text
  // Seed with one poll, then let the 1 Hz broadcast take over. A seed failure
  // (no repo root resolved yet) leaves the idle default in place; the
  // broadcast corrects it once a root resolves.
````

### B3690 · l.237–238 · REWRITE (confirm) · rule 1.2 · 2 → 1

````text
    /** This launch's own `"launched"` row, once it arrived — O(1). */
````

## `sabrage/ui/src/stores/settings.svelte.ts`

### B3694 · l.1–5 · REWRITE (confirm) · rule 1.2 · 5 → 3

````text
// App-wide Settings state (`~/Library/Application Support/Sabrage/settings.json`),
// shared between the Settings screen (the writer) and every screen that reads
// a default (Sidebar's bottle prefill, Session's/Library's launch defaults).
````

### B3695 · l.20–26 · REWRITE (amend) ·p · rule 1.2 · 7 → 4

````text
  /** Write counter, bumped synchronously when `save`/`update` queues a write,
   * so it already reflects the caller's own write on return. If `writeSeq`
   * exceeds `capturedBefore + 1` once a write settles, a later write
   * superseded it and the result is stale. */
````

### B3696 · l.29–35 · REWRITE (amend) · rule 1.2 · 7 → 5

````text
  /** Serializes every write (`save`/`update`) onto one chain, so overlapping
   * autosaves settle in call order: at most one `saveSettings` round-trip is
   * ever in flight, and each queued write merges against the result of the one
   * before it. A queued step's rejection reaches its own caller without
   * breaking the chain for writes behind it. */
````

### B3697 · l.47–53 · REWRITE (confirm) · rule 1.2 · 7 → 5

````text
  /** Fetch `Settings` from disk. A missing file resolves to field defaults,
   * not an error. A corrupt file or IPC failure quarantines the store —
   * `settings` cleared, `loadOk` false — so controls gated on `loadOk` disable
   * rather than autosave over a value nothing has verified; a later successful
   * `load()`/`save()` re-arms them. */
````

### B3698 · l.69–79 · REWRITE (confirm) ·p · rule 1.4 · 11 → 8

````text
  /**
   * Persist `next` optimistically: `settings` updates before the round-trip
   * resolves, so bound controls never snap back. On rejection `error` is set
   * and the store re-`load()`s from disk; it does not roll back, because that
   * would discard a write another process (setup, the CLI) landed in between,
   * and the UI has no test harness to catch such interleavings. Queued through
   * `enqueue`, so a direct call still serializes against `update`.
   */
````

### B3699 · l.97–115 · REWRITE (confirm) ·p · rule 1.7 · 19 → 13

````text
  /**
   * Shallow-merge `patch` onto `settings` and save, serialized through
   * `enqueue`. Rejects when nothing has loaded yet, so callers cannot report
   * success for a write that never happened. `patch.launch` replaces the whole
   * `LaunchDefaults` object — pass `{ ...settings.launch, … }` to change one
   * flag.
   *
   * `patch` may be a function of the current `settings`, resolved inside the
   * queued step against whatever `settings` is when this write runs. Prefer
   * this form when building a patch from a nested sub-object (e.g. `launch`):
   * otherwise a field captured before an earlier write's failure-triggered
   * `load()` rides along into this write.
   */
````

### B3700 · l.133–137 · REWRITE (confirm) · rule 1.7 · 5 → 3

````text
    /** `true` only after a load that actually succeeded. Gate controls on this,
     * not on `loaded` ("a load was attempted"), which a corrupt or unreadable
     * settings.json also sets while `settings` stays `null`. */
````

## `sabrage/ui/src/stores/stage.svelte.ts`

### B3702 · l.1–10 · REWRITE (amend) ·p · rule 1.2 · 10 → 6

````text
// Cross-component state for the pipeline stage runner: the stage (if any) open
// in GateModal, and StagesPanel visibility. Module-scoped Svelte 5 rune store,
// same shape as doctor.svelte.ts.
//
// GateModal mounts once at the app root (App.svelte) and reads `gate` directly;
// every opener goes through `openGate` instead of mounting its own dialog.
````

### B3703 · l.19–26 · REWRITE (confirm) · rule 1.2 · 8 → 6

````text
  /**
   * Present only when `stage === "run"`. The opener has already called
   * `sessionStore.launch(launch)`, so GateModal reads `sessionStore.launchRows` /
   * `sessionStore.launching` instead of calling `runStage`; `launch` is kept only
   * to re-issue the same launch from a Retry button after a Fatal row.
   */
````

### B3704 · l.28–36 · REWRITE (confirm) · rule 1.2 · 9 → 6

````text
  /**
   * Called once when the run settles: success, a Fatal row, or an `invoke()`
   * rejection. A Fix retry inside the same gate settles again and gets its own
   * call. Unused for `stage === "run"`, where the session store's own reactive
   * state is the settlement signal.
   */
````

### B3705 · l.43–54 · REWRITE (amend) · rule 1.5 · 12 → 5

````text
  /**
   * True while GateModal has a `runStage` call in flight for a non-run stage
   * (set via `setRunning`). Hide clears `gate` but not this, so openGate callers
   * must disable on it — see GateModal's `activeRequest` split, the other guard.
   */
````

## `sabrage/ui/src/types.ts`

### B3708 · l.3–5 · REWRITE (confirm) ·p · rule 1.7 · 3 → 2

````text
/** The state graph the main area switches on: one screen per sidebar nav item,
 * plus "edit" — the Library's add/edit-game form, reached from Library, never the sidebar. */
````

## `scripts/demo/build.sh`

### B3715 · l.14–15 · REWRITE (amend) ·p · rule 1.3 · 2 → 2

````text
# oxrsys: x86_64 (game runs under Rosetta, runtime loads in-process), Debug is the
# live-verified config. ALVR core is cargo-built by cmake (the rustup target check above).
````

## `scripts/demo/install.sh`

### B3742 · l.49–51 · REWRITE (amend) ·p · rule 1.7 · 3 → 4

````text
# Byte-shared with sabrage-core: both sides render contract/active_runtime.x86_64.json.template;
# drift = re-sudo thrash. $(<…) strips trailing newline; `\` and `"` escaped for JSON.
# Pinned: sabrage-parity tests::artifact_goldens::render_host_manifest_matches_the_on_disk_template,
# sabrage-parity tests::artifact_goldens::render_host_manifest_json_escapes_the_dylib_path.
````

## `scripts/demo/lib.sh`

Deleted (nothing carried): B3746, B3754, B3755

### B3744 · l.6–8 · REWRITE (confirm) · rule 1.4 · 3 → 2

````text
# contract.gen.sh is generated from contract/pipeline.toml: edit the contract, then
# regenerate (scripts/dev/parity.sh --regen). Never edit contract.gen.sh by hand.
````

### B3745 · l.11–16 · REWRITE (amend) · rule 1.4 · 6 → 4

````text
# WINEVR_BOTTLE   (--bottle) CrossOver bottle name, e.g. Steam. install/run/stop and
#                 `all` die without it; doctor FAILs bottle.named and carries on.
# WINEVR_BS_DIR   (--bs-dir) Beat Saber 1.29.4 install dir (DepotDownloader output);
#                 defaults to the bottle's standard Steam library once the bottle is known.
````

### B3749 · l.54 · REWRITE (confirm) · rule 1.4 · 1 → 1

````text
# print -r throughout: echo mangles the backslashes in windows paths.
````

### B3750 · l.63–68 · NEEDS-TEST (amend) ·p · rule 1.5 · 6 → 5

test first: `lib_sh_tap_and_chk_return_zero_when_tap_disabled` in `sabrage/crates/sabrage-parity/src/lib.rs (tests module; spawns `zsh -c` over the real scripts/demo/lib.sh with WINEVR_ROOT=repo_root and HOME pointed at a temp dir, PATH untouched, no bottle required)` — With WINEVR_DOCTOR_TAP unset, sourcing scripts/demo/lib.sh in `zsh -c 'set -e'` and calling tap/chk exits 0, so the tap-off branch never kills a set -e stage.

````text
# Check slugs — each has a stable slug shared with sabrage-core's check registry.
# tap: append "<slug> <status>" to $WINEVR_DOCTOR_TAP when set (opt-in parity channel).
# chk: print like ok/warn/fail/info AND tap. Both must return 0: lib.sh is sourced by
# set -e stages and a bare `[ -n ... ] && ...` tail returns 1 with the tap off
# (sabrage-parity::tests::lib_sh_tap_and_chk_return_zero_when_tap_disabled).
````

### B3760 · l.130–131 · REWRITE (confirm) · rule 1.3 · 2 → 2

````text
# DXMT_FILES comes from the contract; the .sha256 provenance marker is written by
# setup after extraction.
````

### B3762 · l.144 · REWRITE (amend) · rule 1.3 · 1 → 1

````text
# bin_path [found_msg] [not_found_msg]
````

### B3763 · l.145–148 · REWRITE (confirm) ·p · rule 1.3 · 4 → 2

````text
  # Returns 0 if a stray was found (kill attempted), 1 if none running; optional messages
  # reported via ok(). Shared by the reap sites in stop.sh/run.sh.
````

## `scripts/demo/run.sh`

### B3766 · l.8–9 · REWRITE (confirm) · rule 1.4 · 2 → 2

````text
# Fail fast with a remedy instead of a black window.
# preflight: game.present
````

### B3775 · l.34–37 · REWRITE (confirm) ·p · rule 2.2 · 4 → 3

````text
# The bottle's Graphics Backend overrides CX_GRAPHICS_BACKEND; the CrossOver GUI's
# "" (= auto) does not select DXMT, so the game spins before D3D11 device creation.
# preflight-autofix: bottle.gfx-dxmt
````

### B3779 · l.96–103 · REWRITE (amend) ·p · rule 1.4 · 8 → 5

````text
# Wired ALVR needs two adb forwards; left behind, they break WiFi discovery on a
# non-wired run ("searching for streamer"). --wired creates them; a normal run clears
# exactly these two. WIRED_PORTS comes from the contract (contract.gen.sh, sourced via lib.sh).
# preflight: run.wired-adb
# launch-action: adb-forward-hygiene
````

### B3781 · l.129–130 · REWRITE (confirm) · rule 1.4 · 2 → 2

````text
# Stale wineservers and steam locks hang startup, so reset the bottle's server first.
# launch-action: wineserver-reset
````

### B3782 · l.143–144 · REWRITE (amend) · rule 1.4 · 2 → 2

````text
# Goldberg emulates the Steam API offline, so no real Steam runs at any point.
# launch-action: goldberg-stage
````

### B3783 · l.157–158 · REWRITE (confirm) · rule 1.4 · 2 → 2

````text
# Route the Mac's default output into BlackHole so ALVR streams the game audio.
# launch-action: audio-route
````

### B3785 · l.179–181 · REWRITE (confirm) ·p · rule 1.1 · 3 → 1

````text
# INT/TERM tear the game down and restore audio, then resignal for the right exit status.
````

### B3787 · l.205–209 · REWRITE (confirm) ·p · rule 1.4 · 5 → 3

````text
# alvr_server_core hosts the dashboard on 127.0.0.1:8082 in-process; the stock UI
# polls until it appears (safe to launch before the game). Closed by the traps above.
# launch-action: dashboard
````

### B3788 · l.222–223 · REWRITE (confirm) · rule 1.4 · 2 → 1

````text
# launch-action: adb-reverse-cleanup
````

### B3790 · l.242–243 · REWRITE (confirm) · rule 1.4 · 2 → 1

````text
# launch-action: launch-wine
````
