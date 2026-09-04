//! The revert-original-`steam_api64.dll` action.
//!
//! Sabrage-only, so a user can launch through real Steam once without hunting
//! down `.orig-steam` by hand: run.sh installs Goldberg and never restores the
//! backup (PARITY.md § Planned for later phases (declared now),
//! "Revert-original-`steam_api64.dll` action").
//!
//! The revert swaps the dll back and leaves `.orig-steam`, `steam_appid.txt`
//! and `steam_settings/` in place, because the next launch through
//! [`crate::stages::run::actions::goldberg_stage`] reinstalls Goldberg
//! unconditionally.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::contract::contract;
use crate::error::{Result, SabrageError};
use crate::executor::Executor;
use crate::paths::Paths;
use crate::session;
use crate::stages::run::actions::steam_api_path;
use crate::util::{cmp_files, file_sha256_matches};

/// The argv substring that means "Beat Saber is running", kept byte-identical
/// to [`crate::stages::stop`]'s private const of the same string so the two
/// doors agree on what a live game looks like.
const BEAT_SABER_EXE_NEEDLE: &str = "Beat Saber.exe";

/// What [`revert_original_steam_dll`] did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevertReport {
    /// `true` iff a `.orig-steam` backup existed, was not itself a Goldberg
    /// dll (see [`revert_original_steam_dll`] for what that is tested
    /// against), and was copied back over the live dll; `false` means nothing
    /// was reverted and `message` says which case applied.
    pub restored: bool,
    /// Human-readable summary, safe to show verbatim in the UI.
    pub message: String,
    /// The dll path that was (or would have been) reverted, for display.
    pub dll_path: String,
}

/// `"$API.orig-steam"` — the backup path
/// [`crate::stages::run::actions::goldberg_stage`] writes.
///
/// `pub(crate)` so [`super::library::validate`] can compute the same path for
/// its `origSteamPresent` probe.
pub(crate) fn orig_steam_path(api: &Path) -> PathBuf {
    let mut s = api.as_os_str().to_os_string();
    s.push(".orig-steam");
    PathBuf::from(s)
}

/// Restore the Steam `steam_api64.dll` from its `.orig-steam` backup.
///
/// Refuses while a session is live ([`session::live_session_reason`], with the
/// operation lock held across the check and the copy), while any Beat Saber
/// process is on argv, and when the backup is itself a Goldberg dll — tested
/// against the contract pin, this checkout's `Paths::gbe_dll` payload, and the
/// provenance a launch records
/// ([`crate::stages::run::actions::goldberg_backup_is_goldberg`]), because the
/// launch tolerates an unpinned payload (PARITY.md § Invariants that must NOT
/// change (byte/behavior parity), "Goldberg hash-tolerance at run").
///
/// The argv scan is not scoped to `bs_dir`: wine puts a `Z:\` Windows path on
/// the command line, which no unix prefix test can match, so any running Beat
/// Saber refuses this dll swap (A13a-2).
///
/// Nothing here proves a backup is the real Steam dll, only that it is not a
/// Goldberg one this pipeline knows; the success message says "the .orig-steam
/// backup", never "the original". See
/// tests::{refuses_when_the_backup_is_itself_the_pinned_goldberg_dll,
/// refuses_when_the_backup_is_an_unpinned_goldberg_build,
/// refuses_when_a_launch_recorded_the_backup_as_goldberg,
/// refuses_while_a_matching_game_process_is_running,
/// the_success_message_never_claims_the_original_was_restored}.
///
/// # Errors
///
/// Fatal when a session is live or a matching game process is running, and
/// whatever the copy returns; the no-backup and backup-is-Goldberg cases are
/// `Ok` with `restored: false`.
pub async fn revert_original_steam_dll(
    executor: &dyn Executor,
    bs_dir: &Path,
) -> Result<RevertReport> {
    // Built here rather than taken as a parameter so the one call site (the
    // Tauri command layer) stays two-argument: the persisted
    // `settings.repo_root` through `resolve_repo_root`, degrading to the empty
    // root exactly as `SettingsPathsCache::snapshot` does when either step
    // fails. Degrading is harmless for the liveness predicate
    // (`sabrage_appsup`/`oxr_appsup` are `$HOME`-derived either way) but not
    // free: the is-Goldberg check reads `paths.gbe_dll`.
    let appsup = Paths::new(PathBuf::new()).sabrage_appsup;
    let repo_root = super::settings::load(&super::settings::settings_path(&appsup))
        .ok()
        .and_then(|s| crate::paths::resolve_repo_root(s.repo_root.as_deref()).ok())
        .unwrap_or_default();
    let paths = Paths::new(repo_root);
    revert_with_pin(executor, &paths, bs_dir, &contract().deps.gbe_dll_sha256).await
}

/// [`revert_original_steam_dll`] with the Goldberg pin passed in — the
/// testability seam, because no test can fabricate bytes hashing to the real
/// contract pin.
pub(crate) async fn revert_with_pin(
    executor: &dyn Executor,
    paths: &Paths,
    bs_dir: &Path,
    gbe_dll_sha256: &str,
) -> Result<RevertReport> {
    revert_probed(
        executor,
        paths,
        bs_dir,
        gbe_dll_sha256,
        BEAT_SABER_EXE_NEEDLE,
    )
    .await
}

/// [`revert_with_pin`] with the running-game argv needle passed in, so a test
/// can hand in a needle that matches nothing or one that matches its own
/// command line (tests::refuses_while_a_matching_game_process_is_running).
async fn revert_probed(
    executor: &dyn Executor,
    paths: &Paths,
    bs_dir: &Path,
    gbe_dll_sha256: &str,
    game_needle: &str,
) -> Result<RevertReport> {
    // Held through the liveness re-check and the copy: a run holds this lock
    // from before its Goldberg step until after it publishes the live session,
    // so it closes the check-then-copy window `live_session_reason` alone
    // leaves open (tests::waits_for_the_operation_lock_then_proceeds).
    let _op = crate::stages::acquire_operation_lock().await;

    // The machine-wide predicate, not a local copy: a `./demo.sh run` session
    // writes neither an in-process handle nor `session-state.json`, and only
    // `live_session_reason` sees it, through its fresh `runtime_status.json`
    // (tests::refuses_while_only_the_runtime_reports_a_live_session). Its
    // rule that an unverifiable record counts as live applies here too.
    if let Some(reason) = session::live_session_reason(paths) {
        return Err(SabrageError::fatal(
            format!("cannot revert steam_api64.dll while a session is live — {reason}"),
            "stop the running session first",
        ));
    }

    // The same running-game signal `live_session_reason` ends on
    // (`session::running_game_pid`), re-probed here with an injectable needle so a
    // test can drive this refusal without starting Beat Saber
    // (tests::refuses_while_a_matching_game_process_is_running). It is the one
    // signal that needs nothing to have been written, which is what covers a
    // `./demo.sh run` between its wine spawn and its first streaming status.
    let games = crate::process::find_processes_by_cmdline(game_needle);
    if let Some(p) = games.first() {
        return Err(SabrageError::fatal(
            format!(
                "cannot revert steam_api64.dll while a session is live — Beat Saber is \
                 running (pid {}), and it has this dll mapped",
                p.pid
            ),
            "stop the running session first (Stop in Sabrage, or ./demo.sh stop --bottle <name>)",
        ));
    }

    let api = steam_api_path(bs_dir);
    let backup = orig_steam_path(&api);
    let dll_path = api.display().to_string();

    if !backup.is_file() {
        return Ok(RevertReport {
            restored: false,
            message: format!(
                "no steam_api64.dll backup found at {} — nothing to revert \
                 (Goldberg may never have been installed here)",
                backup.display()
            ),
            dll_path,
        });
    }

    // Any kind of Goldberg: the contract pin, the payload this checkout installs,
    // or a launch's recorded provenance — only the third recognises a backup from
    // a Goldberg build `gbe_dll` does not match (PARITY.md § Invariants that must
    // NOT change (byte/behavior parity), "Goldberg hash-tolerance at run"), the
    // exact backup a bytes-only test would restore
    // (tests::refuses_when_a_launch_recorded_the_backup_as_goldberg).
    if file_sha256_matches(&backup, gbe_dll_sha256)
        || cmp_files(&paths.gbe_dll, &backup)
        || crate::stages::run::actions::goldberg_backup_is_goldberg(paths, &backup)
    {
        return Ok(RevertReport {
            restored: false,
            message: format!(
                "the backup at {} is itself the Goldberg steam_api64.dll — this install was \
                 already Goldberg'd when Sabrage first saw it, so no copy of the real Steam \
                 dll exists here to restore",
                backup.display()
            ),
            dll_path,
        });
    }

    executor.copy_if_changed(&backup, &api).await?;

    Ok(RevertReport {
        restored: true,
        message: format!(
            "restored steam_api64.dll from the .orig-steam backup at {} — the backup itself, \
             steam_appid.txt and the Goldberg sub-flag files were left in place, since the \
             next launch through Sabrage/demo.sh reinstalls Goldberg unconditionally",
            backup.display()
        ),
        dll_path,
    })
}

#[cfg(test)]
mod tests;
