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
    // The liveness predicate reads `runtime_status.json` out of this
    // directory too, so it has to be scratch as well or a test would
    // consult the developer's own running session.
    p.oxr_appsup = scratch_dir.join("OXRSys");
    p
}

/// The pin a test's own fixture bytes hash to — what `revert_with_pin`
/// exists for.
fn pin_of(bytes: &[u8]) -> String {
    crate::util::sha256_bytes(bytes)
}

/// A pin no fixture in this module can ever match.
const UNMATCHABLE_PIN: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// An argv needle no process on this machine can match — what every test
/// that is *not* about the running-game refusal passes, so no test's
/// verdict depends on what the developer happens to have running.
const NO_GAME_NEEDLE: &str = "sabrage-no-such-process-needle.exe";

/// [`revert_with_pin`] with the real executor and the never-matching game
/// needle — the shape every test below but one wants.
async fn revert(paths: &Paths, bs_dir: &Path, pin: &str) -> Result<RevertReport> {
    revert_probed(&real(), paths, bs_dir, pin, NO_GAME_NEEDLE).await
}

/// A [`Paths`] whose `gbe_dll` payload exists and holds `bytes` — the
/// checkout's `third_party/gbe/steam_api64.dll`, the second thing a
/// backup is tested against.
fn paths_with_payload(scratch_dir: &Path, bytes: &[u8]) -> Paths {
    let p = test_paths(scratch_dir);
    std::fs::create_dir_all(p.gbe_dll.parent().unwrap()).unwrap();
    std::fs::write(&p.gbe_dll, bytes).unwrap();
    p
}

#[tokio::test]
async fn no_backup_reports_restored_false_with_a_says_why_message() {
    let _g = crate::session::lock_session_globals();
    let bs_dir = scratch("no-backup");
    let dir = plugin_dir(&bs_dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("steam_api64.dll"), b"GOLDBERG").unwrap();

    let report = revert(&test_paths(&bs_dir), &bs_dir, UNMATCHABLE_PIN)
        .await
        .unwrap();
    assert!(!report.restored);
    assert!(report.message.contains("nothing to revert"));
    assert!(report.dll_path.ends_with("steam_api64.dll"));
    assert_eq!(
        std::fs::read(dir.join("steam_api64.dll")).unwrap(),
        b"GOLDBERG"
    );

    std::fs::remove_dir_all(&bs_dir).unwrap();
}

#[tokio::test]
async fn a_present_backup_is_restored_and_left_in_place() {
    let _g = crate::session::lock_session_globals();
    let bs_dir = scratch("with-backup");
    let dir = plugin_dir(&bs_dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("steam_api64.dll"), b"GOLDBERG-BYTES").unwrap();
    std::fs::write(dir.join("steam_api64.dll.orig-steam"), b"REAL-STEAM-BYTES").unwrap();
    // Untouched Goldberg artifacts the revert must leave alone.
    std::fs::write(dir.join("steam_appid.txt"), b"620980").unwrap();
    std::fs::create_dir_all(dir.join("steam_settings")).unwrap();
    std::fs::write(dir.join("steam_settings/offline.txt"), b"").unwrap();

    let report = revert(&test_paths(&bs_dir), &bs_dir, UNMATCHABLE_PIN)
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
    let _g = crate::session::lock_session_globals();
    let bs_dir = scratch("idempotent");
    let dir = plugin_dir(&bs_dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("steam_api64.dll"), b"GOLDBERG-BYTES").unwrap();
    std::fs::write(dir.join("steam_api64.dll.orig-steam"), b"REAL-STEAM-BYTES").unwrap();

    let paths = test_paths(&bs_dir);
    revert(&paths, &bs_dir, UNMATCHABLE_PIN).await.unwrap();
    let second = revert(&paths, &bs_dir, UNMATCHABLE_PIN).await.unwrap();
    assert!(second.restored, "backup still present -> still a restore");
    assert_eq!(
        std::fs::read(dir.join("steam_api64.dll")).unwrap(),
        b"REAL-STEAM-BYTES"
    );

    std::fs::remove_dir_all(&bs_dir).unwrap();
}

#[tokio::test]
async fn refuses_when_the_backup_is_itself_the_pinned_goldberg_dll() {
    let _g = crate::session::lock_session_globals();
    let bs_dir = scratch("goldberg-backup");
    let dir = plugin_dir(&bs_dir);
    std::fs::create_dir_all(&dir).unwrap();
    // The install arrived already Goldberg'd: the first launch snapshotted
    // Goldberg's own bytes into `.orig-steam`.
    std::fs::write(dir.join("steam_api64.dll"), b"GOLDBERG-EMULATOR-BYTES").unwrap();
    std::fs::write(
        dir.join("steam_api64.dll.orig-steam"),
        b"GOLDBERG-EMULATOR-BYTES",
    )
    .unwrap();

    let report = revert(
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

/// A13a-1 / A7-3: `run`/`run.sh` install whatever
/// `third_party/gbe/steam_api64.dll` holds and only *warn* when it does
/// not match the contract pin, so an install that arrived already
/// Goldberg'd with a non-pinned build gets those bytes snapshotted into
/// `.orig-steam`. A pin-only test called that backup "the original".
#[tokio::test]
async fn refuses_when_the_backup_is_an_unpinned_goldberg_build() {
    let _g = crate::session::lock_session_globals();
    let bs_dir = scratch("unpinned-goldberg-backup");
    let dir = plugin_dir(&bs_dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("steam_api64.dll"), b"CUSTOM-GOLDBERG-BUILD").unwrap();
    std::fs::write(
        dir.join("steam_api64.dll.orig-steam"),
        b"CUSTOM-GOLDBERG-BUILD",
    )
    .unwrap();

    // The pin is some *other* build entirely — the state after a contract
    // pin bump, or a hand-swapped payload.
    let paths = paths_with_payload(&bs_dir, b"CUSTOM-GOLDBERG-BUILD");
    let report = revert(&paths, &bs_dir, UNMATCHABLE_PIN).await.unwrap();

    assert!(!report.restored, "there is no real Steam dll to restore");
    assert!(
        report.message.contains("itself the Goldberg"),
        "{}",
        report.message
    );
    assert!(
        !report.message.contains("original"),
        "never claim an original was restored: {}",
        report.message
    );
    assert_eq!(
        std::fs::read(dir.join("steam_api64.dll")).unwrap(),
        b"CUSTOM-GOLDBERG-BUILD",
        "the live dll is untouched"
    );

    std::fs::remove_dir_all(&bs_dir).unwrap();
}

/// A7-3 / A13a-1, the half the bytes cannot carry: the launch that minted
/// this `.orig-steam` saw the live dll was already Goldberg and recorded
/// it ([`crate::stages::run::actions::goldberg_backup_marker`]). The
/// payload has since changed — a pin bump, a re-downloaded
/// `third_party/gbe`, a moved repo root — so neither hash test recognises
/// those bytes any more, and only the record stops them being copied back
/// under a "restored" claim.
#[tokio::test]
async fn refuses_when_a_launch_recorded_the_backup_as_goldberg() {
    let _g = crate::session::lock_session_globals();
    let bs_dir = scratch("recorded-goldberg-backup");
    let dir = plugin_dir(&bs_dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("steam_api64.dll"), b"GOLDBERG-NOW").unwrap();
    let backup = dir.join("steam_api64.dll.orig-steam");
    std::fs::write(&backup, b"GOLDBERG-BACK-THEN").unwrap();

    // Today's payload is a different build entirely, so `cmp_files` and
    // the pin both say "not Goldberg" about the backup.
    let paths = paths_with_payload(&bs_dir, b"GOLDBERG-NOW");
    let marker = crate::stages::run::actions::goldberg_backup_marker(&paths, &backup);
    std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
    std::fs::write(&marker, format!("{}\n", backup.display())).unwrap();

    let report = revert(&paths, &bs_dir, UNMATCHABLE_PIN).await.unwrap();

    assert!(!report.restored, "there is no real Steam dll to restore");
    assert!(
        report.message.contains("itself the Goldberg"),
        "{}",
        report.message
    );
    assert_eq!(
        std::fs::read(dir.join("steam_api64.dll")).unwrap(),
        b"GOLDBERG-NOW",
        "the live dll is untouched"
    );
    assert_eq!(
        std::fs::read(&backup).unwrap(),
        b"GOLDBERG-BACK-THEN",
        "and so is the backup"
    );

    std::fs::remove_dir_all(&bs_dir).unwrap();
}

/// A backup that matches neither Goldberg is still restored — the payload
/// test must not swallow the ordinary case.
#[tokio::test]
async fn a_backup_unlike_the_payload_is_still_restored() {
    let _g = crate::session::lock_session_globals();
    let bs_dir = scratch("payload-unlike-backup");
    let dir = plugin_dir(&bs_dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("steam_api64.dll"), b"CUSTOM-GOLDBERG-BUILD").unwrap();
    std::fs::write(dir.join("steam_api64.dll.orig-steam"), b"REAL-STEAM-BYTES").unwrap();

    let paths = paths_with_payload(&bs_dir, b"CUSTOM-GOLDBERG-BUILD");
    let report = revert(&paths, &bs_dir, UNMATCHABLE_PIN).await.unwrap();

    assert!(report.restored);
    assert_eq!(
        std::fs::read(dir.join("steam_api64.dll")).unwrap(),
        b"REAL-STEAM-BYTES"
    );

    std::fs::remove_dir_all(&bs_dir).unwrap();
}

/// A13a-2: the pre-telemetry window. A `./demo.sh run` writes no session
/// record and its `runtime_status.json` does not exist until streaming
/// starts, so `live_session_reason` says "idle" while the game is up. The
/// argv probe is the signal that closes it — driven here by a needle
/// matching *this test binary's own* command line, the same trick
/// `stages::stop`'s process tests use, so a real scan is exercised without
/// starting a game.
#[tokio::test]
async fn refuses_while_a_matching_game_process_is_running() {
    let _g = crate::session::lock_session_globals();
    let bs_dir = scratch("running-game");
    let dir = plugin_dir(&bs_dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("steam_api64.dll"), b"GOLDBERG-BYTES").unwrap();
    std::fs::write(dir.join("steam_api64.dll.orig-steam"), b"REAL-STEAM-BYTES").unwrap();

    let paths = test_paths(&bs_dir);

    let exe = std::env::current_exe().expect("test binary path");
    let name = exe
        .file_name()
        .and_then(|n| n.to_str())
        .expect("utf8 test binary name");
    let needle = &name[name.len().saturating_sub(6)..];

    let err = revert_probed(&real(), &paths, &bs_dir, UNMATCHABLE_PIN, needle)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("session is live"), "{err}");
    assert!(err.to_string().contains("Beat Saber is running"), "{err}");
    assert_eq!(
        std::fs::read(dir.join("steam_api64.dll")).unwrap(),
        b"GOLDBERG-BYTES",
        "the dll the running game has mapped is untouched"
    );

    std::fs::remove_dir_all(&bs_dir).unwrap();
}

#[tokio::test]
async fn the_success_message_never_claims_the_original_was_restored() {
    let _g = crate::session::lock_session_globals();
    let bs_dir = scratch("honest-message");
    let dir = plugin_dir(&bs_dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("steam_api64.dll"), b"GOLDBERG-BYTES").unwrap();
    std::fs::write(dir.join("steam_api64.dll.orig-steam"), b"REAL-STEAM-BYTES").unwrap();

    let report = revert(&test_paths(&bs_dir), &bs_dir, UNMATCHABLE_PIN)
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

/// The regression this door was open for: a `./demo.sh run` session writes
/// no `session-state.json` and publishes no handle, so the old local
/// predicate said "idle" and the revert replaced the `steam_api64.dll` the
/// running game has mapped. The fresh `runtime_status.json` the oxrsys
/// runtime keeps is the signal that closes it.
#[tokio::test]
async fn refuses_while_only_the_runtime_reports_a_live_session() {
    let _g = crate::session::lock_session_globals();
    let bs_dir = scratch("runtime-status-session");
    let dir = plugin_dir(&bs_dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("steam_api64.dll"), b"GOLDBERG-BYTES").unwrap();
    std::fs::write(dir.join("steam_api64.dll.orig-steam"), b"REAL-STEAM-BYTES").unwrap();

    let paths = test_paths(&bs_dir);
    std::fs::create_dir_all(&paths.oxr_appsup).unwrap();
    let now = crate::session::now_unix_ms();
    // Both halves of `watcher::runtime_status_live`: a fresh stamp *and* a
    // `process_id` that is still alive (this test process stands in for the
    // runtime). A status naming no live process is a file neither this door
    // nor the Session screen's `External` phase will vouch for.
    let pid = std::process::id();
    std::fs::write(
        paths.oxr_appsup.join("runtime_status.json"),
        format!(r#"{{"state":"streaming","process_id":{pid},"updated_at_unix_ms":{now}}}"#),
    )
    .unwrap();

    let err = revert(&paths, &bs_dir, UNMATCHABLE_PIN).await.unwrap_err();
    assert!(err.to_string().contains("session is live"), "{err}");
    assert!(err.to_string().contains("streaming"), "{err}");
    assert_eq!(
        std::fs::read(dir.join("steam_api64.dll")).unwrap(),
        b"GOLDBERG-BYTES",
        "the dll the live game has mapped is untouched"
    );

    std::fs::remove_dir_all(&bs_dir).unwrap();
}

#[tokio::test]
async fn refuses_while_a_persisted_session_records_a_live_wine_child() {
    let _g = crate::session::lock_session_globals();
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
    let me =
        crate::process::ProcInfo::observe(std::process::id()).expect("this process is observable");
    let mut state = crate::session::state::SessionState::new(
        uuid::Uuid::new_v4(),
        "TestBottle",
        &bs_dir,
        bs_dir.join("run.log"),
        1,
    );
    state.wine = Some(me);
    std::fs::write(&state_path, serde_json::to_vec_pretty(&state).unwrap()).unwrap();

    let err = revert(&paths, &bs_dir, UNMATCHABLE_PIN).await.unwrap_err();
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
    let _g = crate::session::lock_session_globals();
    let bs_dir = scratch("operation-lock");
    let dir = plugin_dir(&bs_dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("steam_api64.dll"), b"GOLDBERG-BYTES").unwrap();
    std::fs::write(dir.join("steam_api64.dll.orig-steam"), b"REAL-STEAM-BYTES").unwrap();

    let held = crate::stages::acquire_operation_lock().await;
    let paths = test_paths(&bs_dir);
    let task = {
        let bs_dir = bs_dir.clone();
        tokio::spawn(async move { revert(&paths, &bs_dir, UNMATCHABLE_PIN).await })
    };

    // Give the spawned revert a chance to reach the lock and block there.
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

// Every test above holds `session::lock_session_globals()`: a
// `RunPhaseScope` alive on another harness thread otherwise makes these
// reverts fail with "a launch for bottle 'Steam' is in progress". The
// guard deliberately does not reset `LIVE_SESSION` (other modules set it
// without holding the guard), so no `LiveSessionHandle` is faked here;
// tests::refuses_while_a_persisted_session_records_a_live_wine_child
// exercises the same refusal branch of `live_session_reason`, and
// tests::waits_for_the_operation_lock_then_proceeds the window that check
// alone cannot close.
