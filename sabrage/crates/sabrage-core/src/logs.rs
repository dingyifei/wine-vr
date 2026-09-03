//! Log files: naming the wine console log, tailing the three live sources, and
//! listing past runs.
//!
//! # The wine console log
//!
//! run.sh:
//!
//! ```zsh
//! mkdir -p "$ROOT/logs"
//! LOG="$ROOT/logs/beatsaber-$(date +%Y%m%d-%H%M%S).log"
//! "$WINE" … > >(tee "$LOG") 2>&1 &
//! ```
//!
//! Two things follow. The name is **local** civil time (`date` with no `-u`),
//! which is why this crate depends on `chrono` at all — `std::time` has no
//! calendar. And the shell can collide: two launches in the same second write
//! the same path, and `tee` truncates, so the first run's log is simply gone.
//! [`wine_log_candidate`] adds a `-2`, `-3`, … suffix instead — a declared
//! divergence (PARITY.md, "Planned for later phases"; design-core §10.9),
//! enforced by opening the file `create_new` in
//! [`crate::executor::Executor::spawn_detached`] so the collision is detected
//! rather than assumed.
//!
//! Sabrage also replaces `tee` itself: the child writes into the file
//! descriptor directly, and the tail below reads the same file. `tee` can lose
//! the last buffer when the pipeline is torn down; a file fd cannot.
//!
//! # Tailing
//!
//! [`Tailer`] is rotation-aware — an inode change, a size that shrank, or a
//! prefix that no longer matches the bytes last read from it (an in-place
//! `truncate(true)` that grew back past the cursor between two polls, which is
//! how ALVR rewrites `session_log.txt`) all mean the file was replaced, and the
//! tailer reopens from the start and says so ([`LogBatch::rotated`]). One poll
//! reads a bounded number of bytes ([`POLL_BYTE_BUDGET`]), so a large existing
//! log is drained across polls rather than materialised in one. A partial
//! final line is buffered until its newline arrives, so a half-written line
//! never reaches the UI as a complete one. Splitting reuses [`crate::process::ChunkSplitter`] — the same
//! `\n`/`\r`/`\r\n`-tolerant state machine `spawn_streamed` already uses for
//! wine/build-tool output — rather than a second hand-rolled copy of the same
//! rule.

use std::collections::VecDeque;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Result, SabrageError};
use crate::paths::Paths;
use crate::process::ChunkSplitter;

// ── naming ────────────────────────────────────────────────────────────────────

/// The filename stem run.sh builds with `date +%Y%m%d-%H%M%S`.
pub const WINE_LOG_PREFIX: &str = "beatsaber-";

/// The candidate path for attempt `attempt` of this launch's console log,
/// given an already-formatted `YYYYmmdd-HHMMSS` stamp.
///
/// * `attempt == 0` → `beatsaber-YYYYmmdd-HHMMSS.log`, byte-identical to the
///   shell's name for the same instant;
/// * `attempt == n >= 1` → the same name with `-{n+1}` before `.log`, i.e.
///   `beatsaber-20260829-101112-2.log` for the first collision.
///
/// Takes `stamp` as a plain string rather than a `chrono` type so nothing
/// calling this — including test code in other crates — needs a date/time
/// library dependency just to name a log file candidate: `chrono` is an
/// implementation detail of [`wine_log_candidate`]'s stamp computation, not a
/// fact this API should force on its callers.
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

// ── sources ───────────────────────────────────────────────────────────────────

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

// ── tailing ───────────────────────────────────────────────────────────────────

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
    /// Read it as a property of the batch, not of the poll: a rotation
    /// detected while earlier lines were still queued is announced on the
    /// first batch that actually carries bytes from the reopened file, and the
    /// batches before it (the previous incarnation's backlog, which
    /// [`LogBatch::truncated`] promises is never dropped) leave this `false`.
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
/// [`MAX_LINES_PER_POLL`] caps what a poll *delivers*, not what it reads: the
/// old loop did one `read_to_end` from the cursor and split every byte of it
/// into `pending` before that cap was ever applied, so opening a large
/// `--verbose` wine console log (or an `oxrsys-runtime.log` at its 5 MiB
/// rotation size) from offset 0 materialised the whole file as `String`s in a
/// single call. The cursor and the splitter both survive to the next poll, and
/// the poller runs every 250 ms, so a backlog is drained across polls instead.
const POLL_BYTE_BUDGET: usize = 1024 * 1024;

/// One `read` of the loop above. Small enough that the line cap is noticed
/// promptly, large enough that a 1 MiB budget is 16 syscalls.
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
    /// `\n`/`\r`/`\r\n`-tolerant splitter; its internal buffer *is* the
    /// "partial final line" the doc below promises never shows up early.
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
    /// A rotation that finds `pending` non-empty cannot deliver those lines as
    /// the new file's beginning — the consumer clears its buffer on
    /// [`LogBatch::rotated`] and would then label the old session's last lines
    /// as the new one's first. So the backlog goes out under `rotated: false`,
    /// this counter marks how much of it is old, and `drain_capped` never lets
    /// one batch straddle the boundary.
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
                    // `splitter` still holds any genuine trailing partial line
                    // (the file's tail had no terminator yet at the moment we
                    // read it) — left in place so the very next `poll()`
                    // completes it, exactly like an ordinary mid-stream
                    // partial.
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
    /// Detects rotation first — a new inode, a size below the cursor, or a
    /// rewritten prefix (the bytes before the cursor no longer match the
    /// [`Tailer::signature`] read from them, which is what an in-place
    /// `truncate(true)` that grew back past the cursor between two polls looks
    /// like) — and any of the three reopens from the start with
    /// [`LogBatch::rotated`] set. A file that has vanished yields an empty
    /// batch rather than an error — the next poll picks it up when it comes
    /// back.
    ///
    /// One call reads at most [`POLL_BYTE_BUDGET`] bytes and stops early once
    /// [`MAX_LINES_PER_POLL`] lines are queued: the cursor and the splitter
    /// survive to the next call, so a large backlog is drained across polls
    /// instead of materialised in one.
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

            // Anything still queued in `pending` was already read out of the
            // *previous* incarnation of this path — real content, just not yet
            // handed to a caller because an earlier poll hit
            // `MAX_LINES_PER_POLL` (possibly while the path had vanished; see
            // the `NotFound` arm above, which drains but never clears
            // `pending` on its own). `LogBatch::truncated` promises those
            // lines are "never dropped" — a rotation must not silently break
            // that promise by wiping `pending` out from under them. Drain and
            // deliver them now — but as what they are: the tail of the
            // **previous** incarnation. Handing them out under `rotated: true`
            // told the consumer to clear its buffer and then showed it the old
            // session's last lines as the new file's first, which is how a
            // startup failure gets attributed to the wrong session (A8-4). The
            // marker waits in `pending_rotation` for the first batch that
            // really does come out of the reopened file.
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

        // ── the continuity witness, read AGAIN (A8-7) ────────────────────────
        // The precheck above compared the bytes before the cursor *before*
        // this read; a truncate-and-regrow-past-the-cursor landing in the
        // window between the two would be read as a continuation, and the new
        // file's whole prefix below `offset` silently skipped — the next poll
        // then sees the signature this read installed and stays "continuous"
        // forever. Bracketing the read with the same witness closes it: what
        // sits before `offset_before` must still be what sat there when the
        // precheck looked.
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
            // Undo the read whole: those bytes belong to an incarnation this
            // tailer can no longer place. Dropping `open` makes the next poll
            // a fresh open from byte 0, and the deferred marker (A8-4) makes
            // that batch the one that announces the rotation — this one still
            // carries only what was already queued.
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
        // `carry` down to zero, this batch is the first one whose lines really
        // do begin the new incarnation. Until then the marker stays pending —
        // it is never dropped, only deferred.
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

// ── past runs ─────────────────────────────────────────────────────────────────

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
mod tests {
    use super::*;
    use chrono::TimeZone;
    use tokio_util::sync::CancellationToken;
    use uuid::Uuid;

    fn scratch(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sabrage-logs-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // ── the truncate/read-race seam (A8-7) ───────────────────────────────────

    thread_local! {
        /// Per-thread, so one test's hook cannot reach another's `poll` —
        /// `cargo test` gives each `#[test]` its own thread.
        static AFTER_READ: std::cell::RefCell<Option<Box<dyn Fn()>>> =
            const { std::cell::RefCell::new(None) };
    }

    /// Called by [`Tailer::poll`] between its read loop and its post-read
    /// continuity check — the exact window a truncate-and-regrow has to land
    /// in to be read as a continuation. Nothing outside `cfg(test)` can
    /// register anything, and the default is a no-op.
    pub(super) fn after_read_hook() {
        let f = AFTER_READ.with(|h| h.borrow_mut().take());
        if let Some(f) = f {
            f();
            AFTER_READ.with(|h| *h.borrow_mut() = Some(f));
        }
    }

    /// Register a one-shot rewrite to happen inside that window. Fires once:
    /// the test's *later* polls must see an ordinary file.
    fn on_next_read(f: impl Fn() + 'static) {
        let fired = std::cell::Cell::new(false);
        AFTER_READ.with(|h| {
            *h.borrow_mut() = Some(Box::new(move || {
                if !fired.replace(true) {
                    f();
                }
            }))
        });
    }

    /// Every test that registers one must clear it — the thread is reused by
    /// nothing here, but a leaked hook is a trap for the next edit.
    fn clear_read_hook() {
        AFTER_READ.with(|h| *h.borrow_mut() = None);
    }

    /// A [`Paths`] rooted entirely under `root` — including `oxr_appsup` and
    /// `sabrage_appsup`, which [`Paths::new`] otherwise derives from the real
    /// `$HOME`. A test must never touch the real `~/Library`.
    fn fixture_paths(root: &Path) -> Paths {
        let mut paths = Paths::new(root);
        paths.oxr_appsup = root.join("home/Library/Application Support/OXRSys");
        paths.sabrage_appsup = root.join("home/Library/Application Support/Sabrage");
        paths
    }

    fn set_mtime(path: &Path, unix_secs: u64) {
        let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(unix_secs);
        let f = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        f.set_modified(t).unwrap();
    }

    // ── wine_log_candidate ───────────────────────────────────────────────────

    #[test]
    fn attempt_zero_matches_the_shells_date_stamp() {
        let now = chrono::Local
            .with_ymd_and_hms(2026, 8, 29, 10, 11, 12)
            .unwrap();
        let p = wine_log_candidate(Path::new("/repo/logs"), now, 0);
        assert_eq!(p, PathBuf::from("/repo/logs/beatsaber-20260829-101112.log"));
    }

    #[test]
    fn collisions_get_a_dash_n_plus_one_suffix() {
        let now = chrono::Local
            .with_ymd_and_hms(2026, 8, 29, 10, 11, 12)
            .unwrap();
        assert_eq!(
            wine_log_candidate(Path::new("/repo/logs"), now, 1),
            PathBuf::from("/repo/logs/beatsaber-20260829-101112-2.log")
        );
        assert_eq!(
            wine_log_candidate(Path::new("/repo/logs"), now, 3),
            PathBuf::from("/repo/logs/beatsaber-20260829-101112-4.log")
        );
    }

    #[test]
    fn wine_log_candidate_delegates_to_the_stamped_form_byte_for_byte() {
        // Two facts about the same names: `wine_log_candidate_stamped` emits
        // these exact paths for a hand-built stamp with no chrono in sight —
        // the whole point of the split (F16) — and the chrono-typed
        // convenience wrapper delegates to it byte for byte, so neither entry
        // point can drift alone.
        let now = chrono::Local
            .with_ymd_and_hms(2026, 8, 29, 10, 11, 12)
            .unwrap();
        let cases: &[(&str, u32, &str)] = &[
            (
                "attempt 0: bare stamp, no suffix",
                0,
                "/repo/logs/beatsaber-20260829-101112.log",
            ),
            (
                "attempt 1: first collision gets -2",
                1,
                "/repo/logs/beatsaber-20260829-101112-2.log",
            ),
            (
                "attempt 3: fourth candidate gets -4",
                3,
                "/repo/logs/beatsaber-20260829-101112-4.log",
            ),
        ];
        for (label, attempt, expected) in cases {
            let stamped =
                wine_log_candidate_stamped(Path::new("/repo/logs"), "20260829-101112", *attempt);
            assert_eq!(stamped, PathBuf::from(*expected), "{label}");
            assert_eq!(
                wine_log_candidate(Path::new("/repo/logs"), now, *attempt),
                stamped,
                "{label}"
            );
        }
    }

    // ── LogSource wire shape ─────────────────────────────────────────────────

    #[test]
    fn log_source_file_is_a_struct_variant_on_the_wire() {
        let s = LogSource::File {
            path: PathBuf::from("/repo/logs/x.log"),
        };
        let j = serde_json::to_value(&s).unwrap();
        assert_eq!(
            j,
            serde_json::json!({"kind": "file", "path": "/repo/logs/x.log"})
        );
        assert_eq!(serde_json::from_value::<LogSource>(j).unwrap(), s);

        assert_eq!(
            serde_json::to_value(LogSource::WineConsole).unwrap(),
            serde_json::json!({"kind": "wineConsole"})
        );
        assert_eq!(
            serde_json::to_value(LogSource::OxrsysRuntime).unwrap(),
            serde_json::json!({"kind": "oxrsysRuntime"})
        );
        assert_eq!(
            serde_json::to_value(LogSource::AlvrSession).unwrap(),
            serde_json::json!({"kind": "alvrSession"})
        );
    }

    // ── resolve_source ───────────────────────────────────────────────────────

    #[test]
    fn resolve_source_maps_the_two_fixed_oxrsys_paths_and_the_file_variant() {
        let dir = scratch("resolve-fixed");
        let paths = fixture_paths(&dir);
        assert_eq!(
            resolve_source(&paths, &LogSource::OxrsysRuntime),
            Some(paths.oxr_appsup.join("oxrsys-runtime.log"))
        );
        assert_eq!(
            resolve_source(&paths, &LogSource::AlvrSession),
            Some(paths.oxr_appsup.join("alvr/session_log.txt"))
        );
        let explicit = dir.join("some/past-run.log");
        assert_eq!(
            resolve_source(
                &paths,
                &LogSource::File {
                    path: explicit.clone()
                }
            ),
            Some(explicit)
        );
    }

    #[test]
    fn resolve_source_wine_console_is_none_when_nothing_matches() {
        let dir = scratch("resolve-none");
        let paths = fixture_paths(&dir);
        assert_eq!(resolve_source(&paths, &LogSource::WineConsole), None);
    }

    #[test]
    fn resolve_source_wine_console_falls_back_to_persisted_state_log_if_it_exists() {
        let dir = scratch("resolve-state");
        let paths = fixture_paths(&dir);
        std::fs::create_dir_all(paths.logs_dir()).unwrap();
        std::fs::create_dir_all(&paths.sabrage_appsup).unwrap();

        let state_log = paths.logs_dir().join("beatsaber-fromstate.log");
        std::fs::write(&state_log, b"x\n").unwrap();
        // A newer file also sits in logs/ — the persisted state must still win
        // over "newest", per priority order.
        let newer = paths.logs_dir().join("beatsaber-newer.log");
        std::fs::write(&newer, b"y\n").unwrap();

        let json = format!(
            r#"{{"version":1,"runId":"00000000-0000-0000-0000-000000000000","bottle":"Steam","bsDir":"/games/bs","startedAtUnixMs":0,"logPath":"{}"}}"#,
            state_log.display()
        );
        std::fs::write(paths.session_state_path(), json).unwrap();

        assert_eq!(
            resolve_source(&paths, &LogSource::WineConsole),
            Some(state_log)
        );
    }

    #[test]
    fn resolve_source_wine_console_skips_a_persisted_state_whose_log_file_is_gone() {
        let dir = scratch("resolve-state-gone");
        let paths = fixture_paths(&dir);
        std::fs::create_dir_all(paths.logs_dir()).unwrap();
        std::fs::create_dir_all(&paths.sabrage_appsup).unwrap();

        let missing_log = paths.logs_dir().join("beatsaber-vanished.log"); // never created
        let json = format!(
            r#"{{"version":1,"runId":"00000000-0000-0000-0000-000000000000","bottle":"Steam","bsDir":"/games/bs","startedAtUnixMs":0,"logPath":"{}"}}"#,
            missing_log.display()
        );
        std::fs::write(paths.session_state_path(), json).unwrap();

        let real_log = paths.logs_dir().join("beatsaber-real.log");
        std::fs::write(&real_log, b"ok\n").unwrap();

        assert_eq!(
            resolve_source(&paths, &LogSource::WineConsole),
            Some(real_log)
        );
    }

    #[test]
    fn resolve_source_wine_console_falls_back_to_the_newest_past_run() {
        let dir = scratch("resolve-newest");
        let paths = fixture_paths(&dir);
        std::fs::create_dir_all(paths.logs_dir()).unwrap();

        let older = paths.logs_dir().join("beatsaber-20260101-000000.log");
        let newer = paths.logs_dir().join("beatsaber-20260201-000000.log");
        std::fs::write(&older, b"a\n").unwrap();
        std::fs::write(&newer, b"b\n").unwrap();
        set_mtime(&older, 1_000_000_000);
        set_mtime(&newer, 2_000_000_000);

        assert_eq!(resolve_source(&paths, &LogSource::WineConsole), Some(newer));
    }

    #[test]
    fn resolve_source_wine_console_prefers_the_live_session_over_everything() {
        // Touches the process-global LIVE_SESSION slot; self-contained
        // set→read→clear within one test, matching `session::mod`'s own test.
        use crate::process::ProcInfo;
        use crate::session::{
            clear_live_session, live_session, set_live_session, LiveSessionHandle,
        };

        let dir = scratch("resolve-live");
        let paths = fixture_paths(&dir);
        std::fs::create_dir_all(paths.logs_dir()).unwrap();

        let live_log = paths.logs_dir().join("beatsaber-live.log");
        std::fs::write(&live_log, b"live\n").unwrap();
        let stale_log = paths.logs_dir().join("beatsaber-stale.log");
        std::fs::write(&stale_log, b"stale\n").unwrap();
        set_mtime(&stale_log, 9_999_999_999); // newer than live_log's mtime, must still lose

        let run_id = Uuid::new_v4();
        set_live_session(LiveSessionHandle {
            run_id,
            bottle: "Steam".into(),
            identity: ProcInfo {
                pid: 1,
                start_time: 1,
                exe: PathBuf::from("/bin/true"),
            },
            log_path: live_log.clone(),
            started_at_unix_ms: 0,
            cancel: CancellationToken::new(),
            detach: CancellationToken::new(),
        });

        let resolved = resolve_source(&paths, &LogSource::WineConsole);
        clear_live_session(run_id);
        assert!(live_session().is_none());

        assert_eq!(resolved, Some(live_log));
    }

    // ── list_past_runs ───────────────────────────────────────────────────────

    #[test]
    fn list_past_runs_lists_newest_first_and_ignores_non_matching_entries() {
        let dir = scratch("past-runs");
        std::fs::write(dir.join("beatsaber-20260101-000000.log"), b"a").unwrap();
        std::fs::write(dir.join("beatsaber-20260201-000000.log"), b"bb").unwrap();
        std::fs::write(dir.join("notes.txt"), b"irrelevant").unwrap();
        std::fs::create_dir_all(dir.join("beatsaber-adir.log")).unwrap(); // a directory: must be skipped

        set_mtime(&dir.join("beatsaber-20260101-000000.log"), 1_000_000_000);
        set_mtime(&dir.join("beatsaber-20260201-000000.log"), 2_000_000_000);

        let runs = list_past_runs(&dir);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].file_name, "beatsaber-20260201-000000.log");
        assert_eq!(runs[1].file_name, "beatsaber-20260101-000000.log");
        assert_eq!(runs[0].size, 2);
    }

    #[test]
    fn list_past_runs_is_empty_for_a_missing_directory() {
        assert!(list_past_runs(Path::new("/does/not/exist/at/all/sabrage-test")).is_empty());
    }

    // ── Tailer ───────────────────────────────────────────────────────────────

    #[test]
    fn poll_returns_newly_appended_lines() {
        let dir = scratch("append");
        let path = dir.join("a.log");
        std::fs::write(&path, b"line1\nline2\n").unwrap();

        let mut t = Tailer::open(&path, false, 0).unwrap();
        let b1 = t.poll().unwrap();
        assert_eq!(b1.lines, vec!["line1", "line2"]);
        assert!(!b1.rotated);
        assert!(!b1.truncated);

        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(f, "line3").unwrap();

        let b2 = t.poll().unwrap();
        assert_eq!(b2.lines, vec!["line3"]);
    }

    #[test]
    fn poll_detects_rotation_by_rename() {
        let dir = scratch("rotate");
        let path = dir.join("a.log");
        std::fs::write(&path, b"old1\nold2\n").unwrap();

        let mut t = Tailer::open(&path, false, 0).unwrap();
        let b1 = t.poll().unwrap();
        assert_eq!(b1.lines, vec!["old1", "old2"]);

        std::fs::rename(&path, dir.join("a.log.bak")).unwrap();
        std::fs::write(&path, b"new1\n").unwrap();

        let b2 = t.poll().unwrap();
        assert!(b2.rotated);
        assert_eq!(b2.lines, vec!["new1"]);
    }

    #[test]
    fn poll_detects_in_place_truncation() {
        let dir = scratch("truncate");
        let path = dir.join("a.log");
        std::fs::write(&path, b"aaaaaaaaaa\nbbbbbbbbbb\n").unwrap();

        let mut t = Tailer::open(&path, false, 0).unwrap();
        let b1 = t.poll().unwrap();
        assert_eq!(b1.lines, vec!["aaaaaaaaaa", "bbbbbbbbbb"]);
        assert!(!b1.rotated);

        // Same path truncated shorter than our offset (logrotate copytruncate).
        std::fs::write(&path, b"short\n").unwrap();

        let b2 = t.poll().unwrap();
        assert!(
            b2.rotated,
            "shrinking below the last offset must be treated as rotation"
        );
        assert_eq!(b2.lines, vec!["short"]);
    }

    #[test]
    fn a_trailing_partial_line_is_buffered_until_its_newline_arrives() {
        let dir = scratch("partial");
        let path = dir.join("a.log");
        std::fs::write(&path, b"complete\nhalf").unwrap();

        let mut t = Tailer::open(&path, false, 0).unwrap();
        let b1 = t.poll().unwrap();
        assert_eq!(
            b1.lines,
            vec!["complete"],
            "the unterminated tail must not show yet"
        );

        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(f, "-line").unwrap();

        let b2 = t.poll().unwrap();
        assert_eq!(b2.lines, vec!["half-line"]);
    }

    #[test]
    fn open_from_end_preloads_the_last_tail_lines_lines() {
        let dir = scratch("preload");
        let path = dir.join("a.log");
        let content: String = (1..=10).map(|n| format!("line{n}\n")).collect();
        std::fs::write(&path, content.as_bytes()).unwrap();

        let mut t = Tailer::open(&path, true, 3).unwrap();
        let b1 = t.poll().unwrap();
        assert_eq!(b1.lines, vec!["line8", "line9", "line10"]);

        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(f, "line11").unwrap();

        let b2 = t.poll().unwrap();
        assert_eq!(b2.lines, vec!["line11"]);
    }

    #[test]
    fn open_from_end_with_zero_tail_lines_starts_at_eof() {
        let dir = scratch("preload-zero");
        let path = dir.join("a.log");
        std::fs::write(&path, b"old1\nold2\n").unwrap();

        let mut t = Tailer::open(&path, true, 0).unwrap();
        let b1 = t.poll().unwrap();
        assert!(b1.lines.is_empty(), "nothing preloaded, nothing new yet");
    }

    #[test]
    fn open_and_poll_tolerate_a_file_that_does_not_exist_yet() {
        let dir = scratch("missing");
        let path = dir.join("not-yet.log");

        let mut t = Tailer::open(&path, false, 5).unwrap();
        let b1 = t.poll().unwrap();
        assert!(b1.lines.is_empty() && !b1.rotated);

        std::fs::write(&path, b"first\n").unwrap();
        let b2 = t.poll().unwrap();
        assert_eq!(b2.lines, vec!["first"]);
        assert!(
            b2.rotated,
            "a file appearing for the first time counts as a fresh open"
        );
    }

    #[test]
    fn a_burst_over_the_cap_is_deferred_not_dropped() {
        let dir = scratch("burst");
        let path = dir.join("a.log");
        let content: String = (0..2500).map(|n| format!("l{n}\n")).collect();
        std::fs::write(&path, content.as_bytes()).unwrap();

        let mut t = Tailer::open(&path, false, 0).unwrap();
        let b1 = t.poll().unwrap();
        assert_eq!(b1.lines.len(), MAX_LINES_PER_POLL);
        assert!(b1.truncated);
        assert_eq!(b1.lines[0], "l0");

        let b2 = t.poll().unwrap();
        assert_eq!(b2.lines.len(), 500);
        assert!(!b2.truncated);
        assert_eq!(b2.lines[0], "l2000");
        assert_eq!(*b2.lines.last().unwrap(), "l2499");
    }

    /// A poll must not read the whole file: `MAX_LINES_PER_POLL` caps what is
    /// *delivered*, `POLL_BYTE_BUDGET` caps what is *read*. A file far larger
    /// than the budget is drained over several polls, in order and complete.
    #[test]
    fn one_poll_reads_at_most_the_byte_budget_and_still_delivers_every_line() {
        let dir = scratch("byte-budget");
        let path = dir.join("a.log");
        // ~3 MiB of 128-byte lines: three budgets' worth.
        let line = "x".repeat(127);
        let count = (3 * POLL_BYTE_BUDGET) / 128;
        let content: String = (0..count).map(|_| format!("{line}\n")).collect();
        std::fs::write(&path, content.as_bytes()).unwrap();
        let size = std::fs::metadata(&path).unwrap().len();

        let mut t = Tailer::open(&path, false, 0).unwrap();
        let b1 = t.poll().unwrap();
        assert!(
            t.offset <= POLL_BYTE_BUDGET as u64,
            "one poll read {} bytes of a {size}-byte file",
            t.offset
        );
        assert!(b1.truncated, "the caller is told there is more to come");
        assert_eq!(b1.lines.len(), MAX_LINES_PER_POLL);

        // Every line still arrives, in order, across as many polls as it takes.
        let mut delivered = b1.lines.len();
        for _ in 0..1000 {
            if delivered == count {
                break;
            }
            let b = t.poll().unwrap();
            assert!(b.lines.iter().all(|l| *l == line));
            delivered += b.lines.len();
        }
        assert_eq!(delivered, count, "nothing was dropped");
        assert_eq!(t.offset, size, "and the whole file was eventually read");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A file with no delimiter at all must not grow the splitter's buffer
    /// without limit: past `MAX_UNTERMINATED_LINE_BYTES` the partial is broken
    /// into a line of its own.
    #[test]
    fn a_single_line_with_no_newline_is_broken_at_the_bound() {
        let dir = scratch("no-newline");
        let path = dir.join("a.log");
        std::fs::write(&path, vec![b'z'; 2 * MAX_UNTERMINATED_LINE_BYTES + 4096]).unwrap();

        let mut t = Tailer::open(&path, false, 0).unwrap();
        let mut lines = 0usize;
        for _ in 0..16 {
            lines += t.poll().unwrap().lines.len();
        }
        assert!(
            lines >= 2,
            "the unterminated run must have been broken, not buffered whole"
        );
        assert!(
            t.unterminated < MAX_UNTERMINATED_LINE_BYTES,
            "the counter is reset by each forced break"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A8-7: ALVR opens `session_log.txt` with `.truncate(true)`, so a new
    /// session keeps the inode. If it writes back past our cursor before the
    /// next poll, neither the inode check nor `len < offset` sees anything —
    /// only the bytes before the cursor say the file is not the one we were
    /// reading.
    #[test]
    fn truncate_and_regrow_past_the_cursor_between_polls_reports_rotation() {
        let cases: &[(&str, usize)] = &[("equal", 10), ("larger", 30)];
        for (regrow, new_lines) in cases {
            let dir = scratch(&format!("rewrite-{regrow}"));
            let path = dir.join("a.log");
            let old: String = (1..=10)
                .map(|n| format!("OLD session line {n}\n"))
                .collect();
            std::fs::write(&path, old.as_bytes()).unwrap();

            // Opened from the end: the cursor sits at EOF, exactly where a
            // live pane on `session_log.txt` sits.
            let mut t = Tailer::open(&path, true, 5).unwrap();
            let _ = t.poll().unwrap();

            let new: String = (1..=*new_lines)
                .map(|n| format!("NEW session line {n}\n"))
                .collect();
            // In place, same inode — `OpenOptions::truncate`, not remove+create.
            {
                use std::io::Write;
                let mut f = std::fs::OpenOptions::new()
                    .write(true)
                    .truncate(true)
                    .open(&path)
                    .unwrap();
                f.write_all(new.as_bytes()).unwrap();
            }

            let b = t.poll().unwrap();
            assert!(b.rotated, "{regrow}: the rewrite must be reported");
            assert_eq!(
                b.lines.first().map(String::as_str),
                Some("NEW session line 1"),
                "{regrow}: the new session's FIRST line must not be skipped"
            );
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }

    /// The signature check must not cry rotation on an ordinary append — the
    /// bytes before the cursor are unchanged, which is the whole point.
    #[test]
    fn an_ordinary_append_is_never_mistaken_for_a_rewrite() {
        let dir = scratch("append-not-rotation");
        let path = dir.join("a.log");
        std::fs::write(&path, b"one\n").unwrap();

        let mut t = Tailer::open(&path, false, 0).unwrap();
        assert_eq!(t.poll().unwrap().lines, vec!["one"]);
        for n in 2..=20 {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap();
            writeln!(f, "line{n}").unwrap();
            let b = t.poll().unwrap();
            assert!(!b.rotated, "append {n} must not read as a rotation");
            assert_eq!(b.lines, vec![format!("line{n}")]);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// F14 (finding 14): a backlog queued in `pending` from *before* the path
    /// vanished must survive a vanish-then-reappear cycle, even when that
    /// backlog itself spans more than one `MAX_LINES_PER_POLL`-capped poll —
    /// the exact shape that let the old code's unconditional
    /// `self.pending.clear()` in the reopen branch discard real, already-read
    /// content no caller had seen yet.
    #[test]
    fn a_backlog_queued_before_a_vanish_survives_reappearance_uncut() {
        let dir = scratch("vanish-reappear");
        let path = dir.join("a.log");
        // 4500 lines: one poll's cap (2000) leaves a remainder (2500) that
        // itself does not fit in a single further poll either.
        let content: String = (0..4500).map(|n| format!("l{n}\n")).collect();
        std::fs::write(&path, content.as_bytes()).unwrap();

        let mut t = Tailer::open(&path, false, 0).unwrap();

        let b1 = t.poll().unwrap();
        assert_eq!(b1.lines.len(), MAX_LINES_PER_POLL);
        assert!(b1.truncated);
        assert_eq!(b1.lines[0], "l0");
        assert_eq!(*b1.lines.last().unwrap(), "l1999");

        // The file vanishes with 2500 lines still queued in `pending`.
        std::fs::remove_file(&path).unwrap();

        let b2 = t.poll().unwrap();
        assert!(!b2.rotated, "a vanish alone is not a rotation");
        assert_eq!(b2.lines.len(), MAX_LINES_PER_POLL);
        assert!(
            b2.truncated,
            "500 lines of the original backlog are still queued"
        );
        assert_eq!(b2.lines[0], "l2000");
        assert_eq!(*b2.lines.last().unwrap(), "l3999");

        // It reappears as a new file — genuinely rotated — while 500 lines
        // from the vanished original are still sitting in `pending`.
        std::fs::write(&path, b"new1\n").unwrap();

        let b3 = t.poll().unwrap();
        assert!(
            !b3.rotated,
            "A8-4: these are the OLD file's lines. Announcing the rotation here \
             makes the consumer clear its buffer and then label `l4000..l4499` \
             as the new file's beginning"
        );
        assert!(
            !b3.truncated,
            "the whole remaining backlog fit under the cap"
        );
        assert_eq!(
            b3.lines.len(),
            500,
            "the pre-vanish backlog must be delivered, not dropped by the reopen"
        );
        assert_eq!(b3.lines[0], "l4000");
        assert_eq!(*b3.lines.last().unwrap(), "l4499");

        // …and the marker travels with the batch that really does begin the
        // new incarnation.
        let b4 = t.poll().unwrap();
        assert!(b4.rotated, "A8-4: THIS batch is the new file's first");
        assert_eq!(b4.lines, vec!["new1"]);

        // One rotation, one marker: the ordinary appends that follow are
        // continuations.
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap();
            writeln!(f, "new2").unwrap();
        }
        let b5 = t.poll().unwrap();
        assert!(!b5.rotated);
        assert_eq!(b5.lines, vec!["new2"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A8-4, the consumer's half: `Logs.svelte` clears its buffer on
    /// `rotated` and then appends the batch. Replaying that contract over the
    /// sequence above must leave no line of the previous incarnation on
    /// screen — and must not have shown the new file's first lines appended
    /// under the *old* file's tail either.
    #[test]
    fn a_rotation_marker_partitions_the_batches_into_clean_incarnations() {
        let dir = scratch("rotation-epochs");
        let path = dir.join("a.log");
        std::fs::write(&path, b"old1\nold2\nold3\n").unwrap();

        let mut t = Tailer::open(&path, false, 0).unwrap();
        // `Logs.svelte`'s `onBatch`, in three lines.
        fn show(screen: &mut Vec<String>, b: &LogBatch) {
            if b.rotated {
                screen.clear();
            }
            screen.extend(b.lines.iter().cloned());
        }
        let mut screen: Vec<String> = Vec::new();

        let b1 = t.poll().unwrap();
        show(&mut screen, &b1);
        assert_eq!(screen, vec!["old1", "old2", "old3"]);

        // Rewritten in place, past the cursor — ALVR's `.truncate(true)`.
        std::fs::write(&path, b"new1\nnew2\nnew3\nnew4\n").unwrap();
        let b2 = t.poll().unwrap();
        show(&mut screen, &b2);
        assert!(b2.rotated);
        assert_eq!(
            screen,
            vec!["new1", "new2", "new3", "new4"],
            "not one line of the previous session survived the rotation"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A8-7: the continuity witness is read *before* the data. A truncate that
    /// lands between the two used to be read as a continuation — the tailer
    /// kept the stale cursor, overwrote its witness with bytes from the new
    /// file's suffix, and skipped the new file's whole prefix for good.
    #[test]
    fn a_rewrite_landing_inside_the_read_window_is_not_read_as_a_continuation() {
        let dir = scratch("read-window-race");
        let path = dir.join("a.log");
        let first: String = (0..200).map(|n| format!("old{n}\n")).collect();
        std::fs::write(&path, first.as_bytes()).unwrap();

        let mut t = Tailer::open(&path, false, 0).unwrap();
        let b1 = t.poll().unwrap();
        assert_eq!(b1.lines.len(), 200);

        // Append, then arrange for the file to be rewritten in place *after*
        // the read of that append and before the post-read check — the exact
        // window the precheck cannot cover.
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap();
            writeln!(f, "old200").unwrap();
        }
        let rewritten: String = (0..300).map(|n| format!("new{n}\n")).collect();
        let p = path.clone();
        on_next_read(move || {
            // Same inode, grown back past the old cursor: `truncate(true)`,
            // never a rename.
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&p)
                .unwrap();
            f.write_all(rewritten.as_bytes()).unwrap();
        });

        let b2 = t.poll().unwrap();
        clear_read_hook();
        assert!(
            b2.lines.is_empty(),
            "the straddled read is undone whole, not delivered: {:?}",
            b2.lines
        );
        assert!(!b2.rotated, "nothing from the new incarnation is out yet");

        // The next poll reopens from byte 0 and delivers the new file entire —
        // prefix included, which is what used to be lost.
        let b3 = t.poll().unwrap();
        assert!(b3.rotated, "and THIS batch begins the new incarnation");
        assert_eq!(b3.lines.len(), 300);
        assert_eq!(b3.lines[0], "new0");
        assert_eq!(*b3.lines.last().unwrap(), "new299");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
