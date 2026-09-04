//! Session telemetry, derived entirely from files and polls.
//!
//! `runtime_status.json` outlives the runtime, so it evidences a live session
//! only while fresh and while the pid it names is alive. Its `state` is
//! oxrsys's vocabulary: only [`RUNTIME_STATE_STREAMING`] is compared, and only
//! to decide whether a missing heartbeat means anything; every other value is
//! carried through and displayed
//! (tests::runtime_status_live_is_freshness_and_a_live_pid_together,
//! tests::is_fresh_accepts_only_stamps_inside_both_budgets).

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::{run_phase, EncoderInfo, SessionPhase, SessionStatus};
use crate::logs::Tailer;
use crate::paths::Paths;

/// `runtime_status.json` as oxrsys writes it.
///
/// Keys are snake_case on the wire — the one session-layer type not renamed to
/// camelCase, because oxrsys owns the file. Unknown fields are ignored so a
/// runtime that grows a field does not blank the status pill. Shape pinned by
/// tests::parse_runtime_status_accepts_the_observed_and_minimal_documents_and_rejects_a_half_written_one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeStatus {
    /// Free-form. Only [`RUNTIME_STATE_STREAMING`] is ever compared; every
    /// other value is carried through and displayed.
    pub state: String,
    #[serde(default)]
    pub process_id: Option<u32>,
    pub updated_at_unix_ms: u64,
    #[serde(default)]
    pub application_name: Option<String>,
}

/// The `state` oxrsys writes while a client is connected and frames are
/// flowing; the only state with a per-second heartbeat, hence the only one
/// whose staleness means anything (tests::snapshot_tests::snapshot_phase_transitions).
pub const RUNTIME_STATE_STREAMING: &str = "streaming";

/// How stale `runtime_status.json` may be before it stops counting as evidence
/// of a live runtime: comfortably above the observed write cadence, low enough
/// that a killed runtime stops looking alive within one status poll
/// (tests::the_watcher_duration_budgets).
pub const RUNTIME_STATUS_MAX_AGE: Duration = Duration::from_secs(3);

/// How long a fresh launch gets before [`SessionMonitor::snapshot`] will even
/// consider [`SessionPhase::Stalled`].
///
/// The runtime does not start writing `runtime_status.json` (or emitting
/// `enc1s` telemetry) until the client has connected and streaming has
/// begun — headset donning, the ALVR handshake, and the first encoded frame
/// can together take up to ~30s in a normal, healthy startup. Flagging
/// Stalled inside that window would make every ordinary launch look broken
/// for its first half-minute.
pub const SESSION_STARTUP_GRACE: Duration = Duration::from_secs(30);

/// How long `runtime_status.json` may stay stale — **after having been fresh
/// at least once** — before a `Running` session is downgraded to
/// [`SessionPhase::Stalled`].
///
/// This is the documented standby freeze (design-core §7): the game is still
/// alive but has stopped streaming. It is distinct from
/// [`SESSION_STARTUP_GRACE`], which covers the time *before* the runtime has
/// ever reported in at all.
pub const STALL_GRACE_AFTER_FRESH: Duration = Duration::from_secs(10);

/// Backward window [`SessionMonitor::new`] preloads from the runtime log, so a
/// monitor created moments after a session already negotiated its encoder
/// still finds that line rather than waiting for the *next* one (which may
/// never come — the encoder negotiates once per session). Generous on line
/// count; still bounded by [`crate::logs`]'s 256 KiB preload cap regardless.
const RUNTIME_LOG_PRELOAD_LINES: usize = 200;

/// Parse `runtime_status.json`'s bytes. `None` for anything unparseable — a
/// half-written file caught mid-rewrite is a normal event, not an error.
pub fn parse_runtime_status(json: &str) -> Option<RuntimeStatus> {
    serde_json::from_str(json).ok()
}

/// How far into the future an `updated_at_unix_ms` stamp may sit and still be
/// believed: ordinary clock skew between the runtime and this process.
///
/// Without a bound, a stamp an hour ahead (clock correction or corruption)
/// reads as "written now" and suppresses [`SessionPhase::Stalled`] until wall
/// time catches up
/// (tests::is_fresh_accepts_only_stamps_inside_both_budgets).
pub const MAX_FUTURE_SKEW: Duration = Duration::from_secs(2);

/// Is a `updated_at_unix_ms` stamp recent enough to believe?
///
/// Tolerant of a stamp slightly in the future (clock skew between the writer
/// and this process) — but only up to [`MAX_FUTURE_SKEW`]; past that the stamp
/// is not skew, it is wrong, and a wrong stamp must not read as fresh.
pub fn is_fresh(updated_at_unix_ms: u64, now_unix_ms: u64) -> bool {
    let ahead = updated_at_unix_ms.saturating_sub(now_unix_ms);
    let behind = now_unix_ms.saturating_sub(updated_at_unix_ms);
    ahead <= MAX_FUTURE_SKEW.as_millis() as u64
        && behind <= RUNTIME_STATUS_MAX_AGE.as_millis() as u64
}

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
pub fn runtime_status_live(rs: &RuntimeStatus, now_unix_ms: u64) -> bool {
    is_fresh(rs.updated_at_unix_ms, now_unix_ms)
        && rs.process_id.is_some_and(crate::process::is_alive)
}

/// The wall-clock time an oxrsys log line carries, in [`super::now_unix_ms`]'s
/// units, or `None` when the line does not start with one.
///
/// The stamp follows oxrsys's spdlog format (`Config.cpp`:
/// `[%Y-%m-%d %H:%M:%S.%e] [%l] %v`) and is **local** time, making it
/// comparable with `started_at_unix_ms`. It exists for one question: the
/// runtime log is a single appending sink shared by every session, so a line in
/// `RUNTIME_LOG_PRELOAD_LINES`'s backward window is this session's only if
/// written after this session started
/// (tests::snapshot_tests::an_adopted_session_only_inherits_lines_written_after_it_started).
pub fn parse_log_timestamp(line: &str) -> Option<u64> {
    use chrono::TimeZone;

    let rest = line.strip_prefix('[')?;
    let stamp = rest.split(']').next()?;
    let naive = chrono::NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d %H:%M:%S%.f").ok()?;
    // A DST fall-back hour is ambiguous: take the earlier of the two readings,
    // which is the conservative one here (an earlier line is more likely to be
    // judged history and dropped than wrongly adopted).
    let local = chrono::Local
        .from_local_datetime(&naive)
        .earliest()
        .or_else(|| chrono::Local.from_local_datetime(&naive).latest())?;
    u64::try_from(local.timestamp_millis()).ok()
}

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
pub fn parse_encoder_ready(line: &str) -> Option<EncoderInfo> {
    const MARKER: &str = "OXRSys/ALVR: encoder ready ";
    let idx = line.find(MARKER)?;
    let rest = line[idx + MARKER.len()..].trim_end();

    let open_paren = rest.find('(')?;
    let close_paren_rel = rest[open_paren..].find(')')?;
    let close_paren = open_paren + close_paren_rel;

    let dims_and_rates = rest[..open_paren].trim();
    let inside = &rest[open_paren + 1..close_paren];
    let (codec, path) = inside.split_once(", ")?;

    let mut fields = dims_and_rates.split_whitespace();
    let wh = fields.next()?;
    let hz = fields.next()?;
    let mbps = fields.next()?;

    let (w_str, h_str) = wh.split_once('x')?;
    let width: u32 = w_str.parse().ok()?;
    let height: u32 = h_str.parse().ok()?;
    let refresh_hz: u32 = hz.strip_prefix('@')?.strip_suffix("Hz")?.parse().ok()?;
    let bitrate_mbps: u32 = mbps.strip_suffix("Mbps")?.parse().ok()?;

    Some(EncoderInfo {
        codec: codec.trim().to_string(),
        path: path.trim().to_string(),
        width,
        height,
        refresh_hz,
        bitrate_mbps,
    })
}

/// Polls the session's file sources and folds them into one [`SessionStatus`].
///
/// `&mut self` because it carries the log-tail cursors between polls: the
/// encoder line is found once and then remembered, rather than re-read from
/// the top of a growing log every second.
pub struct SessionMonitor {
    paths: Paths,
    /// Tail on `oxrsys-runtime.log`, opened `from_end` — see
    /// [`RUNTIME_LOG_PRELOAD_LINES`]. `None` only if the very first open
    /// failed for a reason other than "the file does not exist yet" (that
    /// case is `Some` with no open file inside — see [`Tailer::open`]).
    runtime_log_tailer: Option<Tailer>,
    /// The most recent `encoder ready` line parsed so far. Cleared on the edge
    /// into [`SessionPhase::Idle`] / [`SessionPhase::Exited`]
    /// (tests::snapshot_tests::snapshot_phase_transitions, F16) and whenever
    /// the run it belongs to differs from the run being reported, so a new
    /// session never inherits the previous one's chip as a false-healthy signal
    /// (tests::snapshot_tests::a_previous_sessions_encoder_line_is_never_published_for_a_new_run).
    encoder: Option<EncoderInfo>,
    /// Which run `encoder` was parsed for. `None` is a real value: a chip
    /// picked up while nothing identifiable is running belongs to no run, and
    /// must not survive into one.
    encoder_run_id: Option<crate::events::RunId>,
    /// When this monitor was built, against [`super::now_unix_ms`]'s clock.
    ///
    /// Preloaded log lines (`RUNTIME_LOG_PRELOAD_LINES`) predate the monitor,
    /// so they belong to the current session only when that session predates the
    /// monitor too: Sabrage opened onto a game already running
    /// (tests::snapshot_tests::an_adopted_session_only_inherits_lines_written_after_it_started,
    /// tests::snapshot_tests::a_previous_sessions_encoder_line_is_never_published_for_a_new_run).
    created_at_unix_ms: u64,
    /// Has the first (history-bearing) tail poll happened yet?
    preload_pending: bool,
    /// The last successfully parsed status file.
    runtime_status: Option<RuntimeStatus>,
    /// Has `runtime_status.json` ever been observed fresh **for the run this
    /// monitor is currently reporting**? [`SessionPhase::Stalled`] only makes
    /// sense once the runtime has proven it can report in at all.
    ever_fresh: bool,
    /// Wall-clock time of the most recent fresh observation, for that same
    /// run.
    last_fresh_unix_ms: Option<u64>,
    /// Which run `ever_fresh`/`last_fresh_unix_ms`/`runtime_status` describe.
    ///
    /// The monitor outlives every session it watches, so freshness history not
    /// reset on a run change decides the *next* session's `Stalled`. `None` is
    /// a real value (nothing identifiable is running), and history recorded
    /// under it must not survive into a run either
    /// (tests::snapshot_tests::freshness_history_never_crosses_from_one_session_to_the_next).
    fresh_run_id: Option<crate::events::RunId>,
    /// The phase reported by the *previous* [`snapshot`](Self::snapshot) call.
    /// Purely for detecting the Idle/Exited *entry* edge that clears
    /// `encoder` — nothing else here is a function of history across polls.
    last_phase: SessionPhase,
}

/// Which derived source produced a snapshot's phase, before the run stage's
/// published phase is weighed against it. See [`SessionMonitor::snapshot`]'s
/// precedence rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Base {
    /// [`super::live_session`] — this process owns a launched session.
    Live,
    /// `session-state.json` — a session this process did not launch.
    Persisted,
    /// A session neither of the above knows about: a fresh
    /// `runtime_status.json` naming a live process, or — before that file
    /// exists at all — a live `Beat Saber.exe` on the process table. A
    /// `demo.sh run` in another terminal ([`SessionPhase::External`]).
    External,
    /// Neither: the derived phase is `Idle`.
    None,
}

impl SessionMonitor {
    pub fn new(paths: Paths) -> SessionMonitor {
        let mut monitor = SessionMonitor {
            paths,
            runtime_log_tailer: None,
            encoder: None,
            encoder_run_id: None,
            created_at_unix_ms: super::now_unix_ms(),
            preload_pending: true,
            runtime_status: None,
            ever_fresh: false,
            last_fresh_unix_ms: None,
            fresh_run_id: None,
            last_phase: SessionPhase::Idle,
        };
        let log_path = monitor.runtime_log_path();
        monitor.runtime_log_tailer = Tailer::open(&log_path, true, RUNTIME_LOG_PRELOAD_LINES).ok();
        monitor
    }

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
    pub async fn snapshot(&mut self) -> SessionStatus {
        let now = super::now_unix_ms();
        let mut status = SessionStatus::default();

        // Which of the three derived branches produced `status.phase` — the
        // published-phase precedence below is a function of it, not just of
        // the phase value.
        let mut base = Base::None;
        if let Some(handle) = super::live_session() {
            base = Base::Live;
            status.owned_by_this_process = true;
            status.run_id = Some(handle.run_id);
            status.bottle = Some(handle.bottle.clone());
            status.pid = Some(handle.identity.pid);
            status.started_at_unix_ms = Some(handle.started_at_unix_ms);
            status.log_path = Some(handle.log_path.display().to_string());
            // Through `classify`, not `is_same_process()`: an alive pid whose
            // recorded `start_time` is the spawn fallback's 0 is `Unverifiable`
            // — live as far as anything can tell, and every door treats it that
            // way
            // (tests::snapshot_tests::an_alive_pid_with_no_verifiable_start_time_is_never_reported_exited).
            status.phase = if super::reconcile::classify_identity(Some(&handle.identity)).is_live()
            {
                SessionPhase::Running
            } else {
                SessionPhase::Exited
            };
        } else if let Ok(Some(state)) = super::state::load(&self.paths.session_state_path()) {
            base = Base::Persisted;
            status.run_id = Some(state.run_id);
            status.bottle = Some(state.bottle.clone());
            status.started_at_unix_ms = Some(state.started_at_unix_ms);
            status.log_path = Some(state.log_path.display().to_string());
            status.pid = state.wine.as_ref().map(|w| w.pid);
            // Same predicate as the live-handle branch above, and as
            // `session_block_at`'s third signal: `Unverifiable` is alive.
            let wine_alive = super::reconcile::classify(&state).is_live();
            status.phase = match (wine_alive, state.detached) {
                (true, true) => SessionPhase::Detached,
                (true, false) => SessionPhase::Running,
                (false, _) => SessionPhase::Exited,
            };
            status.detached = state.detached;
        }
        // else: nothing live, nothing persisted — `status.phase` stays Idle
        // (`SessionStatus::default()`).

        // `ever_fresh`, `last_fresh_unix_ms` and the cached status are what the
        // stall rule below reasons over, and this monitor outlives every
        // session it watches: carrying session A's history into session B
        // classifies B as `Stalled` off A's timestamps
        // (tests::snapshot_tests::freshness_history_never_crosses_from_one_session_to_the_next).
        if self.fresh_run_id != status.run_id {
            self.fresh_run_id = status.run_id;
            self.ever_fresh = false;
            self.last_fresh_unix_ms = None;
            self.runtime_status = None;
        }

        // The file is global and outlives every session, so a stamp written
        // *before* the session being reported started is the previous
        // session's — evidence about a runtime that is gone, not this one
        // (tests::snapshot_tests::a_status_written_before_this_session_started_is_not_fresh).
        if let Ok(text) = std::fs::read_to_string(self.runtime_status_path()) {
            if let Some(rs) = parse_runtime_status(&text) {
                let predates_session = status
                    .started_at_unix_ms
                    .is_some_and(|started| rs.updated_at_unix_ms < started);
                let fresh = is_fresh(rs.updated_at_unix_ms, now) && !predates_session;
                status.runtime_state = Some(rs.state.clone());
                status.runtime_fresh = fresh;
                if fresh {
                    self.ever_fresh = true;
                    self.last_fresh_unix_ms = Some(now);

                    // No handle, no record, but the runtime is reporting *now*
                    // and the process it names is alive — an external launch.
                    // Reporting idle here invites a second launch onto a live
                    // game. Never from freshness alone: the pid must answer too
                    // (tests::snapshot_tests::a_session_started_outside_sabrage_is_reported_not_called_idle).
                    if base == Base::None && runtime_status_live(&rs, now) {
                        base = Base::External;
                        status.phase = SessionPhase::External;
                        status.pid = rs.process_id;
                    }
                }
                self.runtime_status = Some(rs);
            }
        }

        // The seventh door signal ([`super::running_game_pid`]) rendered.
        // `runtime_status.json` appears only once streaming starts, minutes
        // after `./demo.sh run` spawned the game; every file source above reads
        // idle for that window while the doors already refuse (A13a-2)
        // (tests::snapshot_tests::a_running_game_with_nothing_on_disk_is_external_not_idle).
        //
        // Last, and only for `Base::None`: the one probe that costs a full
        // process-table walk; any branch above already knows more than a pid.
        if base == Base::None {
            if let Some(pid) = super::running_game_pid() {
                base = Base::External;
                status.phase = SessionPhase::External;
                status.pid = Some(pid);
            }
        }

        // Parsed here (the cursor has to advance every poll) but *attributed*
        // at the end of this method, once the phase and the run it belongs to
        // are settled.
        let history = std::mem::take(&mut self.preload_pending);
        // The line's own wall-clock stamp rides along: during the preload it is
        // the only proof of which session an `encoder ready` line belongs to.
        let mut parsed: Option<(EncoderInfo, Option<u64>)> = None;
        if let Some(tailer) = self.runtime_log_tailer.as_mut() {
            if let Ok(batch) = tailer.poll() {
                for line in &batch.lines {
                    if let Some(info) = parse_encoder_ready(line) {
                        parsed = Some((info, parse_log_timestamp(line)));
                    }
                }
            }
        }
        // oxrsys writes `runtime_status.json` on state changes and once per
        // second *only while streaming* — there is no idle heartbeat. Staleness
        // therefore means "the stream's heartbeat stopped" only when the last
        // state is `streaming`; an `idle` runtime is legitimately stale for as
        // long as the user takes to put the headset on
        // (tests::snapshot_tests::snapshot_phase_transitions).
        let runtime_streaming = self
            .runtime_status
            .as_ref()
            .is_some_and(|rs| rs.state == RUNTIME_STATE_STREAMING);
        if status.phase == SessionPhase::Running && runtime_streaming {
            let started = status.started_at_unix_ms.unwrap_or(now);
            let past_startup_grace =
                now.saturating_sub(started) > SESSION_STARTUP_GRACE.as_millis() as u64;
            if past_startup_grace && self.ever_fresh && !status.runtime_fresh {
                let stale_for = self
                    .last_fresh_unix_ms
                    .map(|t| now.saturating_sub(t))
                    .unwrap_or(u64::MAX);
                if stale_for > STALL_GRACE_AFTER_FRESH.as_millis() as u64 {
                    status.phase = SessionPhase::Stalled;
                }
            }
        }

        // `Preflight` / `Launching` / `Stopping` exist only in the run stage's
        // own head — there is no `LIVE_SESSION` yet, or teardown has already
        // started clearing it — so nothing derived above can produce them.
        if let Some(info) = run_phase() {
            let outranks_derived = match info.phase {
                // Teardown is underway even though the handle is still up.
                SessionPhase::Stopping => true,
                // A launch that has not published its handle yet.
                SessionPhase::Preflight | SessionPhase::Launching => base != Base::Live,
                // `Exited` — and defensively anything else the run stage
                // might one day publish — is the weakest signal there is: it
                // survives `run()` returning, so it fills `Idle` (the screen
                // can still say "Exited (code N)") and loses to anything on
                // disk, which is newer truth.
                _ => base == Base::None,
            };
            if outranks_derived {
                status.phase = info.phase;
                // #201: `RUN_PHASE` is an in-process global, so a published
                // phase that wins names a session this Sabrage owns — including
                // the Preflight/Launching window where `LIVE_SESSION` is not
                // populated yet
                // (tests::snapshot_tests::snapshot_identity_and_exit_code_sources).
                status.owned_by_this_process = true;
                // #200: a published phase outranking a *persisted* derivation
                // for a DIFFERENT run means a launch started over a stale
                // `session-state.json`; keeping the old identity under the new
                // phase points Stop at a run the `RunRegistry` never heard of,
                // so the publication's identity is taken wholesale
                // (tests::snapshot_tests::snapshot_identity_and_exit_code_sources).
                //
                // `Base::Live` needs no such branch: a published phase over a
                // live handle is always the same run (teardown's `Stopping`).
                if base == Base::Persisted && status.run_id != Some(info.run_id) {
                    status.run_id = Some(info.run_id);
                    status.bottle = Some(info.bottle.clone());
                    status.pid = None;
                    status.log_path = None;
                    status.started_at_unix_ms = None;
                    status.detached = false;
                }
                // Identity only where the derived branches had none; a live
                // handle — or a state file describing this same run — knows
                // more (pid, log path) than a published phase ever carries.
                if status.run_id.is_none() {
                    status.run_id = Some(info.run_id);
                }
                if status.bottle.is_none() {
                    status.bottle = Some(info.bottle.clone());
                }
            }
            if info.phase == SessionPhase::Exited && status.phase == SessionPhase::Exited {
                status.exit_code = info.exit_code;
            }
        }

        // A session that has just settled into Idle or Exited has nothing left
        // to report a codec for. Edge-triggered on entry, so a monitor that has
        // never seen a session does not have a freshly parsed chip yanked out
        // from under it in the very poll that produced it.
        let was_idle_or_exited =
            matches!(self.last_phase, SessionPhase::Idle | SessionPhase::Exited);
        let now_idle_or_exited = matches!(status.phase, SessionPhase::Idle | SessionPhase::Exited);
        if now_idle_or_exited && !was_idle_or_exited {
            self.encoder = None;
            self.encoder_run_id = None;
        }
        // …and a chip never crosses from one run to another: the edge above
        // cannot catch a monitor that reports Running from its very first poll,
        // which is exactly when the log still holds the previous session's line
        // (tests::snapshot_tests::a_previous_sessions_encoder_line_is_never_published_for_a_new_run).
        if self.encoder_run_id != status.run_id {
            self.encoder = None;
            self.encoder_run_id = None;
        }
        if let Some((info, line_unix_ms)) = parsed {
            // A line the tail read *live* was written after this poll's
            // predecessor, so it belongs to the session being reported. A
            // preloaded line is history, and history is this session's only if
            // the session predates the monitor and the line's own timestamp is
            // not older than the session; the log is one appending sink shared
            // by every session, so a preloaded line with no readable timestamp
            // proves nothing and is dropped
            // (tests::snapshot_tests::an_adopted_session_only_inherits_lines_written_after_it_started).
            let believable = if history {
                match (status.started_at_unix_ms, line_unix_ms) {
                    (Some(started), Some(written)) => {
                        started <= self.created_at_unix_ms && written >= started
                    }
                    _ => false,
                }
            } else {
                true
            };
            if believable {
                self.encoder = Some(info);
                self.encoder_run_id = status.run_id;
            }
        }
        self.last_phase = status.phase;
        status.encoder = self.encoder.clone();

        status
    }

    /// `~/Library/Application Support/OXRSys/runtime_status.json`.
    pub fn runtime_status_path(&self) -> PathBuf {
        self.paths.oxr_appsup.join("runtime_status.json")
    }

    /// `~/Library/Application Support/OXRSys/oxrsys-runtime.log`.
    pub fn runtime_log_path(&self) -> PathBuf {
        self.paths.oxr_appsup.join("oxrsys-runtime.log")
    }
}

#[cfg(test)]
mod tests;
