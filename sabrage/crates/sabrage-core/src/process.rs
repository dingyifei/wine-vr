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

/// Deadline for one read-only probe ([`capture`]).
///
/// Generous by a factor of ~100 against what these actually take: `adb devices`
/// against a healthy server answers in milliseconds, and the slowest legitimate
/// case is the first `adb` call after a reboot, which forks the server first.
/// The point is a bound at all, so a wedged probe cannot hold the operation
/// lock forever.
pub const DEFAULT_PROBE_TIMEOUT: Duration = Duration::from_secs(10);

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChunkEnd {
    /// `\n`, or the `\r\n` pair (which is one terminator, not two).
    #[default]
    Lf,
    /// A bare `\r`: a repaint of the same terminal line.
    Cr,
    /// End of stream with no delimiter at all.
    Eof,
}

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
        self.push_with(bytes, &mut |chunk, _end| out(chunk));
    }

    /// [`ChunkSplitter::push`], with each chunk's terminator.
    pub fn push_with(&mut self, bytes: &[u8], out: &mut impl FnMut(String, ChunkEnd)) {
        for &b in bytes {
            if self.pending_cr {
                self.pending_cr = false;
                // The byte behind a CR decides what that CR was.
                if b == b'\n' {
                    out(self.take(), ChunkEnd::Lf);
                    continue;
                }
                out(self.take(), ChunkEnd::Cr);
            }
            match b {
                b'\n' => out(self.take(), ChunkEnd::Lf),
                b'\r' => self.pending_cr = true,
                _ => self.buf.push(b),
            }
        }
    }

    /// Flush whatever is buffered at EOF (a final chunk with no delimiter).
    pub fn finish(&mut self, out: &mut impl FnMut(String)) {
        self.finish_with(&mut |chunk, _end| out(chunk));
    }

    /// [`ChunkSplitter::finish`], with the final chunk's terminator.
    pub fn finish_with(&mut self, out: &mut impl FnMut(String, ChunkEnd)) {
        if self.pending_cr {
            // A CR with nothing behind it: a bare repaint, terminator included.
            self.pending_cr = false;
            out(self.take(), ChunkEnd::Cr);
        } else if !self.buf.is_empty() {
            out(self.take(), ChunkEnd::Eof);
        }
    }

    fn take(&mut self) -> String {
        let s = String::from_utf8_lossy(&self.buf).into_owned();
        self.buf.clear();
        s
    }
}

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
    // `false`: no caller of `spawn_streamed` reads the tail, so the pumps skip
    // cloning every chunk of a chatty build tool into a buffer nobody looks at
    // (tests::spawn_streamed_does_not_populate_a_tail_nobody_reads).
    let (status, _tail) = spawn_streamed_inner(spec, sink, cancel, false).await?;
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
    let (status, tail) = spawn_streamed_inner(spec, sink, cancel, true).await?;
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
    capture_tail: bool,
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

    let tail: Option<Tail> =
        capture_tail.then(|| Arc::new(Mutex::new(VecDeque::with_capacity(CHILD_TAIL_LINES))));
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
            let deadline = tokio::time::Instant::now() + spec.kill_grace;
            // Reap the leader first, within the same grace period.
            let st = match tokio::time::timeout_at(deadline, child.wait()).await {
                Ok(Ok(st)) => st,
                _ => {
                    let _ = killpg(pgid, Signal::SIGKILL);
                    child
                        .wait()
                        .await
                        .map_err(|e| SabrageError::io(PathBuf::from(&spec.program), e))?
                }
            };
            // A descendant that ignored the SIGTERM and redirected its own
            // stdout/stderr survives both the leader's exit and pipe EOF, so
            // liveness is measured on the group (tests::cancellation_escalates_when_a_descendant_outlives_the_leader).
            while group_alive(pgid) && tokio::time::Instant::now() < deadline {
                tokio::time::sleep(GROUP_POLL_INTERVAL).await;
            }
            if group_alive(pgid) {
                let _ = killpg(pgid, Signal::SIGKILL);
            }
            st
        }
    };

    // The pipes stay open while any descendant still holds them (wine's
    // `reg add` leaves wineserver behind), so waiting for EOF unconditionally
    // would hang the stage (tests::a_backgrounded_descendant_does_not_wedge_the_stage).
    if !drain_pumps(&mut pumps, PUMP_DRAIN_GRACE).await {
        if cancelled {
            // Belt and braces: the group looked empty (or unsignalable) while
            // something still holds the pipe. Escalate on the group, not the
            // leader (tests::cancellation_kills_a_descendant_that_ignored_term_and_released_the_pipes).
            let _ = killpg(pgid, Signal::SIGKILL);
            let _ = drain_pumps(&mut pumps, PUMP_DRAIN_GRACE).await;
        }
        // Whatever still holds the pipe outlives us: stop reading rather than wedge
        // the operation lock. No SIGKILL on the uncancelled path - that survivor is
        // usually one the pipeline wanted, e.g. wineserver (tests::a_backgrounded_descendant_does_not_wedge_the_stage).
        for p in &pumps {
            p.abort();
        }
    }
    let tail_lines: Vec<String> = tail
        .as_ref()
        .and_then(|t| t.lock().ok().map(|t| t.iter().cloned().collect()))
        .unwrap_or_default();

    if cancelled {
        return Err(SabrageError::Cancelled);
    }
    Ok((status, tail_lines))
}

/// How long to wait for the output pipes to reach EOF once the leader is gone,
/// before concluding that a surviving descendant is holding them.
const PUMP_DRAIN_GRACE: Duration = Duration::from_secs(2);

/// How often the cancelled path re-asks whether anything is left in the child's
/// process group. Short enough that the common case (the whole tree obeys the
/// SIGTERM) costs one poll, cheap enough that the worst case is a few dozen
/// `kill(2)`s over the whole `kill_grace`.
const GROUP_POLL_INTERVAL: Duration = Duration::from_millis(25);

/// Does the child's process group still have a member?
///
/// `kill(-pgid, 0)` succeeds while at least one process remains in the group
/// and fails with `ESRCH` once it is empty — the liveness signal the cancelled
/// path needs, because neither the leader's exit status nor pipe EOF says
/// anything about a descendant that ignored the SIGTERM. `EPERM` (a group we
/// could not signal anyway) counts as gone, since escalating on it is a no-op.
fn group_alive(pgid: Pid) -> bool {
    killpg(pgid, None).is_ok()
}

/// Await the pipe pumps with a deadline. `true` when they all finished.
///
/// Takes the handles by reference so the caller can still `abort()` them: a
/// dropped [`tokio::task::JoinHandle`] merely *detaches* its task, which for a
/// pump blocked on a pipe nothing will ever close means leaking a task per
/// wedged child.
async fn drain_pumps(pumps: &mut [tokio::task::JoinHandle<()>], budget: Duration) -> bool {
    tokio::time::timeout(budget, async {
        for p in pumps.iter_mut() {
            let _ = p.await;
        }
    })
    .await
    .is_ok()
}

async fn pump<R>(
    mut reader: R,
    stream: Stream,
    run_id: RunId,
    step: StepId,
    sink: EventSink,
    tail: Option<Tail>,
) where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut splitter = ChunkSplitter::new();
    let mut buf = [0u8; 8192];
    // A14-3: `end` carries each chunk's terminator so a byte-faithful renderer
    // repaints a `\r` chunk in place instead of `println!`-ing curl's and
    // cargo's progress spam (tests::output_events_carry_their_chunk_terminator).
    let mut emit = |chunk: String, end: ChunkEnd| {
        if let Some(tail) = &tail {
            if let Ok(mut t) = tail.lock() {
                if t.len() == CHILD_TAIL_LINES {
                    t.pop_front();
                }
                t.push_back(chunk.clone());
            }
        }
        sink(StageEvent::Output {
            run_id,
            step: step.to_string(),
            stream,
            chunk,
            end,
        });
    };
    loop {
        let n = match reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        splitter.push_with(&buf[..n], &mut emit);
    }
    splitter.finish_with(&mut emit);
}

/// One matched process.
///
/// Serializable because it is the **process identity** persisted in
/// `session-state.json` ([`crate::session::state::SessionState`]): `pid` alone
/// cannot say whether the wine process running under that number is *the* one
/// this Sabrage launched, and signalling a recycled pid is the one
/// unrecoverable mistake the reconcile path can make
/// (identity_tests::identity_rejects_a_recycled_pid_and_the_unobservable_fallback).
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
        use sysinfo::{Pid as SysPid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

        let refresh = ProcessRefreshKind::nothing().with_exe(UpdateKind::Always);
        // `System::new()` loads nothing: `new_with_specifics` would perform its
        // own `ProcessesToUpdate::All` scan of the whole table before the
        // targeted single-pid refresh below ever runs, walking every process on
        // the machine to observe one pid.
        let mut sys = System::new();
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
    /// (identity_tests::identity_rejects_a_recycled_pid_and_the_unobservable_fallback).
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
/// This replaces `pgrep -f <path>` — see this module's header for why. A
/// single-needle convenience over [`ProcessScan::scan`] — a caller that needs
/// several different needles against the same instant (`stop`'s survivor
/// probe, its two reap steps, and its foreign-helper scan) should scan once
/// and call [`ProcessScan::by_exe`]/[`ProcessScan::by_cmdline`] instead.
pub fn find_processes_by_exe(path: &Path) -> Vec<ProcInfo> {
    ProcessScan::scan().by_exe(path)
}

/// True when any element of `cmd` - or the whitespace-joined command line as a
/// whole, the shape `pgrep -f` scans - contains `needle`.
///
/// Wine puts the game's own path on the command line as a single `Z:\...`
/// Windows-path argument (e.g. `Z:\repo\…\Beat Saber.exe`), so per-element and
/// whole-line matching agree in the real case; both are checked so a
/// hypothetical split across two argv elements still matches
/// (identity_tests::cmdline_matching_is_the_pgrep_f_shape).
pub fn cmdline_contains(cmd: &[String], needle: &str) -> bool {
    cmd.iter().any(|arg| arg.contains(needle)) || cmd.join(" ").contains(needle)
}

/// Every running process whose command line matches [`cmdline_contains`] for
/// `needle`, pid-ordered - the argv-based match `pgrep -f` performs, unlike
/// [`find_processes_by_exe`]'s exact exe-path equality (used by the reap
/// steps). Opposite trade-offs on purpose - see this module's header and
/// PARITY.md § Stop, "The Beat Saber survivor probe scans live processes'
/// argv". A single-needle convenience over [`ProcessScan::scan`] - see
/// [`find_processes_by_exe`]'s doc for when a caller should scan once instead.
pub fn find_processes_by_cmdline(needle: &str) -> Vec<ProcInfo> {
    ProcessScan::scan().by_cmdline(needle)
}

/// One full-table process scan, with both `exe` and `cmd` refreshed, kept
/// around so a caller that needs several different needles against the same
/// instant pays for exactly one process walk: `stop`'s survivor check, each of
/// its two reap steps, and its foreign-helper scan share one [`ProcessScan`]
/// instead of walking the process table four times.
pub struct ProcessScan {
    procs: Vec<(ProcInfo, Vec<String>)>,
}

impl ProcessScan {
    /// Scan every live process once, refreshing both its resolved executable
    /// and its command line — the union of what [`find_processes_by_exe`] and
    /// [`find_processes_by_cmdline`] each need.
    pub fn scan() -> ProcessScan {
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

        let refresh = ProcessRefreshKind::nothing()
            .with_exe(UpdateKind::Always)
            .with_cmd(UpdateKind::Always);
        // `System::new()` loads nothing, so the explicit refresh below is the
        // only full-table scan this performs — `new_with_specifics` would have
        // performed a second one first (see `ProcInfo::observe`'s comment).
        let mut sys = System::new();
        sys.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh);

        let mut procs: Vec<(ProcInfo, Vec<String>)> = sys
            .processes()
            .iter()
            .map(|(pid, proc_)| {
                let exe = proc_.exe().map(Path::to_path_buf).unwrap_or_default();
                let cmd: Vec<String> = proc_
                    .cmd()
                    .iter()
                    .map(|a| a.to_string_lossy().into_owned())
                    .collect();
                (
                    ProcInfo {
                        pid: pid.as_u32(),
                        start_time: proc_.start_time(),
                        exe,
                    },
                    cmd,
                )
            })
            .collect();
        procs.sort_by_key(|(p, _)| p.pid);
        ProcessScan { procs }
    }

    /// [`find_processes_by_cmdline`], against this snapshot instead of a fresh
    /// scan.
    pub fn by_cmdline(&self, needle: &str) -> Vec<ProcInfo> {
        self.procs
            .iter()
            .filter(|(_, cmd)| cmdline_contains(cmd, needle))
            .map(|(p, _)| p.clone())
            .collect()
    }

    /// [`find_processes_by_exe`], against this snapshot instead of a fresh
    /// scan.
    ///
    /// `canonicalize()` is a syscall; a process whose exe basename cannot
    /// possibly match `path` is rejected before paying for it, so only real
    /// candidates get resolved.
    pub fn by_exe(&self, path: &Path) -> Vec<ProcInfo> {
        let want = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let want_name = want.file_name();
        let mut found: Vec<ProcInfo> = self
            .procs
            .iter()
            .filter_map(|(p, _)| {
                if p.exe.file_name() != want_name {
                    return None;
                }
                let resolved = p.exe.canonicalize().unwrap_or_else(|_| p.exe.clone());
                let matched = resolved == want;
                matched.then_some(ProcInfo {
                    pid: p.pid,
                    start_time: p.start_time,
                    exe: resolved,
                })
            })
            .collect();
        found.sort_by_key(|p| p.pid);
        found
    }
}

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
        crate::util::strip_trailing_newlines(&self.stdout)
    }
}

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
pub async fn capture(spec: &ChildSpec) -> Result<Captured> {
    capture_with(spec, &CancellationToken::new(), DEFAULT_PROBE_TIMEOUT).await
}

/// [`capture`] with an explicit cancellation token and deadline.
///
/// Cancellation yields [`SabrageError::Cancelled`]; the deadline yields an
/// `Io` error of kind [`std::io::ErrorKind::TimedOut`]. Either way the probe's
/// whole process group is `SIGKILL`ed — a read-only probe has nothing to flush,
/// so there is no SIGTERM grace to observe — and the leader is reaped by
/// `kill_on_drop`.
pub async fn capture_with(
    spec: &ChildSpec,
    cancel: &CancellationToken,
    deadline: Duration,
) -> Result<Captured> {
    let mut cmd = tokio::process::Command::new(&spec.program);
    cmd.args(&spec.args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Own process group, so a probe that forked (adb starting its server)
        // can be stopped as a tree rather than orphaned behind a dead leader.
        .process_group(0)
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
    let child = cmd
        .spawn()
        .map_err(|e| SabrageError::io(PathBuf::from(&spec.program), e))?;
    let pgid = child.id().map(|id| Pid::from_raw(id as i32));

    let out = tokio::select! {
        finished = child.wait_with_output() => {
            finished.map_err(|e| SabrageError::io(PathBuf::from(&spec.program), e))?
        }
        _ = cancel.cancelled() => {
            if let Some(pgid) = pgid {
                let _ = killpg(pgid, Signal::SIGKILL);
            }
            return Err(SabrageError::Cancelled);
        }
        _ = tokio::time::sleep(deadline) => {
            if let Some(pgid) = pgid {
                let _ = killpg(pgid, Signal::SIGKILL);
            }
            return Err(SabrageError::io(
                PathBuf::from(&spec.program),
                std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    format!(
                        "probe `{}` did not answer within {:.0}s",
                        spec.display(),
                        deadline.as_secs_f32()
                    ),
                ),
            ));
        }
    };
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
mod tests;

#[cfg(test)]
mod identity_tests;
