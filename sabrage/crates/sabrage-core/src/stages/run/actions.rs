//! The seven contract-ordered launch actions ([`LAUNCH_ACTION_IDS`]) —
//! unconditional preparation steps, not checks: no pass/fail, no remedy, no gate.
//! `audio-route` and `dashboard` are acquired in [`super::guards`] instead,
//! because acquiring them is the same act as arming their undo. See
//! tests::{the_guarded_actions_are_listed_and_launch_is_last,
//! one_step_id_per_action_plus_the_three_run_only_phases}.
//!
//! Reference: scripts/demo/run.sh, the `# launch-action:` tags.
//!
//! Every mutation goes through [`crate::executor::Executor`], so `--dry-run` plans
//! it; read-only probes run in both modes, keeping the plan accurate.

use std::path::{Path, PathBuf};

use crate::contract::contract;
use crate::error::{Result, SabrageError};
use crate::events::{step, StageEvent, StepId};
use crate::executor::{DetachedChild, DetachedStdio};
use crate::paths::Bottle;
use crate::process::{self, ChildSpec, ProcInfo};
use crate::session::state::{self, SessionState, WiredForward};
use crate::stages::{StageCtx, RUN_WINESERVER_WAIT};
use crate::{logs, util};

use super::PreflightFacts;

/// run.sh's `# launch-action:` ids, in execution order.
///
/// Must equal `contract().launch_actions`' ids, in order — asserted by
/// `sabrage-parity`'s
/// `launch_action_ids_equal_the_contracts_order_and_there_are_exactly_seven`.
pub const LAUNCH_ACTION_IDS: [&str; 7] = [
    "adb-forward-hygiene",
    "wineserver-reset",
    "goldberg-stage",
    "audio-route",
    "dashboard",
    "adb-reverse-cleanup",
    "launch-wine",
];

/// How many `beatsaber-<ts>[-n].log` names to try before giving up and letting
/// the spawn surface the `EEXIST` itself.
///
/// A collision needs two launches inside the same wall-clock second; Sabrage
/// holds [`crate::stages::OPERATION_LOCK`] through the spawn, so the only
/// racer is a `./demo.sh run` in the same second. 100 is "cannot happen"
/// padding, not a budget.
const LOG_NAME_ATTEMPTS: u32 = 100;

/// The device name run.sh routes the Mac's output to.
pub(crate) const BLACKHOLE_DEVICE: &str = "BlackHole 2ch";

/// The first serial in `adb devices`' stdout whose state is exactly `device`.
///
/// Mirrors run.sh's `awk 'NR>1 && $2=="device"{print $1; exit}'`: the header row
/// is skipped, and so is any row that is blank or in another state
/// (`unauthorized`, `offline`). See
/// tests::first_device_serial_matches_the_awk_program.
pub(crate) fn first_device_serial(stdout: &str) -> Option<String> {
    stdout.lines().skip(1).find_map(|line| {
        let mut fields = line.split_whitespace();
        let serial = fields.next()?;
        let state = fields.next()?;
        (state == "device").then(|| serial.to_string())
    })
}

/// The first `device`-state serial from `adb devices`, or `None`.
///
/// A read-only probe: bypasses the executor and runs under `--dry-run` too.
/// Carries the launch's cancellation token because a wedged `adb` here would
/// otherwise hold the operation lock with Cancel unable to interrupt it. A
/// cancelled or timed-out probe returns `None`, matching run.sh's empty
/// `$WIRED_SER`. See tests::a_cancel_during_the_device_probe_is_a_cancellation.
async fn probe_device_serial(ctx: &StageCtx, adb: &Path, step_id: StepId) -> Option<String> {
    let spec = ctx
        .child(adb.to_path_buf(), step_id)
        .arg("devices")
        .env_path(process::default_child_path());
    let out = process::capture_with(&spec, &ctx.cancel, process::DEFAULT_PROBE_TIMEOUT)
        .await
        .ok()?;
    first_device_serial(&out.stdout)
}

/// `tcp:<port>` for each of the contract's `ports.stream` — never a literal
/// `"tcp:9943"` (PARITY.md § Invariants that must NOT change (byte/behavior
/// parity), "adb `forward --remove` per-serial").
fn stream_forward_specs() -> Vec<String> {
    contract()
        .ports
        .stream
        .iter()
        .map(|p| format!("tcp:{p}"))
        .collect()
}

/// Remove both stream forwards, never aborting on a failed removal, then bring
/// the persisted record back in line per port: a removal that succeeded drops its
/// record, a removal that failed keeps it.
///
/// A failed `--remove` is indeterminate — usually the device is gone and the
/// forward with it, but it may equally be a transient adb failure over a
/// still-installed `tcp:9943` — and the record is the only thing that would ever
/// retry it; [`crate::session::reconcile`]'s `restore_forwards` makes the same
/// distinction. The removals run on a fresh, non-cancelled executor
/// ([`super::teardown_ctx`]) because the usual reason to be rolling back is a
/// cancelled launch, whose executor refuses every child and every write. The save
/// is best-effort. See
/// tests::{a_rollback_whose_removal_fails_keeps_the_forward_on_record,
/// a_cancellation_mid_loop_still_rolls_the_first_forward_back}.
async fn rollback_forwards(
    ctx: &StageCtx,
    adb: &Path,
    serial: &str,
    specs: &[String],
    sess: &mut SessionState,
    state_path: &Path,
) {
    let rb = super::teardown_ctx(ctx);
    let mut removed: Vec<u16> = Vec::new();
    for (port, q) in contract().ports.stream.iter().zip(specs) {
        let undo = rb
            .child(adb.to_path_buf(), step::RUN_ADB_FORWARDS)
            .arg("-s")
            .arg(serial)
            .arg("forward")
            .arg("--remove")
            .arg(q);
        if matches!(rb.executor.run_child(&undo).await, Ok(status) if status.success()) {
            removed.push(*port);
        }
    }
    sess.wired_forwards
        .retain(|f| !(f.serial == serial && removed.contains(&f.port)));
    let _ = state::save(&*rb.executor, state_path, sess).await;
}

/// `launch-action: adb-forward-hygiene`. Reference: scripts/demo/run.sh.
///
/// `--wired`: create `tcp:9943` and `tcp:9944` on the first device whose state is
/// exactly `device`; if either fails, remove both and die. Otherwise remove exactly
/// those two local ports per serial — never `--remove-all`, which would delete
/// forwards this pipeline knows nothing about (PARITY.md § Invariants that must NOT
/// change (byte/behavior parity), "adb `forward --remove` per-serial").
///
/// Each intended forward is persisted to `state_path` before the `adb forward` that
/// creates it: an over-recorded forward costs one harmless `--remove` that finds
/// nothing, where an under-recorded one is the stale forward that silently breaks
/// the next WiFi run. See
/// tests::{wired_plans_both_forwards_and_reports_them,
/// a_failed_forward_removes_both_ports_and_dies_with_run_shs_text}.
pub async fn adb_forward_hygiene(
    ctx: &StageCtx,
    sess: &mut SessionState,
    state_path: &Path,
) -> Result<()> {
    let st = ctx.step(step::RUN_ADB_FORWARDS);
    let adb = ctx.paths.adb.clone();

    if !ctx.opts.wired {
        // `_at`, not the plain fix: these rows must be attributed to the run
        // stage's own step, not to the doctor fix list's `fix.remove-adb-forwards`.
        // See tests::the_non_wired_forward_cleanup_is_stamped_with_the_run_stages_step.
        if adb.is_some() {
            crate::fixes::adb::remove_adb_forwards_at(ctx, &ctx.sink, step::RUN_ADB_FORWARDS)
                .await?;
        }
        return Ok(());
    }

    let Some(adb) = adb else {
        return Err(st.fatal(
            "--wired needs adb (Android platform-tools) on PATH or under ~/Library/Android/sdk",
            None,
        ));
    };
    let serial = match probe_device_serial(ctx, &adb, step::RUN_ADB_FORWARDS).await {
        Some(serial) => serial,
        // Stop landing on the probe is a cancellation, not a verdict about the
        // device — saying "no Quest over adb" would be inventing one.
        None if ctx.cancel.is_cancelled() => return Err(SabrageError::Cancelled),
        None => {
            return Err(st.fatal(
                "--wired: no Quest over adb — connect USB and check 'adb devices'",
                None,
            ))
        }
    };

    let specs = stream_forward_specs();
    for (port, local) in contract().ports.stream.iter().zip(&specs) {
        // The record goes down FIRST — before the `adb forward` that could
        // crash mid-flight, exactly as `AudioGuard::acquire` writes its
        // `prev_audio_output` before switching the device.
        sess.wired_forwards.push(WiredForward {
            serial: serial.clone(),
            port: *port,
        });
        // The save leaves through the same door as a failed forward below: a
        // Stop (or a disk error) landing here, after the first port is up,
        // must still take that forward back down — returning through `?`
        // would leave it on the device with nothing on disk naming it.
        if let Err(e) = state::save(&*ctx.executor, state_path, sess).await {
            rollback_forwards(ctx, &adb, &serial, &specs, sess, state_path).await;
            return Err(e);
        }

        let spec = ctx
            .child(adb.clone(), step::RUN_ADB_FORWARDS)
            .arg("-s")
            .arg(&serial)
            .arg("forward")
            .arg(local)
            .arg(local);
        // Every non-success leaves through the same door, a failed *exec*
        // included: a cancellation between the two ports would otherwise leave
        // the first forward on the device with nothing on disk naming it.
        let failure = match ctx.executor.run_child(&spec).await {
            Ok(status) if status.success() => None,
            Ok(_) => Some(None),
            Err(e) => Some(Some(e)),
        };
        if let Some(cause) = failure {
            rollback_forwards(ctx, &adb, &serial, &specs, sess, state_path).await;
            let die = wired_forward_failed_die(local, &serial);
            return Err(match cause {
                // A cancelled launch is not a failed one: Stop's own error
                // travels, and no die row is invented for it.
                Some(SabrageError::Cancelled) => SabrageError::Cancelled,
                // The executor's cause first — a spawn failure is stderr the
                // shell would have shown as adb's own.
                Some(e) => die_with_cause(ctx, step::RUN_ADB_FORWARDS, e, &die),
                None => st.fatal(die, None),
            });
        }
    }
    st.info(wired_forwards_up_line(&specs, &serial));
    Ok(())
}

/// The die run.sh prints when one port's `adb forward` fails on one device.
/// `pub` (A1-3) so `sabrage-parity` can pin it against run.sh by calling the
/// real renderer instead of copying the sentence.
pub fn wired_forward_failed_die(local: &str, serial: &str) -> String {
    format!(
        "adb forward {local} {local} failed on {serial} — check the USB connection \
         (adb devices)"
    )
}

/// The `wired mode: adb forward … up` info line; the caller passes the contract's
/// `ports.stream` specs, so the shell string's `tcp:9943/tcp:9944` cannot drift.
/// `pub` (A1-3), same reason as [`wired_forward_failed_die`].
pub fn wired_forwards_up_line(specs: &[String], serial: &str) -> String {
    format!(
        "wired mode: adb forward {} up on {serial} (a later non-wired run clears these two)",
        specs.join("/")
    )
}

/// `"<pid> <exe-basename>"` per process, space-joined with a trailing space —
/// the shape of `pgrep -lf wineserver | tr '\n' ' '`, and the same rendering
/// `stop`'s private `format_survivors` produces.
///
/// `fallback` names a process whose exe path has no file name. (PARITY.md § Stop,
/// "The survivor warning lists".) See
/// tests::survivors_render_as_pid_basename_pairs_with_a_trailing_space.
fn format_survivors(procs: &[ProcInfo], fallback: &str) -> String {
    let mut out = String::new();
    for p in procs {
        let name = p
            .exe
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(fallback);
        out.push_str(&p.pid.to_string());
        out.push(' ');
        out.push_str(name);
        out.push(' ');
    }
    out
}

/// `launch-action: wineserver-reset`. Reference: scripts/demo/run.sh.
///
/// `wineserver -k` (failure ignored), then a bounded `-w` wait of
/// [`crate::stages::RUN_WINESERVER_WAIT`] — fatal on timeout, unlike `stop`'s 4 s
/// advisory wait. The timeout path warns with the survivor list
/// ([`wineserver_still_alive_warn`] over
/// [`crate::process::find_processes_by_cmdline`]) before dying. See
/// tests::wineserver_reset_plans_k_then_w_and_reports_down.
pub async fn wineserver_reset(ctx: &StageCtx, bottle: &Bottle) -> Result<()> {
    let st = ctx.step(step::RUN_WINESERVER);
    ctx.section(format!("resetting wineserver for bottle '{}'", bottle.name));

    // No CrossOver means `$WINESERVER` is empty: the shell "runs" it, swallows
    // the command-not-found and reaches `ok` anyway. The `run.wine-exec`
    // preflight has already died by then, so this branch is reproduced rather
    // than special-cased. See tests::wineserver_reset_without_crossover_still_reports_down.
    let Some(wineserver) = ctx.paths.wineserver.clone() else {
        st.ok("wineserver down");
        return Ok(());
    };
    let prefix = bottle.prefix.to_string_lossy().into_owned();

    let kill = ctx
        .child(wineserver.clone(), step::RUN_WINESERVER)
        .arg("-k")
        .env("WINEPREFIX", prefix.clone());
    let _ = ctx.executor.run_child(&kill).await;

    let wait = ctx
        .child(wineserver, step::RUN_WINESERVER)
        .arg("-w")
        .env("WINEPREFIX", prefix);
    if tokio::time::timeout(RUN_WINESERVER_WAIT, ctx.executor.run_child(&wait))
        .await
        .is_err()
    {
        st.warn(wineserver_still_alive_warn(&format_survivors(
            &process::find_processes_by_cmdline("wineserver"),
            "wineserver",
        )));
        return Err(st.fatal(WINESERVER_MANUAL_KILL_DIE, None));
    }
    st.ok("wineserver down");
    Ok(())
}

/// `"$API.orig-steam"` — a suffix on the whole file name, not an extension swap.
fn orig_steam_path(api: &Path) -> PathBuf {
    let mut s = api.as_os_str().to_os_string();
    s.push(".orig-steam");
    PathBuf::from(s)
}

/// Where the "the `.orig-steam` minted here is itself a Goldberg build" record
/// lives: `<sabrage_appsup>/goldberg-provenance/<sha256 of the backup path>`.
///
/// Under `~/Library/Application Support/Sabrage/` on purpose (CLAUDE.md: GUI
/// state lives there). demo.sh writes no such file, so putting it next to the
/// game's dlls would add a Sabrage-only artifact to every install — a
/// permanent on-disk divergence for a bit of Sabrage bookkeeping. The path is
/// hashed because the record is keyed by an absolute path with slashes in it.
pub(crate) fn goldberg_backup_marker(paths: &crate::paths::Paths, backup: &Path) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    paths
        .sabrage_appsup
        .join("goldberg-provenance")
        .join(format!(
            "{}.orig-steam-is-goldberg",
            util::sha256_bytes(backup.as_os_str().as_bytes())
        ))
}

/// Did the launch that minted `backup` find the live dll already Goldberg?
///
/// The provenance [`goldberg_stage`] cannot otherwise leave behind: its
/// `already_goldberg` comparison is against the **configured** `gbe_dll`, so it
/// recognises an older or third-party Goldberg build that no pin matches —
/// which is exactly the backup a pin-only revert guard would copy back and call
/// a restore (A7-3 / A13a-1). `false` means "no record", not "provably Steam":
/// nothing on this machine can prove the latter.
pub(crate) fn goldberg_backup_is_goldberg(paths: &crate::paths::Paths, backup: &Path) -> bool {
    goldberg_backup_marker(paths, backup).is_file()
}

/// Write that record. Best-effort on purpose: run.sh has no such step, so a
/// failure to journal Sabrage's own bookkeeping must not turn a launch the
/// shell would complete into a die. The `warn` row is emitted either way.
async fn record_goldberg_backup(ctx: &StageCtx, backup: &Path) {
    let marker = goldberg_backup_marker(&ctx.paths, backup);
    let Some(dir) = marker.parent() else { return };
    if ctx.executor.create_dir_all(dir).await.is_err() {
        return;
    }
    let _ = ctx
        .executor
        .write_atomic(&marker, format!("{}\n", backup.display()).as_bytes())
        .await;
}

/// `$BS_DIR/Beat Saber_Data/Plugins/x86_64/steam_api64.dll`, else
/// `$BS_DIR/steam_api64.dll` — the second is returned even when it is absent, so
/// the caller produces [`steam_api_missing_die`] for it.
pub(crate) fn steam_api_path(bs_dir: &Path) -> PathBuf {
    let plugin = bs_dir.join("Beat Saber_Data/Plugins/x86_64/steam_api64.dll");
    if plugin.is_file() {
        plugin
    } else {
        bs_dir.join("steam_api64.dll")
    }
}

/// The three `steam_settings/` flag files, truncate-created empty (`: >`).
pub(crate) const GOLDBERG_FLAG_FILES: [&str; 3] = [
    "offline.txt",
    "disable_networking.txt",
    "disable_overlay.txt",
];

/// The `wineserver still alive after <n>s` warn; the survivor list is
/// [`format_survivors`]' rendering.
/// `pub` (A1-3), same reason as [`wired_forward_failed_die`].
pub fn wineserver_still_alive_warn(survivors: &str) -> String {
    format!(
        "wineserver still alive after {}s: {survivors}",
        RUN_WINESERVER_WAIT.as_secs()
    )
}

/// The die run.sh prints after the survivor warn, verbatim
/// (scripts/demo/run.sh # launch-action: wineserver-reset). `pub` (A1-3).
pub const WINESERVER_MANUAL_KILL_DIE: &str = "kill the listed wineserver(s) manually, then re-run";

/// The die run.sh prints when no `steam_api64.dll` exists under `$BS_DIR`
/// (scripts/demo/run.sh # launch-action: goldberg-stage). `pub` (A1-3).
pub fn steam_api_missing_die(bs_dir: &Path) -> String {
    format!(
        "steam_api64.dll not found under {} — is this a complete Beat Saber install?",
        bs_dir.display()
    )
}

/// run.sh:148's `info`, verbatim. `pub` (A1-3).
pub const GOLDBERG_ALREADY_INSTALLED: &str = "goldberg already installed";

/// run.sh:149's `ok`. `pub` (A1-3).
pub fn goldberg_installed_line(api: &Path) -> String {
    format!("installed goldberg -> {}", api.display())
}

/// `launch-action: goldberg-stage`. Reference: scripts/demo/run.sh.
///
/// Stages four byte-exact artifacts: `steam_api64.dll` (under
/// `Beat Saber_Data/Plugins/x86_64/` or at the game root); `<api>.orig-steam`,
/// created once and never overwritten (the only copy of the real Steam dll on the
/// machine); the Goldberg dll copied over the live dll when bytes differ, hash
/// mismatch tolerated unlike in setup (parity decision 20); and `steam_appid.txt`
/// (appid digits, no trailing newline) plus the three `steam_settings/` flag files.
///
/// Fatal when no `steam_api64.dll` exists, when `.orig-steam` is not a regular
/// file, or when any copy/write fails. See
/// tests::{goldberg_backs_up_once_installs_and_writes_the_exact_artifacts,
/// goldberg_refuses_when_the_backup_name_is_not_a_regular_file}.
pub async fn goldberg_stage(ctx: &StageCtx) -> Result<()> {
    let st = ctx.step(step::RUN_GOLDBERG);
    ctx.section("Goldberg");

    let api = steam_api_path(&ctx.bs_dir);
    if !api.is_file() {
        return Err(st.fatal(steam_api_missing_die(&ctx.bs_dir), None));
    }
    let api_dir = api
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    // Made once, never refreshed: overwriting it with an already-Goldberged dll
    // would destroy the only copy of the real Steam library on this machine. See
    // tests::goldberg_installs_the_dll_and_flag_files_and_never_refreshes_the_backup.
    let backup = orig_steam_path(&api);
    if backup.is_file() {
        // The ordinary post-first-launch state: a usable backup is already
        // there (a symlink resolving to a regular file counts — the copy would
        // read through it just the same).
    } else if std::fs::symlink_metadata(&backup).is_ok() {
        // Sabrage-only refusal: run.sh's `-f` fails on non-regular names too,
        // so `[ ! -f "$API.orig-steam" ]` fires, `cp` writes into the directory
        // or through the symlink, and Goldberg goes over the live dll with no
        // usable backup (PARITY.md § Declared by the 2026-08-30 adversarial
        // review (round 1 fixes), "A non-regular `.orig-steam` refuses the
        // launch"). See tests::goldberg_refuses_when_the_backup_name_is_not_a_regular_file.
        return Err(st.fatal(
            format!(
                "{} is not a regular file — it is the reserved name for the original \
                 steam_api64.dll, and Sabrage will not install Goldberg over the live dll \
                 without a usable backup",
                backup.display()
            ),
            Some(format!("move or delete {}", backup.display())),
        ));
    } else {
        // Sabrage-only row (run.sh is silent here): the live dll is ALREADY the
        // Goldberg build, so the backup this line mints holds Goldberg, not
        // Steam — and `store::goldberg::revert_original_steam_dll` would
        // otherwise copy these bytes back and call it a restore.
        let already_goldberg = util::cmp_files(&ctx.paths.gbe_dll, &api);
        if let Err(e) = ctx.executor.copy_if_changed(&api, &backup).await {
            return Err(die_with_cause(
                ctx,
                step::RUN_GOLDBERG,
                e,
                "backup of original steam_api64.dll failed",
            ));
        }
        if already_goldberg {
            record_goldberg_backup(ctx, &backup).await;
            st.warn(format!(
                "steam_api64.dll was already the Goldberg build, so {} is a copy of \
                 Goldberg — the real Steam dll was never seen here and cannot be restored",
                backup.display()
            ));
        }
    }

    // `cmp -s`, not a hash: run tolerates a Goldberg dll that does not match
    // setup's pin (parity decision 20).
    if util::cmp_files(&ctx.paths.gbe_dll, &api) {
        st.info(GOLDBERG_ALREADY_INSTALLED);
    } else {
        if let Err(e) = ctx.executor.copy_if_changed(&ctx.paths.gbe_dll, &api).await {
            return Err(die_with_cause(
                ctx,
                step::RUN_GOLDBERG,
                e,
                "goldberg install failed",
            ));
        }
        st.ok(goldberg_installed_line(&api));
    }

    // `printf '%s' "$BS_APPID"`: the digits, no trailing newline.
    let appid = contract().game.appid.to_string();
    if let Err(e) = ctx
        .executor
        .write_atomic(&api_dir.join("steam_appid.txt"), appid.as_bytes())
        .await
    {
        return Err(die_with_cause(
            ctx,
            step::RUN_GOLDBERG,
            e,
            "writing steam_appid.txt failed",
        ));
    }

    // `mkdir -p` then three `: >` truncate-creates. The shell has no `|| die` on
    // either; a failure here is propagated rather than swallowed
    // (docs/design/design-core.md §6.6: no silent aborts, no silent successes).
    let gset = api_dir.join("steam_settings");
    ctx.executor.create_dir_all(&gset).await?;
    for name in GOLDBERG_FLAG_FILES {
        ctx.executor.write_atomic(&gset.join(name), b"").await?;
    }
    Ok(())
}

/// `die "<text>"` for a failed executor primitive, with the io cause surfaced
/// first as a stderr-shaped [`StageEvent::Output`] line.
///
/// The shell shows `cp`'s own stderr and then dies, and Sabrage has no `cp` child
/// to borrow stderr from, so a plain `PermissionDenied`, a read-only volume and
/// `ENOSPC` stay distinguishable instead of collapsing into one die string
/// (PARITY.md § Install (the one privileged write), "A copy failure prints the OS
/// error").
fn die_with_cause(
    ctx: &StageCtx,
    step_id: StepId,
    cause: SabrageError,
    message: &str,
) -> SabrageError {
    ctx.emit(StageEvent::Output {
        run_id: ctx.run_id,
        step: step_id.to_string(),
        stream: crate::events::Stream::Stderr,
        chunk: cause.to_string(),
        end: process::ChunkEnd::Lf,
    });
    ctx.fatal(message.to_string(), None)
}

/// `launch-action: adb-reverse-cleanup`. Reference: scripts/demo/run.sh.
///
/// Under `protocol = "alvr"`, runs `adb reverse --remove-all` on the connected
/// device if there is one: oxrsys-era reverse tunnels squat the ALVR client's
/// stream port (`EADDRINUSE`). The legacy `protocol = "oxrsys"` branch is
/// deliberately absent: the contract gates `cfg.protocol.legacy-oxrsys` as `block`
/// natively, so the preflight has already died (PARITY.md § Run preflight (encoded
/// in the contract's per-side gates), "Launch refuses `protocol=oxrsys` outright").
/// See tests::adb_reverse_cleanup_is_silent_without_adb_or_on_the_legacy_protocol.
pub async fn adb_reverse_cleanup(ctx: &StageCtx, facts: &PreflightFacts) -> Result<()> {
    if facts.protocol != "alvr" {
        return Ok(());
    }
    let Some(adb) = ctx.paths.adb.clone() else {
        return Ok(());
    };
    let Some(serial) = probe_device_serial(ctx, &adb, step::RUN_ADB_REVERSE).await else {
        return Ok(());
    };
    // `reverse --remove-all` IS correct here, unlike `forward --remove-all`:
    // different namespaces, and the ALVR client owns every reverse tunnel it
    // needs (PARITY.md § Invariants that must NOT change (byte/behavior parity),
    // "adb `forward --remove` per-serial").
    let spec = ctx
        .child(adb, step::RUN_ADB_REVERSE)
        .arg("-s")
        .arg(&serial)
        .arg("reverse")
        .arg("--remove-all");
    let _ = ctx.executor.run_child(&spec).await;
    ctx.step(step::RUN_ADB_REVERSE)
        .info(adb_reverse_cleared_line(&serial));
    Ok(())
}

/// run.sh:222's `info`. `pub` (A1-3), same reason as
/// [`wired_forward_failed_die`].
pub fn adb_reverse_cleared_line(serial: &str) -> String {
    format!("Quest {serial}: cleared adb reverse tunnels (ALVR manages its own)")
}

/// The wine child's spec: program, argv, cwd, env. Pure — builds and spawns
/// nothing, so the "copy the equivalent command" affordance and tests can both
/// read it.
///
/// argv matches run.sh exactly:
/// `"$WINE" --bottle "$WINEVR_BOTTLE" --no-update --cx-app "$BS_WIN"`, where
/// `BS_WIN` is [`bs_win_path`]. `paths.wine` is `Option` (no CrossOver); the bare
/// name `wine` stands in, matching the shell's empty `"$WINE"`, and the
/// `run.wine-exec` preflight blocks long before this. See
/// tests::wine_spec_is_run_shs_argv.
pub fn wine_spec(ctx: &StageCtx, bottle: &Bottle) -> ChildSpec {
    let wine = ctx
        .paths
        .wine
        .clone()
        .unwrap_or_else(|| PathBuf::from("wine"));
    let mut spec = ChildSpec::new(wine, step::RUN_LAUNCH, ctx.run_id)
        .arg("--bottle")
        .arg(&bottle.name)
        .arg("--no-update")
        .arg("--cx-app")
        .arg(bs_win_path(ctx, bottle));
    for (k, v) in wine_env(
        ctx.opts.verbose,
        std::env::var("WINEDEBUG").ok().as_deref(),
        contract().game.appid,
        &ctx.paths.oxr_runtime_json,
    ) {
        spec = spec.env(k, v);
    }
    // A Finder-launched `.app` inherits a bare PATH; demo.sh never needed this
    // because it runs from a login shell.
    spec.env_path(process::default_child_path())
}

/// `BS_WIN` — `win_path "$BS_DIR/Beat Saber.exe"`.
///
/// `pub` (A1-3) so `sabrage-parity` can pin `run.sh`'s `   exe: <BS_WIN>` line
/// against the real renderer instead of a copied substring.
pub fn bs_win_path(ctx: &StageCtx, bottle: &Bottle) -> String {
    util::win_path(Some(&bottle.prefix), &ctx.bs_dir.join("Beat Saber.exe"))
}

/// The launch environment: `XR_RUNTIME_JSON`, `CX_GRAPHICS_BACKEND=dxmt`,
/// `WINEDEBUG`, and `SteamAppId`/`SteamGameId`. Pure and table-testable.
/// Reference: scripts/demo/run.sh's `# launch-action: launch-wine` block.
///
/// The load-bearing detail is `WINEDEBUG`: the caller's preset wins in both
/// branches (`${WINEDEBUG:-…}`), so `WINEDEBUG=+d3d11` survives `--verbose`
/// (parity decision 21; PARITY.md § Invariants that must NOT change
/// (byte/behavior parity), "`WINEDEBUG` caller-precedence"). `inherited_winedebug` is that preset; `None`
/// and `Some("")` both take the branch default (zsh's `:-` treats unset and empty alike).
pub fn wine_env(
    verbose: bool,
    inherited_winedebug: Option<&str>,
    appid: u64,
    runtime_json: &Path,
) -> Vec<(String, String)> {
    let fallback = if verbose { "fixme-all,+openxr" } else { "-all" };
    let winedebug = match inherited_winedebug {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => fallback.to_string(),
    };
    let appid = appid.to_string();
    vec![
        (
            "XR_RUNTIME_JSON".to_string(),
            runtime_json.display().to_string(),
        ),
        ("CX_GRAPHICS_BACKEND".to_string(), "dxmt".to_string()),
        ("WINEDEBUG".to_string(), winedebug),
        ("SteamAppId".to_string(), appid.clone()),
        ("SteamGameId".to_string(), appid),
    ]
}

/// The six-line launch banner, verbatim, as the exact event sequence the CLI
/// reproduces byte-for-byte (PARITY.md § Invariants that must NOT change
/// (byte/behavior parity), "the six-line launch banner text").
///
/// The `-- launching …` line is a [`StageEvent::Section`]; the rest are
/// [`StageEvent::Text`], leading spaces and empty lines included. See
/// tests::the_banner_is_one_section_with_every_text_row_on_the_launch_step.
///
/// `pub` (A1-3) so `sabrage-parity` can pin this banner against `run.sh` by
/// calling the real renderer instead of copying a substring per line.
pub fn banner_events(
    run_id: crate::events::RunId,
    bottle_name: &str,
    bs_win: &str,
    log: &Path,
) -> Vec<StageEvent> {
    let t = |s: String| StageEvent::text(run_id, Some(step::RUN_LAUNCH), s);
    vec![
        t(String::new()),
        StageEvent::Section {
            run_id,
            title: "launching Beat Saber through the bridge".to_string(),
        },
        t("   put the headset ON and open the ALVR client; first frame can take ~30s.".into()),
        t("   pause in-game = X/A button or the Quest system button".into()),
        t(
            "   (the left-menu-button pause is a Beat Saber/Unity limitation on every OpenXR runtime)"
                .into(),
        ),
        t(format!(
            "   stop: Ctrl-C here, or ./demo.sh stop --bottle {bottle_name} from another shell"
        )),
        t(format!("   exe: {bs_win}")),
        t(format!("   log: {}", log.display())),
        t(String::new()),
    ]
}

/// True for the `EEXIST` [`crate::executor::Executor::spawn_detached`] raises
/// when the chosen log name was taken between the probe and the open.
fn is_already_exists(e: &SabrageError) -> bool {
    matches!(e, SabrageError::Io { source, .. } if source.kind() == std::io::ErrorKind::AlreadyExists)
}

/// The first `beatsaber-<ts>[-n].log` name in `logs_dir` that does not exist
/// yet, starting from `start`.
///
/// The `exists()` probe is advisory — [`crate::executor::Executor::spawn_detached`]
/// opens `create_new`, so the file's absence is only ever confirmed by the
/// open itself. Falling through at [`LOG_NAME_ATTEMPTS`] hands the last
/// candidate to the spawn and lets its `EEXIST` be the error.
fn pick_log_path(logs_dir: &Path, start: u32) -> (PathBuf, u32) {
    let mut attempt = start;
    loop {
        let candidate = logs::wine_log_candidate(logs_dir, chrono::Local::now(), attempt);
        if attempt + 1 >= LOG_NAME_ATTEMPTS || !candidate.exists() {
            return (candidate, attempt);
        }
        attempt += 1;
    }
}

/// `launch-action: launch-wine`. Reference: scripts/demo/run.sh.
///
/// Emits the banner ([`banner_events`]) before the spawn, picks a non-colliding
/// log name ([`crate::logs::wine_log_candidate`]), and spawns **detached**
/// ([`crate::executor::Executor::spawn_detached`] — never
/// [`crate::process::spawn_streamed`], whose `kill_on_drop(true)` would SIGKILL the
/// game when Sabrage quits). Returns the child and log path, or `Ok(None)` under a
/// dry run. See tests::the_banner_is_one_section_with_every_text_row_on_the_launch_step.
///
/// If the spawn loses the `create_new` race, the next candidate is taken and a
/// corrected `   log: <path>` line is emitted (PARITY.md § Run (launch), "The wine
/// console log is a plain file").
pub async fn launch_wine(
    ctx: &StageCtx,
    bottle: &Bottle,
) -> Result<Option<(DetachedChild, PathBuf)>> {
    let spec = wine_spec(ctx, bottle);
    let logs_dir = ctx.paths.logs_dir();
    ctx.executor.create_dir_all(&logs_dir).await?;

    let (mut log, mut attempt) = pick_log_path(&logs_dir, 0);
    for ev in banner_events(ctx.run_id, &bottle.name, &bs_win_path(ctx, bottle), &log) {
        ctx.emit(ev);
    }

    loop {
        match ctx
            .executor
            .spawn_detached(&spec, DetachedStdio::LogFile(log.clone()))
            .await
        {
            Ok(Some(child)) => return Ok(Some((child, log))),
            // A dry run plans the launch and returns no child — never a failure.
            Ok(None) => return Ok(None),
            Err(e) if is_already_exists(&e) && attempt + 1 < LOG_NAME_ATTEMPTS => {
                let (next, next_attempt) = pick_log_path(&logs_dir, attempt + 1);
                log = next;
                attempt = next_attempt;
                ctx.emit(StageEvent::text(
                    ctx.run_id,
                    Some(step::RUN_LAUNCH),
                    format!("   log: {}", log.display()),
                ));
            }
            Err(e) => return Err(e),
        }
    }
}

#[cfg(test)]
mod tests;
