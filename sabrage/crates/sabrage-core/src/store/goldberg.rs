//! The revert-original-`steam_api64.dll` action.
//!
//! // DIVERGENCE: `run.sh` has no counterpart — it only ever *installs*
//! Goldberg (`stages::run::actions::goldberg_stage`), never puts the backed-up
//! dll back. This is Sabrage-only, declared in `PARITY.md` ("Revert-
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

use crate::contract::contract;
use crate::error::{Result, SabrageError};
use crate::executor::Executor;
use crate::paths::Paths;
use crate::session;
use crate::stages::run::actions::steam_api_path;
use crate::util::file_sha256_matches;

/// What [`revert_original_steam_dll`] did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevertReport {
    /// `true` iff a `.orig-steam` backup existed, was **not** itself the
    /// pinned Goldberg dll, and was copied back over the live dll. `false`
    /// means there was nothing to revert: no backup at all, or a backup whose
    /// bytes are Goldberg's (see [`revert_original_steam_dll`]'s second
    /// refusal) — `message` says which.
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

/// Is a session live *anywhere* — this process, or another front-end that got
/// as far as writing `session-state.json`?
///
/// [`session::live_session`] only knows about the session this process owns.
/// The `sabrage` CLI writes the state file before it spawns wine
/// ([`crate::session::state`]'s write-before-mutate invariant), so a wine child
/// recorded there and still running (`is_same_process` rules out a recycled
/// pid) is a live session even though this process has no handle for it.
///
/// A corrupt or unreadable state file counts as "live": the conservative
/// answer is the only safe one when the question is "may I overwrite a dll
/// some process may have mapped".
///
/// // NOTE: a `./demo.sh run` session is still invisible here — the shell
/// writes no `session-state.json` at all (that file is Sabrage-only state).
/// Closing that gap needs a process probe for a wine child under `bs_dir`,
/// which is a bigger change than this refusal deserves; the operation lock
/// plus this probe cover both front-ends that Sabrage itself starts.
pub(crate) fn a_session_is_live(session_state_path: &Path) -> bool {
    if session::live_session().is_some() {
        return true;
    }
    match session::state::load(session_state_path) {
        Ok(Some(state)) => state.wine.is_some_and(|w| w.is_same_process()),
        Ok(None) => false,
        Err(_) => true,
    }
}

/// Restore the Steam `steam_api64.dll` this machine had before Goldberg, from
/// its `.orig-steam` backup.
///
/// Two refusals, both of them about not lying to the user:
///
/// * **while a session is live** ([`a_session_is_live`]) — the dll is
///   memory-mapped by a running Beat Saber process, and swapping it out from
///   under a live game is exactly the kind of "mutate the machine
///   mid-session" the operation lock and this check both exist to prevent.
///   The lock is taken *first* and held through the copy, because the
///   interesting window is the one inside a run that has already installed
///   Goldberg but not yet published its live-session handle: only the lock
///   covers that.
/// * **when the backup is itself the Goldberg dll** — `run.sh:147` and
///   [`crate::stages::run::actions::goldberg_stage`] snapshot whatever dll was
///   in place at the first launch, with no way to tell a real Steam library
///   from an install that arrived already Goldberg'd. Copying those bytes back
///   and reporting success would leave the user on Goldberg under an explicit
///   "restored" claim, so this refuses instead.
///
/// Nothing here can prove a backup *is* the real Steam dll — only that it is
/// not the pinned Goldberg one. The success message says "the .orig-steam
/// backup" for that reason, never "the original".
pub async fn revert_original_steam_dll(
    executor: &dyn Executor,
    bs_dir: &Path,
) -> Result<RevertReport> {
    // Built here rather than taken as a parameter, so the one call site (the
    // Tauri command layer) stays a two-argument one. Only `sabrage_appsup` is
    // read below, and `Paths::new` derives that from `$HOME` alone — never
    // from `repo_root` — so the empty root is immaterial.
    let paths = Paths::new(PathBuf::new());
    revert_with_pin(executor, &paths, bs_dir, &contract().deps.gbe_dll_sha256).await
}

/// [`revert_original_steam_dll`] with the Goldberg pin passed in — the
/// testability seam ([`super::library`]'s `validate_pinned` has the same one,
/// for the same reason: no test can fabricate bytes hashing to the real pin).
pub(crate) async fn revert_with_pin(
    executor: &dyn Executor,
    paths: &Paths,
    bs_dir: &Path,
    gbe_dll_sha256: &str,
) -> Result<RevertReport> {
    // Held through the liveness re-check and the copy: a run holds this lock
    // from before its Goldberg step until well after it publishes the live
    // session, so acquiring it here is what closes the check-then-copy window
    // that `live_session()` alone leaves open.
    let _op = crate::stages::acquire_operation_lock().await;

    if a_session_is_live(&paths.session_state_path()) {
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
                "no steam_api64.dll backup found at {} — nothing to revert \
                 (Goldberg may never have been installed here)",
                backup.display()
            ),
            dll_path,
        });
    }

    if file_sha256_matches(&backup, gbe_dll_sha256) {
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

    /// A [`Paths`] whose Sabrage store points **inside the scratch dir**, so a
    /// test never reads (let alone writes) the real
    /// `~/Library/Application Support/Sabrage/session-state.json`.
    fn test_paths(scratch_dir: &Path) -> Paths {
        let mut p = Paths::new(scratch_dir);
        p.sabrage_appsup = scratch_dir.join("Sabrage");
        p
    }

    /// The pin a test's own fixture bytes hash to — what `revert_with_pin`
    /// exists for.
    fn pin_of(bytes: &[u8]) -> String {
        crate::util::sha256_bytes(bytes)
    }

    /// A pin no fixture in this module can ever match.
    const UNMATCHABLE_PIN: &str =
        "0000000000000000000000000000000000000000000000000000000000000000";

    #[tokio::test]
    async fn no_backup_reports_restored_false_with_a_says_why_message() {
        let bs_dir = scratch("no-backup");
        let dir = plugin_dir(&bs_dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("steam_api64.dll"), b"GOLDBERG").unwrap();

        let report = revert_with_pin(&real(), &test_paths(&bs_dir), &bs_dir, UNMATCHABLE_PIN)
            .await
            .unwrap();
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

        let report = revert_with_pin(&real(), &test_paths(&bs_dir), &bs_dir, UNMATCHABLE_PIN)
            .await
            .unwrap();
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

        let paths = test_paths(&bs_dir);
        revert_with_pin(&real(), &paths, &bs_dir, UNMATCHABLE_PIN)
            .await
            .unwrap();
        let second = revert_with_pin(&real(), &paths, &bs_dir, UNMATCHABLE_PIN)
            .await
            .unwrap();
        assert!(second.restored, "backup still present -> still a restore");
        assert_eq!(
            std::fs::read(dir.join("steam_api64.dll")).unwrap(),
            b"REAL-STEAM-BYTES"
        );

        std::fs::remove_dir_all(&bs_dir).unwrap();
    }

    #[tokio::test]
    async fn refuses_when_the_backup_is_itself_the_pinned_goldberg_dll() {
        let bs_dir = scratch("goldberg-backup");
        let dir = plugin_dir(&bs_dir);
        std::fs::create_dir_all(&dir).unwrap();
        // The install arrived already Goldberg'd: the first launch snapshotted
        // Goldberg's own bytes into `.orig-steam` (run.sh:147 / actions.rs).
        std::fs::write(dir.join("steam_api64.dll"), b"GOLDBERG-EMULATOR-BYTES").unwrap();
        std::fs::write(
            dir.join("steam_api64.dll.orig-steam"),
            b"GOLDBERG-EMULATOR-BYTES",
        )
        .unwrap();

        let report = revert_with_pin(
            &real(),
            &test_paths(&bs_dir),
            &bs_dir,
            &pin_of(b"GOLDBERG-EMULATOR-BYTES"),
        )
        .await
        .unwrap();

        assert!(!report.restored, "there is no real Steam dll to restore");
        assert!(
            report.message.contains("itself the Goldberg"),
            "{}",
            report.message
        );
        assert!(
            !report.message.contains("restored the original"),
            "never claim an original was restored: {}",
            report.message
        );
        assert_eq!(
            std::fs::read(dir.join("steam_api64.dll")).unwrap(),
            b"GOLDBERG-EMULATOR-BYTES",
            "the live dll is untouched"
        );

        std::fs::remove_dir_all(&bs_dir).unwrap();
    }

    #[tokio::test]
    async fn the_success_message_never_claims_the_original_was_restored() {
        let bs_dir = scratch("honest-message");
        let dir = plugin_dir(&bs_dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("steam_api64.dll"), b"GOLDBERG-BYTES").unwrap();
        std::fs::write(dir.join("steam_api64.dll.orig-steam"), b"REAL-STEAM-BYTES").unwrap();

        let report = revert_with_pin(&real(), &test_paths(&bs_dir), &bs_dir, UNMATCHABLE_PIN)
            .await
            .unwrap();
        assert!(report.restored);
        assert!(
            report.message.contains(".orig-steam backup"),
            "{}",
            report.message
        );
        assert!(
            !report.message.contains("original"),
            "nothing here can prove the backup is the original: {}",
            report.message
        );

        std::fs::remove_dir_all(&bs_dir).unwrap();
    }

    #[tokio::test]
    async fn refuses_while_a_persisted_session_records_a_live_wine_child() {
        let bs_dir = scratch("persisted-session");
        let dir = plugin_dir(&bs_dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("steam_api64.dll"), b"GOLDBERG-BYTES").unwrap();
        std::fs::write(dir.join("steam_api64.dll.orig-steam"), b"REAL-STEAM-BYTES").unwrap();

        // A session-state.json whose "wine child" is *this* test process:
        // alive, and its start time matches, so `is_same_process` holds — the
        // shape a `sabrage` CLI session leaves behind while it runs.
        let paths = test_paths(&bs_dir);
        let state_path = paths.session_state_path();
        std::fs::create_dir_all(state_path.parent().unwrap()).unwrap();
        let me = crate::process::ProcInfo::observe(std::process::id())
            .expect("this process is observable");
        let mut state = crate::session::state::SessionState::new(
            uuid::Uuid::new_v4(),
            "TestBottle",
            &bs_dir,
            bs_dir.join("run.log"),
            1,
        );
        state.wine = Some(me);
        std::fs::write(&state_path, serde_json::to_vec_pretty(&state).unwrap()).unwrap();

        let err = revert_with_pin(&real(), &paths, &bs_dir, UNMATCHABLE_PIN)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("session is live"), "{err}");
        assert_eq!(
            std::fs::read(dir.join("steam_api64.dll")).unwrap(),
            b"GOLDBERG-BYTES",
            "the live dll is untouched"
        );

        std::fs::remove_dir_all(&bs_dir).unwrap();
    }

    #[tokio::test]
    async fn waits_for_the_operation_lock_then_proceeds() {
        let bs_dir = scratch("operation-lock");
        let dir = plugin_dir(&bs_dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("steam_api64.dll"), b"GOLDBERG-BYTES").unwrap();
        std::fs::write(dir.join("steam_api64.dll.orig-steam"), b"REAL-STEAM-BYTES").unwrap();

        let held = crate::stages::acquire_operation_lock().await;
        let paths = test_paths(&bs_dir);
        let task = {
            let bs_dir = bs_dir.clone();
            tokio::spawn(
                async move { revert_with_pin(&real(), &paths, &bs_dir, UNMATCHABLE_PIN).await },
            )
        };

        // While the lock is held the copy cannot have happened.
        tokio::task::yield_now().await;
        assert_eq!(
            std::fs::read(dir.join("steam_api64.dll")).unwrap(),
            b"GOLDBERG-BYTES",
            "revert must not touch the dll while another operation holds the lock"
        );

        drop(held);
        let report = task.await.unwrap().unwrap();
        assert!(report.restored);
        assert_eq!(
            std::fs::read(dir.join("steam_api64.dll")).unwrap(),
            b"REAL-STEAM-BYTES"
        );

        std::fs::remove_dir_all(&bs_dir).unwrap();
    }

    // The in-process half of the liveness rule — a `LiveSessionHandle`
    // published through `session::set_live_session` — is still not faked
    // here: that global is shared with every other test in this binary
    // (`session::lock_session_globals`, which serializes its writers, is
    // `pub(crate)` to the `session` module's own test submodule). The
    // persisted half above covers the same branch of `a_session_is_live`, and
    // `waits_for_the_operation_lock_then_proceeds` covers the window that
    // check alone could not close.
}
