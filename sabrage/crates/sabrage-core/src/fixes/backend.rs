//! `fix.set-graphics-backend` — force `CX_GRAPHICS_BACKEND` to `dxmt` in the
//! bottle's `cxbottle.conf`.
//!
//! CrossOver's default `"auto"` silently fails to load DXMT, which presents as a
//! black window with no VR (see CLAUDE.md's closed-investigation notes). `run.sh`
//! rewrites the line unconditionally in preflight — a **permanent** mutation
//! that is never unwound, matching the "permanent vs guarded" boundary in
//! design-core §3.2.
//!
//! Implementation notes for the fixes agent (design-core §10.25):
//! * the target line is exactly `"CX_GRAPHICS_BACKEND" = "dxmt"` — doctor greps
//!   it anchored, so spacing is a byte contract;
//! * three-branch edit logic (key absent / key present with another value / key
//!   already correct), preserving the rest of the file;
//! * refuse while the bottle's wineserver is live — the shell races the
//!   CrossOver GUI, which rewrites the file from memory on exit.

use std::path::Path;

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System, UpdateKind};

use crate::error::Result;
use crate::fixes::FixReport;
use crate::stages::{require_bottle, EventSink, StageCtx};

/// The exact line doctor's `bottle.gfx-dxmt` greps for, anchored at both ends
/// (`^"CX_GRAPHICS_BACKEND" = "dxmt"$`). Byte-for-byte the same literal
/// `checks::bottle::bottle_gfx_dxmt` compares against.
const TARGET_LINE: &str = "\"CX_GRAPHICS_BACKEND\" = \"dxmt\"";
/// `run.sh`'s prefix test for "some `CX_GRAPHICS_BACKEND` line exists, whatever
/// its value" (`grep -q '^"CX_GRAPHICS_BACKEND"'`).
const KEY_PREFIX: &str = "\"CX_GRAPHICS_BACKEND\"";
/// Everything up to and including the opening quote of the value, for the
/// per-line rewrite test (`^"CX_GRAPHICS_BACKEND" = ".*"$`).
const VALUE_PREFIX: &str = "\"CX_GRAPHICS_BACKEND\" = \"";
const ENV_SECTION_HEADER: &str = "[EnvironmentVariables]";

/// The console text `run.sh` prints after a successful edit, verbatim. Also
/// [`FixReport::changed`]'s description, so a CLI renderer and a structured
/// `AutoFixed` consumer show the same words.
const FORCED_DESCRIPTION: &str =
    "bottle graphics backend forced to dxmt (was auto/other — the CrossOver GUI can reset this)";

/// Which of `run.sh` lines 39–50's three branches [`rewrite_graphics_backend`]
/// took.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Branch {
    /// An existing `"CX_GRAPHICS_BACKEND" = "..."` line was rewritten in place
    /// (`sed -i '' 's/^"CX_GRAPHICS_BACKEND" = ".*"$/.../''`).
    Rewrote,
    /// No line started with the key at all, but `[EnvironmentVariables]` did
    /// exist; the target line was inserted immediately after that header.
    InsertedAfterEnvSection,
    /// Neither existed: a new `[EnvironmentVariables]` section was appended.
    AppendedSection,
}

/// Pure port of `run.sh` lines 39–50. Callers are expected to have already
/// established (as `run.sh`'s own outer `if` does) that `TARGET_LINE` is
/// **not** already present verbatim — this function does not re-check that,
/// it only decides *how* to introduce it.
///
/// Line-oriented reconstruction preserves every untouched line byte-for-byte,
/// including whether the file ends with a trailing newline; the append branch
/// instead does a raw string concatenation, matching `printf … >> "$CXCONF"`
/// exactly (it does not care whether `conf` already ended in a newline).
///
/// Caveat, faithfully reproduced: like `sed`, the rewrite branch only touches
/// a matching `CX_GRAPHICS_BACKEND` line that is *itself* shaped
/// `"CX_GRAPHICS_BACKEND" = "..."` (quoted value, one line). A line that starts
/// with the key but is not shaped that way (e.g. an unquoted value) is left
/// untouched, exactly as `sed`'s anchored substitution would skip it — the
/// caller does not re-verify after the edit, matching `run.sh`.
///
/// Sed fidelity is not absolute, though — measured, not assumed. BSD
/// `sed -i '' '/^\[EnvironmentVariables\]$/a\…'` mangles the header in exactly
/// one cell: when `[EnvironmentVariables]` is the **last line of the file and
/// that line has no trailing newline**, `a\` has nothing to append *after*, so
/// it concatenates the appended text directly onto the header —
/// `[EnvironmentVariables]"CX_GRAPHICS_BACKEND" = "dxmt"` — and still adds a
/// trailing newline to the file sed never had. This implementation always
/// inserts a real line break between the header and the new line and
/// preserves the absence of a trailing newline exactly (see
/// `branch_insert_preserves_absence_of_a_trailing_newline` below): strictly
/// better than `sed` in that one cell, not byte-parity with it. Every other
/// cell measured — the header mid-file with no trailing newline, the rewrite
/// branch with no trailing newline, and the append branch — is sed-identical.
/// Real `cxbottle.conf` files always end in a newline, so the mangled cell
/// does not arise in practice.
pub fn rewrite_graphics_backend(conf: &str) -> (String, Branch) {
    if conf.lines().any(|l| l.starts_with(KEY_PREFIX)) {
        let (mut lines, trailing_newline) = split_lines(conf);
        for line in lines.iter_mut() {
            if let Some(rest) = line.strip_prefix(VALUE_PREFIX) {
                if rest.ends_with('"') {
                    *line = TARGET_LINE.to_string();
                }
            }
        }
        return (rejoin(&lines, trailing_newline), Branch::Rewrote);
    }

    if conf.lines().any(|l| l == ENV_SECTION_HEADER) {
        let (lines, trailing_newline) = split_lines(conf);
        let mut out = Vec::with_capacity(lines.len() + 1);
        for line in lines {
            let is_header = line == ENV_SECTION_HEADER;
            out.push(line);
            if is_header {
                out.push(TARGET_LINE.to_string());
            }
        }
        return (
            rejoin(&out, trailing_newline),
            Branch::InsertedAfterEnvSection,
        );
    }

    // `printf '\n[EnvironmentVariables]\n"CX_GRAPHICS_BACKEND" = "dxmt"\n' >> "$CXCONF"`:
    // a raw append, indifferent to whether `conf` already ended in a newline.
    let appended = format!("{conf}\n{ENV_SECTION_HEADER}\n{TARGET_LINE}\n");
    (appended, Branch::AppendedSection)
}

fn split_lines(conf: &str) -> (Vec<String>, bool) {
    let trailing_newline = conf.ends_with('\n');
    (conf.lines().map(str::to_string).collect(), trailing_newline)
}

fn rejoin(lines: &[String], trailing_newline: bool) -> String {
    let mut out = lines.join("\n");
    if trailing_newline {
        out.push('\n');
    }
    out
}

// ── wineserver liveness (shared with `fixes::session_json`) ───────────────────

/// One running process whose resolved executable is a CrossOver `wineserver`.
struct WineserverProc {
    /// The `WINEPREFIX` value from its environment. `None` covers both "the
    /// variable was absent" and "the environment could not be read at all" —
    /// no caller here needs to tell those apart, since both mean "cannot
    /// positively rule this process out."
    wineprefix: Option<String>,
}

/// Every live process whose resolved executable equals `wineserver_exe`
/// (canonicalized on both sides, same convention as
/// [`crate::process::find_processes_by_exe`], which this deliberately does not
/// call — that helper does not read `environ`, and adding a second full
/// process scan just to get pids back would cost the same syscalls twice).
fn scan_wineservers(wineserver_exe: &Path) -> Vec<WineserverProc> {
    let want = wineserver_exe
        .canonicalize()
        .unwrap_or_else(|_| wineserver_exe.to_path_buf());
    let refresh = ProcessRefreshKind::nothing()
        .with_exe(UpdateKind::Always)
        .with_environ(UpdateKind::Always);
    let mut sys = System::new_with_specifics(RefreshKind::nothing().with_processes(refresh));
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh);

    sys.processes()
        .values()
        .filter_map(|proc_| {
            let exe = proc_.exe()?;
            let resolved = exe.canonicalize().unwrap_or_else(|_| exe.to_path_buf());
            if resolved != want {
                return None;
            }
            let wineprefix = proc_.environ().iter().find_map(|kv| {
                kv.to_str()
                    .and_then(|s| s.strip_prefix("WINEPREFIX="))
                    .map(str::to_string)
            });
            Some(WineserverProc { wineprefix })
        })
        .collect()
}

/// The decision `bottle_wineserver_is_live` makes, factored out as a pure
/// function of the `WINEPREFIX` values *observed* on every live wineserver
/// process (one entry per process; `None` means its environment could not be
/// read at all, or lacked the variable — both "cannot rule this one out").
///
/// Kept separate from [`scan_wineservers`] so the actual decision — including
/// the "when in doubt, refuse" rule — has a test that does not depend on
/// spawning or observing real OS processes (system-wide process state is
/// exactly the kind of thing a parallel test run cannot pin down: some other
/// test in this same binary may have its own child alive at the same instant,
/// and sandboxed CI runners have been observed to `SIGKILL` a copied-to-`/tmp`
/// executable before it can even be scanned).
fn wineservers_indicate_live(observed_wineprefixes: &[Option<String>], want_prefix: &str) -> bool {
    observed_wineprefixes.iter().any(|wp| match wp.as_deref() {
        Some(p) => p == want_prefix,
        None => true,
    })
}

/// Whether the CrossOver wineserver appears to be alive **for `bottle_prefix`
/// specifically**.
///
/// `wineserver -w` would block indefinitely if a live server were probed
/// directly, so this reads process state instead: every running process whose
/// resolved executable is `wineserver_exe`, checked against its `WINEPREFIX`
/// environment variable (set verbatim to the bottle prefix by both `run.sh`
/// and this crate's own launch code). When a matching process's environment
/// cannot be read at all, or simply lacks `WINEPREFIX`, this refuses to guess
/// which bottle it belongs to and treats it as live for `bottle_prefix` — the
/// shell races the CrossOver GUI here, and a false "clear" would let this fix
/// corrupt a file CrossOver still has open. See [`wineservers_indicate_live`]
/// for the actual (unit-tested) decision.
pub(crate) fn bottle_wineserver_is_live(wineserver_exe: &Path, bottle_prefix: &Path) -> bool {
    let observed: Vec<Option<String>> = scan_wineservers(wineserver_exe)
        .into_iter()
        .map(|p| p.wineprefix)
        .collect();
    wineservers_indicate_live(&observed, &bottle_prefix.to_string_lossy())
}

/// Whether **any** CrossOver wineserver is alive, for callers with no single
/// bottle to narrow the probe against (ALVR's `session.json` is machine-global,
/// not scoped to one bottle — see [`crate::fixes::session_json`]).
pub(crate) fn any_wineserver_alive(wineserver_exe: &Path) -> bool {
    !scan_wineservers(wineserver_exe).is_empty()
}

// ── the fix ─────────────────────────────────────────────────────────────────

/// Rewrite the bottle's graphics backend to `dxmt`.
pub async fn set_graphics_backend(ctx: &StageCtx, sink: &EventSink) -> Result<FixReport> {
    let bottle = require_bottle(ctx)?;
    let conf_path = bottle.conf_path();
    let conf = std::fs::read_to_string(&conf_path)
        .map_err(|e| crate::error::SabrageError::io(&conf_path, e))?;

    if conf.lines().any(|l| l == TARGET_LINE) {
        return Ok(FixReport::unchanged(
            crate::fixes::FixAction::SetGraphicsBackend,
            format!("{} already forces dxmt", ctx.paths.rel_display(&conf_path)),
        ));
    }

    if let Some(wineserver) = &ctx.paths.wineserver {
        if bottle_wineserver_is_live(wineserver, &bottle.prefix) {
            return Err(ctx.fatal(
                format!(
                    "refusing to edit {} while bottle '{}' has a live wineserver — CrossOver \
                     may rewrite this file from memory on exit and clobber the change; stop \
                     the session first",
                    conf_path.display(),
                    bottle.name
                ),
                Some(format!("./demo.sh stop --bottle {}", bottle.name)),
            ));
        }
    }

    let (rewritten, _branch) = rewrite_graphics_backend(&conf);
    ctx.executor
        .write_atomic(&conf_path, rewritten.as_bytes())
        .await?;

    sink(crate::events::StageEvent::ok(
        ctx.run_id,
        None,
        FORCED_DESCRIPTION,
    ));
    Ok(FixReport::changed(
        crate::fixes::FixAction::SetGraphicsBackend,
        FORCED_DESCRIPTION,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixes::FixAction;
    use crate::paths::{Bottle, Paths};
    use crate::stages::{null_sink, StageCtx, StageOptions};
    use std::sync::{Arc, Mutex as StdMutex};
    use tokio_util::sync::CancellationToken;

    fn matches_doctor_anchor(s: &str) -> bool {
        s.lines().any(|l| l == TARGET_LINE)
    }

    // ── rewrite_graphics_backend: the three branches, byte-exact ────────────

    #[test]
    fn branch_rewrites_an_existing_key_line_in_place() {
        let conf =
            "\"Template\" = \"win11_64\"\n\"CX_GRAPHICS_BACKEND\" = \"auto\"\n\"Other\" = \"1\"\n";
        let (out, branch) = rewrite_graphics_backend(conf);
        assert_eq!(branch, Branch::Rewrote);
        assert_eq!(
            out,
            "\"Template\" = \"win11_64\"\n\"CX_GRAPHICS_BACKEND\" = \"dxmt\"\n\"Other\" = \"1\"\n"
        );
        assert!(matches_doctor_anchor(&out));
    }

    #[test]
    fn branch_rewrite_handles_an_empty_existing_value() {
        let conf = "\"CX_GRAPHICS_BACKEND\" = \"\"\n";
        let (out, branch) = rewrite_graphics_backend(conf);
        assert_eq!(branch, Branch::Rewrote);
        assert_eq!(out, "\"CX_GRAPHICS_BACKEND\" = \"dxmt\"\n");
        assert!(matches_doctor_anchor(&out));
    }

    #[test]
    fn branch_rewrite_preserves_absence_of_a_trailing_newline() {
        let conf = "\"CX_GRAPHICS_BACKEND\" = \"auto\"";
        let (out, branch) = rewrite_graphics_backend(conf);
        assert_eq!(branch, Branch::Rewrote);
        assert_eq!(out, "\"CX_GRAPHICS_BACKEND\" = \"dxmt\"");
        assert!(!out.ends_with('\n'));
    }

    #[test]
    fn branch_rewrite_leaves_a_malformed_key_line_untouched_like_sed_would() {
        // Starts with the key (enters the Rewrote branch overall) but is not
        // shaped `"CX_GRAPHICS_BACKEND" = "..."`, so sed's anchored `s///`
        // would not match it either. Faithfully reproduced: the line survives
        // unmodified and the file does NOT end up containing the target line.
        let conf = "\"CX_GRAPHICS_BACKEND\" = auto\n";
        let (out, branch) = rewrite_graphics_backend(conf);
        assert_eq!(branch, Branch::Rewrote);
        assert_eq!(out, conf, "an unquoted value must not be touched");
        assert!(!matches_doctor_anchor(&out));
    }

    #[test]
    fn branch_inserts_immediately_after_the_environment_variables_header() {
        let conf = "[EnvironmentVariables]\n\"SOME_OTHER\" = \"1\"\n";
        let (out, branch) = rewrite_graphics_backend(conf);
        assert_eq!(branch, Branch::InsertedAfterEnvSection);
        assert_eq!(
            out,
            "[EnvironmentVariables]\n\"CX_GRAPHICS_BACKEND\" = \"dxmt\"\n\"SOME_OTHER\" = \"1\"\n"
        );
        assert!(matches_doctor_anchor(&out));
    }

    /// Documented improvement over BSD `sed`, not parity with it (review
    /// finding #10; measured table in the review). Run for real against this
    /// exact input,
    /// `sed -i '' '/^\[EnvironmentVariables\]$/a\"CX_GRAPHICS_BACKEND" = "dxmt"'`
    /// mangles the header — because `[EnvironmentVariables]` is both the match
    /// for `a\` *and* the file's last line with no trailing newline, sed has
    /// nothing to append "after" and instead concatenates the appended text
    /// straight onto the header, producing
    /// `[EnvironmentVariables]"CX_GRAPHICS_BACKEND" = "dxmt"\n` (header
    /// corrupted, and a trailing newline appears that the original file never
    /// had). This implementation never does that: the header and the new line
    /// are always joined by a real `\n`, and the file's own trailing-newline
    /// state is preserved exactly, as asserted below. Real `cxbottle.conf`
    /// files always end in a newline, so the sed-mangled case never arises in
    /// practice — but the byte-exact assertion here is what would catch a
    /// regression toward it.
    #[test]
    fn branch_insert_preserves_absence_of_a_trailing_newline() {
        let conf = "[EnvironmentVariables]";
        let (out, branch) = rewrite_graphics_backend(conf);
        assert_eq!(branch, Branch::InsertedAfterEnvSection);
        assert_eq!(
            out, "[EnvironmentVariables]\n\"CX_GRAPHICS_BACKEND\" = \"dxmt\"",
            "header and inserted line must be joined by a real newline, not \
             concatenated the way BSD sed's `a\\` mangles this exact case"
        );
        assert!(
            !out.ends_with('\n'),
            "must not gain a trailing newline the original file never had"
        );
    }

    #[test]
    fn branch_appends_a_new_section_when_neither_exists() {
        let conf = "\"Template\" = \"win11_64\"\n";
        let (out, branch) = rewrite_graphics_backend(conf);
        assert_eq!(branch, Branch::AppendedSection);
        assert_eq!(
            out,
            "\"Template\" = \"win11_64\"\n\n[EnvironmentVariables]\n\"CX_GRAPHICS_BACKEND\" = \"dxmt\"\n"
        );
        assert!(matches_doctor_anchor(&out));
    }

    #[test]
    fn branch_append_does_not_care_whether_the_original_had_a_trailing_newline() {
        let conf = "\"Template\" = \"win11_64\""; // no trailing newline
        let (out, branch) = rewrite_graphics_backend(conf);
        assert_eq!(branch, Branch::AppendedSection);
        assert_eq!(
            out,
            "\"Template\" = \"win11_64\"\n[EnvironmentVariables]\n\"CX_GRAPHICS_BACKEND\" = \"dxmt\"\n"
        );
    }

    // ── wineserver liveness: the pure decision, unit-tested directly ────────
    //
    // `scan_wineservers`'s actual OS-process scan is deliberately NOT
    // exercised by spawning a stand-in child here: this crate's sandboxed test
    // runner has been observed to SIGKILL a copied-to-`/tmp` executable before
    // it can even be scanned (a code-signing/exec restriction on temp-dir
    // binaries, unrelated to this logic), and `cargo test`'s own parallelism
    // means unrelated tests elsewhere in this binary may have their own real
    // child processes alive at the same instant — system-wide process state is
    // not a fixture this suite can pin down deterministically. The decision
    // logic itself (the part actually worth testing) is the pure function
    // below, which takes the same shape `scan_wineservers` produces.

    #[test]
    fn wineservers_indicate_live_matches_by_exact_wineprefix() {
        let observed = vec![Some("/bottles/A".to_string())];
        assert!(wineservers_indicate_live(&observed, "/bottles/A"));
        assert!(!wineservers_indicate_live(&observed, "/bottles/B"));
    }

    #[test]
    fn wineservers_indicate_live_refuses_when_a_match_cannot_be_ruled_out() {
        // No WINEPREFIX readable on a live wineserver at all: cannot tell
        // which bottle it belongs to -> refuse (true) for every candidate.
        let observed = vec![None];
        assert!(wineservers_indicate_live(&observed, "/anything"));

        // One process for a different bottle AND one unreadable: still
        // refuses, because the unreadable one might be this bottle's.
        let observed = vec![Some("/bottles/Other".to_string()), None];
        assert!(wineservers_indicate_live(&observed, "/bottles/A"));
    }

    #[test]
    fn wineservers_indicate_live_is_false_when_nothing_is_running() {
        assert!(!wineservers_indicate_live(&[], "/anything"));
    }

    #[test]
    fn wineservers_indicate_live_is_false_when_every_match_is_a_different_bottle() {
        let observed = vec![
            Some("/bottles/Other1".to_string()),
            Some("/bottles/Other2".to_string()),
        ];
        assert!(!wineservers_indicate_live(&observed, "/bottles/A"));
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

    // ── set_graphics_backend (the async fix) ────────────────────────────────

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

        let seen: Arc<StdMutex<Vec<crate::events::StageEvent>>> =
            Arc::new(StdMutex::new(Vec::new()));
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

    #[tokio::test]
    async fn set_graphics_backend_under_dry_run_does_not_touch_the_file() {
        let root = scratch("dry");
        let (ctx, conf) = fixture_ctx(&root, true);
        std::fs::write(&conf, "\"CX_GRAPHICS_BACKEND\" = \"auto\"\n").unwrap();
        assert!(ctx.executor.is_dry_run());

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

        // Stand in for a live wineserver with THIS test binary's own running
        // process — guaranteed alive with no spawning needed (same trick
        // `process::tests::finds_this_test_binary_by_its_exe_path` uses).
        // Whatever `WINEPREFIX` this process does or does not have, the
        // refusal must still hold: either it happens to match, or (the
        // overwhelmingly likely case — nothing sets WINEPREFIX for `cargo
        // test`) it is absent entirely, which is exactly the "cannot rule
        // this one out" branch `wineservers_indicate_live` refuses on.
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
}
