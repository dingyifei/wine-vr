//! The revert-original-`steam_api64.dll` action.
//!
//! // DIVERGENCE: `run.sh` has no counterpart — it only ever *installs*
//! Goldberg (`stages::run::actions::goldberg_stage`), never restores the real
//! Steam library. This is Sabrage-only, declared in `PARITY.md` ("Revert-
//! original-`steam_api64.dll` action (no shell counterpart either …) — Phase
//! 4+, if ever)"). It exists so a user who wants to launch through real Steam
//! once (screenshots, a Steam-only mod, sanity-checking a purchase) does not
//! have to go hunt down `.orig-steam` by hand.
//!
//! Reverting is deliberately narrow: it swaps the dll back and leaves every
//! other Goldberg artifact in place (`.orig-steam` itself, `steam_appid.txt`,
//! `steam_settings/`), because the very next `./demo.sh run` /
//! [`crate::stages::run::actions::goldberg_stage`] reinstalls Goldberg
//! unconditionally — there is nothing to clean up that the next launch
//! wouldn't just redo.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Result, SabrageError};
use crate::executor::Executor;
use crate::session;
use crate::stages::run::actions::steam_api_path;

/// What [`revert_original_steam_dll`] did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevertReport {
    /// `true` iff a `.orig-steam` backup existed and was copied back over the
    /// live dll. `false` means there was nothing to revert (never installed,
    /// or installed with no `.orig-steam` this action could find).
    pub restored: bool,
    /// Human-readable summary, safe to show verbatim in the UI.
    pub message: String,
    /// The dll path that was (or would have been) reverted, for display.
    pub dll_path: String,
}

/// `"$API.orig-steam"` — the exact suffix
/// [`crate::stages::run::actions::goldberg_stage`] uses, reproduced here
/// rather than imported from that module (its copy is a private fn, and this
/// one-line `OsString` push is not worth a `pub(crate)` seam there for).
/// `pub(crate)` so [`super::library::validate`] can compute the same path for
/// its `origSteamPresent` probe without a third copy.
pub(crate) fn orig_steam_path(api: &Path) -> PathBuf {
    let mut s = api.as_os_str().to_os_string();
    s.push(".orig-steam");
    PathBuf::from(s)
}

/// Restore the real Steam `steam_api64.dll` from its `.orig-steam` backup.
///
/// Refuses while a session is live ([`session::live_session`]) — the dll is
/// memory-mapped by a running Beat Saber process, and swapping it out from
/// under a live game is exactly the kind of "mutate the machine mid-session"
/// the operation lock and this check both exist to prevent.
pub async fn revert_original_steam_dll(
    executor: &dyn Executor,
    bs_dir: &Path,
) -> Result<RevertReport> {
    if session::live_session().is_some() {
        return Err(SabrageError::fatal(
            "cannot revert steam_api64.dll while a session is live",
            "stop the running session first",
        ));
    }

    let api = steam_api_path(bs_dir);
    let backup = orig_steam_path(&api);
    let dll_path = api.display().to_string();

    if !backup.is_file() {
        return Ok(RevertReport {
            restored: false,
            message: format!(
                "no original steam_api64.dll backup found at {} — nothing to revert \
                 (Goldberg may never have been installed here)",
                backup.display()
            ),
            dll_path,
        });
    }

    executor.copy_if_changed(&backup, &api).await?;

    Ok(RevertReport {
        restored: true,
        message: format!(
            "restored the original steam_api64.dll from {} — the .orig-steam backup, \
             steam_appid.txt and the Goldberg sub-flag files were left in place, since the \
             next launch through Sabrage/demo.sh reinstalls Goldberg unconditionally",
            backup.display()
        ),
        dll_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::RealExecutor;
    use crate::stages::null_sink;
    use tokio_util::sync::CancellationToken;
    use uuid::Uuid;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sabrage-store-goldberg-test-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn real() -> RealExecutor {
        RealExecutor::new(Uuid::nil(), null_sink(), CancellationToken::new())
    }

    fn plugin_dir(bs_dir: &Path) -> PathBuf {
        bs_dir.join("Beat Saber_Data/Plugins/x86_64")
    }

    #[tokio::test]
    async fn no_backup_reports_restored_false_with_a_says_why_message() {
        let bs_dir = scratch("no-backup");
        let dir = plugin_dir(&bs_dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("steam_api64.dll"), b"GOLDBERG").unwrap();

        let report = revert_original_steam_dll(&real(), &bs_dir).await.unwrap();
        assert!(!report.restored);
        assert!(report.message.contains("nothing to revert"));
        assert!(report.dll_path.ends_with("steam_api64.dll"));
        // The dll is untouched.
        assert_eq!(
            std::fs::read(dir.join("steam_api64.dll")).unwrap(),
            b"GOLDBERG"
        );

        std::fs::remove_dir_all(&bs_dir).unwrap();
    }

    #[tokio::test]
    async fn a_present_backup_is_restored_and_left_in_place() {
        let bs_dir = scratch("with-backup");
        let dir = plugin_dir(&bs_dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("steam_api64.dll"), b"GOLDBERG-BYTES").unwrap();
        std::fs::write(dir.join("steam_api64.dll.orig-steam"), b"REAL-STEAM-BYTES").unwrap();
        // Untouched Goldberg artifacts the revert must leave alone.
        std::fs::write(dir.join("steam_appid.txt"), b"620980").unwrap();
        std::fs::create_dir_all(dir.join("steam_settings")).unwrap();
        std::fs::write(dir.join("steam_settings/offline.txt"), b"").unwrap();

        let report = revert_original_steam_dll(&real(), &bs_dir).await.unwrap();
        assert!(report.restored);
        assert!(report.message.contains("restored"));
        assert!(report.message.contains("next launch"));

        assert_eq!(
            std::fs::read(dir.join("steam_api64.dll")).unwrap(),
            b"REAL-STEAM-BYTES",
            "the live dll now holds the backup's bytes"
        );
        // The backup, appid marker, and steam_settings/ are all still there.
        assert_eq!(
            std::fs::read(dir.join("steam_api64.dll.orig-steam")).unwrap(),
            b"REAL-STEAM-BYTES"
        );
        assert!(dir.join("steam_appid.txt").is_file());
        assert!(dir.join("steam_settings/offline.txt").is_file());

        std::fs::remove_dir_all(&bs_dir).unwrap();
    }

    #[tokio::test]
    async fn reverting_twice_is_idempotent() {
        let bs_dir = scratch("idempotent");
        let dir = plugin_dir(&bs_dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("steam_api64.dll"), b"GOLDBERG-BYTES").unwrap();
        std::fs::write(dir.join("steam_api64.dll.orig-steam"), b"REAL-STEAM-BYTES").unwrap();

        revert_original_steam_dll(&real(), &bs_dir).await.unwrap();
        let second = revert_original_steam_dll(&real(), &bs_dir).await.unwrap();
        assert!(second.restored, "backup still present -> still a restore");
        assert_eq!(
            std::fs::read(dir.join("steam_api64.dll")).unwrap(),
            b"REAL-STEAM-BYTES"
        );

        std::fs::remove_dir_all(&bs_dir).unwrap();
    }

    // The third rule — refusal while a session is live — is not exercised
    // here: faking a `LiveSessionHandle` requires a real CancellationToken-
    // backed handle published through `session::set_live_session`, which
    // races every other test in this binary touching the same global
    // (`session::lock_session_globals` serializes writers, but is
    // `pub(crate)` to the `session` module's own test submodule only). The
    // refusal branch itself is a direct, unconditional
    // `session::live_session().is_some()` check at the top of this function
    // — equivalent in spirit to `store::library`'s tests, which also stop
    // short of faking liveness for the same reason.
}
