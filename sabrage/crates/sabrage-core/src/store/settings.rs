//! `settings.json` — Sabrage's own global preferences, at
//! `~/Library/Application Support/Sabrage/settings.json`
//! ([`crate::paths::sabrage_support_dir`], [`settings_path`]). GUI-only state
//! with no demo.sh counterpart (design-core §4.2); written through the
//! [`Executor`] like every other mutation, read with plain `std::fs`.
//!
//! Unknown keys — top-level and inside `launch` — are preserved verbatim so an
//! older build's autosave cannot delete what a newer one wrote, and [`load`]
//! refuses a file whose `version` is newer than [`SETTINGS_VERSION`] rather
//! than reading it half-way. See
//! tests::{unknown_fields_survive_a_load_save_round_trip,
//! unknown_nested_launch_keys_survive_a_load_save_round_trip,
//! a_newer_version_is_refused_and_its_bytes_left_alone}.

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
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LaunchDefaults {
    pub no_audio: bool,
    pub no_dashboard: bool,
    pub wired: bool,
    pub verbose: bool,
    /// Keys of this object a newer Sabrage wrote and this one has no field
    /// for, kept exactly as read and written straight back out. The outer
    /// flattened map on [`Settings`] collects top-level keys only, so without
    /// this a `launch.someFlag` is dropped during deserialization and deleted
    /// by the next autosave — the UI hands the whole object back on save
    /// (tests::unknown_nested_launch_keys_survive_a_load_save_round_trip).
    #[serde(flatten, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

/// Schema version this Sabrage writes into `settings.json`. Bump it only for
/// a change the [`Settings::extra`]/[`LaunchDefaults::extra`] verbatim round
/// trip cannot absorb, and always for such a change: [`load`] refusing a newer
/// file is the only guard against an older build's autosave
/// (tests::a_newer_version_is_refused_and_its_bytes_left_alone).
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
    /// [`SETTINGS_VERSION`] at write time. A file without the key reads as the
    /// current version: its shape *is* the current shape.
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
    /// daemon as a side effect). Defaults **true**, matching
    /// [`crate::checks::CheckOptions::new`]'s doctor-parity default, so an
    /// absent or freshly-created settings file behaves exactly like doctor.
    pub allow_adb_probes: bool,
    /// One-time acknowledgement of the runtime-config write-once override
    /// (design-core §4.1 rule 2): the Settings screen shows a confirmation
    /// panel the first time it writes `oxrsys-runtime.toml`, and this flag
    /// suppresses it afterward. Defaults **false**, so a file without the key
    /// still shows the panel once.
    pub runtime_config_edit_acknowledged: bool,
    /// Every top-level key this binary does not have a field for, kept exactly
    /// as read and written straight back out.
    ///
    /// The UI autosaves a complete `Settings` object on every control change,
    /// so a key not represented here would be deleted the first time an older
    /// build touched one toggle. Flattened, so the keys sit at the top level
    /// where they were found; skipped when empty, so an ordinary file's bytes
    /// are unchanged (tests::an_ordinary_settings_file_carries_no_extra_keys).
    ///
    /// [`Settings`] is `PartialEq` but not `Eq`, because [`Value`] is not.
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
/// * absent → `Ok(Settings::default())`, the ordinary first-run case;
/// * present but unparseable → `Err`, never a silent reset
///   (tests::a_corrupt_file_is_an_error_never_a_silent_reset);
/// * `version` newer than [`SETTINGS_VERSION`] → `Err` with the bytes left
///   untouched, the same refusal [`super::library::load`] makes: the two
///   `extra` maps preserve unknown *keys*, so a version bump is reserved for a
///   change they cannot express, and reading such a file would let the next
///   autosave persist the loss
///   (tests::a_newer_version_is_refused_and_its_bytes_left_alone).
pub fn load(path: &Path) -> Result<Settings> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Settings::default()),
        Err(e) => return Err(SabrageError::io(path, e)),
    };
    let s: Settings = serde_json::from_str(&text).map_err(|e| {
        SabrageError::io(
            path,
            std::io::Error::new(std::io::ErrorKind::InvalidData, e),
        )
    })?;
    if s.version > SETTINGS_VERSION {
        return Err(SabrageError::fatal(
            format!(
                "{} is version {} — this Sabrage understands version {} \
                 and would silently drop everything the newer one wrote",
                path.display(),
                s.version,
                SETTINGS_VERSION
            ),
            "update Sabrage (or move settings.json aside to start from defaults)",
        ));
    }
    Ok(s)
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
mod tests;
