//! The stage event stream: everything a running stage tells the world.
//!
//! One serde enum, two consumers — `sabrage-cli` renders it as demo.sh-shaped
//! console text, and the Tauri layer forwards it to the GUI. Nothing else may
//! read a stage's progress (design-core §6.2: structured results are the primary
//! artifact; the human renderer is a view over the same data).
//!
//! # What maps to what
//!
//! | zsh (`lib.sh` / stage scripts) | event |
//! |---|---|
//! | `print -r -- "-- global DXMT overlay ($CX/lib/dxmt)"` | [`StageEvent::Section`] |
//! | `info "…"` / `ok "…"` / `warn "…"` / `fail "…" "…"` | [`StageEvent::Line`] with the matching [`Severity`] |
//! | a child's stdout/stderr (`cmake --build`, `curl`, `git`) | [`StageEvent::Output`] |
//! | curl's progress bar / ninja's `[n/m]` | [`StageEvent::Progress`] |
//! | `die "…"` | [`StageEvent::Fatal`] |
//!
//! `Line.text` is the shell's message **verbatim** — no prefix, no colour, no
//! `OK  `/`WARN` marker. The renderer adds those, exactly as `lib.sh`'s `ok()`
//! and `warn()` do, so a single source string can be printed by the CLI and
//! shown as a styled row by the GUI.
//!
//! # Stability
//!
//! Slugs, step ids, and the serde tags here are a wire format: the GUI mirrors
//! them by hand in `ui/src/ipc.ts`, and the JSONL event log is grepped against
//! the shell scripts. Renaming one is a breaking change on both sides.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::SabrageError;
use crate::fixes::FixAction;

/// Identifies one stage invocation. Every event from that invocation carries it,
/// and it names the per-run event-log directory.
pub type RunId = Uuid;

/// A stable step id, e.g. `"install.4.host-manifest"`. See [`step`].
pub type StepId = &'static str;

/// The five mutating pipeline stages.
///
/// `all` is deliberately absent: it is a caller-level loop over fresh contexts,
/// one per stage of [`Stage::ALL_CHAIN`], not a sixth stage. Doctor is absent
/// too — it is read-only and lives in [`crate::checks`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    Setup,
    Build,
    Install,
    Run,
    Stop,
}

impl Stage {
    /// The demo.sh subcommand word (`"setup"`, `"build"`, …).
    pub fn as_str(self) -> &'static str {
        match self {
            Stage::Setup => "setup",
            Stage::Build => "build",
            Stage::Install => "install",
            Stage::Run => "run",
            Stage::Stop => "stop",
        }
    }

    /// The four stages `./demo.sh all` chains, in order.
    pub const ALL_CHAIN: [Stage; 4] = [Stage::Setup, Stage::Build, Stage::Install, Stage::Run];

    /// Every stage, in demo.sh's dispatcher order.
    pub const EVERY: [Stage; 5] = [
        Stage::Setup,
        Stage::Build,
        Stage::Install,
        Stage::Run,
        Stage::Stop,
    ];

    /// Does this stage require a bottle? (`setup` and `build` do not — see
    /// `demo.sh`'s per-stage `require_bottle` calls.)
    pub fn requires_bottle(self) -> bool {
        matches!(self, Stage::Install | Stage::Run | Stage::Stop)
    }

    /// The step ids this stage emits, in execution order.
    pub fn steps(self) -> &'static [StepId] {
        match self {
            Stage::Setup => step::SETUP,
            Stage::Build => step::BUILD,
            Stage::Install => step::INSTALL,
            Stage::Run => step::RUN,
            Stage::Stop => step::STOP,
        }
    }
}

impl fmt::Display for Stage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Stage {
    type Err = SabrageError;

    fn from_str(s: &str) -> Result<Stage, SabrageError> {
        match s {
            "setup" => Ok(Stage::Setup),
            "build" => Ok(Stage::Build),
            "install" => Ok(Stage::Install),
            "run" => Ok(Stage::Run),
            "stop" => Ok(Stage::Stop),
            other => Err(SabrageError::InvalidInput(format!(
                "unknown stage '{other}'"
            ))),
        }
    }
}

/// The four `lib.sh` row kinds. Serialized lowercase, matching the tap channel's
/// words (`crate::tap::tap_word`) for `ok`/`warn`/`fail`/`info`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    /// `info "…"` — two leading spaces, no marker.
    Info,
    /// `ok "…"` — green `OK` marker.
    Ok,
    /// `warn "…"` — yellow `WARN` marker.
    Warn,
    /// `fail "…" "…"` — red `FAIL` marker plus an indented `remedy:` line.
    /// Note that a stage `fail` does **not** abort; `die()` maps to
    /// [`StageEvent::Fatal`].
    Fail,
}

impl Severity {
    /// The word the tap/JSON channels use.
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Ok => "ok",
            Severity::Warn => "warn",
            Severity::Fail => "fail",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Which pipe a child-output chunk came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Stream {
    Stdout,
    Stderr,
}

/// One thing that happened inside a stage.
///
/// Internally tagged (`{"kind": "line", …}`) so the GUI can switch on `kind`
/// without a wrapper object, with camelCase field names to match the rest of the
/// IPC surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum StageEvent {
    /// A stage began. Always the first event of a run.
    StageStarted { run_id: RunId, stage: Stage },

    /// A section banner: install.sh's `print -r -- "-- global DXMT overlay (…)"`,
    /// stop.sh's `-- stopping wineserver …`. `title` excludes the `-- ` prefix
    /// the shell prints; the renderer adds it back.
    Section { run_id: RunId, title: String },

    /// One `info`/`ok`/`warn`/`fail` row. `text` is the shell's message
    /// **verbatim**; `remedy` is the string printed after `remedy:` (only ever
    /// set for [`Severity::Fail`]).
    Line {
        run_id: RunId,
        /// `None` for rows a stage prints outside any step.
        step: Option<String>,
        severity: Severity,
        text: String,
        remedy: Option<String>,
    },

    /// A raw chunk of child output — one line, or one `\r`-delimited progress
    /// segment (curl's bar, cargo's status line). `chunk` never carries its
    /// own terminator; `end` (A14-3, [`crate::process::ChunkEnd`]) says what
    /// it was: `\n` (or `\r\n`, reported once), a bare `\r` repaint, or end of
    /// stream with no delimiter at all. `#[serde(default)]` so a JSONL log or
    /// an IPC message from before this field existed still deserializes, and
    /// then reads as the newline every emitter meant at the time.
    Output {
        run_id: RunId,
        step: String,
        stream: Stream,
        chunk: String,
        #[serde(default)]
        end: crate::process::ChunkEnd,
    },

    /// Quantified progress within a step: bytes for a download, `[n/m]` for
    /// ninja. `total` is `None` when the endpoint is unknown.
    Progress {
        run_id: RunId,
        step: String,
        label: String,
        current: u64,
        total: Option<u64>,
    },

    /// A preflight auto-fix ran and changed something (`description` is
    /// human-facing: "bottle graphics backend forced to dxmt").
    AutoFixed {
        run_id: RunId,
        step: String,
        fix: FixAction,
        description: String,
    },

    /// Announced *before* the one privileged write in the pipeline, so the UI can
    /// explain the authorization prompt instead of letting it appear unheralded.
    NeedsAdmin {
        run_id: RunId,
        step: String,
        reason: String,
    },

    /// `die()` equivalent: the stage is over. `message` is the shell's die text
    /// verbatim; `fix` is set when the remedy maps to an actionable
    /// [`FixAction`] the GUI can offer as a button.
    Fatal {
        run_id: RunId,
        message: String,
        remedy: Option<String>,
        fix: Option<FixAction>,
    },

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
    Text {
        run_id: RunId,
        /// `None` for lines a stage prints outside any step.
        step: Option<String>,
        /// The shell's line without its trailing newline. May be empty.
        text: String,
    },

    /// One launch-preflight check resolved.
    ///
    /// The **final** outcome: a check gated `autofix` that failed, was fixed,
    /// and passed the re-check emits exactly one `Check` (the passing one),
    /// preceded by the [`StageEvent::AutoFixed`] describing the fix. `gate` is
    /// the contract's `native_gate` for the slug, so a consumer can tell "this
    /// warn is advisory" from "this warn would have blocked on the zsh side"
    /// without re-reading the contract.
    Check {
        run_id: RunId,
        step: String,
        outcome: crate::checks::CheckOutcome,
        gate: crate::contract::Gate,
    },

    /// The wine child is up: the run has left preflight/prepare and is now a
    /// session.
    ///
    /// `pid` + `start_time` together are the process **identity**
    /// ([`crate::process::ProcInfo`]) — the pair that survives into
    /// `session-state.json` and lets a later reconcile tell this process from a
    /// recycled pid. `started_at_unix_ms` is wall-clock, for the session
    /// timer; `start_time` is the OS's own boot-relative-ish stamp and is
    /// **not** a clock. `log_path` is the file the child writes to directly.
    Launched {
        run_id: RunId,
        pid: u32,
        start_time: u64,
        log_path: String,
        started_at_unix_ms: u64,
    },

    /// The stage ended. `exit_code_equiv` is what `./demo.sh <stage>` would have
    /// exited with.
    StageFinished {
        run_id: RunId,
        stage: Stage,
        ok: bool,
        exit_code_equiv: i32,
    },
}

impl StageEvent {
    /// The run this event belongs to.
    pub fn run_id(&self) -> RunId {
        match self {
            StageEvent::StageStarted { run_id, .. }
            | StageEvent::Section { run_id, .. }
            | StageEvent::Line { run_id, .. }
            | StageEvent::Output { run_id, .. }
            | StageEvent::Progress { run_id, .. }
            | StageEvent::AutoFixed { run_id, .. }
            | StageEvent::NeedsAdmin { run_id, .. }
            | StageEvent::Text { run_id, .. }
            | StageEvent::Check { run_id, .. }
            | StageEvent::Launched { run_id, .. }
            | StageEvent::Fatal { run_id, .. }
            | StageEvent::StageFinished { run_id, .. } => *run_id,
        }
    }

    /// The step this event is attributed to, when it has one.
    pub fn step(&self) -> Option<&str> {
        match self {
            StageEvent::Line { step, .. } | StageEvent::Text { step, .. } => step.as_deref(),
            StageEvent::Output { step, .. }
            | StageEvent::Progress { step, .. }
            | StageEvent::AutoFixed { step, .. }
            | StageEvent::NeedsAdmin { step, .. }
            | StageEvent::Check { step, .. } => Some(step),
            _ => None,
        }
    }

    /// `info` row constructor.
    pub fn info(run_id: RunId, step: Option<StepId>, text: impl Into<String>) -> StageEvent {
        StageEvent::row(run_id, step, Severity::Info, text, None)
    }

    /// `ok` row constructor.
    pub fn ok(run_id: RunId, step: Option<StepId>, text: impl Into<String>) -> StageEvent {
        StageEvent::row(run_id, step, Severity::Ok, text, None)
    }

    /// `warn` row constructor.
    pub fn warn(run_id: RunId, step: Option<StepId>, text: impl Into<String>) -> StageEvent {
        StageEvent::row(run_id, step, Severity::Warn, text, None)
    }

    /// `fail` row constructor (non-aborting — `die` is [`StageEvent::Fatal`]).
    pub fn fail(
        run_id: RunId,
        step: Option<StepId>,
        text: impl Into<String>,
        remedy: Option<String>,
    ) -> StageEvent {
        StageEvent::row(run_id, step, Severity::Fail, text, remedy)
    }

    /// A verbatim `print -r --` line. See [`StageEvent::Text`].
    pub fn text(run_id: RunId, step: Option<StepId>, text: impl Into<String>) -> StageEvent {
        StageEvent::Text {
            run_id,
            step: step.map(|s| s.to_string()),
            text: text.into(),
        }
    }

    fn row(
        run_id: RunId,
        step: Option<StepId>,
        severity: Severity,
        text: impl Into<String>,
        remedy: Option<String>,
    ) -> StageEvent {
        StageEvent::Line {
            run_id,
            step: step.map(|s| s.to_string()),
            severity,
            text: text.into(),
            remedy,
        }
    }
}

/// Stable step ids, one per numbered block of the shell stage scripts.
///
/// The numbers mirror the comments in `scripts/demo/*.sh` (`# 1. submodules`,
/// `# 4. host loader registration`) so a JSONL event log greps straight back to
/// the reference implementation. `build`'s numbering is implicit in the shell
/// (its blocks are unnumbered); the order here is build.sh's order.
pub mod step {
    use super::StepId;

    // setup.sh
    /// `git submodule update --init` for oxrsys/wineopenxr/ALVR (+ their nested
    /// submodules) and the ALVR patch-set grep.
    pub const SETUP_SUBMODULES: StepId = "setup.1.submodules";
    /// Goldberg dll + DXMT artifact tarball: sha256-pinned fetch, extract, marker.
    pub const SETUP_PINNED: StepId = "setup.2.pinned";
    /// `oxrsys-runtime.toml` write-once creation from the shared template.
    pub const SETUP_CONFIG: StepId = "setup.3.config";
    /// Beat Saber presence probe (never automated — needs a Steam account).
    pub const SETUP_GAME: StepId = "setup.4.game";

    // build.sh
    /// `cmake`/`ninja`/`x86_64-w64-mingw32-gcc` + the rustup x86_64 target.
    pub const BUILD_TOOLS: StepId = "build.1.tools";
    /// oxrsys `build-x64` (Debug, x86_64, ALVR on).
    pub const BUILD_OXRSYS: StepId = "build.2.oxrsys";
    /// The native-arm64 encoder helper: `build-helper-arm64` + arch gate + stage.
    pub const BUILD_HELPER: StepId = "build.3.helper";
    /// wineopenxr (PE dll via mingw + unix `.so`).
    pub const BUILD_WINEOPENXR: StepId = "build.4.wineopenxr";
    /// `cargo build -p alvr_dashboard --release` (native arch).
    pub const BUILD_DASHBOARD: StepId = "build.5.dashboard";
    /// The seven-artifact presence sweep build.sh ends with.
    pub const BUILD_OUTPUTS: StepId = "build.6.outputs";

    // install.sh
    /// Layer 1: global DXMT overlay in `$CX/lib/dxmt` (+ one-time stock backup).
    pub const INSTALL_DXMT_OVERLAY: StepId = "install.1.dxmt-overlay";
    /// Layer 2: global wineopenxr in `$CX/lib/wine`.
    pub const INSTALL_WINEOPENXR: StepId = "install.2.wineopenxr";
    /// Layer 3: per-bottle dll + `drive_c/openxr/` manifest + ActiveRuntime key.
    pub const INSTALL_BOTTLE: StepId = "install.3.bottle";
    /// Layer 4: the host OpenXR registration — the pipeline's ONLY privileged write.
    pub const INSTALL_HOST_MANIFEST: StepId = "install.4.host-manifest";

    // run.sh
    /// The ordered preflight block: every `# preflight:` / `# preflight-warn:` /
    /// `# preflight-autofix:` tagged check of `scripts/demo/run.sh`. The native
    /// side evaluates the same set in contract order — see
    /// [`crate::stages::run::preflight`].
    pub const RUN_PREFLIGHT: StepId = "run.1.preflight";
    /// `launch-action: adb-forward-hygiene` — `--wired` creates
    /// `tcp:9943`/`tcp:9944` per-serial; a normal run removes exactly those two.
    pub const RUN_ADB_FORWARDS: StepId = "run.2.adb-forwards";
    /// `launch-action: wineserver-reset` — `wineserver -k` then a bounded `-w`
    /// wait ([`crate::stages::RUN_WINESERVER_WAIT`], **fatal** on timeout).
    pub const RUN_WINESERVER: StepId = "run.3.wineserver";
    /// `launch-action: goldberg-stage` — the `steam_api64.dll` swap (one
    /// `.orig-steam` backup), `steam_appid.txt`, and the `steam_settings/` flags.
    pub const RUN_GOLDBERG: StepId = "run.4.goldberg";
    /// `launch-action: audio-route` — the guarded BlackHole 2ch switch.
    pub const RUN_AUDIO: StepId = "run.5.audio";
    /// `launch-action: dashboard` — the guarded `alvr_dashboard` spawn.
    pub const RUN_DASHBOARD: StepId = "run.6.dashboard";
    /// `launch-action: adb-reverse-cleanup` — `adb reverse --remove-all` on the
    /// alvr path; the legacy oxrsys path additionally re-creates its tunnels
    /// and `am start`s the Android client.
    pub const RUN_ADB_REVERSE: StepId = "run.7.adb-reverse";
    /// `launch-action: launch-wine` — the banner block and the detached spawn,
    /// ending in [`super::StageEvent::Launched`].
    pub const RUN_LAUNCH: StepId = "run.8.launch";
    /// Waiting on the session: the wine child, the log tail, and the telemetry
    /// watcher. Holds no operation lock (see [`crate::stages`]).
    pub const RUN_SUPERVISE: StepId = "run.9.supervise";
    /// Guard release (audio, dashboard, helper reap) plus, on the INT/TERM
    /// path only, `stop_wine`. Ends with the verbatim
    /// `wine exited with status <rc> (log: <path>)` line.
    pub const RUN_TEARDOWN: StepId = "run.10.teardown";

    // stop.sh
    /// `wineserver -k` + bounded `-w` wait (4 s, non-fatal) and the survivor probe.
    pub const STOP_WINESERVER: StepId = "stop.1.wineserver";
    /// `lsof` on the streaming ports.
    pub const STOP_PORTS: StepId = "stop.2.ports";
    /// Leftover encoder-helper / ALVR-dashboard reap.
    pub const STOP_REAP: StepId = "stop.3.reap";
    /// The BlackHole-still-selected audio warning.
    pub const STOP_AUDIO: StepId = "stop.4.audio";

    /// setup's steps, in order.
    pub const SETUP: &[StepId] = &[SETUP_SUBMODULES, SETUP_PINNED, SETUP_CONFIG, SETUP_GAME];
    /// build's steps, in order.
    pub const BUILD: &[StepId] = &[
        BUILD_TOOLS,
        BUILD_OXRSYS,
        BUILD_HELPER,
        BUILD_WINEOPENXR,
        BUILD_DASHBOARD,
        BUILD_OUTPUTS,
    ];
    /// install's steps, in order (= the four layers).
    pub const INSTALL: &[StepId] = &[
        INSTALL_DXMT_OVERLAY,
        INSTALL_WINEOPENXR,
        INSTALL_BOTTLE,
        INSTALL_HOST_MANIFEST,
    ];
    /// run's steps, in order — the launch state machine of design-core §3.2
    /// (Preflight → Prepare → Guards → Launch → Supervise → Teardown), with
    /// each `# launch-action:` tag of run.sh given its own id.
    pub const RUN: &[StepId] = &[
        RUN_PREFLIGHT,
        RUN_ADB_FORWARDS,
        RUN_WINESERVER,
        RUN_GOLDBERG,
        RUN_AUDIO,
        RUN_DASHBOARD,
        RUN_ADB_REVERSE,
        RUN_LAUNCH,
        RUN_SUPERVISE,
        RUN_TEARDOWN,
    ];
    /// stop's steps, in order.
    pub const STOP: &[StepId] = &[STOP_WINESERVER, STOP_PORTS, STOP_REAP, STOP_AUDIO];

    /// Every step id declared here.
    pub fn all() -> Vec<StepId> {
        let mut v = Vec::new();
        v.extend_from_slice(SETUP);
        v.extend_from_slice(BUILD);
        v.extend_from_slice(INSTALL);
        v.extend_from_slice(RUN);
        v.extend_from_slice(STOP);
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rid() -> RunId {
        Uuid::nil()
    }

    #[test]
    fn stage_round_trips_through_its_demo_sh_word() {
        for s in Stage::EVERY {
            assert_eq!(s.to_string().parse::<Stage>().unwrap(), s);
            assert_eq!(
                serde_json::to_string(&s).unwrap(),
                format!("\"{}\"", s.as_str())
            );
        }
        assert_eq!("setup".parse::<Stage>().unwrap(), Stage::Setup);
        let err = "doctor".parse::<Stage>().unwrap_err();
        assert_eq!(err.exit_code(), 2);
        assert_eq!(err.to_string(), "invalid input: unknown stage 'doctor'");
    }

    #[test]
    fn only_bottle_stages_require_a_bottle() {
        assert!(!Stage::Setup.requires_bottle());
        assert!(!Stage::Build.requires_bottle());
        assert!(Stage::Install.requires_bottle());
        assert!(Stage::Run.requires_bottle());
        assert!(Stage::Stop.requires_bottle());
    }

    #[test]
    fn step_ids_are_unique_and_prefixed_by_their_stage() {
        let all = step::all();
        let unique: std::collections::BTreeSet<_> = all.iter().collect();
        assert_eq!(unique.len(), all.len(), "duplicate step id");
        assert_eq!(all.len(), 28);
        for stage in Stage::EVERY {
            for s in stage.steps() {
                assert!(
                    s.starts_with(&format!("{}.", stage.as_str())),
                    "{s} is not prefixed by its stage"
                );
            }
        }
        assert_eq!(Stage::Run.steps(), step::RUN);
        assert_eq!(Stage::Run.steps().len(), 10);
        assert_eq!(Stage::Run.steps().first(), Some(&step::RUN_PREFLIGHT));
        assert_eq!(Stage::Run.steps().last(), Some(&step::RUN_TEARDOWN));
    }

    #[test]
    fn events_serialize_internally_tagged_with_camel_case_fields() {
        let ev = StageEvent::StageFinished {
            run_id: rid(),
            stage: Stage::Install,
            ok: true,
            exit_code_equiv: 0,
        };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["kind"], "stageFinished");
        assert_eq!(json["stage"], "install");
        assert_eq!(json["exitCodeEquiv"], 0);

        let ev = StageEvent::ok(
            rid(),
            Some(step::INSTALL_BOTTLE),
            "ActiveRuntime registered",
        );
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["kind"], "line");
        assert_eq!(json["severity"], "ok");
        assert_eq!(json["step"], "install.3.bottle");
        // Verbatim: no marker, no colour, no leading spaces.
        assert_eq!(json["text"], "ActiveRuntime registered");
    }

    #[test]
    fn the_run_events_keep_their_wire_shape() {
        // Text is verbatim: leading spaces survive, and an empty line is legal
        // (run.sh's bare `print ""`).
        let ev = StageEvent::text(rid(), None, "   exe: C:\\Beat Saber.exe");
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["kind"], "text");
        assert_eq!(json["text"], "   exe: C:\\Beat Saber.exe");
        assert_eq!(json["step"], serde_json::Value::Null);
        let empty = StageEvent::text(rid(), None, "");
        assert_eq!(serde_json::to_value(&empty).unwrap()["text"], "");

        let ev = StageEvent::Check {
            run_id: rid(),
            step: step::RUN_PREFLIGHT.into(),
            outcome: crate::checks::CheckOutcome::fail(
                "run.bridge-built",
                "bridge not built",
                "./demo.sh build",
            ),
            gate: crate::contract::Gate::Block,
        };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["kind"], "check");
        assert_eq!(json["gate"], "block");
        assert_eq!(json["outcome"]["slug"], "run.bridge-built");
        assert_eq!(json["outcome"]["status"], "fail");

        let ev = StageEvent::Launched {
            run_id: rid(),
            pid: 59004,
            start_time: 1786300214,
            log_path: "/repo/logs/beatsaber-20260829-101112.log".into(),
            started_at_unix_ms: 1786300214181,
        };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["kind"], "launched");
        assert_eq!(json["pid"], 59004);
        assert_eq!(json["startTime"], 1786300214u64);
        assert_eq!(json["startedAtUnixMs"], 1786300214181u64);
        assert_eq!(json["logPath"], "/repo/logs/beatsaber-20260829-101112.log");
    }

    #[test]
    fn every_event_carries_its_run_id() {
        let evs = vec![
            StageEvent::StageStarted {
                run_id: rid(),
                stage: Stage::Setup,
            },
            StageEvent::Section {
                run_id: rid(),
                title: "global DXMT overlay".into(),
            },
            StageEvent::info(rid(), None, "x"),
            StageEvent::Output {
                run_id: rid(),
                step: step::BUILD_OXRSYS.into(),
                stream: Stream::Stderr,
                chunk: "[1/2] cc".into(),
                end: crate::process::ChunkEnd::Lf,
            },
            StageEvent::Progress {
                run_id: rid(),
                step: step::SETUP_PINNED.into(),
                label: "DXMT fork artifacts".into(),
                current: 1,
                total: Some(2),
            },
            StageEvent::AutoFixed {
                run_id: rid(),
                step: step::BUILD_HELPER.into(),
                fix: FixAction::RestageHelper,
                description: "restaged".into(),
            },
            StageEvent::NeedsAdmin {
                run_id: rid(),
                step: step::INSTALL_HOST_MANIFEST.into(),
                reason: "writes the host OpenXR registration".into(),
            },
            StageEvent::text(rid(), Some(step::RUN_LAUNCH), "   log: /repo/logs/x.log"),
            StageEvent::Check {
                run_id: rid(),
                step: step::RUN_PREFLIGHT.into(),
                outcome: crate::checks::CheckOutcome::pass("run.wine-exec", "wine present"),
                gate: crate::contract::Gate::Block,
            },
            StageEvent::Launched {
                run_id: rid(),
                pid: 4242,
                start_time: 1786300214,
                log_path: "/repo/logs/beatsaber-20260829-101112.log".into(),
                started_at_unix_ms: 1786300214181,
            },
            StageEvent::Fatal {
                run_id: rid(),
                message: "boom".into(),
                remedy: None,
                fix: None,
            },
            StageEvent::StageFinished {
                run_id: rid(),
                stage: Stage::Stop,
                ok: false,
                exit_code_equiv: 1,
            },
        ];
        for ev in &evs {
            assert_eq!(ev.run_id(), rid());
            let text = serde_json::to_string(ev).unwrap();
            assert_eq!(&serde_json::from_str::<StageEvent>(&text).unwrap(), ev);
        }
        assert_eq!(evs[3].step(), Some(step::BUILD_OXRSYS));
        assert_eq!(evs[2].step(), None);
        // Text carries its step; Launched never does.
        assert_eq!(evs[7].step(), Some(step::RUN_LAUNCH));
        assert_eq!(evs[8].step(), Some(step::RUN_PREFLIGHT));
        assert_eq!(evs[9].step(), None);
        assert_eq!(evs[10].step(), None);
    }
}
