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
//! `sweep_stale_staging` (`StagedTemp`).
//!
//! [`AdminMethod::detect`] picks `sudo` whenever a controlling terminal is
//! reachable and never consults stdout, so `sabrage install | tee log` prompts
//! in the terminal exactly like `./demo.sh install | tee`; a GUI-launched `.app`
//! has no controlling terminal and takes the osascript path. The two `die`
//! strings on the `sudo` path are install.sh's, verbatim.
//! Reference: scripts/demo/install.sh.

use std::ffi::OsString;
use std::io::{IsTerminal, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::error::{Result, SabrageError};
use crate::events::{step, StageEvent, Stream};
use crate::stages::StageCtx;

/// Absolute paths only: a GUI-launched `.app` inherits a bare `PATH`, and a
/// privileged command is the last place to let `PATH` decide what runs.
const OSASCRIPT: &str = "/usr/bin/osascript";
const SUDO: &str = "/usr/bin/sudo";
const MKDIR: &str = "/bin/mkdir";
const INSTALL: &str = "/usr/bin/install";

/// System Settings deep link for the App Management pane.
pub const APP_MANAGEMENT_SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_AppBundles";

/// What [`StageEvent::NeedsAdmin`] says before the osascript dialog appears, so
/// the user can predict the dialog and knows it will not return on every install
/// (design-core § 5. Privilege boundary, implementation step 4).
pub const NEEDS_ADMIN_REASON: &str =
    "this writes the host OpenXR registration — one password, only when the repo path changes";

/// The same announcement for the [`AdminMethod::Sudo`] path, which prompts on
/// the controlling terminal, not in a macOS dialog.
///
/// It says where the prompt is because a GUI started from a shell inherits that
/// terminal and the password prompt can sit behind the window the user is
/// looking at (tests::the_announcement_names_the_method_detect_actually_picked).
pub const NEEDS_ADMIN_REASON_SUDO: &str = "sudo is waiting for your password in the terminal that launched Sabrage — this writes the host OpenXR registration, only when the repo path changes";

/// The announcement for `method`: [`NEEDS_ADMIN_REASON`] for the dialog,
/// [`NEEDS_ADMIN_REASON_SUDO`] for the terminal.
pub fn needs_admin_reason(method: AdminMethod) -> &'static str {
    match method {
        AdminMethod::Osascript => NEEDS_ADMIN_REASON,
        AdminMethod::Sudo => NEEDS_ADMIN_REASON_SUDO,
    }
}

/// The `--dry-run` stand-in for the prompt: a dry run must never emit
/// [`StageEvent::NeedsAdmin`], which the GUI renders as "macOS will ask for your
/// password" (PARITY.md § CLI / GUI, "A dry run's layer 4 prints").
pub const WOULD_PROMPT_DRY_RUN: &str = "would prompt for administrator authorization";

/// osascript reports a dismissed authorization dialog as
/// `execution error: User canceled. (-128)`.
const USER_CANCELLED_MARKER: &str = "-128";

/// install.sh's two `die` strings on the sudo path, in the order the commands
/// run. Indices line up with [`elevation_argv`]'s `Sudo` output.
const SUDO_DIE: [&str; 2] = ["sudo mkdir failed", "sudo write failed"];

/// How long a cancellation waits for the elevated child to actually exit before
/// giving up on it.
///
/// Bounded on purpose: past `sudo`'s exec the child's real uid is 0 and an
/// unprivileged `kill(2)` comes back `EPERM`, so a wait on a signalled child
/// that never notices could run forever; a bounded one still reaps the ordinary
/// case, where the child is still ours
/// (tests::a_cancelled_child_is_reaped_before_the_call_returns).
const CANCEL_REAP_GRACE: Duration = Duration::from_millis(500);

/// How old a leftover `host-manifest-*.json` must be before
/// [`sweep_stale_staging`] removes it. Comfortably longer than any elevated
/// `install` can take, so a *concurrent* sabrage's staging file is never pulled
/// out from under it.
const STAGING_SWEEP_AGE: Duration = Duration::from_secs(3600);

/// What a cancelled elevation reports about the staging file it deliberately
/// left behind.
const CANCELLED_MID_ELEVATION_WARN: &str =
    "cancelled while the elevated write may still be running — leaving the staging file in \
     place (removing it could corrupt a privileged write already in flight); the next install \
     sweeps it";

/// Outcome of the one privileged write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivilegedWrite {
    /// On-disk bytes already matched: no prompt, nothing written.
    Skipped,
    /// The manifest was written (one authorization prompt).
    Written,
    /// `--dry-run`: the staging write and the elevated argv were *planned*
    /// (recorded through the executor), nothing was prompted for and nothing
    /// was written. A separate variant because the caller renders it — the row
    /// a preview prints must not be the row a completed install prints.
    Planned,
}

/// How to ask for administrator rights.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminMethod {
    /// GUI authorization dialog — the `.app`, and any run with no controlling
    /// terminal to prompt on.
    Osascript,
    /// `sudo`, prompting on the process's controlling terminal — where `sudo`
    /// reads the password from regardless of what stdout is connected to.
    Sudo,
}

impl AdminMethod {
    /// Choose by controlling terminal: a session that has one gets `sudo` (the
    /// user is already looking at a shell, exactly like `./demo.sh install`),
    /// anything else gets the GUI dialog.
    ///
    /// Both probes are read here and the decision itself is
    /// [`AdminMethod::choose`], so the rule is unit-testable without a tty.
    pub fn detect() -> AdminMethod {
        AdminMethod::choose(std::io::stdin().is_terminal(), controlling_tty_available())
    }

    /// The decision, as a pure function of the two probes.
    ///
    /// Deliberately not a function of stdout: `sabrage install | tee log` has a
    /// non-tty stdout and a perfectly good terminal to prompt on, and either probe
    /// alone is sufficient
    /// (tests::admin_method_is_decided_by_the_controlling_terminal_never_by_stdout).
    pub fn choose(stdin_is_tty: bool, controlling_tty: bool) -> AdminMethod {
        if stdin_is_tty || controlling_tty {
            AdminMethod::Sudo
        } else {
            AdminMethod::Osascript
        }
    }
}

/// True when this process has a controlling terminal — the thing `sudo` prompts
/// on. Opening `/dev/tty` is the portable test: it is the controlling terminal
/// by definition, and the open fails with `ENXIO` when there is none (a
/// GUI-launched `.app`, a daemon, a session-leaderless child).
fn controlling_tty_available() -> bool {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .is_ok()
}

/// Why a write was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteErrorKind {
    /// `PermissionDenied` on a path inside a `.app` bundle: almost certainly
    /// macOS App Management. `sudo` does **not** help; the user must grant the
    /// permission in System Settings and relaunch.
    TccAppManagementLikely,
    /// `PermissionDenied` elsewhere — a genuine ownership/mode problem.
    PermissionDenied,
    /// Anything else (missing parent, read-only volume, ENOSPC).
    Other,
}

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
pub fn applescript_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out
}

/// Quote one argv element for `/bin/sh`.
///
/// Single quotes, with `'` itself spelled `'\''` — inside single quotes `/bin/sh`
/// treats every byte literally, which is the only quoting rule on that side with
/// no exceptions to remember.
pub fn shell_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

/// The `/bin/sh` command the authorization dialog runs: `mkdir -p` of the
/// destination's directory and `install -m 0644 -o root -g wheel` of the staging
/// file, joined with `&&` so there is exactly one prompt
/// (tests::the_command_creates_the_directory_and_installs_root_wheel_0644).
pub fn privileged_install_command(tmp: &Path, dest: &Path) -> String {
    let dir = dest.parent().unwrap_or_else(|| Path::new("/"));
    format!(
        "{MKDIR} -p {} && {INSTALL} -m 0644 -o root -g wheel {} {}",
        shell_quote(&dir.to_string_lossy()),
        shell_quote(&tmp.to_string_lossy()),
        shell_quote(&dest.to_string_lossy()),
    )
}

/// Wrap a `/bin/sh` command as the single `-e` argument to `osascript`.
///
/// The result is passed as **one argv element**, so no further shell quoting
/// exists at that level — the two layers are AppleScript's string literal and
/// the `/bin/sh` line inside it, nothing more.
pub fn do_shell_script(command: &str) -> String {
    format!(
        "do shell script \"{}\" with administrator privileges",
        applescript_escape(command)
    )
}

/// The exact argv of every child the elevation runs, in order.
///
/// One vector for [`AdminMethod::Osascript`], two for [`AdminMethod::Sudo`]
/// (`sudo mkdir -p …`, then `sudo install …`), mirroring install.sh's two sudo
/// calls and its two `die` strings
/// (tests::elevation_argv_is_one_osascript_or_install_shs_two_sudo_calls).
/// Reference: scripts/demo/install.sh.
pub fn elevation_argv(method: AdminMethod, tmp: &Path, dest: &Path) -> Vec<Vec<OsString>> {
    let dir = dest.parent().unwrap_or_else(|| Path::new("/"));
    match method {
        AdminMethod::Osascript => vec![vec![
            OsString::from(OSASCRIPT),
            OsString::from("-e"),
            OsString::from(do_shell_script(&privileged_install_command(tmp, dest))),
        ]],
        AdminMethod::Sudo => vec![
            vec![
                OsString::from(SUDO),
                OsString::from(MKDIR),
                OsString::from("-p"),
                dir.as_os_str().to_os_string(),
            ],
            vec![
                OsString::from(SUDO),
                OsString::from(INSTALL),
                OsString::from("-m"),
                OsString::from("0644"),
                OsString::from("-o"),
                OsString::from("root"),
                OsString::from("-g"),
                OsString::from("wheel"),
                tmp.as_os_str().to_os_string(),
                dest.as_os_str().to_os_string(),
            ],
        ],
    }
}

/// The exact bytes install layer 4 stages and installs: the rendered manifest
/// plus the single trailing newline `print -- "$WANT"` appends
/// ([`crate::util::host_manifest_file_bytes`]).
///
/// Named so tests and `sabrage-parity`'s golden pin the byte source of the
/// privileged write directly, rather than the util helper the write path might
/// not call.
pub fn host_manifest_bytes(oxr_dylib: &Path) -> String {
    crate::util::host_manifest_file_bytes(oxr_dylib)
}

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
pub async fn write_host_manifest_privileged(
    ctx: &StageCtx,
    oxr_dylib: &Path,
    dest: &Path,
) -> Result<PrivilegedWrite> {
    // Defence in depth: install layer 4 refuses the same path before it even
    // renders the comparison form, so this fires only for a future caller.
    reject_unrepresentable_manifest_path(ctx, oxr_dylib)?;
    let content = host_manifest_bytes(oxr_dylib);
    if crate::util::host_manifest_is_current(dest, crate::util::strip_trailing_newlines(&content)) {
        return Ok(PrivilegedWrite::Skipped);
    }

    let method = AdminMethod::detect();

    if ctx.executor.is_dry_run() {
        // No NeedsAdmin here: a dry run prompts for nothing, and the GUI's
        // gate modal renders that event as "macOS will ask for your password".
        ctx.step(step::INSTALL_HOST_MANIFEST)
            .info(WOULD_PROMPT_DRY_RUN);
        return plan_privileged_write(ctx, method, &content, dest).await;
    }

    // Nothing below this line is undoable from our side: past `osascript`'s or
    // `sudo`'s exec we cannot signal the elevated child at all, so a run
    // cancelled during layer 3 must not surface an authorization prompt
    // (tests::a_cancelled_run_neither_announces_nor_stages_the_privileged_write).
    if ctx.cancel.is_cancelled() {
        return Err(SabrageError::Cancelled);
    }

    ctx.emit(StageEvent::NeedsAdmin {
        run_id: ctx.run_id,
        step: step::INSTALL_HOST_MANIFEST.into(),
        reason: needs_admin_reason(method).to_string(),
    });

    // Anything a previous cancellation deliberately left behind (see below).
    sweep_stale_staging(&sabrage_temp_dir());

    // Dropped — and so deleted — on every exit path below, except the
    // cancellation arm, which defuses it.
    let mut staged = StagedTemp::create(&sabrage_temp_dir(), &content)?;
    let argv = elevation_argv(method, &staged.path, dest);
    let elevated = match method {
        AdminMethod::Osascript => elevate_osascript(ctx, &argv[0]).await,
        AdminMethod::Sudo => elevate_sudo(ctx, &argv).await,
    };
    if let Err(e) = elevated {
        // A cancelled elevation is the one case where the privileged command
        // may still be reading this file, so it is kept rather than unlinked
        // (tests::a_defused_staging_file_outlives_its_drop).
        if matches!(e, SabrageError::Cancelled) {
            staged.defuse();
            ctx.step(step::INSTALL_HOST_MANIFEST)
                .warn(CANCELLED_MID_ELEVATION_WARN);
        }
        return Err(e);
    }
    verify_written(ctx, dest, &content)?;
    Ok(PrivilegedWrite::Written)
}

/// The `--dry-run` half of [`write_host_manifest_privileged`]: record the
/// staging write and the elevated argv through the executor, mutate nothing.
///
/// Reported as [`PrivilegedWrite::Planned`], never `Written`: a preview that
/// prints the row a real install prints is indistinguishable from completed work
/// in the event log.
async fn plan_privileged_write(
    ctx: &StageCtx,
    method: AdminMethod,
    content: &str,
    dest: &Path,
) -> Result<PrivilegedWrite> {
    let exec = ctx.executor_for(step::INSTALL_HOST_MANIFEST);
    let dir = sabrage_temp_dir();
    let tmp = dir.join(temp_file_name());
    exec.create_dir_all(&dir).await?;
    exec.write_atomic(&tmp, content.as_bytes()).await?;
    for argv in elevation_argv(method, &tmp, dest) {
        let spec = ctx
            .child(argv[0].clone(), step::INSTALL_HOST_MANIFEST)
            .args(argv[1..].iter().cloned());
        exec.run_child(&spec).await?;
    }
    Ok(PrivilegedWrite::Planned)
}

/// One `osascript -e 'do shell script … with administrator privileges'`.
///
/// Exit status alone cannot tell a declined dialog from a failed command — both
/// are non-zero — so the `(-128)` marker in stderr is what separates
/// [`SabrageError::AdminDeclined`] from a real failure; everything else on
/// stderr is surfaced as [`StageEvent::Output`] rather than swallowed
/// (tests::declined_and_failed_authorization_are_told_apart_by_stderr;
/// design-core § 6, "No swallowed diagnostics").
async fn elevate_osascript(ctx: &StageCtx, argv: &[OsString]) -> Result<()> {
    let out = run_capturing(argv, &ctx.cancel).await?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    if is_user_cancelled(&stderr) {
        ctx.emit(StageEvent::Fatal {
            run_id: ctx.run_id,
            message: SabrageError::AdminDeclined.to_string(),
            remedy: Some(terminal_fallback_remedy(ctx)),
            fix: None,
        });
        return Err(SabrageError::AdminDeclined);
    }
    for line in stderr.lines() {
        ctx.emit(StageEvent::Output {
            run_id: ctx.run_id,
            step: step::INSTALL_HOST_MANIFEST.into(),
            stream: Stream::Stderr,
            chunk: line.to_string(),
            end: crate::process::ChunkEnd::Lf,
        });
    }
    Err(ctx.fatal(
        format!(
            "host registration write failed (osascript exited {})",
            crate::process::exit_code_of(out.status)
        ),
        Some(terminal_fallback_remedy(ctx)),
    ))
}

/// install.sh's sudo path, structurally unchanged, with
/// `install -m 0644 -o root -g wheel` in place of `sudo tee` — same bytes, mode
/// and ownership, without the JSON ever crossing a pipe or a command line
/// (PARITY.md § Install (the one privileged write), "The elevated write is").
/// Reference: scripts/demo/install.sh.
///
/// Both children inherit this process's stdio so `sudo`'s own password prompt
/// works, which is why they cannot go through `spawn_streamed`.
async fn elevate_sudo(ctx: &StageCtx, argv: &[Vec<OsString>]) -> Result<()> {
    for (i, one) in argv.iter().enumerate() {
        let status = run_inheriting(one, &ctx.cancel).await?;
        if !status.success() {
            let message = SUDO_DIE.get(i).copied().unwrap_or(SUDO_DIE[1]);
            return Err(ctx.fatal(message, Some(terminal_fallback_remedy(ctx))));
        }
    }
    Ok(())
}

/// Re-read the destination and compare it to what we meant to write.
///
/// The elevated command could have half-run, been sandboxed, or raced another
/// installer; claiming success without looking is how a "successful" install
/// leaves the game with no VR.
fn verify_written(ctx: &StageCtx, dest: &Path, content: &str) -> Result<()> {
    match std::fs::read(dest) {
        Ok(bytes) if bytes == content.as_bytes() => Ok(()),
        Ok(_) => Err(ctx.fatal(
            format!(
                "{} was written but does not match the expected content",
                dest.display()
            ),
            Some(terminal_fallback_remedy(ctx)),
        )),
        Err(e) => Err(ctx.fatal(
            format!("{}: {e}", dest.display()),
            Some(terminal_fallback_remedy(ctx)),
        )),
    }
}

/// True for osascript's dismissed-dialog status (`User canceled. (-128)`).
fn is_user_cancelled(stderr: &str) -> bool {
    stderr.contains(USER_CANCELLED_MARKER)
}

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
pub fn reject_unrepresentable_manifest_path(ctx: &StageCtx, dylib: &Path) -> Result<()> {
    // The JSON minimum, exactly: U+0000..U+001F are the only characters a JSON
    // string literal may not carry raw. DEL and the C1 block are legal there,
    // so a path is not rejected for containing them.
    if !dylib.to_string_lossy().chars().any(|c| (c as u32) < 0x20) {
        return Ok(());
    }
    Err(ctx.fatal(
        format!(
            "{} contains a control character — the host OpenXR manifest would not be valid JSON",
            dylib.display()
        ),
        Some(
            "move the checkout to a path without tabs, newlines or other control characters, then re-run install".to_string(),
        ),
    ))
}

/// Spawn `argv` with both output streams captured and stdin closed.
async fn run_capturing(
    argv: &[OsString],
    cancel: &CancellationToken,
) -> Result<std::process::Output> {
    // An already-cancelled run spawns nothing at all: the `select!` below can
    // only cancel a prompt that is already on screen.
    if cancel.is_cancelled() {
        return Err(SabrageError::Cancelled);
    }
    let mut cmd = tokio::process::Command::new(&argv[0]);
    cmd.args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child = cmd
        .spawn()
        .map_err(|e| SabrageError::io(PathBuf::from(&argv[0]), e))?;
    let wait = child.wait_with_output();
    tokio::pin!(wait);
    tokio::select! {
        out = &mut wait => {
            out.map_err(|e| SabrageError::io(PathBuf::from(&argv[0]), e))
        }
        _ = cancel.cancelled() => {
            // Bounded moment for the child to finish: returning the instant
            // Stop is pressed would tear the staging file down under a live
            // privileged `install` (CANCEL_REAP_GRACE; run_inheriting's twin
            // arm is pinned by tests::a_cancelled_child_is_reaped_before_the_call_returns).
            let _ = tokio::time::timeout(CANCEL_REAP_GRACE, &mut wait).await;
            Err(SabrageError::Cancelled)
        }
    }
}

/// Spawn `argv` with this process's stdio inherited — `sudo` needs the tty.
///
/// No process group of its own: `sudo` must stay in the foreground group to
/// read the terminal, and a Ctrl-C in that terminal is already delivered to the
/// whole group.
async fn run_inheriting(argv: &[OsString], cancel: &CancellationToken) -> Result<ExitStatus> {
    // As in `run_capturing`: never spawn `sudo` for a run that is already over.
    if cancel.is_cancelled() {
        return Err(SabrageError::Cancelled);
    }
    let mut cmd = tokio::process::Command::new(&argv[0]);
    cmd.args(&argv[1..]).kill_on_drop(true);
    let mut child = cmd
        .spawn()
        .map_err(|e| SabrageError::io(PathBuf::from(&argv[0]), e))?;
    tokio::select! {
        status = child.wait() => {
            status.map_err(|e| SabrageError::io(PathBuf::from(&argv[0]), e))
        }
        _ = cancel.cancelled() => {
            // Signal, then reap — bounded, and the kill is best effort:
            // after `sudo` execs, its real uid is 0 and this process cannot
            // signal it at all
            // (tests::a_cancelled_child_is_reaped_before_the_call_returns).
            let _ = child.start_kill();
            let _ = tokio::time::timeout(CANCEL_REAP_GRACE, child.wait()).await;
            Err(SabrageError::Cancelled)
        }
    }
}

/// A `0600` file that deletes itself when dropped — unless it has been
/// [`defuse`](StagedTemp::defuse)d.
#[derive(Debug)]
struct StagedTemp {
    path: PathBuf,
    /// False once [`StagedTemp::defuse`] has run: `Drop` then leaves the file
    /// alone.
    armed: bool,
}

impl StagedTemp {
    /// Create `dir` (mode `0700`) and a fresh randomly-named `0600` file in it
    /// holding `content`.
    ///
    /// `create_new` is the load-bearing flag: it fails rather than opening a
    /// path something else pre-created, so with the random name the bytes root
    /// installs can only have come from us
    /// (tests::staging_creates_the_directory_0700_and_never_reuses_a_name,
    /// tests::staged_temp_is_0600_and_deletes_itself).
    fn create(dir: &Path, content: &str) -> Result<StagedTemp> {
        std::fs::create_dir_all(dir).map_err(|e| SabrageError::io(dir, e))?;
        // Best effort: an existing directory keeps whatever mode it has, and a
        // wrong mode here is not worth failing the install over.
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        let path = dir.join(temp_file_name());
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| SabrageError::io(&path, e))?;
        let staged = StagedTemp { path, armed: true };
        file.write_all(content.as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(|e| SabrageError::io(&staged.path, e))?;
        Ok(staged)
    }

    /// Keep the file: `Drop` will not unlink it.
    ///
    /// Used on exactly one path — a cancelled elevation, where the privileged
    /// command may still be reading this file. [`sweep_stale_staging`] is what
    /// collects it later.
    fn defuse(&mut self) {
        self.armed = false;
    }
}

impl Drop for StagedTemp {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Remove staging files a previous run left behind (see [`StagedTemp::defuse`]).
///
/// Only files older than `STAGING_SWEEP_AGE` go, so a concurrent sabrage's
/// live staging file is never taken out from under its own elevated write.
/// Best effort throughout: a file we cannot remove is a stale `0600` file in our
/// own `0700` directory, not a reason to fail an install.
fn sweep_stale_staging(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !(name.starts_with("host-manifest-") && name.ends_with(".json")) {
            continue;
        }
        let stale = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|age| age >= STAGING_SWEEP_AGE);
        if stale {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// `~/Library/Application Support/Sabrage/tmp` — the staging directory.
pub fn sabrage_temp_dir() -> PathBuf {
    sabrage_support_dir().join("tmp")
}

/// A fresh unpredictable staging filename.
fn temp_file_name() -> String {
    format!("host-manifest-{}.json", uuid::Uuid::new_v4().as_simple())
}

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
pub fn classify_write_error(err: &std::io::Error, path: &Path) -> WriteErrorKind {
    match err.kind() {
        std::io::ErrorKind::PermissionDenied if is_inside_app_bundle(path) => {
            WriteErrorKind::TccAppManagementLikely
        }
        std::io::ErrorKind::PermissionDenied => WriteErrorKind::PermissionDenied,
        _ => WriteErrorKind::Other,
    }
}

/// Turn a plain [`SabrageError::Io`] from install layers 1-2 into the App
/// Management error when the path and errno say that is the likely cause,
/// emitting the explanation as a [`StageEvent::Fatal`] on the way out; anything
/// else passes through untouched. Use it at the call site of a write inside
/// `CrossOver.app`:
///
/// ```ignore
/// exec.copy_if_changed(src, dst)
///     .await
///     .map_err(|e| privilege::upgrade_write_error(ctx, e))?;
/// ```
///
/// The returned error keeps its own `Display` (`kind() == "tcc_denied"`, which
/// the GUI's permission panel branches on) and the prose the user reads travels
/// in the emitted event, so a caller that gets `TccDenied` back must propagate
/// it rather than emit a second `Fatal`
/// (tests::only_a_tcc_shaped_io_error_is_upgraded).
pub fn upgrade_write_error(ctx: &StageCtx, err: SabrageError) -> SabrageError {
    let SabrageError::Io { path, source } = &err else {
        return err;
    };
    if classify_write_error(source, path) != WriteErrorKind::TccAppManagementLikely {
        return err;
    }
    let path = path.clone();
    ctx.emit(StageEvent::Fatal {
        run_id: ctx.run_id,
        message: app_management_message(&path),
        remedy: Some(app_management_remedy(ctx.opts.bottle_name.as_deref())),
        fix: None,
    });
    SabrageError::TccDenied { path }
}

/// [`upgrade_write_error`] for a failure that arrives as a child's exit status
/// rather than an `io::Error` — install layer 1's `cp -R` of the stock DXMT
/// tree, the first write the pipeline makes into `CrossOver.app`.
///
/// There is no errno to classify, only the child's output tail, so the test is
/// "destination inside a `.app` and the tail says permission denied". Same
/// contract as [`upgrade_write_error`] otherwise: the prose is emitted here,
/// once, and the caller propagates the returned [`SabrageError::TccDenied`]
/// (tests::a_refused_cp_into_a_bundle_is_upgraded_like_a_refused_write).
pub fn upgrade_child_write_error(ctx: &StageCtx, err: SabrageError, dest: &Path) -> SabrageError {
    let SabrageError::ChildFailed { tail, .. } = &err else {
        return err;
    };
    if !is_inside_app_bundle(dest) || !tail_is_permission_denied(tail) {
        return err;
    }
    ctx.emit(StageEvent::Fatal {
        run_id: ctx.run_id,
        message: app_management_message(dest),
        remedy: Some(app_management_remedy(ctx.opts.bottle_name.as_deref())),
        fix: None,
    });
    SabrageError::TccDenied {
        path: dest.to_path_buf(),
    }
}

/// True when a failed child's output tail reads like a refused write — `cp`'s
/// own `Permission denied` / `Operation not permitted` (the two spellings macOS
/// produces for `EACCES` and `EPERM`, and a sandbox refusal arrives as one of
/// them).
pub fn tail_is_permission_denied(tail: &[String]) -> bool {
    tail.iter().any(|line| {
        let line = line.to_lowercase();
        line.contains("permission denied") || line.contains("operation not permitted")
    })
}

/// The hypothesis, phrased as one.
pub fn app_management_message(path: &Path) -> String {
    format!(
        "cannot write {} — likely macOS App Management permission, which sudo cannot grant",
        path.display()
    )
}

/// The grant flow (deep link + the relaunch requirement) and the Terminal
/// fallback, on one `remedy:` line.
///
/// The relaunch note is load-bearing: macOS only applies a TCC grant to a
/// process started after it was given, so without it the user's experience is
/// "granted, still broken"
/// (tests::the_app_management_strings_stay_a_hypothesis_with_a_way_out).
pub fn app_management_remedy(bottle: Option<&str>) -> String {
    format!(
        "grant App Management to Sabrage in System Settings ({APP_MANAGEMENT_SETTINGS_URL}), relaunch Sabrage, then retry — or run {} in Terminal",
        demo_install_command(bottle)
    )
}

/// The always-available fallback: the same install, from a shell the user
/// already trusts.
pub fn demo_install_command(bottle: Option<&str>) -> String {
    format!("./demo.sh install --bottle {}", bottle.unwrap_or("<name>"))
}

/// doctor's `host.manifest` remedy, verbatim — the fallback for a declined or
/// failed authorization.
fn terminal_fallback_remedy(ctx: &StageCtx) -> String {
    format!(
        "{} (sudo writes it)",
        demo_install_command(ctx.opts.bottle_name.as_deref())
    )
}

/// True when any component of `path` ends in `.app` — the App Management
/// heuristic [`classify_write_error`] keys on.
pub fn is_inside_app_bundle(path: &Path) -> bool {
    path.components().any(|c| {
        Path::new(c.as_os_str())
            .extension()
            .is_some_and(|e| e == "app")
    })
}

/// Sabrage's own support directory, `~/Library/Application Support/Sabrage` —
/// where the temp file for the privileged write is staged (never `/tmp`: a
/// world-writable staging path for a root-installed file is a swap race).
///
/// One implementation, in [`crate::paths::sabrage_support_dir`]: two spellings
/// of one path is how the two front-ends of one store drift apart.
pub fn sabrage_support_dir() -> PathBuf {
    crate::paths::sabrage_support_dir()
}

#[cfg(test)]
mod tests;
