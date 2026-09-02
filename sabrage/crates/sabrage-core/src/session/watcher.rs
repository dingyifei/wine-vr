//! Telemetry v1: what the session is doing, from files only.
//!
//! Every source here is a file or a poll — oxrsys needs to cooperate with
//! nothing (design-core §7). Three of them:
//!
//! | source | read as |
//! |---|---|
//! | `~/Library/Application Support/OXRSys/runtime_status.json` | [`RuntimeStatus`], believed only while [`is_fresh`] |
//! | `…/OXRSys/oxrsys-runtime.log` | tailed; [`parse_encoder_ready`] pulls the encoder line out |
//! | the wine child + `session-state.json` | liveness and identity |
//!
//! # Liveness is freshness, never existence
//!
//! `runtime_status.json` **persists after the runtime dies**. Treating its
//! presence — or its `state` field — as "a session is running" reports a dead
//! session as healthy forever. The only honest signal is
//! `updated_at_unix_ms` against the wall clock
//! ([`RUNTIME_STATUS_MAX_AGE`]). `state` itself stays an **opaque string**:
//! its vocabulary is unverified upstream (design-core §10, unverified fact 1),
//! so nothing here may branch on a particular word.

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::{run_phase, EncoderInfo, SessionPhase, SessionStatus};
use crate::logs::Tailer;
use crate::paths::Paths;

/// `~/Library/Application Support/OXRSys/runtime_status.json`, as observed:
///
/// ```json
/// {"state":"idle","transport":"","process_id":59004,
///  "application_name":"Beat Saber","updated_at_unix_ms":1786300214181}
/// ```
///
/// Keys are **snake_case on the wire** — this file is written by oxrsys, not
/// by Sabrage, so it is the one type in the session layer that does *not*
/// rename to camelCase. Unknown fields (`transport`, and whatever a future
/// runtime adds) are ignored rather than rejected: a runtime that grows a
/// field must not blank the status pill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeStatus {
    /// Opaque. Displayed, never branched on.
    pub state: String,
    #[serde(default)]
    pub process_id: Option<u32>,
    pub updated_at_unix_ms: u64,
    #[serde(default)]
    pub application_name: Option<String>,
}

/// How stale `runtime_status.json` may be before it stops counting as
/// evidence of a live runtime.
///
/// Three seconds: comfortably above the observed write cadence, low enough
/// that a killed runtime stops looking alive within one status poll.
/// The `state` value oxrsys writes while a client is connected and frames are
/// flowing (`RuntimeStatus::SetStreaming`); the only state with a per-second
/// heartbeat, hence the only one whose staleness means anything.
pub const RUNTIME_STATE_STREAMING: &str = "streaming";

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

/// How far into the future a `updated_at_unix_ms` stamp may sit and still be
/// believed: ordinary skew between the runtime's clock read and this one.
///
/// Unbounded tolerance is what a saturating subtraction gives by accident, and
/// it is not tolerance at all — a stamp an hour ahead (a clock correction, a
/// corrupted number) then reads as "written now" and keeps reading that way
/// until wall time catches up, suppressing [`SessionPhase::Stalled`] for the
/// whole hour.
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
/// [`is_fresh`] *and* a `process_id` that is still alive — the two halves
/// PARITY.md's "External sessions" row states, in the one function both readers
/// call: [`SessionMonitor::snapshot`]'s [`SessionPhase::External`] derivation
/// and [`crate::session::session_block_at`]'s status signal (the door every
/// mutating operation goes through). They used to spell it differently — the
/// door believed freshness alone — so the Session screen could report Idle
/// while Settings refused to save, over the very same file.
///
/// A status with no `process_id` at all is therefore *not* evidence of a live
/// runtime: oxrsys has always written that field (`RuntimeStatus.cpp` writes it
/// unconditionally), so its absence means a file this build cannot vouch for,
/// and a door that cannot be reasoned about is worse than a door that stays
/// with the other six signals.
pub fn runtime_status_live(rs: &RuntimeStatus, now_unix_ms: u64) -> bool {
    is_fresh(rs.updated_at_unix_ms, now_unix_ms)
        && rs.process_id.is_some_and(crate::process::is_alive)
}

/// The wall-clock time an oxrsys log line carries, in [`super::now_unix_ms`]'s
/// units, or `None` when the line does not start with one.
///
/// The pattern is oxrsys's spdlog format (`Config.cpp`:
/// `[%Y-%m-%d %H:%M:%S.%e] [%l] %v`), and the stamp is **local** time, which is
/// what makes this comparable with `started_at_unix_ms` at all.
///
/// It exists for one question: the runtime log is a single appending, rotating
/// sink shared by every session, so a line found in
/// [`RUNTIME_LOG_PRELOAD_LINES`]'s backward window is only *this* session's if
/// it was written after this session started. Without the stamp, reopening
/// Sabrage onto a running game republished the previous session's
/// `(HEVC, native helper)` chip and hid the current one's `(H.264, in-process)`
/// downgrade.
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

/// Pull an [`EncoderInfo`] out of one oxrsys log line.
///
/// The line, verbatim:
///
/// ```text
/// OXRSys/ALVR: encoder ready 2064x2208 @72Hz 100Mbps (HEVC, native helper)
/// ```
///
/// The marker is searched for anywhere in the input, so a full timestamped
/// spdlog line (`[2026-08-10 01:30:13.017] [info] OXRSys/ALVR: encoder ready
/// …`) parses identically to the bare message shown above.
///
/// `(H.264, in-process)` in the parenthesis is the silent-downgrade signature
/// the Session screen must surface: it means the native arm64 helper did not
/// take and encoding fell back to Rosetta H.264 (CLAUDE.md's encoder-helper
/// note). Returns `None` for any other line.
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
    /// The most recent `encoder ready` line parsed so far. Kept until a newer
    /// one replaces it *within the same session* — cleared on the edge into
    /// [`SessionPhase::Idle`] / [`SessionPhase::Exited`] (tracked via
    /// `last_phase`, below) and whenever the run it belongs to is no longer the
    /// run being reported (`encoder_run_id`), so a new session never inherits
    /// the previous one's chip as a false-healthy signal. Nothing here does
    /// design-core §7's Phase-5 downgrade detection — that is about a codec
    /// change *within* one still-running session, not this.
    encoder: Option<EncoderInfo>,
    /// Which run `encoder` was parsed for. `None` is a real value: a chip
    /// picked up while nothing identifiable is running belongs to no run, and
    /// must not survive into one.
    encoder_run_id: Option<crate::events::RunId>,
    /// When this monitor was built, against [`super::now_unix_ms`]'s clock.
    ///
    /// The preload window ([`RUNTIME_LOG_PRELOAD_LINES`]) reads log lines
    /// written *before* this monitor existed. Those are only this session's
    /// lines when the session predates the monitor — Sabrage opened onto a
    /// game that was already running, which is the case preload exists for. A
    /// session that starts *after* the monitor writes its own encoder line,
    /// and anything already in the file then belongs to a previous one: the
    /// stale `(HEVC, native helper)` chip that would otherwise sit where
    /// "waiting for encoder…" belongs and mask an `(H.264, in-process)`
    /// downgrade for as long as the session lasts.
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
    /// The monitor is built once and outlives every session it watches, so
    /// without this the *previous* session's freshness history decides whether
    /// the current one is `Stalled` — a fresh launch inherits `ever_fresh` and
    /// a timestamp it never wrote, and flips to Stalled the moment it passes
    /// the startup grace. `None` is a real value (nothing identifiable is
    /// running), and history recorded under it must not survive into a run
    /// either.
    fresh_run_id: Option<crate::events::RunId>,
    /// The phase reported by the *previous* [`snapshot`](Self::snapshot) call.
    /// Purely for detecting the Idle/Exited *entry* edge that clears
    /// `encoder` — nothing else here is a function of history across polls.
    last_phase: SessionPhase,
}

/// Which derived source produced a snapshot's phase, before the run stage's
/// published phase is weighed against it. See [`SessionMonitor::snapshot`]'s
/// precedence table.
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

    /// One snapshot, combining [`super::live_session`], the run stage's
    /// published [`super::RunPhaseInfo`], the persisted
    /// [`super::state::SessionState`], the wine child's liveness, the
    /// freshness of `runtime_status.json`, a running `Beat Saber.exe` nothing
    /// here started, and the newest `encoder ready` line in the runtime log.
    ///
    /// # Phase precedence (highest first)
    ///
    /// | # | source | phase |
    /// |---|---|---|
    /// | 1 | published | `Stopping` |
    /// | 2 | `live_session()` | `Running` / `Stalled` / `Exited` |
    /// | 3 | published | `Preflight` / `Launching` |
    /// | 4 | `session-state.json` | `Detached` / `Running` / `Exited` |
    /// | 5 | fresh `runtime_status.json` + a live pid, or a live `Beat Saber.exe` | `External` |
    /// | 6 | published | `Exited` (carrying `exit_code`) |
    /// | 7 | — | `Idle` |
    ///
    /// The two inversions are the whole point of the ordering:
    ///
    /// * **Published `Stopping` outranks a live handle.** Teardown publishes
    ///   `Stopping` *before* it clears [`super::LIVE_SESSION`]; without this
    ///   row a session being torn down keeps reading `Running` (or `Exited`,
    ///   which is worse — it looks finished while wineserver is still going
    ///   down) for the whole of it.
    /// * **Published `Exited` ranks below `session-state.json`.** It survives
    ///   `run()` returning, so the Session screen can say "Exited (code N)"
    ///   until the next launch overwrites it with `Preflight` — but a session
    ///   that is genuinely on disk, detached or adopted from a previous
    ///   process, is the newer truth and wins.
    ///
    /// Identity (`run_id`, `bottle`) is taken from the published info where
    /// the derived branches supplied none — a phase without a bottle name is a
    /// Stop button that deadlocks and then dies (`bottle name required`) — and
    /// **wholesale**, pid and log path blanked, when a published phase beats a
    /// `session-state.json` describing a *different* run (#200): the phase and
    /// the run it names must be the same run, or Stop targets a session the
    /// `RunRegistry` does not know. A winning publication also marks the
    /// snapshot `owned_by_this_process` (#201): `RUN_PHASE` is in-process by
    /// construction, so nothing else can have written it.
    ///
    /// `exit_code` rides along with any published `Exited` whenever the
    /// resulting phase is `Exited`, even if the phase itself came from
    /// elsewhere — the status line and the number it names must agree.
    ///
    /// Never fails: an unreadable source degrades one field (a `None`, a
    /// `runtime_fresh: false`) rather than the whole snapshot. A status poll
    /// that can error is a status poll that spends the user's attention on
    /// itself.
    pub async fn snapshot(&mut self) -> SessionStatus {
        let now = super::now_unix_ms();
        let mut status = SessionStatus::default();

        // ── phase + identity: live_session() > persisted state > Idle ──────
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
            // recorded `start_time` is the spawn fallback's 0 is
            // `Unverifiable` — live as far as anything can tell — and every
            // door treats it that way. Rendering it `Exited` put a Launch
            // button under a session the launch path refuses.
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

        // ── freshness history belongs to ONE run ────────────────────────────
        // `ever_fresh`, `last_fresh_unix_ms` and the cached status are what the
        // stall rule below reasons over, and this monitor outlives every
        // session it watches (the app builds one, once). Carrying session A's
        // history into session B classified B as `Stalled` off A's timestamps
        // the moment B passed the startup grace — a healthy launch reported as
        // the standby freeze, without B ever having reported in at all.
        if self.fresh_run_id != status.run_id {
            self.fresh_run_id = status.run_id;
            self.ever_fresh = false;
            self.last_fresh_unix_ms = None;
            self.runtime_status = None;
        }

        // ── runtime_status.json freshness ───────────────────────────────────
        // The file is global and outlives every session, so a stamp written
        // *before* the session being reported started is the previous
        // session's — evidence about a runtime that is gone, not this one.
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

                    // ── a session nothing here started ──────────────────────
                    // No handle, no record, but the runtime is reporting *now*
                    // and the process it names is alive: `demo.sh run` in
                    // another terminal. Saying "No session running" over that
                    // invites a second launch onto a live game. Never derived
                    // from freshness alone — the pid has to answer too
                    // ([`runtime_status_live`], the same predicate every
                    // mutating door uses) — and the phase carries nothing
                    // else, because nothing else about that session is
                    // knowable from here.
                    if base == Base::None && runtime_status_live(&rs, now) {
                        base = Base::External;
                        status.phase = SessionPhase::External;
                        status.pid = rs.process_id;
                    }
                }
                self.runtime_status = Some(rs);
            }
        }

        // ── a session with nothing written about it at all ──────────────────
        // The seventh door signal ([`super::running_game_pid`]) rendered.
        // `runtime_status.json` only appears once *streaming* starts, which is
        // minutes after a `./demo.sh run` spawned the game — and for that whole
        // window every file-based source above reads idle. The doors already
        // refuse there (A13a-2), so without this the Session screen said
        // "Idle" and offered Launch/Run/Fix while every one of them died with
        // a Fatal: exactly the disagreement the `runtime_status_live` fix
        // above removed, one signal later.
        //
        // Last, and only for `Base::None`: it is the one probe that costs a
        // full process-table walk, and any of the branches above already knows
        // more about the session than a pid.
        if base == Base::None {
            if let Some(pid) = super::running_game_pid() {
                base = Base::External;
                status.phase = SessionPhase::External;
                status.pid = Some(pid);
            }
        }

        // ── encoder chip: the last `encoder ready` line seen so far ─────────
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
        // ── Stalled: Running + streaming, past the startup grace, stale too long
        //
        // oxrsys writes `runtime_status.json` on state changes (`SetIdle` /
        // `SetStreaming`) and then once per second *only while streaming*
        // (`SetStreamingStats`, StreamingServer.cpp) — there is no idle
        // heartbeat. So staleness means "the stream's heartbeat stopped" only
        // when the file's last state is `streaming`; an `idle` runtime waiting
        // for the headset is legitimately stale for as long as the user takes
        // to put it on, and must never read as Stalled (live-verified
        // 2026-08-29: a fresh launch flipped to Stalled 13 s in, before the
        // Quest had connected).
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

        // ── the run stage's published phase, at its rank ────────────────────
        // The table in this method's doc comment, in code. `Preflight` /
        // `Launching` / `Stopping` exist only in the run stage's own head —
        // there is no `LIVE_SESSION` yet, or teardown has already started
        // clearing it — so nothing derived above can produce them at all.
        if let Some(info) = run_phase() {
            let outranks_derived = match info.phase {
                // 1: teardown is underway even though the handle is still up.
                SessionPhase::Stopping => true,
                // 3: a launch that has not published its handle yet.
                SessionPhase::Preflight | SessionPhase::Launching => base != Base::Live,
                // 5: `Exited` — and defensively anything else the run stage
                // might one day publish — is the weakest signal there is: it
                // fills `Idle` and nothing else.
                _ => base == Base::None,
            };
            if outranks_derived {
                status.phase = info.phase;
                // #201: `RUN_PHASE` is an in-process global (`session::mod`) —
                // only *this* process's run stage can have published it. So a
                // published phase that wins is by construction a session this
                // Sabrage owns, even in the window where `LIVE_SESSION` is not
                // populated yet (Preflight/Launching) or has already been
                // cleared. Without this the Session screen shows "a session is
                // running outside this Sabrage instance" through every normal
                // launch's preflight.
                status.owned_by_this_process = true;
                // #200: a published phase outranking a *persisted* derivation
                // for a DIFFERENT run means a launch has started over a stale
                // `session-state.json` — a detached or crashed session A on
                // disk while the user launches bottle B. Keeping A's identity
                // under B's phase points Stop at a run the `RunRegistry` has
                // never heard of (a silent no-op while B keeps launching), so
                // the publication's identity is taken wholesale and the fields
                // it cannot carry are dropped rather than left describing A.
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

        // A session that has just settled into Idle or Exited has nothing
        // left to report a codec for — carrying the *previous* session's
        // `encoder ready` line forward here would show it as a still-healthy
        // chip for a session that no longer exists. Edge-triggered (only on
        // entry into Idle/Exited from something else) so a monitor that has
        // never seen a session — the common Idle-from-`new()` case exercised
        // by tests that never touch `LIVE_SESSION` at all — does not have a
        // freshly parsed chip yanked back out from under it in the very same
        // poll that produced it.
        let was_idle_or_exited =
            matches!(self.last_phase, SessionPhase::Idle | SessionPhase::Exited);
        let now_idle_or_exited = matches!(status.phase, SessionPhase::Idle | SessionPhase::Exited);
        if now_idle_or_exited && !was_idle_or_exited {
            self.encoder = None;
            self.encoder_run_id = None;
        }
        // …and a chip never crosses from one run to another. The edge above
        // cannot catch a monitor that reports Running from its very first poll
        // (a fresh launch under a Sabrage that was already open never passes
        // through Idle *within this monitor*), which is exactly the case where
        // the log still holds the previous session's line.
        if self.encoder_run_id != status.run_id {
            self.encoder = None;
            self.encoder_run_id = None;
        }
        if let Some((info, line_unix_ms)) = parsed {
            // A line the tail read *live* was written after this poll's
            // predecessor, so the session being reported is the one that wrote
            // it. A line out of the preload window is history, and history is
            // only this session's when two things hold at once: the session
            // predates the monitor (see `created_at_unix_ms`), and the line's
            // own timestamp is not older than the session
            // ([`parse_log_timestamp`]). The log is one appending, rotating
            // sink shared by every session that ever ran, so without the second
            // half a Sabrage opened onto a live game republished *some*
            // previous session's chip — masking an `(H.264, in-process)`
            // downgrade for as long as the session lasted, since the line is
            // emitted once per session. A preloaded line with no readable
            // timestamp proves nothing and is dropped.
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
mod tests {
    use super::*;

    #[test]
    fn the_monitor_derives_its_source_paths_from_oxr_appsup() {
        let mut paths = Paths::new("/repo");
        paths.oxr_appsup = PathBuf::from("/home/Library/Application Support/OXRSys");
        let m = SessionMonitor::new(paths);
        assert_eq!(
            m.runtime_status_path(),
            PathBuf::from("/home/Library/Application Support/OXRSys/runtime_status.json")
        );
        assert_eq!(
            m.runtime_log_path(),
            PathBuf::from("/home/Library/Application Support/OXRSys/oxrsys-runtime.log")
        );
    }

    #[test]
    fn parse_runtime_status_accepts_the_observed_and_minimal_documents_and_rejects_a_half_written_one(
    ) {
        // The observed file, verbatim — `transport` is not modelled.
        let json = r#"{"state":"idle","transport":"","process_id":59004,
                       "application_name":"Beat Saber","updated_at_unix_ms":1786300214181}"#;
        let s = parse_runtime_status(json).expect("the observed document parses");
        assert_eq!(s.state, "idle");
        assert_eq!(s.process_id, Some(59004));
        assert_eq!(s.application_name.as_deref(), Some("Beat Saber"));
        assert_eq!(s.updated_at_unix_ms, 1786300214181);

        // Only `state` + `updated_at_unix_ms` are required.
        let bare = parse_runtime_status(r#"{"state":"idle","updated_at_unix_ms":1}"#)
            .expect("the minimal document parses");
        assert!(bare.process_id.is_none() && bare.application_name.is_none());

        assert!(parse_runtime_status(r#"{"state":"idle","updated_at_"#).is_none());
        assert!(parse_runtime_status("").is_none());
    }

    #[test]
    fn the_staleness_budget_is_three_seconds() {
        assert_eq!(RUNTIME_STATUS_MAX_AGE, Duration::from_secs(3));
    }

    #[test]
    fn the_startup_and_stall_grace_budgets() {
        assert_eq!(SESSION_STARTUP_GRACE, Duration::from_secs(30));
        assert_eq!(STALL_GRACE_AFTER_FRESH, Duration::from_secs(10));
    }

    #[test]
    fn is_fresh_tolerates_a_clock_skewed_slightly_into_the_future() {
        assert!(is_fresh(1_000, 1_000)); // exactly now
        assert!(is_fresh(1_000, 900)); // "updated" is in the future — clock skew, not staleness
        assert!(is_fresh(1_000, 1_000 + 3_000)); // exactly at the budget
        assert!(!is_fresh(1_000, 1_000 + 3_001)); // one ms past it
    }

    #[test]
    fn a_stamp_far_in_the_future_is_wrong_not_fresh() {
        let now = 1_786_300_214_181u64;
        assert!(
            is_fresh(now + 1_000, now),
            "ordinary skew is still believed"
        );
        assert!(
            is_fresh(now + MAX_FUTURE_SKEW.as_millis() as u64, now),
            "exactly at the allowance"
        );
        assert!(!is_fresh(now + 2_001, now), "one ms past it");
        assert!(
            !is_fresh(now + 3_600_000, now),
            "an hour ahead is a clock correction or a corrupt number — and it would \
             otherwise read as fresh for that whole hour, suppressing Stalled"
        );
    }

    /// A10-8. One predicate, two readers: the `External` phase the Session
    /// screen shows and the door every mutating operation goes through. They
    /// used to spell "is the runtime live" differently, so the UI could say
    /// Idle while Settings refused to save over the same file.
    #[test]
    fn runtime_status_live_is_freshness_and_a_live_pid_together() {
        let now = crate::session::now_unix_ms();
        let status = |pid: Option<u32>, at: u64| RuntimeStatus {
            state: "streaming".into(),
            process_id: pid,
            updated_at_unix_ms: at,
            application_name: None,
        };
        let me = std::process::id();
        assert!(runtime_status_live(&status(Some(me), now), now));
        assert!(
            !runtime_status_live(&status(Some(me), now - 60_000), now),
            "the file outlives the runtime"
        );
        assert!(
            !runtime_status_live(&status(Some(u32::MAX - 1), now), now),
            "fresh, but nothing is there"
        );
        assert!(
            !runtime_status_live(&status(None, now), now),
            "oxrsys always writes process_id; a file without one vouches for nothing"
        );
    }

    /// A9-6. The spdlog prefix oxrsys writes (`Config.cpp`'s
    /// `[%Y-%m-%d %H:%M:%S.%e] [%l] %v`), read back as local wall-clock time —
    /// the only thing that can say which session a preloaded log line belongs
    /// to.
    #[test]
    fn parse_log_timestamp_reads_the_spdlog_prefix_as_local_time() {
        use chrono::TimeZone;
        let at = chrono::Local
            .with_ymd_and_hms(2026, 8, 10, 1, 30, 13)
            .single()
            .expect("an unambiguous local time");
        assert_eq!(
            parse_log_timestamp(
                "[2026-08-10 01:30:13.017] [info] OXRSys/ALVR: encoder ready 2064x2208 @72Hz \
                 100Mbps (HEVC, native helper)"
            ),
            Some(at.timestamp_millis() as u64 + 17)
        );
        // Anything that is not that prefix carries no time at all.
        assert!(parse_log_timestamp(
            "OXRSys/ALVR: encoder ready 2064x2208 @72Hz 100Mbps (HEVC, native helper)"
        )
        .is_none());
        assert!(parse_log_timestamp("[info] no date here]").is_none());
        assert!(parse_log_timestamp("").is_none());
    }

    // ── parse_encoder_ready ──────────────────────────────────────────────────

    fn fixture_log_lines() -> Vec<String> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/phase3/oxrsys-runtime-sample.log.txt");
        std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn parse_encoder_ready_reads_the_hevc_native_helper_form_from_the_fixture() {
        let lines = fixture_log_lines();
        let line = lines
            .iter()
            .find(|l| l.contains("(HEVC, native helper)"))
            .expect("fixture has a HEVC/native-helper encoder-ready line");
        let info = parse_encoder_ready(line).expect("parses");
        assert_eq!(info.codec, "HEVC");
        assert_eq!(info.path, "native helper");
        assert_eq!(info.width, 3008);
        assert_eq!(info.height, 1664);
        assert_eq!(info.refresh_hz, 72);
        assert_eq!(info.bitrate_mbps, 80);
    }

    #[test]
    fn parse_encoder_ready_reads_the_h264_in_process_downgrade_form_from_the_fixture() {
        let lines = fixture_log_lines();
        let line = lines
            .iter()
            .find(|l| l.contains("(H.264, in-process)"))
            .expect("fixture has the downgrade form");
        let info = parse_encoder_ready(line).expect("parses");
        assert_eq!(info.codec, "H.264");
        assert_eq!(info.path, "in-process");
        assert_eq!(info.width, 3008);
        assert_eq!(info.height, 1664);
        assert_eq!(info.refresh_hz, 72);
        assert_eq!(info.bitrate_mbps, 80);
    }

    #[test]
    fn parse_encoder_ready_ignores_unrelated_fixture_lines() {
        for line in fixture_log_lines() {
            if !line.contains("encoder ready") {
                assert!(
                    parse_encoder_ready(&line).is_none(),
                    "false match on: {line}"
                );
            }
        }
        assert!(parse_encoder_ready("").is_none());
    }

    #[test]
    fn parse_encoder_ready_works_on_the_bare_message_with_no_timestamp_prefix() {
        let info = parse_encoder_ready(
            "OXRSys/ALVR: encoder ready 2064x2208 @72Hz 100Mbps (HEVC, native helper)",
        )
        .unwrap();
        assert_eq!(
            (info.width, info.height, info.refresh_hz, info.bitrate_mbps),
            (2064, 2208, 72, 100)
        );
    }

    #[test]
    fn the_encoder_ready_format_string_pin_is_unchanged() {
        // F11: if oxrsys ever changes this spdlog format string, this test
        // goes red before `parse_encoder_ready` silently starts missing every
        // encoder-ready line.
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .canonicalize()
            .expect("repo root resolves");
        let cpp = root.join("ext/oxrsys/runtime/src/AlvrStreamingBackend.cpp");
        let text = std::fs::read_to_string(&cpp)
            .unwrap_or_else(|e| panic!("could not read {cpp:?} (submodule not checked out?): {e}"));
        assert!(
            text.contains("encoder ready {}x{} @{}Hz {}Mbps ({}, {})"),
            "the oxrsys spdlog format string changed — update parse_encoder_ready to match ({cpp:?})"
        );
    }

    // ── SessionMonitor::snapshot ─────────────────────────────────────────────

    mod snapshot_tests {
        use super::super::*;
        use crate::process::ProcInfo;
        use crate::session::{
            clear_live_session, lock_session_globals, publish_run_phase, set_live_session,
            LiveSessionHandle, RunPhaseInfo, LIVE_SESSION,
        };
        use std::path::{Path, PathBuf};
        use tokio_util::sync::CancellationToken;
        use uuid::Uuid;

        fn scratch(tag: &str) -> PathBuf {
            let dir = std::env::temp_dir()
                .join(format!("sabrage-watcher-test-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            dir
        }

        fn fixture_paths(root: &Path) -> Paths {
            let mut paths = Paths::new(root);
            paths.oxr_appsup = root.join("home/Library/Application Support/OXRSys");
            paths.sabrage_appsup = root.join("home/Library/Application Support/Sabrage");
            paths
        }

        /// Best-effort cleanup of the process-global live-session slot — and
        /// the run-phase override slot, the same kind of global — before a
        /// test that must start from Idle with nothing published. Cannot
        /// fully rule out interference from a concurrently running test in
        /// another module that also touches these globals (same caveat
        /// `session::mod`'s own tests carry).
        fn force_idle() {
            if let Ok(mut g) = LIVE_SESSION.lock() {
                *g = None;
            }
            publish_run_phase(None);
        }

        /// A live handle for `run_id`, whose recorded identity is this test
        /// process — i.e. one `snapshot()` will read as still alive.
        fn live(run_id: uuid::Uuid) {
            set_live_session(LiveSessionHandle {
                run_id,
                bottle: "LiveBottle".into(),
                identity: ProcInfo::observe(std::process::id()).unwrap(),
                log_path: PathBuf::from("/repo/logs/live.log"),
                started_at_unix_ms: crate::session::now_unix_ms(),
                cancel: CancellationToken::new(),
                detach: CancellationToken::new(),
            });
        }

        /// Write a `session-state.json` under `paths` whose recorded wine pid
        /// is either this process (alive -> `Running`) or the pid that cannot
        /// exist (dead -> `Exited`).
        fn persisted(paths: &Paths, alive: bool) {
            std::fs::create_dir_all(&paths.sabrage_appsup).unwrap();
            let (pid, start) = if alive {
                let me = ProcInfo::observe(std::process::id()).unwrap();
                (me.pid, me.start_time)
            } else {
                (u32::MAX - 1, 1)
            };
            std::fs::write(
                paths.session_state_path(),
                format!(
                    r#"{{"version":1,"runId":"00000000-0000-0000-0000-000000000000",
                        "bottle":"PersistedBottle","bsDir":"/games/bs","startedAtUnixMs":0,
                        "logPath":"/repo/logs/x.log",
                        "wine":{{"pid":{pid},"startTime":{start},"exe":""}}}}"#
                ),
            )
            .unwrap();
        }

        fn publish(phase: SessionPhase, run_id: uuid::Uuid, exit_code: Option<i32>) {
            publish_run_phase(Some(RunPhaseInfo {
                phase,
                run_id,
                bottle: "PublishedBottle".into(),
                exit_code,
            }));
        }

        /// A9-6. The runtime log is global and append-only across sessions
        /// (oxrsys opens it with a rotating sink; neither front-end truncates
        /// it), and a new monitor preloads the last 200 lines. Publishing an
        /// `encoder ready` line from *before* this session as its chip shows a
        /// healthy `(HEVC, native helper)` where "waiting for encoder…"
        /// belongs — and hides an `(H.264, in-process)` downgrade for as long
        /// as the session lasts, since the line is emitted once per session.
        #[tokio::test]
        async fn a_previous_sessions_encoder_line_is_never_published_for_a_new_run() {
            let _g = lock_session_globals();
            force_idle();

            let dir = scratch("encoder-previous-session");
            let paths = fixture_paths(&dir);
            std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
            let log_path = paths.oxr_appsup.join("oxrsys-runtime.log");
            // Yesterday's session, still in the file.
            std::fs::write(
                &log_path,
                "[2026-08-28 21:00:00.000] [info] OXRSys/ALVR: encoder ready 3008x1664 @72Hz \
                 80Mbps (HEVC, native helper)\n",
            )
            .unwrap();

            let mut m = SessionMonitor::new(paths);

            // A session that starts *after* this monitor: its own encoder line
            // has not been written yet.
            let run_id = Uuid::new_v4();
            set_live_session(LiveSessionHandle {
                run_id,
                bottle: "Steam".into(),
                identity: ProcInfo::observe(std::process::id()).unwrap(),
                log_path: PathBuf::from("/repo/logs/x.log"),
                started_at_unix_ms: crate::session::now_unix_ms() + 1,
                cancel: CancellationToken::new(),
                detach: CancellationToken::new(),
            });

            let s = m.snapshot().await;
            assert_eq!(s.phase, SessionPhase::Running);
            assert!(
                s.encoder.is_none(),
                "yesterday's chip must not be this session's: {:?}",
                s.encoder
            );

            // …and this session's own line, appended while it runs, IS the chip.
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&log_path)
                .unwrap();
            writeln!(
                f,
                "[2026-08-29 10:00:00.000] [info] OXRSys/ALVR: encoder ready 3008x1664 @72Hz \
                 80Mbps (H.264, in-process)"
            )
            .unwrap();
            let s = m.snapshot().await;
            let enc = s.encoder.expect("the running session's own line");
            assert_eq!(
                (enc.codec.as_str(), enc.path.as_str()),
                ("H.264", "in-process")
            );

            // A different run inherits nothing, even without an Idle edge in
            // between: the chip names the run it was parsed for.
            clear_live_session(run_id);
            let next = Uuid::new_v4();
            publish(SessionPhase::Launching, next, None);
            let s = m.snapshot().await;
            publish_run_phase(None);
            assert_eq!(s.phase, SessionPhase::Launching);
            assert!(s.encoder.is_none(), "a chip never crosses runs");

            std::fs::remove_dir_all(&dir).ok();
        }

        /// One oxrsys log line, timestamped in the machine's local time the
        /// way spdlog writes it (`[%Y-%m-%d %H:%M:%S.%e] [%l] %v`).
        fn log_line(at_unix_ms: u64, message: &str) -> String {
            use chrono::TimeZone;
            let at = chrono::Local
                .timestamp_millis_opt(at_unix_ms as i64)
                .single()
                .expect("a representable local time");
            format!(
                "[{}] [info] {message}\n",
                at.format("%Y-%m-%d %H:%M:%S%.3f")
            )
        }

        /// A9-6, both halves. The preload window is believed for an *adopted*
        /// session — one that started before the monitor — and the 200 lines it
        /// reads span every session that ever ran. A line stamped after this
        /// session started is its own, and the chip it negotiated is published;
        /// a line stamped before it belongs to a previous session, and
        /// publishing that one puts a healthy `(HEVC, native helper)` chip where
        /// "waiting for encoder…" belongs.
        #[tokio::test]
        async fn an_adopted_session_only_inherits_lines_written_after_it_started() {
            let _g = lock_session_globals();

            // (a) older than the session, (b) and (c) written after it started, (d) no timestamp at all.
            for (row, line, want_codec) in [
                (
                    "a line from before this session started",
                    log_line(
                        crate::session::now_unix_ms() - 3_600_000,
                        "OXRSys/ALVR: encoder ready 3008x1664 @72Hz 80Mbps (HEVC, native helper)",
                    ),
                    None,
                ),
                (
                    "a line this session wrote before the monitor opened",
                    log_line(
                        crate::session::now_unix_ms() - 4_000,
                        "OXRSys/ALVR: encoder ready 3008x1664 @72Hz 80Mbps (H.264, in-process)",
                    ),
                    Some("H.264"),
                ),
                (
                    "r1:A9-6 regression: a session that predates the monitor keeps the chip it negotiated",
                    log_line(
                        crate::session::now_unix_ms() - 4_000,
                        "OXRSys/ALVR: encoder ready 3008x1664 @72Hz 80Mbps (HEVC, native helper)",
                    ),
                    Some("HEVC"),
                ),
                (
                    "an undated line proves nothing",
                    "OXRSys/ALVR: encoder ready 3008x1664 @72Hz 80Mbps (HEVC, native helper)\n"
                        .to_string(),
                    None,
                ),
            ] {
                force_idle();
                let dir = scratch("encoder-adopted-window");
                let paths = fixture_paths(&dir);
                std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
                std::fs::write(paths.oxr_appsup.join("oxrsys-runtime.log"), &line).unwrap();

                let run_id = Uuid::new_v4();
                set_live_session(LiveSessionHandle {
                    run_id,
                    bottle: "Steam".into(),
                    identity: ProcInfo::observe(std::process::id()).unwrap(),
                    log_path: PathBuf::from("/repo/logs/x.log"),
                    started_at_unix_ms: crate::session::now_unix_ms() - 5_000,
                    cancel: CancellationToken::new(),
                    detach: CancellationToken::new(),
                });
                let mut m = SessionMonitor::new(paths);
                let s = m.snapshot().await;
                clear_live_session(run_id);

                assert_eq!(s.phase, SessionPhase::Running, "{row}");
                assert_eq!(s.encoder.map(|e| e.codec).as_deref(), want_codec, "{row}");
                std::fs::remove_dir_all(&dir).ok();
            }
        }

        /// A8-5. `demo.sh run` publishes no handle and writes no
        /// `session-state.json`, but the runtime it launched reports in every
        /// second. Reporting that as Idle ("No session running") is how a user
        /// launches a second game over a live one.
        #[tokio::test]
        async fn a_session_started_outside_sabrage_is_reported_not_called_idle() {
            let _g = lock_session_globals();
            force_idle();
            let now = crate::session::now_unix_ms();

            // Fresh status naming a process that is genuinely alive.
            {
                let dir = scratch("external-live");
                let paths = fixture_paths(&dir);
                std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
                std::fs::write(
                    paths.oxr_appsup.join("runtime_status.json"),
                    format!(
                        r#"{{"state":"streaming","process_id":{},"updated_at_unix_ms":{now}}}"#,
                        std::process::id()
                    ),
                )
                .unwrap();

                let mut m = SessionMonitor::new(paths);
                let s = m.snapshot().await;
                assert_eq!(s.phase, SessionPhase::External);
                assert_eq!(s.pid, Some(std::process::id()));
                assert!(!s.owned_by_this_process, "it is not ours");
                assert!(s.run_id.is_none() && s.bottle.is_none());
                assert!(s.runtime_fresh);
                std::fs::remove_dir_all(&dir).ok();
            }

            // Never from freshness alone: the pid has to answer too.
            {
                let dir = scratch("external-dead-pid");
                let paths = fixture_paths(&dir);
                std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
                std::fs::write(
                    paths.oxr_appsup.join("runtime_status.json"),
                    format!(
                        r#"{{"state":"streaming","process_id":4294967294,"updated_at_unix_ms":{now}}}"#
                    ),
                )
                .unwrap();
                let mut m = SessionMonitor::new(paths);
                assert_eq!(m.snapshot().await.phase, SessionPhase::Idle);
                std::fs::remove_dir_all(&dir).ok();
            }

            // …nor from a stale file, however alive the pid it names.
            {
                let dir = scratch("external-stale");
                let paths = fixture_paths(&dir);
                std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
                std::fs::write(
                    paths.oxr_appsup.join("runtime_status.json"),
                    format!(
                        r#"{{"state":"streaming","process_id":{},"updated_at_unix_ms":{}}}"#,
                        std::process::id(),
                        now - 60_000
                    ),
                )
                .unwrap();
                let mut m = SessionMonitor::new(paths);
                assert_eq!(m.snapshot().await.phase, SessionPhase::Idle);
                std::fs::remove_dir_all(&dir).ok();
            }

            // Our own launch still outranks it: Preflight is the truth about
            // what this Sabrage is doing.
            {
                let dir = scratch("external-vs-preflight");
                let paths = fixture_paths(&dir);
                std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
                std::fs::write(
                    paths.oxr_appsup.join("runtime_status.json"),
                    format!(
                        r#"{{"state":"streaming","process_id":{},"updated_at_unix_ms":{now}}}"#,
                        std::process::id()
                    ),
                )
                .unwrap();
                publish(SessionPhase::Preflight, Uuid::new_v4(), None);
                let mut m = SessionMonitor::new(paths);
                let s = m.snapshot().await;
                publish_run_phase(None);
                assert_eq!(s.phase, SessionPhase::Preflight);
                std::fs::remove_dir_all(&dir).ok();
            }
        }

        /// A13a-2, rendered. The door grew a seventh signal — a running
        /// `Beat Saber.exe` — for the window a `./demo.sh run` spends between
        /// its wine spawn and its first `runtime_status.json`. The phase has
        /// to carry it too: with the monitor still reporting `Idle` there,
        /// Library/Session leave Launch enabled and Doctor leaves every Fix
        /// enabled, and each one then dies with the Fatal the door raises.
        #[tokio::test]
        async fn a_running_game_with_nothing_on_disk_is_external_not_idle() {
            let _g = lock_session_globals();
            force_idle();

            let dir = scratch("external-running-game");
            let paths = fixture_paths(&dir);
            std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
            let mut m = SessionMonitor::new(paths);
            assert_eq!(
                m.snapshot().await.phase,
                SessionPhase::Idle,
                "nothing on disk, no game: idle"
            );

            // Stand in for the wine child, argv exactly as wine spells it —
            // the shape `pgrep -f 'Beat Saber.exe'` and the door both match.
            let mut game = std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg("sleep 20 # Z:\\games\\Beat Saber 1294\\Beat Saber.exe")
                .spawn()
                .expect("/bin/sh is on every macOS");

            // The process table is refreshed per call; give the spawn a moment
            // to appear rather than assuming it already has.
            let mut seen = SessionStatus::default();
            for _ in 0..50 {
                seen = m.snapshot().await;
                if seen.phase != SessionPhase::Idle {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            let game_pid = game.id();
            let _ = game.kill();
            let _ = game.wait();

            assert_eq!(
                seen.phase,
                SessionPhase::External,
                "a running game the door refuses over must not render as Idle"
            );
            assert_eq!(seen.pid, Some(game_pid));
            assert!(!seen.owned_by_this_process, "it is not ours");
            assert!(seen.run_id.is_none() && seen.bottle.is_none());
            std::fs::remove_dir_all(&dir).ok();
        }

        /// A9-7. The monitor is built once and outlives every session it
        /// watches, so the freshness history it accumulates has to name the
        /// run it belongs to. Session A's `ever_fresh` + last-fresh timestamp
        /// classified session B as `Stalled` the moment B passed the startup
        /// grace — the standby freeze reported for a launch that had never
        /// reported in at all.
        #[tokio::test]
        async fn freshness_history_never_crosses_from_one_session_to_the_next() {
            let _g = lock_session_globals();
            force_idle();

            let dir = scratch("stall-history");
            let paths = fixture_paths(&dir);
            std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
            let now = crate::session::now_unix_ms();
            // The `streaming` status session A left behind, long stale.
            std::fs::write(
                paths.oxr_appsup.join("runtime_status.json"),
                format!(
                    r#"{{"state":"streaming","process_id":{},"updated_at_unix_ms":{}}}"#,
                    std::process::id(),
                    now - 60_000
                ),
            )
            .unwrap();

            // Session B: ours, running, well past the startup grace.
            let run_b = Uuid::new_v4();
            set_live_session(LiveSessionHandle {
                run_id: run_b,
                bottle: "Steam".into(),
                identity: ProcInfo::observe(std::process::id()).unwrap(),
                log_path: PathBuf::from("/repo/logs/x.log"),
                started_at_unix_ms: now - 60_000,
                cancel: CancellationToken::new(),
                detach: CancellationToken::new(),
            });

            // The history is session A's: not evidence about B.
            let mut m = SessionMonitor::new(paths.clone());
            m.ever_fresh = true;
            m.last_fresh_unix_ms = Some(now - 30_000);
            m.fresh_run_id = Some(Uuid::new_v4());
            let s = m.snapshot().await;
            assert_eq!(
                s.phase,
                SessionPhase::Running,
                "session B has never reported in; A's timestamps cannot stall it"
            );
            assert!(!s.runtime_fresh);

            // …and the same history, recorded for THIS run, still stalls it —
            // the reset must not disable stall detection.
            let mut m = SessionMonitor::new(paths);
            m.ever_fresh = true;
            m.last_fresh_unix_ms = Some(now - 30_000);
            m.fresh_run_id = Some(run_b);
            let s = m.snapshot().await;
            clear_live_session(run_b);
            assert_eq!(s.phase, SessionPhase::Stalled);

            std::fs::remove_dir_all(&dir).ok();
        }

        /// A9-5. The spawn fallback records `start_time: 0` when the child
        /// could not be observed (`Executor::spawn_detached`), and
        /// `is_same_process()` is false for it forever after. Reconciliation
        /// calls that alive pid `Unverifiable` and every door treats it as
        /// live; the monitor used to call it `Exited`, which is a Launch button
        /// over a session `run` then refuses.
        #[tokio::test]
        async fn an_alive_pid_with_no_verifiable_start_time_is_never_reported_exited() {
            let _g = lock_session_globals();
            let unverifiable = ProcInfo {
                pid: std::process::id(),
                start_time: 0,
                exe: PathBuf::new(),
            };
            assert_eq!(
                crate::session::reconcile::classify_identity(Some(&unverifiable)),
                crate::session::reconcile::Classification::Unverifiable,
                "the premise: alive, and nothing about it can be checked"
            );

            // …as a live handle.
            {
                force_idle();
                let dir = scratch("unverifiable-handle");
                let paths = fixture_paths(&dir);
                std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
                let run_id = Uuid::new_v4();
                set_live_session(LiveSessionHandle {
                    run_id,
                    bottle: "Steam".into(),
                    identity: unverifiable.clone(),
                    log_path: PathBuf::from("/repo/logs/x.log"),
                    started_at_unix_ms: crate::session::now_unix_ms(),
                    cancel: CancellationToken::new(),
                    detach: CancellationToken::new(),
                });
                let mut m = SessionMonitor::new(paths);
                let s = m.snapshot().await;
                clear_live_session(run_id);
                assert_eq!(s.phase, SessionPhase::Running);
                std::fs::remove_dir_all(&dir).ok();
            }

            // …and as a persisted record, which is the shape a Sabrage that
            // reopens onto that session reads.
            {
                force_idle();
                let dir = scratch("unverifiable-record");
                let paths = fixture_paths(&dir);
                std::fs::create_dir_all(&paths.sabrage_appsup).unwrap();
                std::fs::write(
                    paths.session_state_path(),
                    format!(
                        r#"{{"version":1,"runId":"00000000-0000-0000-0000-000000000000",
                            "bottle":"Steam","bsDir":"/games/bs","startedAtUnixMs":0,
                            "logPath":"/repo/logs/x.log",
                            "wine":{{"pid":{},"startTime":0,"exe":""}}}}"#,
                        std::process::id()
                    ),
                )
                .unwrap();
                let mut m = SessionMonitor::new(paths.clone());
                let s = m.snapshot().await;
                assert_eq!(s.phase, SessionPhase::Running);
                assert!(
                    crate::session::session_block_at(
                        &paths.session_state_path(),
                        &paths.oxr_appsup.join("runtime_status.json"),
                    )
                    .is_some(),
                    "the door and the phase have to agree about this record"
                );
                std::fs::remove_dir_all(&dir).ok();
            }
        }

        /// A9-7. `runtime_status.json` is one global file that outlives every
        /// session: a stamp written *before* this session started describes the
        /// runtime that is gone, and believing it hides a stall in the one that
        /// is here.
        #[tokio::test]
        async fn a_status_written_before_this_session_started_is_not_fresh() {
            let _g = lock_session_globals();
            force_idle();
            let now = crate::session::now_unix_ms();

            let dir = scratch("status-predates-session");
            let paths = fixture_paths(&dir);
            std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
            // Written a second ago — recent by the clock, but before this
            // session began.
            std::fs::write(
                paths.oxr_appsup.join("runtime_status.json"),
                format!(
                    r#"{{"state":"streaming","updated_at_unix_ms":{}}}"#,
                    now - 1_000
                ),
            )
            .unwrap();

            let run_id = Uuid::new_v4();
            set_live_session(LiveSessionHandle {
                run_id,
                bottle: "Steam".into(),
                identity: ProcInfo::observe(std::process::id()).unwrap(),
                log_path: PathBuf::from("/repo/logs/x.log"),
                started_at_unix_ms: now,
                cancel: CancellationToken::new(),
                detach: CancellationToken::new(),
            });
            let mut m = SessionMonitor::new(paths);
            let s = m.snapshot().await;
            clear_live_session(run_id);

            assert_eq!(s.phase, SessionPhase::Running);
            assert!(
                !s.runtime_fresh,
                "the previous runtime's last word is not this session's heartbeat"
            );
            assert_eq!(
                s.runtime_state.as_deref(),
                Some("streaming"),
                "the state is still shown — it is the freshness that is withheld"
            );
            std::fs::remove_dir_all(&dir).ok();
        }

        /// #2/#100: the precedence table in [`SessionMonitor::snapshot`]'s doc
        /// comment, row by row. Consolidated into one test for the same reason
        /// `snapshot_phase_transitions` is — these all read the same
        /// process-global slots.
        #[tokio::test]
        async fn snapshot_phase_precedence_table() {
            let _g = lock_session_globals();
            force_idle();

            // Each row: (what is live, what is persisted, what is published)
            // -> the phase the snapshot must report.
            for (row, live_run, persist, published, want) in [
                (
                    "published Stopping beats a live handle",
                    true,
                    None,
                    Some(SessionPhase::Stopping),
                    SessionPhase::Stopping,
                ),
                (
                    "a live handle beats published Preflight",
                    true,
                    None,
                    Some(SessionPhase::Preflight),
                    SessionPhase::Running,
                ),
                (
                    "a live handle beats published Launching",
                    true,
                    None,
                    Some(SessionPhase::Launching),
                    SessionPhase::Running,
                ),
                (
                    "a live handle beats published Exited",
                    true,
                    None,
                    Some(SessionPhase::Exited),
                    SessionPhase::Running,
                ),
                (
                    "a live handle alone is Running",
                    true,
                    None,
                    None,
                    SessionPhase::Running,
                ),
                (
                    "published Launching beats persisted state",
                    false,
                    Some(true),
                    Some(SessionPhase::Launching),
                    SessionPhase::Launching,
                ),
                (
                    "published Stopping beats persisted state",
                    false,
                    Some(true),
                    Some(SessionPhase::Stopping),
                    SessionPhase::Stopping,
                ),
                (
                    "persisted state beats published Exited",
                    false,
                    Some(true),
                    Some(SessionPhase::Exited),
                    SessionPhase::Running,
                ),
                (
                    "persisted state alone",
                    false,
                    Some(false),
                    None,
                    SessionPhase::Exited,
                ),
                (
                    "published Preflight beats the Idle fallthrough",
                    false,
                    None,
                    Some(SessionPhase::Preflight),
                    SessionPhase::Preflight,
                ),
                (
                    "published Exited beats the Idle fallthrough",
                    false,
                    None,
                    Some(SessionPhase::Exited),
                    SessionPhase::Exited,
                ),
                (
                    "nothing at all is Idle",
                    false,
                    None,
                    None,
                    SessionPhase::Idle,
                ),
            ] {
                let dir = scratch(&format!("prec-{}", want as u8));
                let paths = fixture_paths(&dir);
                std::fs::create_dir_all(&paths.oxr_appsup).unwrap();

                publish_run_phase(None);
                let run_id = Uuid::new_v4();
                if live_run {
                    live(run_id);
                }
                if let Some(alive) = persist {
                    persisted(&paths, alive);
                }
                if let Some(phase) = published {
                    publish(phase, run_id, Some(7));
                }

                let mut m = SessionMonitor::new(paths);
                let got = m.snapshot().await;
                // Run-id guarded, so this row cannot blank a handle some
                // other module's test legitimately owns right now.
                clear_live_session(run_id);
                publish_run_phase(None);

                assert_eq!(got.phase, want, "{row}");
                assert!(got.bottle.is_some() || want == SessionPhase::Idle, "{row}");
                std::fs::remove_dir_all(&dir).ok();
            }
        }

        /// The identity a snapshot reports comes from the strongest source
        /// that has one — and a published `Exited`'s code rides along even
        /// when the phase itself was derived from `session-state.json`.
        #[tokio::test]
        async fn snapshot_identity_and_exit_code_sources() {
            let _g = lock_session_globals();
            force_idle();

            // Published only: identity comes from the publication (#100 —
            // without it the Session screen's Stop has no bottle).
            {
                let dir = scratch("ident-published");
                let paths = fixture_paths(&dir);
                std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
                publish_run_phase(None);
                let run_id = Uuid::new_v4();
                publish(SessionPhase::Launching, run_id, None);
                let mut m = SessionMonitor::new(paths);
                let s = m.snapshot().await;
                publish_run_phase(None);
                assert_eq!(s.phase, SessionPhase::Launching);
                assert_eq!(s.run_id, Some(run_id));
                assert_eq!(s.bottle.as_deref(), Some("PublishedBottle"));
                assert!(s.exit_code.is_none(), "only Exited carries a code");
                std::fs::remove_dir_all(&dir).ok();
            }

            // Published Exited with nothing else: the code is reported (#7).
            {
                let dir = scratch("ident-exited");
                let paths = fixture_paths(&dir);
                std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
                publish_run_phase(None);
                let run_id = Uuid::new_v4();
                publish(SessionPhase::Exited, run_id, Some(139));
                let mut m = SessionMonitor::new(paths);
                let s = m.snapshot().await;
                publish_run_phase(None);
                assert_eq!(s.phase, SessionPhase::Exited);
                assert_eq!(s.exit_code, Some(139));
                assert_eq!(s.bottle.as_deref(), Some("PublishedBottle"));
                std::fs::remove_dir_all(&dir).ok();
            }

            // Derived Exited (a dead pid on disk) + a published Exited: the
            // identity is the state file's, the code is the publication's.
            {
                let dir = scratch("ident-both-exited");
                let paths = fixture_paths(&dir);
                std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
                publish_run_phase(None);
                persisted(&paths, false);
                publish(SessionPhase::Exited, Uuid::new_v4(), Some(3));
                let mut m = SessionMonitor::new(paths);
                let s = m.snapshot().await;
                publish_run_phase(None);
                assert_eq!(s.phase, SessionPhase::Exited);
                assert_eq!(
                    s.bottle.as_deref(),
                    Some("PersistedBottle"),
                    "the state file knows more than a published phase does"
                );
                assert_eq!(
                    s.exit_code,
                    Some(3),
                    "the status line and the number it names must agree"
                );
                std::fs::remove_dir_all(&dir).ok();
            }

            // #200/#201: stale persisted state for run A (detached, still
            // "alive") while THIS process is in preflight for run B. The
            // phase is B's, so the identity must be B's too — otherwise the
            // Session screen routes Stop to `cancelStage(A)`, which the
            // RunRegistry has never heard of, and the launch of B carries on
            // behind a dead button. A's pid/log/started/detached go with it.
            {
                let dir = scratch("ident-stale-persisted-vs-preflight");
                let paths = fixture_paths(&dir);
                std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
                publish_run_phase(None);
                std::fs::create_dir_all(&paths.sabrage_appsup).unwrap();
                let me = ProcInfo::observe(std::process::id()).unwrap();
                std::fs::write(
                    paths.session_state_path(),
                    format!(
                        r#"{{"version":1,"runId":"11111111-1111-1111-1111-111111111111",
                            "bottle":"StaleBottleA","bsDir":"/games/bs","startedAtUnixMs":42,
                            "logPath":"/repo/logs/a.log",
                            "wine":{{"pid":{},"startTime":{},"exe":""}},"detached":true}}"#,
                        me.pid, me.start_time
                    ),
                )
                .unwrap();

                let run_b = Uuid::new_v4();
                publish(SessionPhase::Preflight, run_b, None);
                let mut m = SessionMonitor::new(paths);
                let s = m.snapshot().await;
                publish_run_phase(None);

                assert_eq!(s.phase, SessionPhase::Preflight);
                assert_eq!(s.run_id, Some(run_b), "#200: the phase names its own run");
                assert_eq!(
                    s.bottle.as_deref(),
                    Some("PublishedBottle"),
                    "#200: and its own bottle, not the stale one on disk"
                );
                assert!(s.pid.is_none(), "#200: A's pid must not ride under B");
                assert!(s.log_path.is_none(), "#200: nor A's log");
                assert!(s.started_at_unix_ms.is_none(), "#200: nor A's start time");
                assert!(!s.detached, "#200: B's preflight is not detached");
                assert!(
                    s.owned_by_this_process,
                    "#201: RUN_PHASE is in-process — a winning publication is ours"
                );
                std::fs::remove_dir_all(&dir).ok();
            }

            // #201: the same ownership claim with nothing on disk at all —
            // the ordinary launch window, where `LIVE_SESSION` is not
            // populated yet.
            {
                let dir = scratch("ident-owned-during-launch");
                let paths = fixture_paths(&dir);
                std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
                publish_run_phase(None);
                publish(SessionPhase::Launching, Uuid::new_v4(), None);
                let mut m = SessionMonitor::new(paths);
                let s = m.snapshot().await;
                publish_run_phase(None);
                assert_eq!(s.phase, SessionPhase::Launching);
                assert!(
                    s.owned_by_this_process,
                    "#201: no 'running outside this Sabrage instance' during our own launch"
                );
                std::fs::remove_dir_all(&dir).ok();
            }

            // A published Stopping over a live handle keeps the live
            // handle's identity — pid and log path included.
            {
                let dir = scratch("ident-stopping");
                let paths = fixture_paths(&dir);
                std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
                publish_run_phase(None);
                let run_id = Uuid::new_v4();
                live(run_id);
                publish(SessionPhase::Stopping, run_id, None);
                let mut m = SessionMonitor::new(paths);
                let s = m.snapshot().await;
                clear_live_session(run_id);
                publish_run_phase(None);
                assert_eq!(s.phase, SessionPhase::Stopping);
                assert_eq!(s.bottle.as_deref(), Some("LiveBottle"));
                assert!(s.pid.is_some() && s.owned_by_this_process);
                std::fs::remove_dir_all(&dir).ok();
            }
        }

        /// Every scenario below runs in strict sequence inside one test
        /// function, rather than as separate `#[tokio::test]`s. `snapshot()`
        /// reads the process-global `LIVE_SESSION` slot, so two of these
        /// scenarios genuinely race each other if the standard test harness
        /// schedules them on different OS threads at the same moment (a
        /// `detached`/`exited`/`idle` case expects the slot to be *empty*,
        /// exactly while another case is legitimately setting it) — this was
        /// caught as a real, reproducible flake, not a theoretical one.
        /// Consolidating removes the intra-file race entirely; the residual
        /// cross-file risk (another module's test touching the same global)
        /// is the same caveat `session::mod`'s own test already carries.
        #[tokio::test]
        async fn snapshot_phase_transitions() {
            let _g = lock_session_globals();
            force_idle();

            // Idle: nothing live, nothing persisted.
            {
                let dir = scratch("idle");
                let mut m = SessionMonitor::new(fixture_paths(&dir));
                let s = m.snapshot().await;
                assert_eq!(s.phase, SessionPhase::Idle);
                assert!(s.run_id.is_none() && s.pid.is_none());
            }

            // Detached: persisted state says so, and the recorded wine pid is
            // still alive (this test process, observed for real).
            {
                let dir = scratch("detached");
                let paths = fixture_paths(&dir);
                std::fs::create_dir_all(&paths.sabrage_appsup).unwrap();
                std::fs::create_dir_all(&paths.oxr_appsup).unwrap();

                let me = ProcInfo::observe(std::process::id()).unwrap();
                let json = format!(
                    r#"{{"version":1,"runId":"00000000-0000-0000-0000-000000000000","bottle":"Steam","bsDir":"/games/bs","startedAtUnixMs":0,"logPath":"/repo/logs/x.log","wine":{{"pid":{},"startTime":{},"exe":"{}"}},"detached":true}}"#,
                    me.pid,
                    me.start_time,
                    me.exe.display()
                );
                std::fs::write(paths.session_state_path(), json).unwrap();

                let mut m = SessionMonitor::new(paths);
                let s = m.snapshot().await;

                assert_eq!(s.phase, SessionPhase::Detached);
                assert!(!s.owned_by_this_process);
                assert!(
                    s.detached,
                    "F3: status.detached must mirror the persisted flag"
                );
            }

            // Exited: persisted state's recorded wine pid is dead.
            {
                let dir = scratch("exited");
                let paths = fixture_paths(&dir);
                std::fs::create_dir_all(&paths.sabrage_appsup).unwrap();
                std::fs::create_dir_all(&paths.oxr_appsup).unwrap();

                // u32::MAX - 1: the "cannot exist for this user" pid, same
                // idiom process.rs's own tests use.
                let json = r#"{"version":1,"runId":"00000000-0000-0000-0000-000000000000","bottle":"Steam","bsDir":"/games/bs","startedAtUnixMs":0,"logPath":"/repo/logs/x.log","wine":{"pid":4294967294,"startTime":1,"exe":""}}"#;
                std::fs::write(paths.session_state_path(), json).unwrap();

                let mut m = SessionMonitor::new(paths);
                let s = m.snapshot().await;

                assert_eq!(s.phase, SessionPhase::Exited);
                assert!(
                    !s.detached,
                    "no `detached` key in the fixture — must default false"
                );
            }

            // F2: the run stage's published phase wins over whatever would
            // otherwise be derived — here, plain Idle (nothing live, nothing
            // persisted) — for exactly the three phases only the run stage
            // can know about. #100: it must also publish the identity that
            // goes with the phase, or the Session screen offers a Stop button
            // with no bottle to stop.
            for phase in [
                SessionPhase::Preflight,
                SessionPhase::Launching,
                SessionPhase::Stopping,
            ] {
                let dir = scratch(&format!("run-phase-{phase:?}"));
                let run_id = Uuid::new_v4();
                publish_run_phase(Some(RunPhaseInfo {
                    phase,
                    run_id,
                    bottle: "Steam".into(),
                    exit_code: None,
                }));
                let mut m = SessionMonitor::new(fixture_paths(&dir));
                let s = m.snapshot().await;
                publish_run_phase(None);
                assert_eq!(s.phase, phase, "run_phase() must override the derived Idle");
                assert_eq!(s.run_id, Some(run_id), "#100: the phase names its run");
                assert_eq!(
                    s.bottle.as_deref(),
                    Some("Steam"),
                    "#100: the phase names its bottle — Stop needs it"
                );
            }

            // F16: the encoder chip must not survive a session actually
            // ending — it must clear on the edge into Exited, not linger as a
            // false-healthy chip for a session that no longer exists. Same
            // monitor throughout, so `last_phase` genuinely transitions
            // Running -> Exited within one instance, unlike the "Idle
            // throughout" encoder-chip scenario above.
            {
                let dir = scratch("encoder-clears-on-exit");
                let paths = fixture_paths(&dir);
                std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
                std::fs::create_dir_all(&paths.sabrage_appsup).unwrap();
                let log_path = paths.oxr_appsup.join("oxrsys-runtime.log");
                let started = crate::session::now_unix_ms() - 1_000;
                // Timestamped, and after this session started: a preloaded
                // line has to prove whose it is (A9-6).
                std::fs::write(
                    &log_path,
                    log_line(
                        started,
                        "OXRSys/ALVR: encoder ready 3008x1664 @72Hz 80Mbps (HEVC, native helper)",
                    ),
                )
                .unwrap();

                let run_id = Uuid::new_v4();
                set_live_session(LiveSessionHandle {
                    run_id,
                    bottle: "Steam".into(),
                    identity: ProcInfo::observe(std::process::id()).unwrap(),
                    log_path: PathBuf::from("/repo/logs/x.log"),
                    started_at_unix_ms: started,
                    cancel: CancellationToken::new(),
                    detach: CancellationToken::new(),
                });

                let mut m = SessionMonitor::new(paths);
                let s_running = m.snapshot().await;
                assert_eq!(s_running.phase, SessionPhase::Running);
                assert!(
                    s_running.encoder.is_some(),
                    "the chip must populate while the session is live"
                );

                // The session ends: no live handle, no persisted state either
                // (this scenario never wrote `session-state.json`) — the
                // derived phase falls all the way back to Idle.
                clear_live_session(run_id);
                let s_idle = m.snapshot().await;
                assert_eq!(s_idle.phase, SessionPhase::Idle);
                assert!(
                    s_idle.encoder.is_none(),
                    "F16: the previous session's encoder chip must not survive into Idle"
                );
            }

            // Encoder chip: picked up from a fresh line appended after the
            // monitor already exists (Idle phase throughout — no live session
            // involved, but sequenced here anyway for consistency).
            {
                let dir = scratch("encoder-chip");
                let paths = fixture_paths(&dir);
                std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
                let log_path = paths.oxr_appsup.join("oxrsys-runtime.log");
                std::fs::write(&log_path, b"").unwrap();

                let mut m = SessionMonitor::new(paths);
                let s0 = m.snapshot().await;
                assert!(s0.encoder.is_none());

                use std::io::Write;
                let mut f = std::fs::OpenOptions::new()
                    .append(true)
                    .open(&log_path)
                    .unwrap();
                writeln!(
                    f,
                    "[2026-08-29 10:00:00.000] [info] OXRSys/ALVR: encoder ready 3008x1664 @72Hz 80Mbps (HEVC, native helper)"
                )
                .unwrap();

                let s1 = m.snapshot().await;
                let enc = s1.encoder.expect("encoder chip should now be set");
                assert_eq!(enc.codec, "HEVC");
                assert_eq!(enc.path, "native helper");
                assert_eq!(
                    (enc.width, enc.height, enc.refresh_hz, enc.bitrate_mbps),
                    (3008, 1664, 72, 80)
                );
            }

            let now = crate::session::now_unix_ms();

            // Running + fresh, via the live session.
            {
                let dir = scratch("live-running");
                let paths = fixture_paths(&dir);
                std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
                std::fs::write(
                    paths.oxr_appsup.join("runtime_status.json"),
                    format!(r#"{{"state":"streaming","updated_at_unix_ms":{now}}}"#),
                )
                .unwrap();
                let run_id = Uuid::new_v4();
                set_live_session(LiveSessionHandle {
                    run_id,
                    bottle: "Steam".into(),
                    identity: ProcInfo::observe(std::process::id()).unwrap(),
                    log_path: PathBuf::from("/repo/logs/x.log"),
                    started_at_unix_ms: now,
                    cancel: CancellationToken::new(),
                    detach: CancellationToken::new(),
                });

                let mut m = SessionMonitor::new(paths);
                let s = m.snapshot().await;
                clear_live_session(run_id);

                assert_eq!(s.phase, SessionPhase::Running);
                assert!(s.owned_by_this_process);
                assert_eq!(s.run_id, Some(run_id));
                assert!(s.runtime_fresh);
                assert_eq!(s.runtime_state.as_deref(), Some("streaming"));
            }

            // Stalled: past the startup grace, the *streaming* heartbeat stale
            // for longer than the stall grace (simulated directly — no real
            // 30s+10s sleep). The file's last state must be `streaming`: that
            // is the only state oxrsys heartbeats.
            {
                let dir = scratch("live-stalled");
                let paths = fixture_paths(&dir);
                std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
                std::fs::write(
                    paths.oxr_appsup.join("runtime_status.json"),
                    format!(
                        r#"{{"state":"streaming","updated_at_unix_ms":{}}}"#,
                        now - 20_000
                    ),
                )
                .unwrap();
                let run_id = Uuid::new_v4();
                set_live_session(LiveSessionHandle {
                    run_id,
                    bottle: "Steam".into(),
                    identity: ProcInfo::observe(std::process::id()).unwrap(),
                    log_path: PathBuf::from("/repo/logs/x.log"),
                    started_at_unix_ms: now - 120_000,
                    cancel: CancellationToken::new(),
                    detach: CancellationToken::new(),
                });

                let mut m = SessionMonitor::new(paths);
                m.ever_fresh = true;
                m.last_fresh_unix_ms = Some(now - 20_000);
                // …recorded for THIS run: freshness history that names another
                // run is not evidence about this one (A9-7).
                m.fresh_run_id = Some(run_id);
                let s = m.snapshot().await;
                clear_live_session(run_id);

                assert_eq!(s.phase, SessionPhase::Stalled);
                assert!(!s.runtime_fresh);
            }

            // Idle runtime waiting for the headset: the file was written once
            // (`SetIdle`) and is now arbitrarily stale, past every grace —
            // oxrsys has no idle heartbeat, so this is Running, never Stalled
            // (live-verified 2026-08-29: the pre-fix rule flipped a fresh
            // launch to Stalled 13 s in, before the Quest had connected).
            {
                let dir = scratch("live-idle-waiting");
                let paths = fixture_paths(&dir);
                std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
                std::fs::write(
                    paths.oxr_appsup.join("runtime_status.json"),
                    format!(
                        r#"{{"state":"idle","updated_at_unix_ms":{}}}"#,
                        now - 120_000
                    ),
                )
                .unwrap();
                let run_id = Uuid::new_v4();
                set_live_session(LiveSessionHandle {
                    run_id,
                    bottle: "Steam".into(),
                    identity: ProcInfo::observe(std::process::id()).unwrap(),
                    log_path: PathBuf::from("/repo/logs/x.log"),
                    started_at_unix_ms: now - 120_000,
                    cancel: CancellationToken::new(),
                    detach: CancellationToken::new(),
                });

                let mut m = SessionMonitor::new(paths);
                m.ever_fresh = true;
                m.last_fresh_unix_ms = Some(now - 100_000);
                let s = m.snapshot().await;
                clear_live_session(run_id);

                assert_eq!(s.phase, SessionPhase::Running);
                assert!(!s.runtime_fresh);
                assert_eq!(s.runtime_state.as_deref(), Some("idle"));
            }

            // Same staleness, but still inside the startup grace window: must
            // not flag Stalled.
            {
                let dir = scratch("live-grace");
                let paths = fixture_paths(&dir);
                std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
                let run_id = Uuid::new_v4();
                set_live_session(LiveSessionHandle {
                    run_id,
                    bottle: "Steam".into(),
                    identity: ProcInfo::observe(std::process::id()).unwrap(),
                    log_path: PathBuf::from("/repo/logs/x.log"),
                    started_at_unix_ms: now - 5_000,
                    cancel: CancellationToken::new(),
                    detach: CancellationToken::new(),
                });

                let mut m = SessionMonitor::new(paths);
                m.ever_fresh = true;
                m.last_fresh_unix_ms = Some(now - 20_000);
                let s = m.snapshot().await;
                clear_live_session(run_id);

                assert_eq!(
                    s.phase,
                    SessionPhase::Running,
                    "still inside the startup grace window"
                );
            }
        }
    }
}
