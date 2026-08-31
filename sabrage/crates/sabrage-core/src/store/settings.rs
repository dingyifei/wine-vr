//! `settings.json` — Sabrage's own global preferences.
//!
//! `~/Library/Application Support/Sabrage/settings.json`
//! ([`crate::paths::sabrage_support_dir`], [`settings_path`]). GUI-only state:
//! demo.sh has no counterpart (design-core §4.2). Written through the
//! [`Executor`] like every other mutation (`--dry-run` plans instead of
//! writing); read with plain `std::fs`, matching every other store module's
//! convention (`session::state::load`).
//!
//! # Forward compatibility
//!
//! Every field carries `#[serde(default)]` (via the struct-level attribute on
//! [`Settings`] and [`LaunchDefaults`]), so an older file — or a hand-trimmed
//! one — still loads: a missing field falls back to its default. A file that
//! fails to *parse* at all is a hard [`crate::error::SabrageError`], never a
//! silent reset — a corrupt `settings.json` should be visible, not quietly
//! replaced.
//!
//! Fields this binary does **not** know are not ignored either: they are
//! captured into [`Settings::extra`] and written back out verbatim by the next
//! [`save`]. Without that, running an older Sabrage once — and touching one
//! toggle, which autosaves the whole object — would silently delete everything
//! a newer build had written. [`SETTINGS_VERSION`] rides along for the case a
//! future change cannot be expressed as "unknown keys, preserved".

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::{Result, SabrageError};
use crate::executor::Executor;
use crate::stages::StageOptions;

/// The four demo.sh launch flags Settings lets the user default.
///
/// Mirrors [`StageOptions`]'s flag quartet exactly — see
/// [`Settings::effective_stage_options`] and `store::library::effective_options`
/// for where the two meet.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LaunchDefaults {
    pub no_audio: bool,
    pub no_dashboard: bool,
    pub wired: bool,
    pub verbose: bool,
}

/// Schema version written by this Sabrage into `settings.json`. Bump it only
/// for a change [`Settings::extra`]'s verbatim round-trip cannot absorb.
pub const SETTINGS_VERSION: u32 = 1;

/// Sabrage's global preferences.
///
/// `repo_root`/`default_bottle`/`default_bs_dir` are `None` until the user has
/// set one — `None` is not "use some hardcoded fallback", it is "nothing
/// configured yet", and every reader (`resolve_repo_root`,
/// `library::new_entry_template`) already has its own fallback chain for that
/// case.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// [`SETTINGS_VERSION`] at write time. A file without one (every file
    /// written before this field existed) reads as the current version — its
    /// shape *is* the current shape.
    pub version: u32,
    /// Persisted override for [`crate::paths::resolve_repo_root`]'s first
    /// precedence tier.
    pub repo_root: Option<String>,
    /// Prefills the bottle picker on Session/Library/Add-game.
    pub default_bottle: Option<String>,
    /// Prefills the Beat Saber directory field.
    pub default_bs_dir: Option<String>,
    /// Global defaults for the four demo.sh flags; a [`crate::store::library::GameEntry`]
    /// may override any of them individually.
    pub launch: LaunchDefaults,
    /// Whether doctor/preflight probes may shell out to `adb` (which starts its
    /// daemon as a side effect). Defaults **true** (see [`Settings`]'s
    /// `Default` impl) — matches [`crate::checks::CheckOptions::new`]'s
    /// doctor-parity default, via the struct-level `#[serde(default)]`
    /// pulling missing fields from that impl — so an absent or
    /// freshly-created settings file behaves exactly like doctor always has.
    pub allow_adb_probes: bool,
    /// One-time acknowledgement of the runtime-config write-once override
    /// (design-core §4.1 rule 2's UX decision): the Settings screen shows a
    /// confirmation panel the first time it writes `oxrsys-runtime.toml`, and
    /// this flag suppresses it afterward. Defaults **false** — an existing
    /// deployed settings file (this field is new) must still show the panel
    /// once.
    pub runtime_config_edit_acknowledged: bool,
    /// Every top-level key this binary does not have a field for, kept exactly
    /// as read and written straight back out.
    ///
    /// This is the whole downgrade-safety story: the UI autosaves a complete
    /// `Settings` object on every control change, so anything not represented
    /// here would be deleted the first time an older build touched one toggle.
    /// Flattened, so the keys sit at the top level of the JSON where they were
    /// found; skipped when empty, so an ordinary file's bytes are unchanged.
    ///
    /// (This is also why [`Settings`] is no longer `Eq`: [`Value`] is only
    /// `PartialEq` — `f64` has no total equality.)
    #[serde(flatten, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            version: SETTINGS_VERSION,
            repo_root: None,
            default_bottle: None,
            default_bs_dir: None,
            launch: LaunchDefaults::default(),
            allow_adb_probes: true,
            runtime_config_edit_acknowledged: false,
            extra: Map::new(),
        }
    }
}

impl Settings {
    /// [`StageOptions`] built from this settings' bottle/dir/launch defaults.
    /// Always `dry_run: false` — dry-run is a caller decision, never a stored
    /// preference.
    pub fn effective_stage_options(&self) -> StageOptions {
        StageOptions {
            bottle_name: self.default_bottle.clone(),
            bs_dir_override: self.default_bs_dir.clone().map(PathBuf::from),
            dry_run: false,
            verbose: self.launch.verbose,
            no_audio: self.launch.no_audio,
            no_dashboard: self.launch.no_dashboard,
            wired: self.launch.wired,
        }
    }
}

/// `<sabrage_appsup>/settings.json`.
pub fn settings_path(sabrage_appsup: &Path) -> PathBuf {
    sabrage_appsup.join("settings.json")
}

/// Load `settings.json`.
///
/// * absent → `Ok(Settings::default())` — the ordinary first-run case;
/// * present but unparseable → `Err` — never silently reset a file the user
///   (or a bug) actually wrote something into.
pub fn load(path: &Path) -> Result<Settings> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Settings::default()),
        Err(e) => return Err(SabrageError::io(path, e)),
    };
    serde_json::from_str(&text).map_err(|e| {
        SabrageError::io(
            path,
            std::io::Error::new(std::io::ErrorKind::InvalidData, e),
        )
    })
}

/// Write `settings.json` atomically (pretty JSON plus a trailing newline),
/// matching [`crate::session::state::save`]'s convention.
pub async fn save(executor: &dyn Executor, path: &Path, s: &Settings) -> Result<()> {
    if let Some(parent) = path.parent() {
        executor.create_dir_all(parent).await?;
    }
    let mut bytes = serde_json::to_vec_pretty(s)
        .map_err(|e| SabrageError::io(path, std::io::Error::other(e)))?;
    bytes.push(b'\n');
    executor.write_atomic(path, &bytes).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::{DryRunExecutor, PlannedKind, RealExecutor};
    use crate::stages::null_sink;
    use tokio_util::sync::CancellationToken;
    use uuid::Uuid;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sabrage-store-settings-test-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn real() -> RealExecutor {
        RealExecutor::new(Uuid::nil(), null_sink(), CancellationToken::new())
    }

    #[test]
    fn settings_path_is_the_json_file_under_appsup() {
        assert_eq!(
            settings_path(Path::new("/x/Sabrage")),
            PathBuf::from("/x/Sabrage/settings.json")
        );
    }

    #[test]
    fn missing_file_loads_as_default() {
        let s = load(Path::new("/nonexistent/sabrage/settings.json")).unwrap();
        assert_eq!(s, Settings::default());
        assert!(s.allow_adb_probes, "doctor-parity default is true");
        assert!(!s.runtime_config_edit_acknowledged);
        assert!(s.repo_root.is_none() && s.default_bottle.is_none());
    }

    #[test]
    fn a_corrupt_file_is_an_error_never_a_silent_reset() {
        let dir = scratch("corrupt");
        let path = dir.join("settings.json");
        std::fs::write(&path, b"{not json at all").unwrap();
        let err = load(&path).unwrap_err();
        assert_eq!(err.kind(), "io");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unknown_fields_load_into_extra_instead_of_failing() {
        let dir = scratch("unknown-fields");
        let path = dir.join("settings.json");
        std::fs::write(
            &path,
            r#"{"repoRoot":"/repo","futureField":{"nested":true},"anotherOne":42}"#,
        )
        .unwrap();
        let s = load(&path).unwrap();
        assert_eq!(s.repo_root.as_deref(), Some("/repo"));
        assert_eq!(s.extra.len(), 2, "{:?}", s.extra);
        assert_eq!(s.extra["anotherOne"], serde_json::json!(42));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn unknown_fields_survive_a_load_save_round_trip() {
        // The downgrade case: an older binary loads a newer file, the user
        // flips one toggle, and the whole object is autosaved back.
        let dir = scratch("downgrade");
        let path = dir.join("settings.json");
        std::fs::write(
            &path,
            r#"{"repoRoot":"/repo","futureSetting":{"nested":true},"futureFlag":false}"#,
        )
        .unwrap();

        let mut s = load(&path).unwrap();
        s.allow_adb_probes = false; // the one control the old build knows
        save(&real(), &path, &s).await.unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"futureSetting\""), "{text}");
        assert!(text.contains("\"futureFlag\""), "{text}");
        let reread = load(&path).unwrap();
        assert!(!reread.allow_adb_probes);
        assert_eq!(reread.extra, s.extra);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_file_without_a_version_reads_as_the_current_one() {
        let s: Settings = serde_json::from_str(r#"{"repoRoot":"/repo"}"#).unwrap();
        assert_eq!(s.version, SETTINGS_VERSION);
        assert!(s.extra.is_empty());
    }

    #[tokio::test]
    async fn an_ordinary_settings_file_carries_no_extra_keys() {
        let dir = scratch("no-extra");
        let path = dir.join("settings.json");
        save(&real(), &path, &Settings::default()).await.unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        let keys: Vec<&String> = parsed.as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            vec![
                "version",
                "repoRoot",
                "defaultBottle",
                "defaultBsDir",
                "launch",
                "allowAdbProbes",
                "runtimeConfigEditAcknowledged"
            ],
            "an empty `extra` must add nothing to the file"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn round_trips_camel_case_through_the_file() {
        let dir = scratch("roundtrip");
        let path = dir.join("nested/settings.json");
        let s = Settings {
            repo_root: Some("/repo".into()),
            default_bottle: Some("Steam".into()),
            default_bs_dir: Some("/games/bs".into()),
            launch: LaunchDefaults {
                no_audio: true,
                no_dashboard: false,
                wired: true,
                verbose: false,
            },
            allow_adb_probes: false,
            runtime_config_edit_acknowledged: true,
            ..Settings::default()
        };
        assert_eq!(
            load(&path).unwrap(),
            Settings::default(),
            "absent -> default"
        );
        save(&real(), &path, &s).await.unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.ends_with("}\n"), "pretty JSON plus one newline");
        assert!(text.contains("\"repoRoot\""));
        assert!(text.contains("\"defaultBottle\""));
        assert!(text.contains("\"noAudio\""));
        assert!(text.contains("\"allowAdbProbes\""));
        assert!(text.contains("\"runtimeConfigEditAcknowledged\""));
        assert_eq!(load(&path).unwrap(), s);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_minimal_file_loads_on_defaults() {
        let json = r#"{}"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn effective_stage_options_carries_bottle_dir_and_flags_with_dry_run_false() {
        let s = Settings {
            default_bottle: Some("Steam".into()),
            default_bs_dir: Some("/games/bs".into()),
            launch: LaunchDefaults {
                no_audio: true,
                no_dashboard: true,
                wired: false,
                verbose: true,
            },
            ..Settings::default()
        };
        let opts = s.effective_stage_options();
        assert_eq!(opts.bottle_name.as_deref(), Some("Steam"));
        assert_eq!(opts.bs_dir_override, Some(PathBuf::from("/games/bs")));
        assert!(!opts.dry_run);
        assert!(opts.no_audio && opts.no_dashboard && opts.verbose && !opts.wired);
    }

    #[tokio::test]
    async fn a_dry_run_executor_plans_the_write_instead_of_performing_it() {
        let dir = scratch("dry");
        let path = dir.join("settings.json");
        let ex = DryRunExecutor::new(Uuid::nil(), null_sink(), CancellationToken::new());
        save(&ex, &path, &Settings::default()).await.unwrap();
        assert!(!path.exists(), "dry run wrote the settings file");
        let kinds: Vec<PlannedKind> = ex.planned().iter().map(|p| p.kind).collect();
        assert_eq!(kinds, vec![PlannedKind::CreateDir, PlannedKind::Write]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
