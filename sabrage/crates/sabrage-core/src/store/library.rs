//! `library.json` — Sabrage's own game registry.
//!
//! demo.sh has no such concept — "a game" is just whatever `BS_DIR` happens to
//! point at (design-core §4.3). This module is Sabrage-only state:
//! `~/Library/Application Support/Sabrage/library.json`
//! ([`crate::paths::sabrage_support_dir`], [`library_path`]). Read/write
//! follow the same convention as [`super::settings`] and
//! [`crate::session::state`] (missing file → default, corrupt file → `Err`,
//! atomic pretty-JSON write through the [`Executor`]).
//!
//! [`validate`] is the other half of this module: read-only probes over a
//! `(bs_dir, bottle)` pair, reusing the same rules `checks::bottle` and
//! `checks::game` already encode for doctor — but expressed as a full
//! snapshot ([`GameValidity`]) rather than a pass/fail/warn tap row, because
//! the Library and Edit-game screens render every facet of it at once
//! (design-app §4).

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::contract::contract;
use crate::error::{Result, SabrageError};
use crate::executor::Executor;
use crate::paths::{resolve_bs_dir, Bottle, Paths};
use crate::stages::run::actions::steam_api_path;
use crate::stages::StageOptions;
use crate::util::{bs_version, cmp_files, file_sha256_matches};

use super::goldberg::orig_steam_path;
use super::settings::Settings;

/// A per-game partial override of [`super::settings::LaunchDefaults`].
/// `None` on any field means "use the global setting" — see
/// [`effective_options`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LaunchOverrides {
    pub no_audio: Option<bool>,
    pub no_dashboard: Option<bool>,
    pub wired: Option<bool>,
    pub verbose: Option<bool>,
}

/// The most recent launch of one game, written by the Tauri launch command
/// after a `run` stage returns (via [`Library::record_last_session`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastSession {
    pub started_at_unix_ms: u64,
    pub ended_at_unix_ms: u64,
    pub exit_code: Option<i32>,
    pub log_path: Option<String>,
}

/// One library entry: everything Sabrage knows about a Beat Saber install
/// beyond what `checks`/`validate` can probe live.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GameEntry {
    pub id: Uuid,
    pub name: String,
    pub bs_dir: String,
    pub bottle: String,
    pub appid: u32,
    pub added_at_unix_ms: u64,
    pub launch_overrides: LaunchOverrides,
    pub last_session: Option<LastSession>,
}

/// The whole library file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Library {
    pub version: u32,
    pub games: Vec<GameEntry>,
}

/// Schema version written by this Sabrage. Not [`Default::default`]'s `0` —
/// see the manual [`Default`] impl below.
pub const LIBRARY_VERSION: u32 = 1;

impl Default for Library {
    fn default() -> Library {
        Library {
            version: LIBRARY_VERSION,
            games: Vec::new(),
        }
    }
}

impl Library {
    /// Insert `entry`, or replace the existing entry sharing its `id`.
    /// Returns a reference to the entry now stored.
    pub fn upsert(&mut self, entry: GameEntry) -> &GameEntry {
        let id = entry.id;
        if let Some(existing) = self.games.iter_mut().find(|g| g.id == id) {
            *existing = entry;
        } else {
            self.games.push(entry);
        }
        self.games
            .iter()
            .find(|g| g.id == id)
            .expect("just inserted or replaced")
    }

    /// Remove the entry with `id`. `true` iff one was found and removed.
    pub fn remove(&mut self, id: Uuid) -> bool {
        let before = self.games.len();
        self.games.retain(|g| g.id != id);
        self.games.len() != before
    }

    /// The entry with `id`, if the library has one.
    pub fn get(&self, id: Uuid) -> Option<&GameEntry> {
        self.games.iter().find(|g| g.id == id)
    }

    /// [`Library::upsert`] for the **Edit-game form**: every editable field comes
    /// from `incoming`, while `last_session`, `added_at_unix_ms` and `appid` are
    /// kept from the stored entry. An unknown id is a plain insert (Add-game
    /// wizard's first save).
    ///
    /// The form submits a whole [`GameEntry`] cloned minutes earlier, so a plain
    /// `upsert` would delete a session recorded while it was open
    /// (tests::an_edit_racing_a_recorded_session_keeps_both).
    pub fn upsert_editable(&mut self, incoming: GameEntry) -> &GameEntry {
        let mut merged = incoming;
        if let Some(existing) = self.get(merged.id) {
            merged.last_session = existing.last_session.clone();
            merged.added_at_unix_ms = existing.added_at_unix_ms;
            merged.appid = existing.appid;
        }
        self.upsert(merged)
    }

    /// Attach `session` to the entry with `id` as its `last_session`.
    /// `true` iff the entry was found (a game removed between launch and
    /// exit is not an error — the caller just has nothing left to update).
    pub fn record_last_session(&mut self, id: Uuid, session: LastSession) -> bool {
        match self.games.iter_mut().find(|g| g.id == id) {
            Some(g) => {
                g.last_session = Some(session);
                true
            }
            None => false,
        }
    }

    /// `settings ⊕ this game's overrides`, or `None` when the library has no
    /// entry with `game_id`.
    ///
    /// The one entry point for "what flags does *this* game launch with": the
    /// Tauri launch command resolves the merge here rather than letting the
    /// front-end keep a second copy of the precedence rule
    /// (tests::launch_options_for_resolves_the_merge_by_id_and_is_none_for_a_stranger).
    pub fn launch_options_for(&self, game_id: Uuid, settings: &Settings) -> Option<StageOptions> {
        self.get(game_id).map(|e| effective_options(settings, e))
    }
}

/// `<sabrage_appsup>/library.json`.
pub fn library_path(sabrage_appsup: &Path) -> PathBuf {
    sabrage_appsup.join("library.json")
}

/// Load `library.json`.
///
/// An absent file is `Ok(Library::default())` (version 1, no games): first run.
///
/// # Errors
///
/// A present but unparseable file — an `Err`, never a silent reset
/// (tests::a_corrupt_file_is_an_error_never_a_silent_reset) — and a `version`
/// newer than [`LIBRARY_VERSION`], refused **before** a caller can mutate and
/// re-save it, because [`Library`] is a closed serde struct and the re-save
/// would drop the newer build's fields while keeping its `version`. Any schema
/// addition here must bump [`LIBRARY_VERSION`]
/// (tests::a_newer_schema_version_is_refused_not_silently_rewritten).
pub fn load(path: &Path) -> Result<Library> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Library::default()),
        Err(e) => return Err(SabrageError::io(path, e)),
    };
    let lib: Library = serde_json::from_str(&text).map_err(|e| {
        SabrageError::io(
            path,
            std::io::Error::new(std::io::ErrorKind::InvalidData, e),
        )
    })?;
    if lib.version > LIBRARY_VERSION {
        return Err(SabrageError::fatal(
            format!(
                "{} is version {} — this Sabrage understands version {} \
                 and would silently drop everything the newer one wrote",
                path.display(),
                lib.version,
                LIBRARY_VERSION
            ),
            "update Sabrage (or move library.json aside to start a fresh library)",
        ));
    }
    Ok(lib)
}

/// Serializes every [`transact`] against every other one in this process.
///
/// [`save`] is atomic per *write*, but a library edit is a
/// load → mutate → save *transaction*: two interleaving means the second
/// one's `save` writes a snapshot taken before the first one's
/// (tests::interleaved_transactions_do_not_resurrect_a_removed_game).
///
/// A lock of its own rather than [`crate::stages::OPERATION_LOCK`]: the
/// library is written from inside a run (the post-launch
/// `record_last_session`) as well as from the Library screen, so borrowing
/// the operation lock would deadlock the run that already holds it, or block
/// every library edit for the length of a session.
static LIBRARY_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

/// Run one complete `library.json` read-modify-write transaction under
/// [`LIBRARY_LOCK`], returning whatever `f` returns.
///
/// `f` sees the freshly loaded library and mutates it in place; the file is
/// rewritten only if `f` actually changed something, so a no-op removal
/// never mints a `library.json` that did not exist
/// (tests::transact_writes_only_when_the_library_actually_changed).
///
/// **Every** writer must go through this: a bare [`load`]/[`save`] pair
/// around the same file re-opens exactly the window this closes.
pub async fn transact<T>(
    executor: &dyn Executor,
    path: &Path,
    f: impl FnOnce(&mut Library) -> T,
) -> Result<T> {
    let _guard = LIBRARY_LOCK.lock().await;
    let before = load(path)?;
    let mut lib = before.clone();
    let out = f(&mut lib);
    if lib != before {
        save(executor, path, &lib).await?;
    }
    Ok(out)
}

/// Write `library.json` atomically (pretty JSON plus a trailing newline).
///
/// Prefer [`transact`] for anything that first *reads* the library — a bare
/// `save` of a snapshot loaded earlier is the lost-update shape.
pub async fn save(executor: &dyn Executor, path: &Path, lib: &Library) -> Result<()> {
    if let Some(parent) = path.parent() {
        executor.create_dir_all(parent).await?;
    }
    let mut bytes = serde_json::to_vec_pretty(lib)
        .map_err(|e| SabrageError::io(path, std::io::Error::other(e)))?;
    bytes.push(b'\n');
    executor.write_atomic(path, &bytes).await
}

/// A fresh [`GameEntry`] for the Add-game wizard, seeded from `settings` and
/// the machine's current bottle list.
///
/// Precedence (brief, "B — store"):
/// * `bottle` — `settings.default_bottle`, else the first of `bottles`, else
///   `""`;
/// * `bs_dir` — `settings.default_bs_dir`, else `env_bs_dir`
///   (`WINEVR_BS_DIR`, passed in rather than read here so this stays a pure
///   function of its arguments), else [`resolve_bs_dir`]'s own default for
///   the chosen bottle.
pub fn new_entry_template(
    settings: &Settings,
    bottles: &[String],
    env_bs_dir: Option<&str>,
) -> GameEntry {
    let bottle = settings
        .default_bottle
        .clone()
        .or_else(|| bottles.first().cloned())
        .unwrap_or_default();

    let bs_dir = settings
        .default_bs_dir
        .clone()
        .or_else(|| env_bs_dir.map(str::to_string))
        .unwrap_or_else(|| {
            let unvalidated = Bottle::unvalidated(&bottle);
            resolve_bs_dir(Some(&unvalidated), None)
                .display()
                .to_string()
        });

    GameEntry {
        id: Uuid::new_v4(),
        name: "Beat Saber 1.29.4".to_string(),
        bs_dir,
        bottle,
        // The contract pins this as u64 (headroom for a hypothetically huge
        // Steam appid); Beat Saber's is 620980, well inside u32.
        appid: contract().game.appid as u32,
        added_at_unix_ms: crate::session::now_unix_ms(),
        launch_overrides: LaunchOverrides::default(),
        last_session: None,
    }
}

/// `settings.launch ⊕ entry.launch_overrides` (an override's `Some` wins over
/// the global default), with `bottle`/`bs_dir` taken from `entry`.
/// `dry_run` is always `false` — a caller decides that, it is never a stored
/// preference.
pub fn effective_options(settings: &Settings, entry: &GameEntry) -> StageOptions {
    let o = &entry.launch_overrides;
    StageOptions {
        bottle_name: Some(entry.bottle.clone()),
        bs_dir_override: Some(PathBuf::from(&entry.bs_dir)),
        dry_run: false,
        verbose: o.verbose.unwrap_or(settings.launch.verbose),
        no_audio: o.no_audio.unwrap_or(settings.launch.no_audio),
        no_dashboard: o.no_dashboard.unwrap_or(settings.launch.no_dashboard),
        wired: o.wired.unwrap_or(settings.launch.wired),
    }
}

/// Where the installed `steam_api64.dll` stands relative to the Goldberg
/// dll and its `.orig-steam` backup.
///
/// "Is Goldberg" means **either** the contract-pinned build
/// (`gbe_dll_sha256`) **or** the payload this checkout would install
/// (`Paths::gbe_dll`) byte for byte: `run` installs whatever is at that
/// path and only warns on a pin mismatch
/// (PARITY.md § Invariants that must NOT change (byte/behavior parity),
/// "Goldberg hash-tolerance at run"), and a pin bump orphans a dll
/// installed before it. A pin-only test would call both `Original`, and the
/// revert door would offer to "restore" Goldberg's own bytes
/// (tests::goldberg_state_covers_all_five_variants).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GoldbergState {
    /// The Goldberg dll is installed and a `.orig-steam` backup exists
    /// — the ordinary post-launch state. The backup is still only *a* backup:
    /// nothing on this machine can prove it is the real Steam dll (it is
    /// whatever the first launch found in place), which is why
    /// [`super::goldberg::revert_original_steam_dll`] talks about restoring
    /// the backup rather than "the original".
    Applied,
    /// The Goldberg dll is installed and there is **no** `.orig-steam`
    /// backup: this install arrived already Goldberg'd (or the backup was
    /// deleted), so no copy of the real Steam dll exists here at all. Not
    /// [`GoldbergState::Original`] — the bytes are provably Goldberg's.
    AppliedUnverified,
    /// A dll is present, is neither the pinned Goldberg build nor this
    /// checkout's Goldberg payload, and there is no `.orig-steam` backup:
    /// as far as anything here can tell, Goldberg was never installed (the
    /// real Steam dll, untouched).
    Original,
    /// A `.orig-steam` backup exists but the live dll is not Goldberg's —
    /// installed once, then swapped for something else since.
    Modified,
    /// No `steam_api64.dll` at all, under either search path.
    NoDll,
}

/// Where a [`GameEntry`] stands, for the Library screen's status tag.
///
/// Invariant: **`Ready` means every hard gate the launch itself enforces is
/// already satisfied** — exe, 1.29.4, bottle, `z:` when outside `drive_c`,
/// and `steam_api64.dll`. Anything `run.sh`/[`crate::stages::run`] would
/// `die` on must show as something other than `Ready`, or the badge
/// contradicts the button next to it
/// (tests::healthy_game_without_steam_dll_is_not_ready).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GameStatus {
    Ready,
    NeedsAttention,
    NotFound,
    NeedsSetup,
}

/// One full snapshot of a game's install health — every fact the Library and
/// Edit-game screens render, computed fresh on every call (never persisted:
/// the machine can change under a stored entry at any time).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameValidity {
    pub exe_present: bool,
    pub detected_version: Option<String>,
    pub version_ok: bool,
    pub bottle_exists: bool,
    pub bottle_template: Option<String>,
    pub bottle_backend_dxmt: bool,
    pub outside_drive_c: bool,
    pub z_drive_ok: Option<bool>,
    pub goldberg: GoldbergState,
    pub orig_steam_present: bool,
    pub status: GameStatus,
    pub problems: Vec<String>,
}

/// Extract the value of a `"Template" = "…"` line from `cxbottle.conf` text,
/// if the key is present. A display value, unlike `checks::bottle`'s
/// `bottle_template` evaluator (which only tests equality against
/// `win11_64`) — this returns whatever the bottle's Template key actually
/// says, for the Library detail row.
fn bottle_template_value(conf: &str) -> Option<String> {
    let line = conf.lines().find(|l| l.starts_with("\"Template\""))?;
    let rest = line.strip_prefix("\"Template\"")?.trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Read-only probes over one `(bs_dir, bottle)` pair. Never touches the
/// machine beyond `stat`/`read` — same contract as every `checks::*`
/// evaluator.
///
/// `paths` is accepted whole (like `CheckCtx`'s) because callers already
/// have one; its `gbe_dll` is the Goldberg payload *this checkout* installs,
/// read by the classification alongside the contract pin.
///
/// Thin wrapper around [`validate_with_bottle`].
pub fn validate(paths: &Paths, bs_dir: &Path, bottle_name: &str) -> GameValidity {
    let bottle = Bottle::unvalidated(bottle_name);
    validate_with_bottle(&paths.gbe_dll, bs_dir, bottle_name, &bottle)
}

/// [`validate`]'s actual logic, taking an already-resolved [`Bottle`] rather
/// than building one from `bottle_name` itself.
///
/// The split exists **entirely for testability**: [`Bottle::unvalidated`]
/// resolves against the real `$HOME`-derived bottles root
/// (`paths::bottles_root` is not `Paths`-derived), so only a caller-supplied
/// `Bottle` — whose `prefix` may point anywhere — lets a test make
/// `bottle_exists` true from a fixture. [`validate`] is the only caller that
/// derives one from `$HOME`.
fn validate_with_bottle(
    gbe_dll: &Path,
    bs_dir: &Path,
    bottle_name: &str,
    bottle: &Bottle,
) -> GameValidity {
    validate_pinned(
        gbe_dll,
        bs_dir,
        bottle_name,
        bottle,
        &contract().deps.gbe_dll_sha256,
    )
}

/// [`validate_with_bottle`] with the Goldberg pin passed in.
///
/// Second testability seam: no test can fabricate bytes hashing to the real
/// contract pin, so a test exercising the pin-matched branches hands in the
/// digest of its own fixture. [`validate_with_bottle`] is the only caller
/// that reads the contract pin.
fn validate_pinned(
    gbe_dll: &Path,
    bs_dir: &Path,
    bottle_name: &str,
    bottle: &Bottle,
    gbe_dll_sha256: &str,
) -> GameValidity {
    let exe_present = bs_dir.join("Beat Saber.exe").is_file();
    let raw_version = bs_version(bs_dir);
    let detected_version = (raw_version != "?").then_some(raw_version);
    let version_ok = detected_version
        .as_deref()
        .is_some_and(|v| v.starts_with("1.29.4"));

    let bottle_exists = !bottle_name.is_empty() && bottle.exists();

    let conf = if bottle_exists {
        std::fs::read_to_string(bottle.conf_path()).unwrap_or_default()
    } else {
        String::new()
    };
    let bottle_template = bottle_template_value(&conf);
    let bottle_backend_dxmt = conf
        .lines()
        .any(|l| l == "\"CX_GRAPHICS_BACKEND\" = \"dxmt\"");

    // The same prefix test `checks::bottle::bs_dir_outside_drive_c` makes,
    // computed unconditionally: this module has no "bottle resolved" gate,
    // and an unresolved bottle's prefix is still a meaningful string.
    let outside_drive_c = {
        let glob = format!("{}/drive_c/", bottle.prefix.display());
        !bs_dir.to_string_lossy().starts_with(&glob)
    };
    let z_drive_ok = outside_drive_c.then(|| bottle.z_drive().exists());

    // The dll's own bytes are the only positive evidence available here:
    // deriving "original" from a missing backup labels an already-Goldberg'd
    // install untouched, then backs it up and "restores" it — see
    // `super::goldberg`'s refusal.
    let api = steam_api_path(bs_dir);
    let dll_present = api.is_file();
    let orig_steam_present = orig_steam_path(&api).is_file();
    let dll_is_goldberg =
        dll_present && (file_sha256_matches(&api, gbe_dll_sha256) || cmp_files(gbe_dll, &api));
    let goldberg = match (dll_present, dll_is_goldberg, orig_steam_present) {
        (false, _, _) => GoldbergState::NoDll,
        (_, true, true) => GoldbergState::Applied,
        (_, true, false) => GoldbergState::AppliedUnverified,
        (_, false, true) => GoldbergState::Modified,
        (_, false, false) => GoldbergState::Original,
    };

    let mut problems = Vec::new();
    if !exe_present {
        problems.push(format!("Beat Saber.exe not found at {}", bs_dir.display()));
    } else if !version_ok {
        problems.push(format!(
            "Beat Saber version '{}' is not 1.29.4",
            detected_version.as_deref().unwrap_or("?")
        ));
    }
    if !bottle_exists {
        problems.push(format!("CrossOver bottle '{bottle_name}' not found"));
    } else {
        if bottle_template.as_deref() != Some("win11_64") {
            problems.push(format!(
                "bottle template is not win11_64 ({})",
                bottle_template.as_deref().unwrap_or("")
            ));
        }
        if !bottle_backend_dxmt {
            problems.push("bottle graphics backend is not dxmt (auto-fixed at launch)".to_string());
        }
    }
    if z_drive_ok == Some(false) {
        problems.push("Beat Saber is outside drive_c but the bottle has no z: drive".to_string());
    }
    if goldberg == GoldbergState::NoDll {
        // run.sh's `# launch-action: goldberg-stage` dies here in these same
        // words (`stages::run::actions::goldberg_stage`); a game that cannot
        // launch is never `Ready`.
        problems.push(format!(
            "steam_api64.dll not found under {} — is this a complete Beat Saber install?",
            bs_dir.display()
        ));
    }

    // Most-specific first; `Ready` is the invariant documented on
    // `GameStatus`.
    let status = if !exe_present {
        GameStatus::NotFound
    } else if !bottle_exists {
        GameStatus::NeedsSetup
    } else if !version_ok || z_drive_ok == Some(false) || goldberg == GoldbergState::NoDll {
        GameStatus::NeedsAttention
    } else {
        GameStatus::Ready
    };

    GameValidity {
        exe_present,
        detected_version,
        version_ok,
        bottle_exists,
        bottle_template,
        bottle_backend_dxmt,
        outside_drive_c,
        z_drive_ok,
        goldberg,
        orig_steam_present,
        status,
        problems,
    }
}

#[cfg(test)]
mod tests;
