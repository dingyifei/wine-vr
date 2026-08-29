//! Child processes: spawn, stream, cancel, and the reap primitive.
//!
//! # Scope warning — build tools only
//!
//! [`spawn_streamed`] sets `kill_on_drop(true)` and puts the child in its own
//! process group so cancellation can signal the whole tree. That is correct for
//! `git`, `cmake`, `ninja`, `cargo`, `curl`, `tar`, `adb`, `wineserver` — and
//! **wrong for the wine launch in Phase 3**. Cancelling `run` means the
//! INT-trap path: `wineserver -k` plus a bounded `-w` wait, never a SIGKILL of
//! the game, and the log is a file fd the child writes to directly rather than a
//! pipe this process pumps. The run stage must therefore build its own
//! `tokio::process::Command` with `kill_on_drop(false)` and a file-fd redirect;
//! it must not call [`spawn_streamed`].
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
//! `pkill -f`, which will happily match an unrelated process that merely
//! mentions the path on its command line (a `tail -f` of the log, an editor, the
//! very shell running doctor). This is a declared divergence — PARITY.md,
//! "Stop"; design-core §10.7 — and the GUI shows what will
//! be killed before killing it.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nix::sys::signal::{killpg, Signal};
use nix::unistd::Pid;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;
use tokio_util::sync::CancellationToken;

use crate::error::{Result, SabrageError, CHILD_TAIL_LINES};
use crate::events::{RunId, StageEvent, StepId, Stream};
use crate::stages::EventSink;

/// Grace period between `SIGTERM` and `SIGKILL` for a cancelled child.
pub const DEFAULT_KILL_GRACE: Duration = Duration::from_secs(5);

// ── spec ──────────────────────────────────────────────────────────────────────

/// Everything needed to spawn one child.
///
/// `run_id` is part of the spec (rather than a separate `spawn_streamed`
/// argument) because every [`StageEvent::Output`] the child produces carries it;
/// [`crate::stages::StageCtx::child`] fills it in for you.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildSpec {
    /// Program to run: a bare name resolved through `PATH`, or an absolute path.
    pub program: OsString,
    pub args: Vec<OsString>,
    /// Working directory. `None` inherits this process's.
    pub cwd: Option<PathBuf>,
    /// Environment overlay applied on top of the inherited environment.
    pub env: Vec<(String, String)>,
    /// Replacement `PATH`. A Finder-launched `.app` inherits a bare `PATH` that
    /// has neither Homebrew nor rustup on it — see [`default_child_path`].
    pub env_path: Option<String>,
    /// The step this child belongs to; every output chunk is attributed to it.
    pub step: StepId,
    /// SIGTERM→SIGKILL grace period on cancellation.
    pub kill_grace: Duration,
    /// The run this child belongs to.
    pub run_id: RunId,
}

impl ChildSpec {
    /// A spec with no args, inherited cwd/env, and the default kill grace.
    pub fn new(program: impl Into<OsString>, step: StepId, run_id: RunId) -> ChildSpec {
        ChildSpec {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            env: Vec::new(),
            env_path: None,
            step,
            kill_grace: DEFAULT_KILL_GRACE,
            run_id,
        }
    }

    /// Append one argument (accepts `&str`, `String`, `&Path`, `PathBuf`).
    pub fn arg(mut self, a: impl Into<OsString>) -> ChildSpec {
        self.args.push(a.into());
        self
    }

    /// Append several arguments.
    pub fn args<I, S>(mut self, args: I) -> ChildSpec
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Set the working directory.
    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> ChildSpec {
        self.cwd = Some(dir.into());
        self
    }

    /// Add one environment variable.
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> ChildSpec {
        self.env.push((key.into(), value.into()));
        self
    }

    /// Replace `PATH` for this child.
    pub fn env_path(mut self, path: impl Into<String>) -> ChildSpec {
        self.env_path = Some(path.into());
        self
    }

    /// Override the SIGTERM→SIGKILL grace period.
    pub fn kill_grace(mut self, grace: Duration) -> ChildSpec {
        self.kill_grace = grace;
        self
    }

    /// The program name as it appears in error messages.
    pub fn argv0(&self) -> String {
        self.program.to_string_lossy().into_owned()
    }

    /// The whole command line, space-joined — for tracing and for the "copy the
    /// equivalent shell command" affordance. Not shell-quoted: it is a display
    /// string, never something to re-execute.
    pub fn display(&self) -> String {
        let mut s = self.argv0();
        for a in &self.args {
            s.push(' ');
            s.push_str(&a.to_string_lossy());
        }
        s
    }
}

/// `PATH` for children of a GUI-launched Sabrage: Homebrew, `/usr/local`,
/// rustup's `~/.cargo/bin` (which is *not* on the login `PATH` on this machine),
/// and the Android SDK platform-tools, ahead of whatever this process inherited.
///
/// `demo.sh` never needed this — it runs from a login shell that already has
/// them. A `.app` double-clicked in Finder inherits a bare `PATH` and would fail
/// `build`'s very first tool probe.
pub fn default_child_path() -> String {
    let home = crate::paths::home_dir();
    let mut parts: Vec<String> = vec![
        "/opt/homebrew/bin".to_string(),
        "/usr/local/bin".to_string(),
        home.join(".cargo/bin").display().to_string(),
        home.join("Library/Android/sdk/platform-tools")
            .display()
            .to_string(),
    ];
    if let Ok(inherited) = std::env::var("PATH") {
        parts.extend(inherited.split(':').map(str::to_string));
    }
    let mut seen = std::collections::BTreeSet::new();
    parts.retain(|p| !p.is_empty() && seen.insert(p.clone()));
    parts.join(":")
}

// ── output splitting ──────────────────────────────────────────────────────────

/// Splits a byte stream into chunks on `\n` **and** `\r`, counting `\r\n` once.
///
/// Progress-bar writers (curl, cargo, git) repaint by emitting `\r`; a plain
/// line splitter would buffer the entire download into a single chunk delivered
/// at EOF. Empty chunks are preserved (a blank line in build output is real
/// output), except for the phantom one `\r\n` would otherwise produce.
#[derive(Debug, Default)]
pub struct ChunkSplitter {
    buf: Vec<u8>,
    pending_cr: bool,
}

impl ChunkSplitter {
    pub fn new() -> ChunkSplitter {
        ChunkSplitter::default()
    }

    /// Feed bytes, calling `out` once per completed chunk.
    pub fn push(&mut self, bytes: &[u8], out: &mut impl FnMut(String)) {
        for &b in bytes {
            if self.pending_cr {
                self.pending_cr = false;
                if b == b'\n' {
                    continue; // CRLF: the CR already flushed this chunk
                }
            }
            match b {
                b'\n' => {
                    out(self.take());
                }
                b'\r' => {
                    out(self.take());
                    self.pending_cr = true;
                }
                _ => self.buf.push(b),
            }
        }
    }

    /// Flush whatever is buffered at EOF (a final chunk with no delimiter).
    pub fn finish(&mut self, out: &mut impl FnMut(String)) {
        if !self.buf.is_empty() {
            out(self.take());
        }
        self.pending_cr = false;
    }

    fn take(&mut self) -> String {
        let s = String::from_utf8_lossy(&self.buf).into_owned();
        self.buf.clear();
        s
    }
}

// ── spawn ─────────────────────────────────────────────────────────────────────

/// The last [`CHILD_TAIL_LINES`] output chunks, shared by both pumps.
type Tail = Arc<Mutex<VecDeque<String>>>;

/// Spawn `spec`, streaming every stdout/stderr chunk to `sink` as a
/// [`StageEvent::Output`], and wait for it.
///
/// Cancellation: `SIGTERM` to the child's **process group**, then `SIGKILL`
/// after `spec.kill_grace`, then [`SabrageError::Cancelled`]. Build tools spawn
/// their own children (cmake→ninja→cc, cargo→rustc); signalling the group is
/// what actually stops them.
///
/// Returns the raw [`ExitStatus`] — a non-zero exit is **not** an error here.
/// Use [`run_ok`] where the shell would `die` on failure; a few sites
/// legitimately tolerate a non-zero child (`grep`, `pgrep`, `wineserver -k`).
pub async fn spawn_streamed(
    spec: &ChildSpec,
    sink: &EventSink,
    cancel: &CancellationToken,
) -> Result<ExitStatus> {
    let (status, _tail) = spawn_streamed_inner(spec, sink, cancel).await?;
    Ok(status)
}

/// [`spawn_streamed`], mapping a non-zero exit to
/// [`SabrageError::ChildFailed`] with the last [`CHILD_TAIL_LINES`] output
/// chunks attached (design-core §6.5: a failing child explains itself instead of
/// leaving a bare exit code).
pub async fn run_ok(
    spec: &ChildSpec,
    sink: &EventSink,
    cancel: &CancellationToken,
) -> Result<ExitStatus> {
    let (status, tail) = spawn_streamed_inner(spec, sink, cancel).await?;
    if status.success() {
        Ok(status)
    } else {
        Err(SabrageError::ChildFailed {
            argv0: spec.argv0(),
            status: exit_code_of(status),
            tail,
        })
    }
}

/// A child's exit code, or `128 + signal` when it died from a signal (the shell
/// convention, so `exit_code_equiv` reads the same on both sides).
pub fn exit_code_of(status: ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    status
        .code()
        .or_else(|| status.signal().map(|s| 128 + s))
        .unwrap_or(1)
}

/// Build an `ExitStatus` from a raw code — the dry-run executor's "as if it
/// succeeded" value.
pub fn exit_status_from_code(code: i32) -> ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    ExitStatus::from_raw(code << 8)
}

async fn spawn_streamed_inner(
    spec: &ChildSpec,
    sink: &EventSink,
    cancel: &CancellationToken,
) -> Result<(ExitStatus, Vec<String>)> {
    let mut cmd = tokio::process::Command::new(&spec.program);
    cmd.args(&spec.args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Own process group: cancellation signals the whole tool tree.
        .process_group(0)
        // Belt and braces if the future is dropped without cancellation.
        .kill_on_drop(true);
    if let Some(dir) = &spec.cwd {
        cmd.current_dir(dir);
    }
    for (k, v) in &spec.env {
        cmd.env(k, v);
    }
    if let Some(path) = &spec.env_path {
        cmd.env("PATH", path);
    }

    let mut child = cmd.spawn().map_err(|e| {
        // A missing tool is the single most common failure here; keep the path
        // in the message so it is obvious which one.
        SabrageError::io(PathBuf::from(&spec.program), e)
    })?;
    let pgid = child
        .id()
        .map(|id| Pid::from_raw(id as i32))
        .ok_or_else(|| {
            SabrageError::fatal_bare(format!(
                "{} exited before it could be supervised",
                spec.argv0()
            ))
        })?;

    let tail: Tail = Arc::new(Mutex::new(VecDeque::with_capacity(CHILD_TAIL_LINES)));
    let mut pumps = Vec::new();
    if let Some(out) = child.stdout.take() {
        pumps.push(tokio::spawn(pump(
            out,
            Stream::Stdout,
            spec.run_id,
            spec.step,
            sink.clone(),
            tail.clone(),
        )));
    }
    if let Some(err) = child.stderr.take() {
        pumps.push(tokio::spawn(pump(
            err,
            Stream::Stderr,
            spec.run_id,
            spec.step,
            sink.clone(),
            tail.clone(),
        )));
    }

    let cancelled;
    let status = tokio::select! {
        waited = child.wait() => {
            cancelled = false;
            waited.map_err(|e| SabrageError::io(PathBuf::from(&spec.program), e))?
        }
        _ = cancel.cancelled() => {
            cancelled = true;
            let _ = killpg(pgid, Signal::SIGTERM);
            match tokio::time::timeout(spec.kill_grace, child.wait()).await {
                Ok(Ok(st)) => st,
                _ => {
                    let _ = killpg(pgid, Signal::SIGKILL);
                    child
                        .wait()
                        .await
                        .map_err(|e| SabrageError::io(PathBuf::from(&spec.program), e))?
                }
            }
        }
    };

    // The pipes close when the child (and its group) is gone; drain them so no
    // output is lost, then snapshot the tail.
    for p in pumps {
        let _ = p.await;
    }
    let tail_lines: Vec<String> = tail
        .lock()
        .map(|t| t.iter().cloned().collect())
        .unwrap_or_default();

    if cancelled {
        return Err(SabrageError::Cancelled);
    }
    Ok((status, tail_lines))
}

async fn pump<R>(
    mut reader: R,
    stream: Stream,
    run_id: RunId,
    step: StepId,
    sink: EventSink,
    tail: Tail,
) where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut splitter = ChunkSplitter::new();
    let mut buf = [0u8; 8192];
    let mut emit = |chunk: String| {
        if let Ok(mut t) = tail.lock() {
            if t.len() == CHILD_TAIL_LINES {
                t.pop_front();
            }
            t.push_back(chunk.clone());
        }
        sink(StageEvent::Output {
            run_id,
            step: step.to_string(),
            stream,
            chunk,
        });
    };
    loop {
        let n = match reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        splitter.push(&buf[..n], &mut emit);
    }
    splitter.finish(&mut emit);
}

// ── reaping ───────────────────────────────────────────────────────────────────

/// One matched process.
///
/// Serializable because it is the **process identity** persisted in
/// `session-state.json` ([`crate::session::state::SessionState`]): after a
/// crash, `pid` alone cannot say whether the wine process still running under
/// that number is *the* wine process this Sabrage launched, and signalling a
/// recycled pid is the one unrecoverable mistake the reconcile path can make.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcInfo {
    pub pid: u32,
    /// Seconds since the epoch, as reported by the OS. Distinguishes a recycled
    /// pid from the process actually observed.
    pub start_time: u64,
    /// The resolved executable path that matched.
    pub exe: PathBuf,
}

impl ProcInfo {
    /// Observe one live pid, or `None` when the OS has no such process.
    ///
    /// Refreshes exactly that pid rather than the whole table
    /// ([`find_processes_by_exe`] must scan everything; this must not).
    pub fn observe(pid: u32) -> Option<ProcInfo> {
        use sysinfo::{
            Pid as SysPid, ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System, UpdateKind,
        };

        let refresh = ProcessRefreshKind::nothing().with_exe(UpdateKind::Always);
        let mut sys = System::new_with_specifics(RefreshKind::nothing().with_processes(refresh));
        let target = SysPid::from_u32(pid);
        sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[target]), true, refresh);
        let proc_ = sys.process(target)?;
        Some(ProcInfo {
            pid,
            start_time: proc_.start_time(),
            exe: proc_.exe().map(Path::to_path_buf).unwrap_or_default(),
        })
    }

    /// Is the process this identity names **still the same process**?
    ///
    /// True iff the pid is alive *and* the pid observed right now reports the
    /// same `start_time`. That pair is the recycled-pid guard: pids wrap, start
    /// times do not.
    ///
    /// `exe` is deliberately **not** compared. CrossOver's `wine` launcher
    /// `exec`s into the real loader, which replaces the executable image —
    /// same pid, same start time, a different `exe` path — so an exe equality
    /// test would classify every live session as "not mine" a few hundred
    /// milliseconds after launch.
    ///
    /// A `start_time` of 0 is the "could not observe at spawn" fallback
    /// [`crate::executor::Executor::spawn_detached`] records; it can never
    /// equal a real start time, so such an identity reports `false` here — the
    /// conservative answer (treat it as an identity mismatch, never signal it).
    pub fn is_same_process(&self) -> bool {
        if !is_alive(self.pid) {
            return false;
        }
        match ProcInfo::observe(self.pid) {
            Some(current) => current.start_time == self.start_time,
            None => false,
        }
    }
}

/// Every running process whose executable **is** `path`, ordered by pid.
///
/// Both sides are canonicalized before comparison, so a symlinked repo or a
/// `/private/var` vs `/var` difference still matches; a path that cannot be
/// canonicalized falls back to literal equality.
///
/// This replaces `pgrep -f <path>` — see this module's header for why.
pub fn find_processes_by_exe(path: &Path) -> Vec<ProcInfo> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System, UpdateKind};

    let want = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let refresh = ProcessRefreshKind::nothing().with_exe(UpdateKind::Always);
    let mut sys = System::new_with_specifics(RefreshKind::nothing().with_processes(refresh));
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh);

    let mut found: Vec<ProcInfo> = sys
        .processes()
        .iter()
        .filter_map(|(pid, proc_)| {
            let exe = proc_.exe()?;
            let resolved = exe.canonicalize().unwrap_or_else(|_| exe.to_path_buf());
            (resolved == want).then(|| ProcInfo {
                pid: pid.as_u32(),
                start_time: proc_.start_time(),
                exe: resolved,
            })
        })
        .collect();
    found.sort_by_key(|p| p.pid);
    found
}

/// True when any element of `cmd` — or the whitespace-joined command line as a
/// whole, the shape `pgrep -f` scans — contains `needle`.
///
/// Wine puts the game's own path on the command line as a single `Z:\...`
/// Windows-path argument (e.g. `Z:\repo\…\Beat Saber.exe`), so per-element and
/// whole-line matching agree in the real case; both are checked so a
/// hypothetical split across two argv elements still matches.
///
/// Lives here (rather than beside its first caller in
/// [`crate::stages::stop`]) because Phase 3 needs the same shape twice more:
/// run.sh's wineserver-timeout warning quotes `pgrep -lf wineserver`, and the
/// cancellation teardown probes for survivors the same way.
pub fn cmdline_contains(cmd: &[String], needle: &str) -> bool {
    cmd.iter().any(|arg| arg.contains(needle)) || cmd.join(" ").contains(needle)
}

/// Every running process whose command line matches [`cmdline_contains`] for
/// `needle`, pid-ordered — the argv-based match `pgrep -f` performs, unlike
/// [`find_processes_by_exe`]'s exact exe-path equality (used by the reap
/// steps). See this module's header, and PARITY.md "Stop", for why the two
/// probes make opposite trade-offs on purpose.
pub fn find_processes_by_cmdline(needle: &str) -> Vec<ProcInfo> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System, UpdateKind};

    let refresh = ProcessRefreshKind::nothing()
        .with_exe(UpdateKind::Always)
        .with_cmd(UpdateKind::Always);
    let mut sys = System::new_with_specifics(RefreshKind::nothing().with_processes(refresh));
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh);

    let mut found: Vec<ProcInfo> = sys
        .processes()
        .iter()
        .filter_map(|(pid, proc_)| {
            let cmd: Vec<String> = proc_
                .cmd()
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            cmdline_contains(&cmd, needle).then(|| ProcInfo {
                pid: pid.as_u32(),
                start_time: proc_.start_time(),
                exe: proc_.exe().map(Path::to_path_buf).unwrap_or_default(),
            })
        })
        .collect();
    found.sort_by_key(|p| p.pid);
    found
}

// ── read-only probes ──────────────────────────────────────────────────────────

/// What [`capture`] came back with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Captured {
    pub status: ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

impl Captured {
    /// `stdout` with trailing newlines stripped, the way `$(…)` capture works.
    pub fn stdout_trimmed(&self) -> &str {
        self.stdout.trim_end_matches('\n')
    }
}

/// Run a **read-only probe** and capture both pipes.
///
/// For the handful of run-stage probes whose *output* is the point and whose
/// effect is nil: `adb devices`, `adb forward --list`,
/// `SwitchAudioSource -a -t output`, `SwitchAudioSource -c -t output`. Those
/// are the same class of thing the check layer already shells out to directly,
/// and streaming them as [`StageEvent::Output`] would print machine-readable
/// noise the shell never prints.
///
/// **Mutations must never come through here.** `adb forward`,
/// `adb forward --remove`, `adb reverse --remove-all`,
/// `SwitchAudioSource -t output -s …`, `wineserver -k` and every other write
/// go through [`crate::executor::Executor::run_child`], which is what makes
/// `--dry-run` plan them instead of performing them. A probe routed through
/// this function is invisible to the plan — correct for a probe, a silent
/// dry-run hole for anything else.
///
/// stdin is `/dev/null`; there is no process group and no cancellation hook
/// (these all return in milliseconds), and `spec.env_path` is applied so a
/// Finder-launched `.app` still finds `adb` (see [`default_child_path`]).
pub async fn capture(spec: &ChildSpec) -> Result<Captured> {
    let mut cmd = tokio::process::Command::new(&spec.program);
    cmd.args(&spec.args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = &spec.cwd {
        cmd.current_dir(dir);
    }
    for (k, v) in &spec.env {
        cmd.env(k, v);
    }
    if let Some(path) = &spec.env_path {
        cmd.env("PATH", path);
    }
    let out = cmd
        .output()
        .await
        .map_err(|e| SabrageError::io(PathBuf::from(&spec.program), e))?;
    Ok(Captured {
        status: out.status,
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

fn signal_pid(pid: u32, sig: Signal) -> std::io::Result<()> {
    nix::sys::signal::kill(Pid::from_raw(pid as i32), sig)
        .map_err(|e| std::io::Error::from_raw_os_error(e as i32))
}

/// `SIGTERM` one process.
pub fn terminate(pid: u32) -> std::io::Result<()> {
    signal_pid(pid, Signal::SIGTERM)
}

/// `SIGKILL` one process.
pub fn kill_hard(pid: u32) -> std::io::Result<()> {
    signal_pid(pid, Signal::SIGKILL)
}

/// `kill(pid, 0)`: does the process still exist?
pub fn is_alive(pid: u32) -> bool {
    nix::sys::signal::kill(Pid::from_raw(pid as i32), None).is_ok()
}

/// `SIGTERM`, poll for up to `grace`, then `SIGKILL`. Returns `true` when the
/// process is gone by the end.
pub async fn terminate_and_wait(pid: u32, grace: Duration) -> bool {
    if terminate(pid).is_err() && !is_alive(pid) {
        return true;
    }
    let deadline = tokio::time::Instant::now() + grace;
    while tokio::time::Instant::now() < deadline {
        if !is_alive(pid) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let _ = kill_hard(pid);
    tokio::time::sleep(Duration::from_millis(100)).await;
    !is_alive(pid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::step;
    use std::sync::Mutex as StdMutex;
    use uuid::Uuid;

    fn split(input: &[&[u8]]) -> Vec<String> {
        let mut out = Vec::new();
        let mut s = ChunkSplitter::new();
        for part in input {
            s.push(part, &mut |c| out.push(c));
        }
        s.finish(&mut |c| out.push(c));
        out
    }

    #[test]
    fn splits_on_lf_cr_and_crlf() {
        assert_eq!(split(&[b"a\nb\n"]), vec!["a", "b"]);
        // curl's progress bar: CR-separated repaints, no trailing newline.
        assert_eq!(split(&[b"10%\r50%\r100%"]), vec!["10%", "50%", "100%"]);
        // CRLF counts once.
        assert_eq!(split(&[b"a\r\nb\r\n"]), vec!["a", "b"]);
        // Blank lines survive.
        assert_eq!(split(&[b"a\n\nb\n"]), vec!["a", "", "b"]);
        // A chunk may straddle two reads, including across the CRLF pair.
        assert_eq!(split(&[b"ab\r", b"\ncd\n"]), vec!["ab", "cd"]);
        assert_eq!(split(&[b"partial"]), vec!["partial"]);
        assert!(split(&[b""]).is_empty());
    }

    fn collecting_sink() -> (EventSink, Arc<StdMutex<Vec<StageEvent>>>) {
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let s = seen.clone();
        let sink: EventSink = Arc::new(move |ev| s.lock().unwrap().push(ev));
        (sink, seen)
    }

    fn spec(program: &str, run_id: Uuid) -> ChildSpec {
        ChildSpec::new(program, step::BUILD_TOOLS, run_id)
    }

    #[tokio::test]
    async fn streams_output_and_reports_exit_status() {
        let run_id = Uuid::new_v4();
        let (sink, seen) = collecting_sink();
        let cancel = CancellationToken::new();
        let s = spec("/bin/sh", run_id)
            .arg("-c")
            .arg("printf 'one\\ntwo\\n'; printf 'err\\n' >&2");
        let status = spawn_streamed(&s, &sink, &cancel).await.unwrap();
        assert!(status.success());

        let evs = seen.lock().unwrap().clone();
        let mut out: Vec<(Stream, String)> = evs
            .iter()
            .filter_map(|e| match e {
                StageEvent::Output {
                    stream,
                    chunk,
                    step,
                    run_id: r,
                } => {
                    assert_eq!(step, step::BUILD_TOOLS);
                    assert_eq!(*r, run_id);
                    Some((*stream, chunk.clone()))
                }
                _ => None,
            })
            .collect();
        out.sort_by(|a, b| a.1.cmp(&b.1));
        assert_eq!(
            out,
            vec![
                (Stream::Stderr, "err".to_string()),
                (Stream::Stdout, "one".to_string()),
                (Stream::Stdout, "two".to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn run_ok_attaches_the_output_tail_on_failure() {
        let run_id = Uuid::new_v4();
        let (sink, _seen) = collecting_sink();
        let cancel = CancellationToken::new();
        let s = spec("/bin/sh", run_id)
            .arg("-c")
            .arg("echo boom >&2; exit 3");
        let err = run_ok(&s, &sink, &cancel).await.unwrap_err();
        match &err {
            SabrageError::ChildFailed {
                argv0,
                status,
                tail,
            } => {
                assert_eq!(argv0, "/bin/sh");
                assert_eq!(*status, 3);
                assert_eq!(tail, &vec!["boom".to_string()]);
            }
            other => panic!("expected ChildFailed, got {other:?}"),
        }
        assert_eq!(err.exit_code(), 1);
        assert_eq!(err.kind(), "child_failed");
    }

    #[tokio::test]
    async fn cancellation_kills_the_process_group() {
        let run_id = Uuid::new_v4();
        let (sink, _seen) = collecting_sink();
        let cancel = CancellationToken::new();
        let s = spec("/bin/sh", run_id)
            .arg("-c")
            .arg("sleep 60")
            .kill_grace(Duration::from_millis(300));
        let token = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(120)).await;
            token.cancel();
        });
        let err = spawn_streamed(&s, &sink, &cancel).await.unwrap_err();
        assert!(matches!(err, SabrageError::Cancelled));
        assert_eq!(err.exit_code(), 130);
    }

    #[test]
    fn exit_code_conventions() {
        assert_eq!(exit_code_of(exit_status_from_code(0)), 0);
        assert_eq!(exit_code_of(exit_status_from_code(7)), 7);
        assert!(exit_status_from_code(0).success());
        assert!(!exit_status_from_code(1).success());
    }

    #[test]
    fn finds_this_test_binary_by_its_exe_path() {
        let exe = std::env::current_exe().expect("test binary path");
        let found = find_processes_by_exe(&exe);
        let me = std::process::id();
        assert!(
            found.iter().any(|p| p.pid == me),
            "own pid {me} not found among {found:?}"
        );
        // An exe path nothing runs from matches nothing.
        assert!(find_processes_by_exe(Path::new("/nonexistent/sabrage/helper")).is_empty());
    }

    #[test]
    fn liveness_probe_agrees_with_reality() {
        assert!(is_alive(std::process::id()));
        // pid 0 is the swapper/kernel process group on macOS; a pid that cannot
        // exist for this user is the honest negative case.
        assert!(!is_alive(u32::MAX - 1));
    }

    #[test]
    fn default_child_path_puts_the_toolchains_first_and_dedupes() {
        let p = default_child_path();
        let parts: Vec<&str> = p.split(':').collect();
        assert_eq!(parts[0], "/opt/homebrew/bin");
        assert_eq!(parts[1], "/usr/local/bin");
        assert!(parts[2].ends_with("/.cargo/bin"));
        assert!(parts[3].ends_with("/Library/Android/sdk/platform-tools"));
        let unique: std::collections::BTreeSet<_> = parts.iter().collect();
        assert_eq!(unique.len(), parts.len(), "duplicate PATH entry");
    }

    #[test]
    fn spec_builder_renders_a_display_command() {
        let s = spec("cmake", Uuid::nil())
            .args(["-S", "/repo/ext/oxrsys"])
            .arg("-B")
            .arg(PathBuf::from("/repo/ext/oxrsys/build-x64"))
            .cwd("/repo")
            .env("CMAKE_BUILD_PARALLEL_LEVEL", "8")
            .env_path("/opt/homebrew/bin");
        assert_eq!(
            s.display(),
            "cmake -S /repo/ext/oxrsys -B /repo/ext/oxrsys/build-x64"
        );
        assert_eq!(s.argv0(), "cmake");
        assert_eq!(s.cwd, Some(PathBuf::from("/repo")));
        assert_eq!(s.env_path.as_deref(), Some("/opt/homebrew/bin"));
        assert_eq!(s.kill_grace, DEFAULT_KILL_GRACE);
    }
}

#[cfg(test)]
mod identity_tests {
    use super::*;
    use crate::events::step;
    use uuid::Uuid;

    #[test]
    fn observing_this_process_agrees_with_the_full_scan() {
        let me = std::process::id();
        let observed = ProcInfo::observe(me).expect("own pid is observable");
        assert_eq!(observed.pid, me);
        assert!(observed.start_time > 0);

        // The single-pid refresh and the whole-table scan must agree.
        let exe = std::env::current_exe().unwrap();
        let scanned = find_processes_by_exe(&exe)
            .into_iter()
            .find(|p| p.pid == me)
            .expect("own pid in the scan");
        assert_eq!(scanned.start_time, observed.start_time);

        // A pid that cannot exist for this user.
        assert!(ProcInfo::observe(u32::MAX - 1).is_none());
    }

    #[test]
    fn identity_rejects_a_recycled_pid_and_the_unobservable_fallback() {
        let mut me = ProcInfo::observe(std::process::id()).unwrap();
        assert!(me.is_same_process());

        // Same pid, different start time = a recycled pid: never ours.
        me.start_time += 1;
        assert!(!me.is_same_process());

        // The spawn-time fallback (start_time 0) is deliberately unverifiable.
        let fallback = ProcInfo {
            pid: std::process::id(),
            start_time: 0,
            exe: PathBuf::from("/bin/sh"),
        };
        assert!(!fallback.is_same_process());

        // A dead pid is not the same process either.
        let dead = ProcInfo {
            pid: u32::MAX - 1,
            start_time: 1,
            exe: PathBuf::new(),
        };
        assert!(!dead.is_same_process());
    }

    #[test]
    fn proc_info_round_trips_as_camel_case_json() {
        let p = ProcInfo {
            pid: 4242,
            start_time: 1786300214,
            exe: PathBuf::from("/bin/sh"),
        };
        let j = serde_json::to_value(&p).unwrap();
        assert_eq!(j["startTime"], 1786300214u64);
        assert_eq!(j["pid"], 4242);
        assert_eq!(serde_json::from_value::<ProcInfo>(j).unwrap(), p);
    }

    #[tokio::test]
    async fn capture_collects_both_pipes_and_the_status() {
        let spec = ChildSpec::new("/bin/sh", step::BUILD_TOOLS, Uuid::nil())
            .arg("-c")
            .arg("printf 'devices\\n'; printf 'oops\\n' >&2; exit 2");
        let out = capture(&spec).await.unwrap();
        assert_eq!(out.stdout, "devices\n");
        assert_eq!(out.stdout_trimmed(), "devices");
        assert_eq!(out.stderr, "oops\n");
        assert_eq!(exit_code_of(out.status), 2);

        // A missing program is an Io error naming it, not a panic.
        let missing = ChildSpec::new("/nonexistent/sabrage/adb", step::BUILD_TOOLS, Uuid::nil());
        assert_eq!(capture(&missing).await.unwrap_err().kind(), "io");
    }

    #[test]
    fn cmdline_matching_is_the_pgrep_f_shape() {
        assert!(cmdline_contains(
            &["wine".into(), "Z:\\games\\Beat Saber.exe".into()],
            "Beat Saber.exe"
        ));
        // Split across two argv elements still matches via the joined line.
        assert!(cmdline_contains(
            &["Beat".into(), "Saber.exe".into()],
            "Beat Saber.exe"
        ));
        assert!(!cmdline_contains(&["wineserver".into()], "Beat Saber.exe"));
        assert!(!cmdline_contains(&[], "Beat Saber.exe"));
        // The scan itself finds this test binary, whose argv carries the
        // filter name — the same false-positive class `pgrep -f` has, which is
        // why the reap steps match on the exe path instead (module header).
        let me = std::process::id();
        let filter = std::env::args().nth(1).unwrap_or_default();
        if !filter.is_empty() {
            assert!(find_processes_by_cmdline(&filter)
                .iter()
                .any(|p| p.pid == me));
        }
    }
}
