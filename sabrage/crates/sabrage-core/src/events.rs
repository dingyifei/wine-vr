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

// ── stage ─────────────────────────────────────────────────────────────────────

/// The five mutating pipeline stages.
///
/// `all` is deliberately absent: demo.sh implements it by re-executing itself
/// once per stage (`for stage in setup build install run`), and the native side
/// mirrors that as a *caller-level* loop over fresh contexts, not as a sixth
/// stage. Doctor is absent too — it is read-only and lives in [`crate::checks`].
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
            Stage::Run => &[],
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

// ── row vocabulary ────────────────────────────────────────────────────────────

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

// ── the event ─────────────────────────────────────────────────────────────────

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
    /// segment (curl's bar, cargo's status line). Never newline-terminated.
    Output {
        run_id: RunId,
        step: String,
        stream: Stream,
        chunk: String,
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
            | StageEvent::Fatal { run_id, .. }
            | StageEvent::StageFinished { run_id, .. } => *run_id,
        }
    }

    /// The step this event is attributed to, when it has one.
    pub fn step(&self) -> Option<&str> {
        match self {
            StageEvent::Line { step, .. } => step.as_deref(),
            StageEvent::Output { step, .. }
            | StageEvent::Progress { step, .. }
            | StageEvent::AutoFixed { step, .. }
            | StageEvent::NeedsAdmin { step, .. } => Some(step),
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

// ── step ids ──────────────────────────────────────────────────────────────────

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
    /// stop's steps, in order.
    pub const STOP: &[StepId] = &[STOP_WINESERVER, STOP_PORTS, STOP_REAP, STOP_AUDIO];

    /// Every step id declared here.
    pub fn all() -> Vec<StepId> {
        let mut v = Vec::new();
        v.extend_from_slice(SETUP);
        v.extend_from_slice(BUILD);
        v.extend_from_slice(INSTALL);
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
        assert_eq!(all.len(), 18);
        for stage in [Stage::Setup, Stage::Build, Stage::Install, Stage::Stop] {
            for s in stage.steps() {
                assert!(
                    s.starts_with(&format!("{}.", stage.as_str())),
                    "{s} is not prefixed by its stage"
                );
            }
        }
        assert!(Stage::Run.steps().is_empty(), "run lands in Phase 3");
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
            // Round-trips through JSON unchanged.
            let text = serde_json::to_string(ev).unwrap();
            assert_eq!(&serde_json::from_str::<StageEvent>(&text).unwrap(), ev);
        }
        assert_eq!(evs[3].step(), Some(step::BUILD_OXRSYS));
        assert_eq!(evs[2].step(), None);
        assert_eq!(evs[7].step(), None);
    }
}
