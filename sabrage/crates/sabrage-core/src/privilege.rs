//! The privilege boundary — the pipeline's one privileged write.
//!
//! **Exactly one privileged write exists in the whole pipeline**: install layer
//! 4, creating `/usr/local/share/openxr/1/` and writing
//! `active_runtime.x86_64.json` as `root:wheel 0644`. Everything else in setup /
//! build / install / run / stop is a plain user write, including install layers
//! 1–2 inside `CrossOver.app` — those need macOS **App Management (TCC)**, which
//! `sudo` cannot grant. Conflating the two is the classic mistake here; see
//! [`classify_write_error`].
//!
//! # The mechanism
//!
//! `osascript -e 'do shell script "…" with administrator privileges'`, one
//! prompt total (install.sh can prompt twice — once for `mkdir`, once for
//! `tee`). Alternatives were considered and rejected in design-core §5: an
//! `SMAppService` helper needs a stable Developer ID signature; an embedded
//! `sudo` over a pty is terrible GUI UX and needs a tty the `.app` does not have.
//! Rendering the exact `./demo.sh install` command for the user to paste stays
//! the fallback when authorization is declined.
//!
//! # The byte contract
//!
//! The content written is [`crate::util::host_manifest_file_bytes`] verbatim
//! (`print -- "$WANT"`'s trailing newline included), and the write must be
//! **skipped entirely** when
//! [`crate::util::host_manifest_is_current`] already holds — that is what keeps
//! `./demo.sh install` from re-prompting after Sabrage installed, and vice
//! versa. The JSON never rides on the command line: write it to a temp file
//! under Sabrage's own support directory and have the privileged command
//! `install -m 0644 -o root -g wheel <tmp> <dest>`.
//!
//! [`write_host_manifest_privileged`] takes the **dylib path**, not pre-rendered
//! content, and derives the bytes itself through [`host_manifest_bytes`]. That is
//! deliberate: the comparison form ([`crate::util::render_host_manifest`], no
//! trailing newline) and the file form differ by exactly one byte, and a
//! signature that accepts either is a seam a caller can — and once did — get
//! wrong, writing a host manifest that differs from `./demo.sh install`'s.
//!
//! # Hardening
//!
//! The staging file is the soft spot of any "write it as root from a temp file"
//! scheme, so:
//!
//! * it lives under `~/Library/Application Support/Sabrage/tmp/` (mode `0700`),
//!   **never** `/tmp` — a world-writable staging path for a root-installed file
//!   is a swap race;
//! * its name is randomized (`host-manifest-<uuid>.json`) and it is created
//!   `O_CREAT|O_EXCL` with mode `0600`, so an attacker cannot pre-create it;
//! * it is removed on every exit path ([`StagedTemp`]'s `Drop`) — with one
//!   deliberate exception: a **cancellation** defuses that `Drop`
//!   ([`StagedTemp::defuse`]) and leaves the file, because the elevated
//!   descendant runs as root and cannot be reliably killed or even observed by
//!   us; unlinking the source out from under a privileged `install` still in
//!   flight would turn a cancelled run into a corrupt one. The next privileged
//!   write sweeps anything older than [`STAGING_SWEEP_AGE`]
//!   ([`sweep_stale_staging`]);
//! * every argv element of the elevated command is single-quoted for `/bin/sh`
//!   ([`shell_quote`]) and the whole command is then escaped for the AppleScript
//!   string literal ([`applescript_escape`]) — two independent layers, both
//!   round-trip tested, because a repo path containing a quote would otherwise
//!   be arbitrary code execution *as root*;
//! * after the elevated command returns, the destination is re-read and
//!   byte-compared against the intended content. A mismatch is fatal: we would
//!   rather fail the stage than tell the user the host registration is in place
//!   when something else wrote it.
//!
//! # Which method fires where
//!
//! [`AdminMethod::detect`] picks `sudo` whenever a **controlling terminal** is
//! reachable — stdin is a tty, or `/dev/tty` opens — and never consults stdout.
//! `sudo` reads its password from `/dev/tty`, not from stdout, so
//! `sabrage install | tee log` still prompts in the terminal exactly like
//! `./demo.sh install | tee` does; keying off stdout would have popped a GUI
//! dialog for a piped terminal run. The two `die` strings on that path
//! (`sudo mkdir failed`, `sudo write failed`) are install.sh's, verbatim.
//!
//! A GUI-launched `.app` has no controlling terminal at all, so it takes the
//! osascript path, which has no shell counterpart and therefore no verbatim text
//! to match. (`cargo tauri dev` inherits the launching terminal, so the dev
//! build of the GUI gets the `sudo` prompt in that terminal — correct, and the
//! only way the two probes can disagree with "is this a GUI".)

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

// ── the commands we elevate ───────────────────────────────────────────────────

/// Absolute paths only: a GUI-launched `.app` inherits a bare `PATH`, and a
/// privileged command is the last place to let `PATH` decide what runs.
const OSASCRIPT: &str = "/usr/bin/osascript";
const SUDO: &str = "/usr/bin/sudo";
const MKDIR: &str = "/bin/mkdir";
const INSTALL: &str = "/usr/bin/install";

/// System Settings deep link for the App Management pane.
pub const APP_MANAGEMENT_SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_AppBundles";

/// What [`StageEvent::NeedsAdmin`] says before the **osascript** prompt appears
/// (design-core §5.4). The point is that the user can predict the dialog *and*
/// knows it will not come back on every install.
pub const NEEDS_ADMIN_REASON: &str =
    "this writes the host OpenXR registration — one password, only when the repo path changes";

/// The same announcement for the [`AdminMethod::Sudo`] path, which prompts on
/// the **controlling terminal**, not in a macOS dialog.
///
/// A GUI started from a shell (`npm run tauri dev`, the documented dev
/// workflow) inherits that terminal, so `detect()` picks `sudo` and the
/// password prompt appears in a window that is very likely *behind* the one the
/// user is looking at — the stage then looks hung. Saying where the prompt is
/// is the whole point of announcing it, so the announcement names the mechanism
/// [`AdminMethod::detect`] actually picked rather than one of the two.
pub const NEEDS_ADMIN_REASON_SUDO: &str = "sudo is waiting for your password in the terminal that launched Sabrage — this writes the host OpenXR registration, only when the repo path changes";

/// The announcement for `method`: [`NEEDS_ADMIN_REASON`] for the dialog,
/// [`NEEDS_ADMIN_REASON_SUDO`] for the terminal.
pub fn needs_admin_reason(method: AdminMethod) -> &'static str {
    match method {
        AdminMethod::Osascript => NEEDS_ADMIN_REASON,
        AdminMethod::Sudo => NEEDS_ADMIN_REASON_SUDO,
    }
}

/// The `--dry-run` stand-in for the prompt. A dry run must never emit
/// [`StageEvent::NeedsAdmin`]: the GUI renders that as "macOS will ask for your
/// password", which would be a lie about a run that touches nothing. Native-only
/// row (demo.sh has no `--dry-run`), in the same "would …" voice as the
/// executor's other planned-write rows.
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
/// Bounded on purpose: once `sudo` (or osascript's authorization) has exec'd,
/// the child's real uid is 0 and an unprivileged parent's `kill(2)` comes back
/// `EPERM`, so "wait for the child we just signalled" can be an unbounded wait
/// on a process that will never notice. Waiting *some* bounded time still reaps
/// the ordinary case (authorization not yet granted — the child is still ours).
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

// ── types ─────────────────────────────────────────────────────────────────────

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
    /// Deliberately **not** a function of stdout: `sabrage install | tee log`
    /// has a non-tty stdout and a perfectly good terminal to prompt on, and
    /// `./demo.sh install | tee` prompts there too. Either probe alone is
    /// sufficient — stdin redirected from a file (`sabrage install < /dev/null`)
    /// still leaves `/dev/tty` open, and a process whose stdin is a tty always
    /// has one.
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
///
/// The handle is dropped immediately; this only ever probes.
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

// ── quoting ───────────────────────────────────────────────────────────────────

/// Escape a string for embedding inside an AppleScript string literal.
///
/// `do shell script "…"` nests two levels of quoting: AppleScript's own literal
/// (backslash and double quote are the specials) wrapping a `/bin/sh` command
/// line. Paths here contain spaces routinely and could contain quotes; getting
/// this wrong is a command-injection bug, not a cosmetic one.
///
/// Newline, carriage return and tab are emitted as their AppleScript escapes
/// (`\n`, `\r`, `\t`) rather than raw: a raw newline inside an AppleScript
/// string literal is a *syntax error*, so a path containing one would otherwise
/// turn a hard failure into an unpredictable one.
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

/// The `/bin/sh` command the authorization dialog runs, both halves in one
/// invocation so there is exactly one prompt:
///
/// ```text
/// /bin/mkdir -p '<dir>' && /usr/bin/install -m 0644 -o root -g wheel '<tmp>' '<dest>'
/// ```
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
/// One vector for [`AdminMethod::Osascript`] (a single `osascript -e …`), two
/// for [`AdminMethod::Sudo`] (`sudo mkdir -p …`, then `sudo install …`, mirroring
/// install.sh's two sudo calls and its two `die` strings). Keeping this pure is
/// what lets the argv be unit-tested and lets `--dry-run` print exactly what a
/// real run would execute.
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

// ── the privileged write ──────────────────────────────────────────────────────

/// The exact bytes install layer 4 stages and installs: the rendered manifest
/// plus the single trailing newline `print -- "$WANT"` appends
/// ([`crate::util::host_manifest_file_bytes`]).
///
/// It exists as a named function so the byte source of the privileged write —
/// the pipeline's most drift-sensitive artifact — is something tests and
/// `sabrage-parity`'s golden can pin directly, instead of pinning the util
/// helper and *hoping* the write path calls it.
pub fn host_manifest_bytes(oxr_dylib: &Path) -> String {
    crate::util::host_manifest_file_bytes(oxr_dylib)
}

/// Write the host OpenXR manifest for `oxr_dylib`, prompting for authorization
/// only if needed.
///
/// 1. returns [`PrivilegedWrite::Skipped`] without prompting when `dest` is
///    already current ([`crate::util::host_manifest_is_current`] — install.sh's
///    own currency test, so neither front-end re-prompts after the other ran);
/// 2. emits [`StageEvent::NeedsAdmin`] *before* the prompt, so the UI can
///    explain it (a `--dry-run` prompts for nothing and says so instead);
/// 3. one prompt: `mkdir -p <dirname>` and `install -m 0644 -o root -g wheel`
///    in a single `do shell script` (the `sudo` path keeps install.sh's two
///    invocations and its two `die` strings);
/// 4. a declined prompt is [`SabrageError::AdminDeclined`], not a generic
///    failure — the UI offers the paste-this-in-Terminal fallback;
/// 5. re-reads `dest` and byte-compares it to the intended bytes; a mismatch is
///    fatal.
///
/// The bytes are derived here, from [`host_manifest_bytes`] — the parameter is
/// the **dylib path**, never pre-rendered content, so a caller cannot hand this
/// the newline-less comparison form ([`crate::util::render_host_manifest`]) and
/// silently write a file that differs from `./demo.sh install`'s by one byte.
///
/// The elevated child is the one mutation in the codebase that does **not** go
/// through [`crate::executor::Executor`]: the osascript branch needs its stderr
/// captured to tell "declined" from "failed", and the sudo branch needs the
/// process's own tty for the password prompt — `spawn_streamed` gives neither
/// (it pipes both streams and nulls stdin). A dry run therefore never reaches
/// this code: it records the same staging write and the same argv through the
/// executor and returns.
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

    // Nothing below this line is undoable from our side: `osascript`'s elevated
    // `/bin/sh` is not even in our process tree, and past `sudo`'s exec we
    // cannot signal it at all. So the token is checked *here*, before the
    // announcement, before the staging file, and before any child — a run
    // cancelled during layer 3 must not surface an authorization prompt.
    if ctx.cancel.is_cancelled() {
        return Err(SabrageError::Cancelled);
    }

    // The announcement names the mechanism `detect()` actually picked: the
    // dialog and the terminal prompt are in completely different places, and a
    // GUI launched from a shell takes the terminal one.
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
        // A cancelled elevation is the one case where we do **not** know the
        // privileged command is over: it runs as root, so our kill can come
        // back EPERM, and osascript's elevated `/bin/sh` is not even in our
        // process tree. Deleting the staging file here could hand a still-live
        // `install` an ENOENT half-way through the pipeline's only root write.
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
/// Reported as [`PrivilegedWrite::Planned`], never `Written`: the caller turns
/// the outcome into a row in the run log, and a preview that prints the same
/// `ok "host registration written"` a real install prints is indistinguishable
/// from completed work in the event log.
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
/// [`SabrageError::AdminDeclined`] from a real failure. Everything else on
/// stderr is surfaced as [`StageEvent::Output`] rather than swallowed
/// (design-core §6.5).
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

/// install.sh's sudo path, structurally unchanged:
///
/// ```zsh
/// sudo mkdir -p "$(dirname "$HOST_XR_JSON")" || die "sudo mkdir failed"
/// print -- "$WANT" | sudo tee "$HOST_XR_JSON" >/dev/null || die "sudo write failed"
/// ```
///
/// `install -m 0644 -o root -g wheel` replaces `tee` — same resulting bytes,
/// mode and ownership, without the JSON ever crossing a pipe or a command line.
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
/// install.sh's two `${//}` substitutions escape `\` and `"` and **nothing
/// else**, so a checkout under a path with a tab or a newline in it renders a
/// manifest that is not JSON — and this is the one write that installs its
/// output as `root:wheel` over the file the OpenXR loader reads. Failing
/// closed beats breaking runtime discovery for every app on the machine.
///
/// This runs *before* [`crate::util::render_host_manifest`].
/// [`crate::util::json_escape_string`] is deliberately exactly install.sh's two
/// substitutions (`\` and `"`), so a control character would reach the
/// manifest raw on both sides — as invalid JSON. Refusing here is what keeps
/// the native side from writing that file at all.
///
/// Native-only, and a deliberate divergence: install.sh writes the invalid
/// bytes. It cannot change any existing artifact — every path without a
/// control character is accepted unchanged.
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

// ── children ──────────────────────────────────────────────────────────────────

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
            // Give the child a bounded moment to finish before the future (and
            // with it the child, `kill_on_drop`) is dropped: returning the
            // instant Stop is pressed would let the caller tear the staging
            // file down under a privileged `install` that is already running.
            // Bounded because the elevated half is root-owned and may outlive
            // us regardless — see CANCEL_REAP_GRACE.
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
            // Signal, then *reap* — bounded. Without the wait this returned
            // while the child (and, past the password prompt, its root-owned
            // `mkdir`/`install`) was still running, and the caller's staging
            // file was unlinked underneath it. The kill itself is best-effort
            // for the same reason: after `sudo` execs, its real uid is 0 and
            // this process cannot signal it at all.
            let _ = child.start_kill();
            let _ = tokio::time::timeout(CANCEL_REAP_GRACE, child.wait()).await;
            Err(SabrageError::Cancelled)
        }
    }
}

// ── staging ───────────────────────────────────────────────────────────────────

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
    /// `create_new` is the load-bearing flag: it fails rather than reusing a
    /// path something else pre-created, and combined with the random name it
    /// means the bytes root installs can only have come from us.
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

/// Remove staging files a previous run left behind (see
/// [`StagedTemp::defuse`]), oldest-first safe: only files older than
/// [`STAGING_SWEEP_AGE`] go, so a *concurrent* sabrage's live staging file is
/// never taken out from under its own elevated write.
///
/// Best effort throughout — a staging file we cannot remove is a stale `0600`
/// file in our own `0700` directory, not a reason to fail an install.
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

// ── failure classification ────────────────────────────────────────────────────

/// Classify an `io::Error` from a write to `path`.
///
/// The `.app` test is on the path, not the error: a `PermissionDenied` under
/// `…/CrossOver.app/…` is App Management, and telling the user to try `sudo`
/// there would send them down a road with no end.
///
/// [`WriteErrorKind::TccAppManagementLikely`] is a **hypothesis**, never a
/// diagnosis — macOS reports a TCC refusal as a plain `EPERM`, indistinguishable
/// from a genuine mode problem — which is why every string built from it says
/// "likely" and offers the Terminal fallback alongside the grant flow.
pub fn classify_write_error(err: &std::io::Error, path: &Path) -> WriteErrorKind {
    match err.kind() {
        std::io::ErrorKind::PermissionDenied if is_inside_app_bundle(path) => {
            WriteErrorKind::TccAppManagementLikely
        }
        std::io::ErrorKind::PermissionDenied => WriteErrorKind::PermissionDenied,
        _ => WriteErrorKind::Other,
    }
}

/// Turn a plain [`SabrageError::Io`] from install layers 1–2 into the App
/// Management error when the path and errno say that is the likely cause,
/// emitting the explanation as a [`StageEvent::Fatal`] on the way out.
///
/// Everything else passes through untouched. Use it at the call site of a write
/// inside `CrossOver.app`:
///
/// ```ignore
/// exec.copy_if_changed(src, dst)
///     .await
///     .map_err(|e| privilege::upgrade_write_error(ctx, e))?;
/// ```
///
/// The returned error keeps its own `Display` (`kind() == "tcc_denied"`, which
/// is what the GUI's permission panel branches on); the prose the *user* reads
/// travels in the emitted event, exactly like [`StageCtx::fatal`]. A caller that
/// gets `TccDenied` back must therefore propagate it rather than emitting a
/// second `Fatal`.
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

/// [`upgrade_write_error`] for a failure that arrives as a **child's** exit
/// status rather than an `io::Error` — install layer 1's `cp -R` of the stock
/// DXMT tree, the first write the pipeline makes into `CrossOver.app` and so
/// the most likely place to meet App Management.
///
/// There is no errno to classify, only the child's output tail, so the test is
/// "destination inside a `.app` **and** the tail says permission denied". Same
/// contract as [`upgrade_write_error`] in every other respect: the prose is
/// emitted here, once, as a [`StageEvent::Fatal`], and the caller propagates
/// the returned [`SabrageError::TccDenied`] instead of re-emitting. Anything
/// else passes through untouched — a `ChildFailed` already explains itself.
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
/// The relaunch note is not optional trivia: macOS only re-evaluates a TCC grant
/// for a process started after it was given, so "granted, still broken" is the
/// default experience without it.
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
/// One implementation, in [`crate::paths::sabrage_support_dir`]: Phase 3's
/// session-state file lands in the same directory, and two spellings of one
/// path is how the two front-ends of one store drift apart.
pub fn sabrage_support_dir() -> PathBuf {
    crate::paths::sabrage_support_dir()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::PlannedKind;
    use crate::paths::Paths;
    use crate::stages::{EventSink, StageOptions};
    use std::sync::{Arc, Mutex as StdMutex};

    // NOTE: nothing in this module may execute `osascript` or `sudo`. Every
    // test either exercises a pure function, stages a file in a fixture
    // directory, or drives the dry-run path — which records argv through the
    // executor and spawns nothing.

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

    // ── test-only inverses, so the round-trip is proven and not asserted ──────

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

    // ── quoting ──────────────────────────────────────────────────────────────

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

    // ── staging ──────────────────────────────────────────────────────────────

    #[test]
    fn staged_temp_is_0600_and_deletes_itself() {
        let dir = scratch("staging");
        let path;
        {
            let staged =
                StagedTemp::create(&dir, "{\"file_format_version\": \"1.0.0\"}\n").unwrap();
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

    // ── the write itself ─────────────────────────────────────────────────────

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

        // Nothing happened on disk.
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

    // ── cancellation: reap the child, keep the staging file ──────────────────

    /// The cancel arm must not return while the child it just signalled is
    /// still running: the caller's next act is to drop the staging file, and a
    /// privileged `install` reading it would get an ENOENT half-way through the
    /// pipeline's only root write. `sleep` + `touch` is the observable form —
    /// if the wait were missing, the marker could appear after the call
    /// returned.
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
        // Cancelled *after* the child is up: an already-cancelled token never
        // reaches the spawn at all (see the test below), so pre-cancelling here
        // would prove nothing about the reap.
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

    /// How many `host-manifest-*.json` are staged right now. Read-only: the
    /// point of the assertion above is that this number does not move.
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

    /// install.sh's escaping is two substitutions and nothing else, so a path
    /// carrying a raw control character renders a manifest that is not JSON —
    /// and this is the one write that lands as `root:wheel` over the file the
    /// OpenXR loader reads. It fails closed, before the currency test and
    /// before any prompt; `json_escape_string` itself stays exactly the shell's
    /// two substitutions, so every accepted path renders byte-identically on
    /// both front-ends.
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
        // …and an ordinary path is untouched by the guard: every artifact on
        // every existing machine renders exactly as before.
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

    // ── the announced mechanism ──────────────────────────────────────────────

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
        // design-core §5.4's promise.
        for method in [AdminMethod::Osascript, AdminMethod::Sudo] {
            assert!(
                needs_admin_reason(method).contains("host OpenXR registration")
                    && needs_admin_reason(method).contains("repo path changes"),
                "{}",
                needs_admin_reason(method)
            );
        }
    }

    // ── cp -R refusals (install layer 1) ─────────────────────────────────────

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

    // ── classification ───────────────────────────────────────────────────────

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

    // ── frame-era invariants ─────────────────────────────────────────────────

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

    // ── the byte source (the one artifact that must not drift) ───────────────

    #[test]
    fn the_write_paths_bytes_are_the_file_form_not_the_comparison_form() {
        let dylib = Path::new("/repo/ext/oxrsys/build-x64/runtime/liboxrsys-runtime.dylib");
        let bytes = host_manifest_bytes(dylib);
        assert_eq!(bytes, crate::util::host_manifest_file_bytes(dylib));
        // install.sh's `print -- "$WANT"`: the live file ends `7d 0a 7d 0a`.
        assert!(bytes.ends_with("}\n"), "{bytes:?}");
        assert_eq!(
            bytes,
            format!("{}\n", crate::util::render_host_manifest(dylib)),
            "exactly one newline more than the comparison form"
        );
    }
}
