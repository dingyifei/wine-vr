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

use std::collections::VecDeque;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Result, SabrageError};
use crate::paths::Paths;
use crate::process::ChunkSplitter;

/// Filename prefix shared with run.sh: every wine console log is
/// `beatsaber-<stamp>.log`.
pub const WINE_LOG_PREFIX: &str = "beatsaber-";

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
pub fn wine_log_candidate_stamped(logs_dir: &Path, stamp: &str, attempt: u32) -> PathBuf {
    let name = if attempt == 0 {
        format!("{WINE_LOG_PREFIX}{stamp}.log")
    } else {
        format!("{WINE_LOG_PREFIX}{stamp}-{}.log", attempt + 1)
    };
    logs_dir.join(name)
}

/// [`wine_log_candidate_stamped`], formatting `stamp` from `local_now` the way
/// run.sh's own `date +%Y%m%d-%H%M%S` does.
///
/// `local_now` is passed in rather than read here so the rule is testable
/// without freezing the clock.
pub fn wine_log_candidate(
    logs_dir: &Path,
    local_now: chrono::DateTime<chrono::Local>,
    attempt: u32,
) -> PathBuf {
    let stamp = local_now.format("%Y%m%d-%H%M%S").to_string();
    wine_log_candidate_stamped(logs_dir, &stamp, attempt)
}

/// One tailable log.
///
/// `File` is a struct variant rather than a newtype because the enum is
/// internally tagged (`{"kind":"file","path":"…"}`) and serde cannot serialize
/// an internally tagged newtype variant wrapping a non-map value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum LogSource {
    /// The live session's `logs/beatsaber-<ts>.log`, or the newest one when no
    /// session is running.
    WineConsole,
    /// `~/Library/Application Support/OXRSys/oxrsys-runtime.log`.
    OxrsysRuntime,
    /// `~/Library/Application Support/OXRSys/alvr/session_log.txt` — unbounded,
    /// so it is only ever tailed from the end.
    AlvrSession,
    /// A specific past run, from [`list_past_runs`].
    File { path: PathBuf },
}

/// Resolve a [`LogSource`] to a path on this machine.
///
/// [`LogSource::WineConsole`] prefers the live session's own `log_path`
/// ([`crate::session::live_session`], then the persisted
/// [`crate::session::state::SessionState`] — only when that file still exists
/// on disk) and falls back to the newest `logs/beatsaber-*.log`. `None` when
/// nothing matches — an empty `logs/` directory on a fresh checkout is normal,
/// not an error. A corrupt or missing `session-state.json` is treated the same
/// as "no persisted session" rather than surfaced as an error: this is a
/// best-effort resolution, not a mutation.
pub fn resolve_source(paths: &Paths, source: &LogSource) -> Option<PathBuf> {
    match source {
        LogSource::WineConsole => {
            if let Some(handle) = crate::session::live_session() {
                return Some(handle.log_path);
            }
            if let Ok(Some(state)) = crate::session::state::load(&paths.session_state_path()) {
                if state.log_path.is_file() {
                    return Some(state.log_path);
                }
            }
            list_past_runs(&paths.logs_dir())
                .into_iter()
                .next()
                .map(|r| r.path)
        }
        LogSource::OxrsysRuntime => Some(paths.oxr_appsup.join("oxrsys-runtime.log")),
        LogSource::AlvrSession => Some(paths.oxr_appsup.join("alvr/session_log.txt")),
        LogSource::File { path } => Some(path.clone()),
    }
}

/// One poll's worth of new lines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogBatch {
    /// Complete lines only, without their newlines.
    pub lines: Vec<String>,
    /// **The lines in THIS batch begin a new incarnation of the file** — it
    /// was replaced (new inode), truncated, or rewritten in place. The UI
    /// clears its buffer on this and then appends `lines`.
    ///
    /// It is a property of the batch, not of the poll: a rotation detected
    /// while earlier lines are still queued is announced on the first batch
    /// that carries bytes from the reopened file (A8-4,
    /// tests::a_backlog_queued_before_a_vanish_survives_reappearance_uncut).
    pub rotated: bool,
    /// This batch hit [`MAX_LINES_PER_POLL`] or [`POLL_BYTE_BUDGET`]: the rest
    /// is still queued internally (never dropped) or still unread in the file,
    /// and arrives on a later [`Tailer::poll`] call — the file is producing
    /// faster than the caller is polling.
    pub truncated: bool,
    /// The file these lines came from, for the pane header.
    pub path: String,
}

/// Per-poll delivery cap, so one burst cannot make a single `poll()` call
/// block the UI thread rendering thousands of lines at once. The remainder
/// stays in [`Tailer`]'s internal queue and is delivered on the next call.
const MAX_LINES_PER_POLL: usize = 2000;

/// Bound on the backward read [`Tailer::open`] does for a `from_end` preload,
/// regardless of `tail_lines` — the unbounded `alvr/session_log.txt` must
/// never turn "open a log pane" into "read the whole file".
const TAIL_PRELOAD_CAP_BYTES: u64 = 256 * 1024;

/// Bound on the bytes ONE [`Tailer::poll`] reads, whatever has accumulated in
/// the file since the last one.
///
/// `MAX_LINES_PER_POLL` caps what a poll *delivers*, and it bounds the read
/// only while lines keep arriving: a file with few or no newlines (a large
/// `--verbose` wine console log's unterminated tail, an `oxrsys-runtime.log`
/// at its 5 MiB rotation size) would otherwise be read whole into the
/// splitter in one call. The cursor and splitter survive to the next poll, so
/// a backlog drains across polls
/// (tests::one_poll_reads_at_most_the_byte_budget_and_still_delivers_every_line).
const POLL_BYTE_BUDGET: usize = 1024 * 1024;

/// One `read` inside [`Tailer::poll`]'s read loop. Small enough that the line
/// cap is noticed promptly, large enough that a 1 MiB budget is 16 syscalls.
const READ_CHUNK_BYTES: usize = 64 * 1024;

/// Bound on the splitter's unterminated-line buffer.
///
/// A file with no delimiter at all (a binary blob written over a log path)
/// would otherwise grow that buffer without limit across polls, since the byte
/// budget above bounds only one poll. Past this, the partial is flushed as a
/// line of its own — a synthetic break is a better failure than unbounded
/// memory.
const MAX_UNTERMINATED_LINE_BYTES: usize = 1024 * 1024;

/// How many bytes immediately before the cursor are remembered to detect a
/// same-inode rewrite (see [`Tailer::poll`]).
const CONTINUITY_SIGNATURE_BYTES: usize = 64;

/// A rotation-aware line tailer.
///
/// One per open log pane. Not `Clone`: it owns a file handle and a byte cursor.
pub struct Tailer {
    path: PathBuf,
    /// The open file and its device+inode, or `None` when the path does not
    /// exist (yet). Kept as one field because the two are only ever meaningful
    /// together: the identity describes *this* handle, and a `Some` handle
    /// whose identity was missing would have no way to detect rotation.
    open: Option<(std::fs::File, (u64, u64))>,
    offset: u64,
    /// The last [`CONTINUITY_SIGNATURE_BYTES`] bytes read, i.e. the bytes
    /// immediately *before* `offset`. Re-read at the next poll to catch a
    /// truncate-and-regrow that left the inode alone and grew back past the
    /// cursor before we looked (ALVR opens `session_log.txt` with
    /// `.truncate(true)`, keeping the inode).
    signature: Vec<u8>,
    /// `\n`/`\r`/`\r\n`-tolerant splitter; its internal buffer *is* the partial
    /// final line, which reaches a caller only once it is terminated or
    /// outgrows [`MAX_UNTERMINATED_LINE_BYTES`].
    splitter: ChunkSplitter,
    /// Bytes fed to `splitter` since it last emitted a line — the counter
    /// behind [`MAX_UNTERMINATED_LINE_BYTES`].
    unterminated: usize,
    /// Lines already split out of the file but not yet handed to a caller —
    /// either the `from_end` preload, or the overflow from a batch that hit
    /// [`MAX_LINES_PER_POLL`].
    pending: VecDeque<String>,
    /// How many of the leading entries in `pending` were read out of a
    /// **previous** incarnation of the path (A8-4).
    ///
    /// The backlog goes out under `rotated: false` and `drain_capped` never
    /// lets one batch straddle the boundary, so a consumer that clears on
    /// `rotated` cannot label the old file's last lines as the new one's first
    /// (tests::a_backlog_queued_before_a_vanish_survives_reappearance_uncut).
    carry: usize,
    /// A rotation has been *detected* but not yet *announced*: set with
    /// `carry`, consumed by the first batch that actually carries bytes from
    /// the reopened file.
    pending_rotation: bool,
}

/// `(dev, ino)` — the pair that changes when a path starts naming a different
/// file (rename+recreate, or `logrotate`'s copy step). A same-inode
/// truncate-in-place leaves it unchanged; that case is caught by `len <
/// offset`, or — when the file grew back past the cursor between two polls —
/// by the continuity signature.
fn file_identity(meta: &std::fs::Metadata) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt;
    (meta.dev(), meta.ino())
}

/// Remember the last [`CONTINUITY_SIGNATURE_BYTES`] bytes of what has been
/// read so far, given the bytes just consumed.
fn update_signature(signature: &mut Vec<u8>, chunk: &[u8]) {
    if chunk.len() >= CONTINUITY_SIGNATURE_BYTES {
        signature.clear();
        signature.extend_from_slice(&chunk[chunk.len() - CONTINUITY_SIGNATURE_BYTES..]);
        return;
    }
    signature.extend_from_slice(chunk);
    if signature.len() > CONTINUITY_SIGNATURE_BYTES {
        let excess = signature.len() - CONTINUITY_SIGNATURE_BYTES;
        signature.drain(0..excess);
    }
}

impl Tailer {
    /// Open `path` for tailing.
    ///
    /// A path that does not exist yet is not an error: `open` stays `None`,
    /// and the first [`poll`](Tailer::poll) that finds the file has since
    /// appeared treats that as a fresh open (`rotated: true`) — the log pane
    /// for a session that has not launched yet, or `alvr/session_log.txt`
    /// before ALVR has written a byte, are both normal states, not errors.
    ///
    /// `from_end` starts at EOF with the last `tail_lines` lines pre-loaded —
    /// mandatory for `alvr/session_log.txt`, which is unbounded (design-core
    /// §7). `from_end == false` reads the whole file (nothing is preloaded;
    /// the first [`poll`](Tailer::poll) simply starts at offset 0, and
    /// [`POLL_BYTE_BUDGET`] spreads a large one over several polls), which is
    /// what a past run's log wants.
    pub fn open(path: &Path, from_end: bool, tail_lines: usize) -> Result<Tailer> {
        let mut file = match std::fs::File::open(path) {
            Ok(f) => Some(f),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(SabrageError::io(path, e)),
        };

        let mut offset = 0u64;
        let mut open = None;
        let mut signature = Vec::new();
        let mut splitter = ChunkSplitter::new();
        let mut pending = VecDeque::new();

        if let Some(f) = file.as_mut() {
            let meta = f.metadata().map_err(|e| SabrageError::io(path, e))?;
            let identity = file_identity(&meta);
            let len = meta.len();

            if from_end {
                offset = len;
                if tail_lines > 0 && len > 0 {
                    let read_len = len.min(TAIL_PRELOAD_CAP_BYTES);
                    let start = len - read_len;
                    f.seek(SeekFrom::Start(start))
                        .map_err(|e| SabrageError::io(path, e))?;
                    let mut buf = vec![0u8; read_len as usize];
                    f.read_exact(&mut buf)
                        .map_err(|e| SabrageError::io(path, e))?;

                    let mut lines: Vec<String> = Vec::new();
                    splitter.push(&buf, &mut |l| lines.push(l));
                    if start > 0 && !lines.is_empty() {
                        // The first fragment is missing its head — we started
                        // reading mid-file. Never show it as a complete line.
                        lines.remove(0);
                    }
                    if lines.len() > tail_lines {
                        lines.drain(0..lines.len() - tail_lines);
                    }
                    pending = lines.into();
                    // `splitter` still holds any genuine trailing partial
                    // line — left in place so the very next `poll()` completes
                    // it, exactly like an ordinary mid-stream partial.
                }
                signature = read_signature(f, offset).map_err(|e| SabrageError::io(path, e))?;
            }
            open = Some((file.take().expect("checked above"), identity));
        }

        Ok(Tailer {
            path: path.to_path_buf(),
            open,
            offset,
            signature,
            splitter,
            unterminated: 0,
            pending,
            carry: 0,
            pending_rotation: false,
        })
    }

    /// Read whatever has arrived since the last poll.
    ///
    /// Only complete lines are delivered; a trailing partial line stays
    /// buffered until its newline arrives or it outgrows
    /// `MAX_UNTERMINATED_LINE_BYTES`. A file that was replaced, truncated,
    /// or rewritten in place is reopened from the start and the batch says so
    /// ([`LogBatch::rotated`]); a vanished file yields whatever was already
    /// queued and is picked up on the next poll.
    ///
    /// One call reads at most `POLL_BYTE_BUDGET` bytes and stops early once
    /// `MAX_LINES_PER_POLL` lines are queued; the remainder arrives on later
    /// calls.
    ///
    /// # Errors
    ///
    /// I/O failures other than the file being absent, which is not an error.
    pub fn poll(&mut self) -> Result<LogBatch> {
        let path_str = self.path.display().to_string();

        let meta = match std::fs::metadata(&self.path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                self.open = None;
                self.signature.clear();
                let (lines, truncated) = self.drain_capped();
                return Ok(LogBatch {
                    lines,
                    rotated: false,
                    truncated,
                    path: path_str,
                });
            }
            Err(e) => return Err(SabrageError::io(&self.path, e)),
        };

        let current_id = file_identity(&meta);
        let len = meta.len();
        // `self.open` is `None` exactly when there is no open file yet —
        // either this is the first time the path has ever existed, or it
        // vanished on a previous poll and has just reappeared. Either way
        // that is a fresh open, not a continuation.
        let mut rotated = match &self.open {
            None => true,
            Some((_, id)) => *id != current_id || len < self.offset,
        };
        if !rotated {
            // Same inode, and long enough to contain everything we have read —
            // but the writer may still have truncated it and written a *new*
            // session past our cursor since the last poll. The bytes we last
            // read are the cheapest possible witness.
            let path = &self.path;
            let offset = self.offset;
            let signature = &self.signature;
            if let Some((f, _)) = self.open.as_mut() {
                let seen = read_signature(f, offset).map_err(|e| SabrageError::io(path, e))?;
                rotated = seen != *signature;
            }
        }

        if rotated {
            let file =
                std::fs::File::open(&self.path).map_err(|e| SabrageError::io(&self.path, e))?;

            // Backlog from the previous incarnation goes out under `rotated:
            // false`; under `true` the consumer clears and misattributes it to
            // the new file (A8-4, tests::a_backlog_queued_before_a_vanish_survives_reappearance_uncut).
            let backlog = !self.pending.is_empty();
            self.open = Some((file, current_id));
            self.offset = 0;
            self.signature.clear();
            self.splitter = ChunkSplitter::new();
            self.unterminated = 0;
            if backlog {
                self.carry = self.pending.len();
                self.pending_rotation = true;
                let (lines, truncated) = self.drain_capped();
                return Ok(LogBatch {
                    lines,
                    rotated: false,
                    truncated,
                    path: path_str,
                });
            }
        }

        let Tailer {
            open,
            offset,
            signature,
            splitter,
            unterminated,
            pending,
            path,
            // `carry` and `pending_rotation` are read after these borrows end
            // (the rotation marker is decided against the drained batch).
            ..
        } = self;
        let (file, _) = open
            .as_mut()
            .expect("open is Some here: either just (re)opened above, or already open");
        file.seek(SeekFrom::Start(*offset))
            .map_err(|e| SabrageError::io(&*path, e))?;

        // Snapshot everything the read is about to advance, so a read that
        // turns out to have straddled a rewrite can be undone whole (A8-7).
        let offset_before = *offset;
        let signature_before = signature.clone();
        let pending_before = pending.len();

        let mut buf = vec![0u8; READ_CHUNK_BYTES];
        let mut consumed = 0usize;
        while consumed < POLL_BYTE_BUDGET && pending.len() < MAX_LINES_PER_POLL {
            let want = READ_CHUNK_BYTES.min(POLL_BYTE_BUDGET - consumed);
            let n = file
                .read(&mut buf[..want])
                .map_err(|e| SabrageError::io(&*path, e))?;
            if n == 0 {
                break;
            }
            consumed += n;
            let chunk = &buf[..n];
            update_signature(signature, chunk);
            let mut produced = 0usize;
            splitter.push(chunk, &mut |l| {
                produced += 1;
                pending.push_back(l);
            });
            if produced == 0 {
                *unterminated += n;
            } else {
                *unterminated = 0;
            }
            if *unterminated >= MAX_UNTERMINATED_LINE_BYTES {
                // One line has outgrown the bound: break it here rather than
                // let the splitter's buffer keep growing across polls.
                splitter.finish(&mut |l| pending.push_back(l));
                *unterminated = 0;
            }
        }

        // Bracket the read with the same witness: a truncate-and-regrow
        // landing between the precheck and here would read as a continuation
        // and skip the new file's whole prefix for good (A8-7,
        // tests::a_rewrite_landing_inside_the_read_window_is_not_read_as_a_continuation).
        #[cfg(test)]
        tests::after_read_hook();
        let straddled = consumed > 0
            && (read_signature(file, offset_before).map_err(|e| SabrageError::io(&*path, e))?
                != signature_before
                || file
                    .metadata()
                    .map(|m| file_identity(&m) != current_id)
                    .unwrap_or(true));

        if straddled {
            // Undo the read: those bytes belong to an incarnation this
            // tailer cannot place. Dropping `open` makes the next poll a fresh
            // open from byte 0, and the deferred marker (A8-4) makes that
            // batch the one that announces the rotation.
            self.pending.truncate(pending_before);
            self.splitter = ChunkSplitter::new();
            self.unterminated = 0;
            self.open = None;
            self.offset = 0;
            self.signature.clear();
            self.pending_rotation = true;
            let (lines, truncated) = self.drain_capped();
            return Ok(LogBatch {
                lines,
                rotated: false,
                truncated,
                path: path_str,
            });
        }

        *offset += consumed as u64;
        let budget_hit = consumed >= POLL_BYTE_BUDGET;

        // A rotation detected on an earlier poll is announced HERE: with
        // `carry` down to zero, this batch is the first whose lines really do
        // begin the new incarnation. The marker is deferred, never dropped.
        let rotated = rotated || (self.carry == 0 && std::mem::take(&mut self.pending_rotation));
        let (lines, truncated) = self.drain_capped();
        Ok(LogBatch {
            lines,
            rotated,
            truncated: truncated || budget_hit,
            path: path_str,
        })
    }

    /// Take up to [`MAX_LINES_PER_POLL`] lines off the front of `pending`,
    /// leaving the rest queued for the next call.
    fn drain_capped(&mut self) -> (Vec<String>, bool) {
        // While pre-rotation lines are still queued, stop at the boundary: a
        // batch that mixed the two incarnations would be half cleared and half
        // kept by a consumer acting on one `rotated` flag (A8-4).
        let cap = if self.carry > 0 {
            self.carry.min(MAX_LINES_PER_POLL)
        } else {
            MAX_LINES_PER_POLL
        };
        let mut lines = Vec::with_capacity(self.pending.len().min(cap));
        while lines.len() < cap {
            match self.pending.pop_front() {
                Some(l) => lines.push(l),
                None => break,
            }
        }
        self.carry = self.carry.saturating_sub(lines.len());
        let truncated = !self.pending.is_empty();
        (lines, truncated)
    }
}

/// The [`CONTINUITY_SIGNATURE_BYTES`] bytes immediately before `offset`, or
/// fewer at the start of a file. Empty at offset 0 — there is nothing before
/// it that a rewrite could change.
fn read_signature(f: &mut std::fs::File, offset: u64) -> std::io::Result<Vec<u8>> {
    let want = offset.min(CONTINUITY_SIGNATURE_BYTES as u64) as usize;
    if want == 0 {
        return Ok(Vec::new());
    }
    f.seek(SeekFrom::Start(offset - want as u64))?;
    let mut buf = vec![0u8; want];
    match f.read_exact(&mut buf) {
        Ok(()) => Ok(buf),
        // The file shrank under us between the metadata call and this read;
        // an unreadable signature is a mismatch, which is exactly the
        // "reopen from the start" answer.
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

/// One `logs/beatsaber-*.log` on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PastRun {
    pub path: PathBuf,
    pub file_name: String,
    pub size: u64,
    pub modified_unix_ms: u64,
}

/// Every `beatsaber-*.log` in `logs_dir`, newest first.
///
/// Lists the **shell's** runs too: both front-ends write into the same
/// `logs/` directory, on purpose. A missing `logs_dir` (nothing has ever run)
/// yields an empty list, not an error.
pub fn list_past_runs(logs_dir: &Path) -> Vec<PastRun> {
    let Ok(read_dir) = std::fs::read_dir(logs_dir) else {
        return Vec::new();
    };

    let mut runs: Vec<PastRun> = read_dir
        .filter_map(|e| e.ok())
        .filter_map(|entry| {
            let file_name = entry.file_name().into_string().ok()?;
            if !file_name.starts_with(WINE_LOG_PREFIX) || !file_name.ends_with(".log") {
                return None;
            }
            let meta = entry.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            let modified_unix_ms = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            Some(PastRun {
                path: entry.path(),
                file_name,
                size: meta.len(),
                modified_unix_ms,
            })
        })
        .collect();

    runs.sort_by_key(|r| std::cmp::Reverse(r.modified_unix_ms));
    runs
}

#[cfg(test)]
mod tests;
