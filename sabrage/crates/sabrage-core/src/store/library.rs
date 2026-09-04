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
/// installed before it. A pin-only test calls both `Original`, and the
/// revert door offers to "restore" Goldberg's own bytes
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
mod tests {
    use super::*;
    use crate::executor::{DryRunExecutor, PlannedKind, RealExecutor};
    use crate::stages::null_sink;
    use std::fs;
    use tokio_util::sync::CancellationToken;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sabrage-store-library-test-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn real() -> RealExecutor {
        RealExecutor::new(Uuid::nil(), null_sink(), CancellationToken::new())
    }

    fn entry(name: &str) -> GameEntry {
        GameEntry {
            id: Uuid::new_v4(),
            name: name.to_string(),
            bs_dir: "/games/bs".to_string(),
            bottle: "Steam".to_string(),
            appid: 620980,
            added_at_unix_ms: 1786300214181,
            launch_overrides: LaunchOverrides::default(),
            last_session: None,
        }
    }

    #[test]
    fn library_path_is_the_json_file_under_appsup() {
        assert_eq!(
            library_path(Path::new("/x/Sabrage")),
            PathBuf::from("/x/Sabrage/library.json")
        );
    }

    #[test]
    fn missing_file_loads_as_default() {
        let lib = load(Path::new("/nonexistent/sabrage/library.json")).unwrap();
        assert_eq!(lib, Library::default());
    }

    #[test]
    fn a_corrupt_file_is_an_error_never_a_silent_reset() {
        let dir = scratch("corrupt");
        let path = dir.join("library.json");
        fs::write(&path, b"{not json").unwrap();
        let err = load(&path).unwrap_err();
        assert_eq!(err.kind(), "io");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_newer_schema_version_is_refused_not_silently_rewritten() {
        let dir = scratch("newer-version");
        let path = dir.join("library.json");
        let text = format!(
            r#"{{"version":{},"games":[],"futureTopLevel":"keep-me"}}"#,
            LIBRARY_VERSION + 1
        );
        fs::write(&path, &text).unwrap();

        let err = load(&path).unwrap_err();
        assert!(err.to_string().contains("version"), "{err}");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            text,
            "a refused load never touches the file"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unknown_fields_are_ignored_on_load() {
        let dir = scratch("unknown-fields");
        let path = dir.join("library.json");
        fs::write(
            &path,
            r#"{"version":1,"games":[],"futureField":{"nested":true}}"#,
        )
        .unwrap();
        let lib = load(&path).unwrap();
        assert_eq!(lib, Library::default());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn round_trips_camel_case_through_the_file() {
        let dir = scratch("roundtrip");
        let path = dir.join("nested/library.json");
        let mut lib = Library::default();
        let mut e = entry("Beat Saber 1.29.4");
        e.launch_overrides.wired = Some(true);
        e.last_session = Some(LastSession {
            started_at_unix_ms: 1,
            ended_at_unix_ms: 2,
            exit_code: Some(0),
            log_path: Some("/repo/logs/x.log".into()),
        });
        lib.upsert(e);

        save(&real(), &path, &lib).await.unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.ends_with("}\n"));
        assert!(text.contains("\"bsDir\""));
        assert!(text.contains("\"launchOverrides\""));
        assert!(text.contains("\"startedAtUnixMs\""));
        assert_eq!(load(&path).unwrap(), lib);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn a_dry_run_executor_plans_the_write_instead_of_performing_it() {
        let dir = scratch("dry");
        let path = dir.join("library.json");
        let ex = DryRunExecutor::new(Uuid::nil(), null_sink(), CancellationToken::new());
        save(&ex, &path, &Library::default()).await.unwrap();
        assert!(!path.exists());
        let kinds: Vec<PlannedKind> = ex.planned().iter().map(|p| p.kind).collect();
        assert_eq!(kinds, vec![PlannedKind::CreateDir, PlannedKind::Write]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn transact_writes_only_when_the_library_actually_changed() {
        let dir = scratch("transact-noop");
        let path = dir.join("library.json");

        // A removal that finds nothing must not mint a library.json.
        let removed = transact(&real(), &path, |lib| lib.remove(Uuid::new_v4()))
            .await
            .unwrap();
        assert!(!removed);
        assert!(!path.exists(), "no change, no write");

        let e = entry("A");
        let id = e.id;
        transact(&real(), &path, |lib| {
            lib.upsert(e);
        })
        .await
        .unwrap();
        assert!(load(&path).unwrap().get(id).is_some());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn interleaved_transactions_do_not_resurrect_a_removed_game() {
        // The shape this pins: the Library screen removes a game while the
        // post-launch task records that same game's last session. Without
        // `transact`, each loads its own snapshot and saves the whole file
        // back, and whichever renames last wins outright.
        let dir = scratch("transact-race");
        let path = dir.join("library.json");
        let a = entry("A");
        let b = entry("B");
        let (id_a, id_b) = (a.id, b.id);
        let mut seed = Library::default();
        seed.upsert(a);
        seed.upsert(b);
        save(&real(), &path, &seed).await.unwrap();

        let session = LastSession {
            started_at_unix_ms: 1,
            ended_at_unix_ms: 2,
            exit_code: Some(0),
            log_path: None,
        };
        let (ex_a, ex_b) = (real(), real());
        let (removal, record) = tokio::join!(
            transact(&ex_a, &path, |lib| lib.remove(id_a)),
            transact(&ex_b, &path, {
                let session = session.clone();
                move |lib| lib.record_last_session(id_a, session)
            }),
        );
        assert!(removal.unwrap(), "the removal found the entry");
        let _ = record.unwrap(); // may or may not have found it — order decides

        let after = load(&path).unwrap();
        assert!(
            after.get(id_a).is_none(),
            "a removed game must never come back: {:?}",
            after.games.iter().map(|g| &g.name).collect::<Vec<_>>()
        );
        assert!(after.get(id_b).is_some(), "the other entry is untouched");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn an_edit_racing_a_recorded_session_keeps_both() {
        // A13b-5's half: the editor submits a whole entry it cloned before the
        // session was recorded. `upsert_editable` keeps the server-owned
        // fields, `transact` keeps the two writes from clobbering each other.
        let dir = scratch("transact-edit");
        let path = dir.join("library.json");
        let e = entry("A");
        let id = e.id;
        let mut seed = Library::default();
        seed.upsert(e.clone());
        save(&real(), &path, &seed).await.unwrap();

        let session = LastSession {
            started_at_unix_ms: 10,
            ended_at_unix_ms: 20,
            exit_code: Some(0),
            log_path: Some("/repo/logs/x.log".into()),
        };
        transact(&real(), &path, |lib| {
            lib.record_last_session(id, session.clone())
        })
        .await
        .unwrap();

        // The editor's stale clone: renamed, and still carrying no session.
        let mut stale = e;
        stale.name = "A renamed".to_string();
        transact(&real(), &path, |lib| {
            lib.upsert_editable(stale);
        })
        .await
        .unwrap();

        let after = load(&path).unwrap();
        let stored = after.get(id).unwrap();
        assert_eq!(stored.name, "A renamed", "the edit landed");
        assert_eq!(
            stored.last_session.as_ref(),
            Some(&session),
            "the session recorded while the form was open survived"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn upsert_inserts_then_replaces_by_id() {
        let mut lib = Library::default();
        let e = entry("A");
        let stored = lib.upsert(e.clone());
        assert_eq!(stored, &e);
        assert_eq!(lib.games.len(), 1);

        let mut replacement = e.clone();
        replacement.name = "A renamed".to_string();
        let stored = lib.upsert(replacement.clone());
        assert_eq!(stored.name, "A renamed");
        assert_eq!(lib.games.len(), 1, "same id replaces, does not append");
    }

    #[test]
    fn remove_reports_whether_it_found_something() {
        let mut lib = Library::default();
        let e = entry("A");
        let id = e.id;
        lib.upsert(e);
        assert!(lib.remove(id));
        assert!(lib.games.is_empty());
        assert!(
            !lib.remove(id),
            "removing twice finds nothing the second time"
        );
    }

    #[test]
    fn get_finds_by_id_and_nothing_else() {
        let mut lib = Library::default();
        let e = entry("A");
        let id = e.id;
        lib.upsert(e);
        assert!(lib.get(id).is_some());
        assert!(lib.get(Uuid::new_v4()).is_none());
    }

    #[test]
    fn upsert_editable_keeps_the_server_owned_fields_of_the_stored_entry() {
        let mut lib = Library::default();
        let mut stored = entry("A");
        stored.added_at_unix_ms = 1;
        stored.appid = 620980;
        stored.last_session = Some(LastSession {
            started_at_unix_ms: 5,
            ended_at_unix_ms: 6,
            exit_code: Some(0),
            log_path: None,
        });
        let expected_session = stored.last_session.clone();
        lib.upsert(stored.clone());

        // What the Edit-game form submits: the editable fields it changed,
        // plus whatever the clone happened to carry for the rest.
        let mut incoming = stored.clone();
        incoming.name = "A renamed".to_string();
        incoming.bottle = "Other".to_string();
        incoming.launch_overrides.wired = Some(true);
        incoming.last_session = None;
        incoming.added_at_unix_ms = 999;
        incoming.appid = 1;

        let saved = lib.upsert_editable(incoming).clone();
        assert_eq!(saved.name, "A renamed");
        assert_eq!(saved.bottle, "Other");
        assert_eq!(saved.launch_overrides.wired, Some(true));
        assert_eq!(saved.last_session, expected_session, "server-owned");
        assert_eq!(saved.added_at_unix_ms, 1, "server-owned");
        assert_eq!(saved.appid, 620980, "server-owned");
        assert_eq!(lib.games.len(), 1);

        // An id the library does not know is a plain insert.
        let fresh = entry("B");
        let fresh_id = fresh.id;
        lib.upsert_editable(fresh);
        assert!(lib.get(fresh_id).is_some());
    }

    #[test]
    fn record_last_session_updates_the_matching_entry_only() {
        let mut lib = Library::default();
        let a = entry("A");
        let b = entry("B");
        let (id_a, id_b) = (a.id, b.id);
        lib.upsert(a);
        lib.upsert(b);

        let session = LastSession {
            started_at_unix_ms: 100,
            ended_at_unix_ms: 200,
            exit_code: Some(0),
            log_path: Some("/repo/logs/beatsaber-x.log".into()),
        };
        assert!(lib.record_last_session(id_a, session.clone()));
        assert_eq!(lib.get(id_a).unwrap().last_session.as_ref(), Some(&session));
        assert!(lib.get(id_b).unwrap().last_session.is_none());

        assert!(!lib.record_last_session(Uuid::new_v4(), session));
    }

    #[test]
    fn template_prefers_settings_default_bottle_over_the_bottle_list() {
        let settings = Settings {
            default_bottle: Some("Preferred".into()),
            ..Settings::default()
        };
        let e = new_entry_template(&settings, &["Other".into()], None);
        assert_eq!(e.bottle, "Preferred");
        assert_eq!(e.name, "Beat Saber 1.29.4");
        assert_eq!(e.appid, 620980);
        assert!(e.last_session.is_none());
        assert_eq!(e.launch_overrides, LaunchOverrides::default());
    }

    #[test]
    fn template_falls_back_to_the_first_bottle_then_empty_string() {
        let e = new_entry_template(
            &Settings::default(),
            &["First".into(), "Second".into()],
            None,
        );
        assert_eq!(e.bottle, "First");

        let e = new_entry_template(&Settings::default(), &[], None);
        assert_eq!(e.bottle, "");
    }

    #[test]
    fn template_bs_dir_precedence_settings_then_env_then_resolved_default() {
        let settings_dir = Settings {
            default_bottle: Some("Steam".into()),
            default_bs_dir: Some("/from/settings".into()),
            ..Settings::default()
        };
        let e = new_entry_template(&settings_dir, &[], Some("/from/env"));
        assert_eq!(e.bs_dir, "/from/settings", "settings wins over env");

        let settings_no_dir = Settings {
            default_bottle: Some("Steam".into()),
            ..Settings::default()
        };
        let e = new_entry_template(&settings_no_dir, &[], Some("/from/env"));
        assert_eq!(e.bs_dir, "/from/env", "env wins when settings has none");

        let e = new_entry_template(&settings_no_dir, &[], None);
        assert!(
            e.bs_dir.ends_with(
                "Steam/drive_c/Program Files (x86)/Steam/steamapps/common/Beat Saber 1294"
            ),
            "falls back to resolve_bs_dir's default: {}",
            e.bs_dir
        );
    }

    #[test]
    fn effective_options_merges_overrides_over_settings_and_takes_identity_from_the_entry() {
        let settings = Settings {
            launch: crate::store::settings::LaunchDefaults {
                no_audio: false,
                no_dashboard: true,
                wired: false,
                verbose: false,
                ..Default::default()
            },
            ..Settings::default()
        };
        let mut e = entry("A");
        e.launch_overrides = LaunchOverrides {
            no_audio: Some(true), // override flips the global default
            no_dashboard: None,   // falls through to settings (true)
            wired: None,          // falls through to settings (false)
            verbose: Some(true),  // override sets what settings left false
        };

        let opts = effective_options(&settings, &e);
        assert_eq!(opts.bottle_name.as_deref(), Some("Steam"));
        assert_eq!(opts.bs_dir_override, Some(PathBuf::from("/games/bs")));
        assert!(!opts.dry_run);
        assert!(opts.no_audio, "override Some(true) beats settings false");
        assert!(opts.no_dashboard, "None falls through to settings true");
        assert!(!opts.wired, "None falls through to settings false");
        assert!(opts.verbose, "override Some(true) beats settings false");
    }

    #[test]
    fn launch_options_for_resolves_the_merge_by_id_and_is_none_for_a_stranger() {
        let settings = Settings {
            launch: crate::store::settings::LaunchDefaults {
                no_audio: false,
                no_dashboard: true,
                wired: false,
                verbose: false,
                ..Default::default()
            },
            ..Settings::default()
        };
        let mut e = entry("A");
        e.launch_overrides.no_audio = Some(true);
        let id = e.id;
        let mut lib = Library::default();
        lib.upsert(e.clone());

        let opts = lib.launch_options_for(id, &settings).unwrap();
        assert_eq!(
            opts,
            effective_options(&settings, &e),
            "one merge, one home"
        );
        assert!(lib.launch_options_for(Uuid::new_v4(), &settings).is_none());
    }

    fn fake_bottle(label: &str) -> Bottle {
        let dir = std::env::temp_dir().join(format!(
            "sabrage-library-test-bottle-{label}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Bottle {
            name: "TestBottle".to_string(),
            sys32: dir.join("drive_c/windows/system32"),
            prefix: dir,
        }
    }

    fn paths() -> Paths {
        Paths::new("/nonexistent/sabrage/repo")
    }

    #[test]
    fn no_bottle_no_exe_is_not_found_with_a_says_why_problem() {
        let bs_dir = scratch("validate-notfound");
        let v = validate(&paths(), &bs_dir, "");
        assert!(!v.exe_present);
        assert!(!v.bottle_exists, "empty bottle name never exists");
        assert_eq!(v.status, GameStatus::NotFound);
        assert!(v.problems.iter().any(|p| p.contains("Beat Saber.exe")));
        fs::remove_dir_all(&bs_dir).unwrap();
    }

    #[test]
    fn exe_present_but_bottle_missing_needs_setup() {
        let bs_dir = scratch("validate-needssetup");
        fs::write(bs_dir.join("Beat Saber.exe"), b"stub").unwrap();
        fs::write(bs_dir.join("BeatSaberVersion.txt"), "1.29.4_4575554838\n").unwrap();
        let v = validate(&paths(), &bs_dir, "NoSuchBottle");
        assert!(v.exe_present);
        assert!(v.version_ok);
        assert!(!v.bottle_exists);
        assert_eq!(v.status, GameStatus::NeedsSetup);
        assert!(v
            .problems
            .iter()
            .any(|p| p.contains("CrossOver bottle 'NoSuchBottle' not found")));
        fs::remove_dir_all(&bs_dir).unwrap();
    }

    #[test]
    fn wrong_version_needs_attention() {
        let bs_dir = scratch("validate-wrongversion");
        fs::write(bs_dir.join("Beat Saber.exe"), b"stub").unwrap();
        fs::write(bs_dir.join("BeatSaberVersion.txt"), "1.34.2_9999999999\n").unwrap();
        let b = fake_bottle("wrongversion");
        fs::write(
            b.conf_path(),
            "\"Template\" = \"win11_64\"\n\"CX_GRAPHICS_BACKEND\" = \"dxmt\"\n",
        )
        .unwrap();

        let v = validate_with_bottle(&paths().gbe_dll, &bs_dir, &b.name, &b);
        assert_eq!(v.detected_version.as_deref(), Some("1.34.2_9999999999"));
        assert!(!v.version_ok);
        assert!(v.bottle_exists);
        assert_eq!(v.status, GameStatus::NeedsAttention);
        assert!(v.problems.iter().any(|p| p.contains("is not 1.29.4")));

        fs::remove_dir_all(&bs_dir).unwrap();
        fs::remove_dir_all(&b.prefix).unwrap();
    }

    #[test]
    fn outside_drive_c_without_z_drive_needs_attention() {
        let b = fake_bottle("nozdrive");
        fs::write(
            b.conf_path(),
            "\"Template\" = \"win11_64\"\n\"CX_GRAPHICS_BACKEND\" = \"dxmt\"\n",
        )
        .unwrap();
        let bs_dir = scratch("validate-outside");
        fs::write(bs_dir.join("Beat Saber.exe"), b"stub").unwrap();
        fs::write(bs_dir.join("BeatSaberVersion.txt"), "1.29.4_4575554838\n").unwrap();

        let v = validate_with_bottle(&paths().gbe_dll, &bs_dir, &b.name, &b);
        assert!(
            v.outside_drive_c,
            "scratch dir is not under the bottle's drive_c"
        );
        assert_eq!(v.z_drive_ok, Some(false));
        assert_eq!(v.status, GameStatus::NeedsAttention);
        assert!(v.problems.iter().any(|p| p.contains("no z: drive")));

        fs::remove_dir_all(&bs_dir).unwrap();
        fs::remove_dir_all(&b.prefix).unwrap();
    }

    #[test]
    fn a_fully_healthy_game_is_ready_with_no_problems() {
        let b = fake_bottle("ready");
        fs::create_dir_all(b.prefix.join("drive_c")).unwrap();
        fs::write(
            b.conf_path(),
            "\"Template\" = \"win11_64\"\n\"CX_GRAPHICS_BACKEND\" = \"dxmt\"\n",
        )
        .unwrap();
        let bs_dir = b
            .prefix
            .join("drive_c/Program Files (x86)/Steam/steamapps/common/Beat Saber 1294");
        fs::create_dir_all(&bs_dir).unwrap();
        fs::write(bs_dir.join("Beat Saber.exe"), b"stub").unwrap();
        fs::write(bs_dir.join("BeatSaberVersion.txt"), "1.29.4_4575554838\n").unwrap();
        // Ready requires the dll run.sh's `# launch-action: goldberg-stage`
        // would otherwise die on — see
        // `healthy_game_without_steam_dll_is_not_ready`.
        fs::write(bs_dir.join("steam_api64.dll"), b"REAL-STEAM").unwrap();

        let v = validate_with_bottle(&paths().gbe_dll, &bs_dir, &b.name, &b);
        assert!(v.exe_present && v.version_ok && v.bottle_exists && v.bottle_backend_dxmt);
        assert!(
            !v.outside_drive_c,
            "install lives under the bottle's drive_c"
        );
        assert_eq!(
            v.z_drive_ok, None,
            "z: is irrelevant when not outside drive_c"
        );
        assert_eq!(v.bottle_template.as_deref(), Some("win11_64"));
        assert_eq!(v.status, GameStatus::Ready);
        assert!(v.problems.is_empty(), "{:?}", v.problems);

        fs::remove_dir_all(&b.prefix).unwrap();
    }

    #[test]
    fn bottle_template_and_backend_mismatches_surface_as_problems_without_forcing_needs_attention()
    {
        // Template and backend are detail-row facts, not launch gates: a
        // wrong value surfaces as a problem but never moves `status` off
        // Ready.
        let b = fake_bottle("mismatch");
        fs::create_dir_all(b.prefix.join("drive_c")).unwrap();
        fs::write(
            b.conf_path(),
            "\"Template\" = \"win10_64\"\n\"CX_GRAPHICS_BACKEND\" = \"auto\"\n",
        )
        .unwrap();
        let bs_dir = b.prefix.join("drive_c/bs");
        fs::create_dir_all(&bs_dir).unwrap();
        fs::write(bs_dir.join("Beat Saber.exe"), b"stub").unwrap();
        fs::write(bs_dir.join("BeatSaberVersion.txt"), "1.29.4_4575554838\n").unwrap();
        fs::write(bs_dir.join("steam_api64.dll"), b"REAL-STEAM").unwrap();

        let v = validate_with_bottle(&paths().gbe_dll, &bs_dir, &b.name, &b);
        assert_eq!(v.bottle_template.as_deref(), Some("win10_64"));
        assert!(!v.bottle_backend_dxmt);
        assert_eq!(v.status, GameStatus::Ready);
        assert!(v.problems.iter().any(|p| p.contains("win11_64")));
        assert!(v.problems.iter().any(|p| p.contains("not dxmt")));

        fs::remove_dir_all(&b.prefix).unwrap();
    }

    fn plugin_dir(bs_dir: &Path) -> PathBuf {
        bs_dir.join("Beat Saber_Data/Plugins/x86_64")
    }

    #[test]
    fn goldberg_state_covers_all_five_variants() {
        let bs_dir = scratch("validate-goldberg");
        let dir = plugin_dir(&bs_dir);
        fs::create_dir_all(&dir).unwrap();
        let bottle = fake_bottle("goldberg-matrix");
        // The pin the fixture's "Goldberg" bytes actually hash to — no test
        // can fabricate bytes matching the contract's real digest, which is
        // exactly what `validate_pinned` exists for.
        let pin = crate::util::sha256_bytes(b"GOLDBERG-EMULATOR-BYTES");
        // The checkout's staged Goldberg payload — the *other* way a dll is
        // known to be Goldberg. Absent for every case below except the last.
        let payload = bs_dir.join("third_party-gbe-steam_api64.dll");
        let v = |pin: &str| validate_pinned(&payload, &bs_dir, &bottle.name, &bottle, pin);

        // No dll at all.
        assert_eq!(v(&pin).goldberg, GoldbergState::NoDll);
        assert!(!v(&pin).orig_steam_present);

        // A dll that is not Goldberg, no backup: never Goldberg'd.
        fs::write(dir.join("steam_api64.dll"), b"REAL-STEAM").unwrap();
        assert_eq!(v(&pin).goldberg, GoldbergState::Original);
        assert!(!v(&pin).orig_steam_present);

        // The Goldberg dll with **no** backup: not `Original` — the bytes
        // prove otherwise (an install that arrived already Goldberg'd).
        fs::write(dir.join("steam_api64.dll"), b"GOLDBERG-EMULATOR-BYTES").unwrap();
        let got = v(&pin);
        assert_eq!(got.goldberg, GoldbergState::AppliedUnverified);
        assert!(!got.orig_steam_present);

        // Backup present, live dll does not match the pin: Modified.
        fs::write(dir.join("steam_api64.dll.orig-steam"), b"REAL-STEAM").unwrap();
        fs::write(dir.join("steam_api64.dll"), b"SOME-OTHER-BYTES").unwrap();
        let got = v(&pin);
        assert_eq!(got.goldberg, GoldbergState::Modified);
        assert!(got.orig_steam_present);

        // Backup present, live dll matches the pin: Applied.
        fs::write(dir.join("steam_api64.dll"), b"GOLDBERG-EMULATOR-BYTES").unwrap();
        assert_eq!(v(&pin).goldberg, GoldbergState::Applied);

        // A13a-1: a Goldberg build that is **not** the pin — an older or
        // hand-swapped `third_party/gbe/steam_api64.dll`, or a dll installed
        // before a pin bump — with no backup. Byte-identical to the payload
        // this checkout installs, so it is not the untouched Steam original,
        // and the revert door must not offer to "restore" it.
        fs::remove_file(dir.join("steam_api64.dll.orig-steam")).unwrap();
        fs::write(&payload, b"CUSTOM-GOLDBERG-BUILD").unwrap();
        fs::write(dir.join("steam_api64.dll"), b"CUSTOM-GOLDBERG-BUILD").unwrap();
        let got = v(&pin); // `pin` matches the *other* fixture bytes, never these
        assert_eq!(got.goldberg, GoldbergState::AppliedUnverified);
        // …and with a backup alongside it, the ordinary applied state.
        fs::write(dir.join("steam_api64.dll.orig-steam"), b"REAL-STEAM").unwrap();
        assert_eq!(v(&pin).goldberg, GoldbergState::Applied);
        // Restore the matrix's tail state for the contract-pin assertion below.
        fs::remove_file(&payload).unwrap();
        fs::write(dir.join("steam_api64.dll"), b"GOLDBERG-EMULATOR-BYTES").unwrap();

        // And the real contract pin still flows through `validate` itself:
        // these fixture bytes are not it, so the same tree reads as Modified.
        assert_eq!(
            validate(&paths(), &bs_dir, "").goldberg,
            GoldbergState::Modified
        );

        fs::remove_dir_all(&bs_dir).unwrap();
        fs::remove_dir_all(&bottle.prefix).unwrap();
    }

    #[test]
    fn healthy_game_without_steam_dll_is_not_ready() {
        // Everything run.sh checks except the dll its
        // `# launch-action: goldberg-stage` block dies on.
        let b = fake_bottle("nodll");
        fs::create_dir_all(b.prefix.join("drive_c")).unwrap();
        fs::write(
            b.conf_path(),
            "\"Template\" = \"win11_64\"\n\"CX_GRAPHICS_BACKEND\" = \"dxmt\"\n",
        )
        .unwrap();
        let bs_dir = b.prefix.join("drive_c/bs");
        fs::create_dir_all(&bs_dir).unwrap();
        fs::write(bs_dir.join("Beat Saber.exe"), b"stub").unwrap();
        fs::write(bs_dir.join("BeatSaberVersion.txt"), "1.29.4_4575554838\n").unwrap();

        let v = validate_with_bottle(&paths().gbe_dll, &bs_dir, &b.name, &b);
        assert_eq!(v.goldberg, GoldbergState::NoDll);
        assert_ne!(
            v.status,
            GameStatus::Ready,
            "the launch would die on the missing dll"
        );
        assert_eq!(v.status, GameStatus::NeedsAttention);
        assert!(
            v.problems.iter().any(|p| p.contains("steam_api64.dll")),
            "{:?}",
            v.problems
        );

        fs::remove_dir_all(&b.prefix).unwrap();
    }
}
