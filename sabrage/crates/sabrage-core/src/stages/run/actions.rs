//! The seven ordered launch actions — run.sh's `# launch-action:` tags.
//!
//! Unlike checks these are **unconditional preparation steps**: no pass/fail,
//! no remedy, no gate. The contract lists them (`[[launch_action]]`) purely so
//! both implementations agree on *which* steps exist and in *what order*;
//! [`LAUNCH_ACTION_IDS`] is this side's copy and the parity harness joins the
//! two.
//!
//! Two of the seven are guarded rather than performed here — `audio-route` and
//! `dashboard` live in [`super::guards`], because acquiring them is the same
//! act as arming their undo.
//!
//! # Mutations vs probes
//!
//! Every mutation here goes through [`crate::executor::Executor`] (`adb
//! forward`, `adb forward --remove`, `adb reverse --remove-all`,
//! `wineserver -k`/`-w`, the Goldberg copies and writes, the wine spawn), so
//! `--dry-run` plans them. The three probes whose *output* is the point —
//! `adb devices` twice, and the log-name collision test — use
//! [`crate::process::capture`] / a plain `exists()` and run in both modes,
//! which is what makes the plan accurate rather than optimistic.

use std::path::{Path, PathBuf};

use crate::contract::contract;
use crate::error::{Result, SabrageError};
use crate::events::{step, StageEvent, StepId};
use crate::executor::{DetachedChild, DetachedStdio};
use crate::paths::Bottle;
use crate::process::{self, ChildSpec, ProcInfo};
use crate::session::state::WiredForward;
use crate::stages::{StageCtx, RUN_WINESERVER_WAIT};
use crate::{logs, util};

use super::PreflightFacts;

/// run.sh's `# launch-action:` ids, in execution order.
///
/// Must equal `contract().launch_actions`' ids, in order — asserted below and
/// by the parity crate.
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

// ── 1. adb forward hygiene ────────────────────────────────────────────────────

/// `awk 'NR>1 && $2=="device"{print $1; exit}'` over `adb devices`' stdout.
///
/// `NR>1` skips adb's `List of devices attached` header; a row whose state is
/// anything but exactly `device` (`unauthorized`, `offline`) is skipped, and a
/// blank line has no `$2` at all.
pub(crate) fn first_device_serial(stdout: &str) -> Option<String> {
    stdout.lines().skip(1).find_map(|line| {
        let mut fields = line.split_whitespace();
        let serial = fields.next()?;
        let state = fields.next()?;
        (state == "device").then(|| serial.to_string())
    })
}

/// `"$ADB" devices 2>/dev/null | awk …` — a read-only probe, so it bypasses
/// the executor and runs under `--dry-run` too.
///
/// Carries the launch's cancellation token: this is the one probe a user is
/// likely to be waiting on (they hit Run before plugging the headset in), and
/// a wedged `adb` here would otherwise hold the operation lock with Cancel
/// unable to interrupt it. A cancelled or timed-out probe fails exactly like a
/// missing binary — `None`, which is run.sh's empty `$WIRED_SER`.
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
/// `"tcp:9943"` (PARITY.md's "must NOT change" list).
fn stream_forward_specs() -> Vec<String> {
    contract()
        .ports
        .stream
        .iter()
        .map(|p| format!("tcp:{p}"))
        .collect()
}

/// run.sh:108 — remove **both** ports, ignoring every failure.
///
/// On a fresh, non-cancelled executor ([`super::teardown_ctx`]): the common
/// reason to be rolling back is that Stop cancelled the token mid-loop, and
/// the launch executor refuses every child once it is cancelled — the removal
/// would be a silent no-op exactly when it matters most.
async fn rollback_forwards(ctx: &StageCtx, adb: &Path, serial: &str, specs: &[String]) {
    let rb = super::teardown_ctx(ctx);
    for q in specs {
        let undo = rb
            .child(adb.to_path_buf(), step::RUN_ADB_FORWARDS)
            .arg("-s")
            .arg(serial)
            .arg("forward")
            .arg("--remove")
            .arg(q);
        let _ = rb.executor.run_child(&undo).await;
    }
}

/// `launch-action: adb-forward-hygiene` — run.sh lines 93–124.
///
/// `--wired`: create `tcp:9943` and `tcp:9944` on the first device whose state
/// is exactly `device`; if either fails, remove **both** and die. Otherwise:
/// walk `adb forward --list` and remove exactly those two local ports,
/// per-serial. Never `--remove-all` — that would delete forwards this pipeline
/// knows nothing about (PARITY.md; CLAUDE.md's `--wired` note; the distinction
/// from `adb reverse --remove-all`, which *is* fine, is deliberate).
///
/// Returns the forwards actually created, for [`crate::session::state`] to
/// persist so they can be removed even after a crash.
pub async fn adb_forward_hygiene(ctx: &StageCtx) -> Result<Vec<WiredForward>> {
    let st = ctx.step(step::RUN_ADB_FORWARDS);
    let adb = ctx.paths.adb.clone();

    if !ctx.opts.wired {
        // run.sh's `elif [ -n "$ADB" ]` branch is `fix.remove-adb-forwards`
        // verbatim — same per-serial removal, same `info` text, same tolerant
        // "a failed removal prints nothing" rule. No adb at all: nothing to do.
        //
        // `_at`, not the plain fix: this is the run stage's step 2, and its
        // rows must be attributed to it. The fix's own `fix.remove-adb-forwards`
        // step id belongs to the doctor's fix list, where no stage is running.
        if adb.is_some() {
            crate::fixes::adb::remove_adb_forwards_at(ctx, &ctx.sink, step::RUN_ADB_FORWARDS)
                .await?;
        }
        return Ok(Vec::new());
    }

    // run.sh:104
    let Some(adb) = adb else {
        return Err(st.fatal(
            "--wired needs adb (Android platform-tools) on PATH or under ~/Library/Android/sdk",
            None,
        ));
    };
    // run.sh:105
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
    let mut created: Vec<WiredForward> = Vec::new();
    for (port, local) in contract().ports.stream.iter().zip(&specs) {
        let spec = ctx
            .child(adb.clone(), step::RUN_ADB_FORWARDS)
            .arg("-s")
            .arg(&serial)
            .arg("forward")
            .arg(local)
            .arg(local);
        // Every non-success leaves through the same door. run.sh's `if !
        // "$ADB" … forward` catches a failed *exec* too (a missing or
        // unrunnable adb is just a nonzero exit to the shell), and a
        // cancellation between the two ports would otherwise leave the first
        // forward on the device with nothing on disk naming it — the exact
        // stale forward that silently breaks the next WiFi run.
        let failure = match ctx.executor.run_child(&spec).await {
            Ok(status) if status.success() => None,
            Ok(_) => Some(None),
            Err(e) => Some(Some(e)),
        };
        if let Some(cause) = failure {
            rollback_forwards(ctx, &adb, &serial, &specs).await;
            // run.sh:109.
            let die = format!(
                "adb forward {local} {local} failed on {serial} — check the USB connection \
                 (adb devices)"
            );
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
        created.push(WiredForward {
            serial: serial.clone(),
            port: *port,
        });
    }
    // run.sh:112 — the port list is rendered from the contract so the literal
    // `tcp:9943/tcp:9944` in the shell string cannot drift from `ports.stream`.
    st.info(format!(
        "wired mode: adb forward {} up on {serial} (a later non-wired run clears these two)",
        specs.join("/")
    ));
    Ok(created)
}

// ── 2. wineserver reset ───────────────────────────────────────────────────────

/// `"<pid> <exe-basename>"` per process, space-joined with a trailing space —
/// the shape of `pgrep -lf wineserver | tr '\n' ' '` (one `pid name` pair per
/// line, each line's newline becoming a trailing space).
///
/// Same rendering as `stop`'s private `format_survivors`, with this call
/// site's own fallback name for a process whose exe path has no file name.
/// (PARITY.md, "Stop": pid+basename rather than `pgrep -lf`'s full argv.)
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

/// `launch-action: wineserver-reset` — run.sh lines 126–138.
///
/// `wineserver -k` (failure ignored), then a bounded `-w` wait of
/// [`crate::stages::RUN_WINESERVER_WAIT`] — **fatal** on timeout, unlike
/// `stop`'s 4 s advisory wait. The timeout path warns with the survivor list
/// (`pgrep -lf wineserver`, i.e.
/// [`crate::process::find_processes_by_cmdline`]) and then dies:
///
/// ```text
/// wineserver still alive after 5s: <pid name >…
/// kill the listed wineserver(s) manually, then re-run
/// ```
pub async fn wineserver_reset(ctx: &StageCtx, bottle: &Bottle) -> Result<()> {
    let st = ctx.step(step::RUN_WINESERVER);
    ctx.section(format!("resetting wineserver for bottle '{}'", bottle.name));

    // No CrossOver on this machine means `$WINESERVER` is empty: the shell
    // "runs" it, swallows the command-not-found, and reaches `ok` anyway. The
    // `run.wine-exec` preflight has already died by then, so this is
    // unreachable in practice — reproduced rather than special-cased.
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
        st.warn(format!(
            "wineserver still alive after {}s: {}",
            RUN_WINESERVER_WAIT.as_secs(),
            format_survivors(
                &process::find_processes_by_cmdline("wineserver"),
                "wineserver"
            )
        ));
        return Err(st.fatal("kill the listed wineserver(s) manually, then re-run", None));
    }
    st.ok("wineserver down");
    Ok(())
}

// ── 3. Goldberg ───────────────────────────────────────────────────────────────

/// `"$API.orig-steam"` — a suffix on the whole file name, not an extension swap.
fn orig_steam_path(api: &Path) -> PathBuf {
    let mut s = api.as_os_str().to_os_string();
    s.push(".orig-steam");
    PathBuf::from(s)
}

/// `$BS_DIR/Beat Saber_Data/Plugins/x86_64/steam_api64.dll`, else
/// `$BS_DIR/steam_api64.dll` — the second is returned even when it is absent,
/// so the caller produces run.sh:145's die text for it.
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

/// `launch-action: goldberg-stage` — run.sh lines 140–152.
///
/// Byte-exact artifacts, all four of them:
///
/// * `steam_api64.dll` found under `Beat Saber_Data/Plugins/x86_64/` or at the
///   game root, dying when neither exists;
/// * `<api>.orig-steam` created **once** and never overwritten (it is the only
///   copy of the real Steam dll on the machine);
/// * the Goldberg dll copied over it when the bytes differ — a hash mismatch
///   is *tolerated* here, unlike in setup (parity decision 20);
/// * `steam_appid.txt` = the appid digits with **no trailing newline**
///   (`printf '%s'`), and `steam_settings/{offline,disable_networking,
///   disable_overlay}.txt` truncate-created empty (`: >`).
pub async fn goldberg_stage(ctx: &StageCtx) -> Result<()> {
    let st = ctx.step(step::RUN_GOLDBERG);
    ctx.section("Goldberg");

    let api = steam_api_path(&ctx.bs_dir);
    if !api.is_file() {
        // run.sh:145
        return Err(st.fatal(
            format!(
                "steam_api64.dll not found under {} — is this a complete Beat Saber install?",
                ctx.bs_dir.display()
            ),
            None,
        ));
    }
    let api_dir = api
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    // run.sh:147 — the backup is made once and never refreshed; overwriting it
    // with an already-Goldberged dll would destroy the only copy of the real
    // Steam library on this machine.
    let backup = orig_steam_path(&api);
    if !backup.exists() {
        // Sabrage-only row, no shell counterpart (run.sh is silent here): the
        // live dll is ALREADY the Goldberg build, so the backup this line
        // mints holds Goldberg, not Steam. The bytes are run.sh's either way —
        // saying so is the only honest thing left, because
        // `store::goldberg::revert_original_steam_dll` would otherwise copy
        // these bytes back and call it a restore.
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
            st.warn(format!(
                "steam_api64.dll was already the Goldberg build, so {} is a copy of \
                 Goldberg — the real Steam dll was never seen here and cannot be restored",
                backup.display()
            ));
        }
    }

    // run.sh:148-149 — `cmp -s`, not a hash: run tolerates a Goldberg dll that
    // does not match setup's pin (parity decision 20).
    if util::cmp_files(&ctx.paths.gbe_dll, &api) {
        st.info("goldberg already installed");
    } else {
        if let Err(e) = ctx.executor.copy_if_changed(&ctx.paths.gbe_dll, &api).await {
            return Err(die_with_cause(
                ctx,
                step::RUN_GOLDBERG,
                e,
                "goldberg install failed",
            ));
        }
        st.ok(format!("installed goldberg -> {}", api.display()));
    }

    // run.sh:150 — `printf '%s' "$BS_APPID"`: the digits, no trailing newline.
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

    // run.sh:151-152 — `mkdir -p` then three `: >` truncate-creates. The shell
    // has no `|| die` on either, but a failure here is propagated rather than
    // swallowed (design-core §6.6: no silent aborts, no silent successes).
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
/// Same shape `install`'s `install_if_changed` uses (PARITY.md, "Install"):
/// the shell shows `cp`'s own stderr and then dies, and Sabrage has no `cp`
/// child to borrow stderr from, so a plain `PermissionDenied`, a read-only
/// volume and `ENOSPC` stay distinguishable instead of collapsing into one
/// die string.
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
    });
    ctx.fatal(message.to_string(), None)
}

// ── 6. adb reverse cleanup ────────────────────────────────────────────────────

/// `launch-action: adb-reverse-cleanup` — run.sh lines 219–237.
///
/// `protocol = "alvr"`: `adb reverse --remove-all` on the connected device, if
/// any — oxrsys-era reverse tunnels squat the ALVR client's stream port
/// (`EADDRINUSE`).
///
/// The legacy `protocol = "oxrsys"` branch (warn when no device, else
/// remove-all + re-create the `legacy_reverse` tunnels + `am start` the
/// Android client) is **unreachable on this side**: the contract gates
/// `cfg.protocol.legacy-oxrsys` as `block` natively where the shell only warns
/// (PARITY.md, "Run preflight"), so the preflight has already died before this
/// runs. It is deliberately not written here rather than written and dead —
/// v1 keeps the legacy USB path in `./demo.sh run` territory.
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
    // `reverse --remove-all` IS correct here — unlike `forward --remove-all`,
    // which PARITY.md forbids. The two are different namespaces and the ALVR
    // client owns every reverse tunnel it needs.
    let spec = ctx
        .child(adb, step::RUN_ADB_REVERSE)
        .arg("-s")
        .arg(&serial)
        .arg("reverse")
        .arg("--remove-all");
    let _ = ctx.executor.run_child(&spec).await;
    ctx.step(step::RUN_ADB_REVERSE).info(format!(
        "Quest {serial}: cleared adb reverse tunnels (ALVR manages its own)"
    ));
    Ok(())
}

// ── 7. launch ─────────────────────────────────────────────────────────────────

/// The wine child's spec: program, argv, cwd, env. Pure — builds nothing and
/// spawns nothing, so the "copy the equivalent command" affordance and the
/// tests can both read it.
///
/// argv is run.sh's exactly:
/// `"$WINE" --bottle "$WINEVR_BOTTLE" --no-update --cx-app "$BS_WIN"`, where
/// `BS_WIN` is [`crate::util::win_path`] of `<bs_dir>/Beat Saber.exe`.
///
/// `paths.wine` is `Option` (no CrossOver on the machine); the bare name
/// `wine` stands in, which is what the shell's empty `"$WINE"` amounts to.
/// The `run.wine-exec` preflight blocks long before this.
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
pub(crate) fn bs_win_path(ctx: &StageCtx, bottle: &Bottle) -> String {
    util::win_path(Some(&bottle.prefix), &ctx.bs_dir.join("Beat Saber.exe"))
}

/// The launch environment — run.sh lines 242–248. Pure and table-testable.
///
/// ```zsh
/// export XR_RUNTIME_JSON="$OXR_RUNTIME_JSON"
/// export CX_GRAPHICS_BACKEND=dxmt
/// if [ -n "${WINEVR_VERBOSE:-}" ]; then export WINEDEBUG="${WINEDEBUG:-fixme-all,+openxr}"
/// else export WINEDEBUG="${WINEDEBUG:--all}"; fi
/// export SteamAppId=$BS_APPID SteamGameId=$BS_APPID
/// ```
///
/// The load-bearing detail is `WINEDEBUG`: **the caller's preset wins in both
/// branches** (`${WINEDEBUG:-…}`), so `WINEDEBUG=+d3d11 ./demo.sh run` keeps
/// `+d3d11` whether or not `--verbose` was passed (parity decision 21).
/// `inherited_winedebug` is that preset — `None`, or `Some("")`, takes the
/// default for the branch (zsh's `:-` treats unset and empty alike).
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

/// run.sh lines 252–260, verbatim, as the exact event sequence the CLI
/// reproduces byte-for-byte.
///
/// The `-- launching …` line is a [`StageEvent::Section`] (the CLI renders it
/// as `-- <title>`); the rest are [`StageEvent::Text`], leading spaces and
/// empty lines included.
pub(crate) fn banner_events(
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

/// `launch-action: launch-wine` — run.sh lines 239–266.
///
/// Prints the six-line banner block verbatim as
/// [`crate::events::StageEvent::Text`], picks a non-colliding log name
/// ([`crate::logs::wine_log_candidate`]), and spawns **detached**
/// ([`crate::executor::Executor::spawn_detached`] — never
/// [`crate::process::spawn_streamed`], whose `kill_on_drop(true)` would SIGKILL
/// the game when Sabrage quits). Returns the child and the log path it is
/// writing to, or `Ok(None)` under a dry run.
///
/// The banner is emitted **before** the spawn, exactly where the shell prints
/// it. If the spawn still loses the `create_new` race (a `./demo.sh run` in
/// the same second), the next candidate is taken and a corrected
/// `   log: <path>` line is emitted so the printed path is never a lie — the
/// shell's `tee` simply truncates the older run's log instead (declared
/// divergence, PARITY.md).
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
mod tests {
    use super::*;
    use crate::contract::contract;
    use crate::events::{Severity, Stage};
    use crate::executor::{DryRunExecutor, Executor, PlannedKind};
    use crate::paths::Paths;
    use crate::stages::{EventSink, StageOptions};
    use std::sync::{Arc, Mutex};
    use tokio_util::sync::CancellationToken;
    use uuid::Uuid;

    // ── fixtures ─────────────────────────────────────────────────────────────

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sabrage-run-actions-{tag}-{}-{}",
            std::process::id(),
            Uuid::new_v4().as_simple()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A `StageCtx` whose every path lives under `root` — no real `$HOME`, no
    /// CrossOver, no adb, and a [`DryRunExecutor`] so nothing is written.
    fn dry_ctx(root: &Path, opts: StageOptions) -> (StageCtx, Arc<Mutex<Vec<StageEvent>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        let sink: EventSink = Arc::new(move |ev| s.lock().unwrap().push(ev));
        let run_id = Uuid::new_v4();
        let cancel = CancellationToken::new();
        let executor: Arc<dyn Executor> =
            Arc::new(DryRunExecutor::new(run_id, sink.clone(), cancel.clone()));

        let mut paths = Paths::new(root);
        paths.oxr_appsup = root.join("appsup-oxrsys");
        paths.sabrage_appsup = root.join("appsup-sabrage");
        paths.adb = None;
        paths.wine = None;
        paths.wineserver = None;

        let ctx = StageCtx::with_executor(paths, opts, sink, cancel, executor, run_id);
        (ctx, seen)
    }

    fn bottle(root: &Path) -> Bottle {
        Bottle {
            name: "Steam".to_string(),
            prefix: root.join("bottle"),
            sys32: root.join("bottle/drive_c/windows/system32"),
        }
    }

    fn texts(evs: &[StageEvent]) -> Vec<String> {
        evs.iter()
            .filter_map(|e| match e {
                StageEvent::Text { text, .. } => Some(text.clone()),
                StageEvent::Section { title, .. } => Some(format!("-- {title}")),
                StageEvent::Line { severity, text, .. } => Some(format!("[{severity}] {text}")),
                _ => None,
            })
            .collect()
    }

    // ── contract join ────────────────────────────────────────────────────────

    #[test]
    fn the_action_ids_match_the_contract_in_order() {
        let from_contract: Vec<&str> = contract()
            .launch_actions
            .iter()
            .map(|a| a.id.as_str())
            .collect();
        assert_eq!(LAUNCH_ACTION_IDS.to_vec(), from_contract);
        // The two guarded ones are in the list but implemented in `guards`.
        assert!(LAUNCH_ACTION_IDS.contains(&"audio-route"));
        assert!(LAUNCH_ACTION_IDS.contains(&"dashboard"));
        // Launch is always last.
        assert_eq!(LAUNCH_ACTION_IDS[6], "launch-wine");
    }

    #[test]
    fn one_step_id_per_action_plus_the_three_run_only_phases() {
        // Every launch action maps onto a `run.*` step; the state machine adds
        // preflight, supervise and teardown.
        assert_eq!(
            Stage::Run.steps().len(),
            LAUNCH_ACTION_IDS.len() + 3,
            "run's step list and the contract's action list must stay aligned"
        );
    }

    // ── adb devices parsing ──────────────────────────────────────────────────

    #[test]
    fn first_device_serial_matches_the_awk_program() {
        // NR>1 skips the header; only an exact `device` state counts.
        assert_eq!(
            first_device_serial("List of devices attached\n1WMHH0X\tdevice\n"),
            Some("1WMHH0X".to_string())
        );
        assert_eq!(
            first_device_serial("List of devices attached\nabc\tunauthorized\ndef\tdevice\n"),
            Some("def".to_string())
        );
        // The header itself is never a candidate, even though its second field
        // is a word.
        assert_eq!(first_device_serial("List of devices attached\n"), None);
        assert_eq!(first_device_serial(""), None);
        assert_eq!(first_device_serial("List of devices attached\n\n\n"), None);
        // `offline` / `no permissions` rows are skipped.
        assert_eq!(
            first_device_serial("List of devices attached\nabc\toffline\n"),
            None
        );
    }

    #[test]
    fn the_wired_ports_come_from_the_contract() {
        assert_eq!(
            stream_forward_specs(),
            vec!["tcp:9943".to_string(), "tcp:9944".to_string()]
        );
    }

    // ── adb forward hygiene ──────────────────────────────────────────────────

    #[tokio::test]
    async fn a_non_wired_run_without_adb_does_nothing_at_all() {
        let root = scratch("no-adb");
        let (ctx, seen) = dry_ctx(&root, StageOptions::default());
        let made = adb_forward_hygiene(&ctx).await.unwrap();
        assert!(made.is_empty());
        assert!(seen.lock().unwrap().is_empty());
        assert!(ctx.executor.planned().is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[tokio::test]
    async fn the_non_wired_forward_cleanup_is_stamped_with_the_run_stages_step() {
        // #16c: the removal is `fix.remove-adb-forwards`'s code, but here it
        // is the run stage's step 2 — its rows must sort and group with the
        // rest of the launch, not with a fix that is not running.
        let root = scratch("forward-step-id");
        let (mut ctx, seen) = dry_ctx(&root, StageOptions::default());
        ctx.paths.adb = Some(fake_forward_list_adb(
            &root,
            "SERIALX tcp:9943 tcp:9943\nSERIALX tcp:5555 tcp:5555\n",
        ));

        let made = adb_forward_hygiene(&ctx).await.unwrap();
        assert!(made.is_empty(), "a non-wired run creates nothing");

        let evs = seen.lock().unwrap().clone();
        let rows: Vec<(Option<&str>, String)> = evs
            .iter()
            .filter_map(|e| match e {
                StageEvent::Line { text, .. } => Some((e.step(), text.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].0, Some(step::RUN_ADB_FORWARDS));
        assert!(
            rows[0]
                .1
                .starts_with("would clear stale adb forward tcp:9943 on SERIALX"),
            "{}",
            rows[0].1
        );
        assert!(
            !rows
                .iter()
                .any(|(s, _)| *s == Some("fix.remove-adb-forwards")),
            "the launch path must not borrow the standalone fix's step id"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    /// A `/bin/sh` script standing in for `adb`, answering `forward --list`
    /// with `list_stdout` and succeeding at everything else.
    fn fake_forward_list_adb(dir: &Path, list_stdout: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("adb-forwards");
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\n\
                 if [ \"$1\" = forward ] && [ \"$2\" = --list ]; then\n\
                 \x20 printf '%s' '{list_stdout}'\n\
                 fi\n\
                 exit 0\n"
            ),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[tokio::test]
    async fn wired_without_adb_dies_with_run_shs_text() {
        let root = scratch("wired-no-adb");
        let (ctx, _) = dry_ctx(
            &root,
            StageOptions {
                wired: true,
                ..Default::default()
            },
        );
        let err = adb_forward_hygiene(&ctx).await.unwrap_err();
        assert_eq!(
            err.to_string(),
            "--wired needs adb (Android platform-tools) on PATH or under ~/Library/Android/sdk"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// A `/bin/sh` script standing in for `adb`, so the wired branch is
    /// exercised without an Android SDK, a device, or any real forward.
    fn fake_adb(dir: &Path, devices_stdout: &str, forward_exit: i32) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("adb");
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\n\
                 for a in \"$@\"; do\n\
                 \x20 case \"$a\" in devices) printf '%s' '{devices_stdout}'; exit 0;; esac\n\
                 done\n\
                 echo \"$@\" >> \"$(dirname \"$0\")/adb.calls\"\n\
                 exit {forward_exit}\n"
            ),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[tokio::test]
    async fn wired_with_no_device_dies_with_run_shs_text() {
        let root = scratch("wired-no-device");
        let (mut ctx, _) = dry_ctx(
            &root,
            StageOptions {
                wired: true,
                ..Default::default()
            },
        );
        ctx.paths.adb = Some(fake_adb(&root, "List of devices attached\n", 0));
        let err = adb_forward_hygiene(&ctx).await.unwrap_err();
        assert_eq!(
            err.to_string(),
            "--wired: no Quest over adb — connect USB and check 'adb devices'"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[tokio::test]
    async fn wired_plans_both_forwards_and_reports_them() {
        let root = scratch("wired-ok");
        let (mut ctx, seen) = dry_ctx(
            &root,
            StageOptions {
                wired: true,
                ..Default::default()
            },
        );
        ctx.paths.adb = Some(fake_adb(
            &root,
            "List of devices attached\n1WMHH0X\tdevice\n",
            0,
        ));

        let made = adb_forward_hygiene(&ctx).await.unwrap();
        assert_eq!(
            made,
            vec![
                WiredForward {
                    serial: "1WMHH0X".into(),
                    port: 9943
                },
                WiredForward {
                    serial: "1WMHH0X".into(),
                    port: 9944
                },
            ]
        );
        // Two planned spawns, never a `--remove-all`.
        let plan = ctx.executor.planned();
        assert_eq!(plan.len(), 2);
        for (p, port) in plan.iter().zip(["tcp:9943", "tcp:9944"]) {
            assert_eq!(p.kind, PlannedKind::Spawn);
            assert!(
                p.reason
                    .ends_with(&format!("-s 1WMHH0X forward {port} {port}")),
                "{}",
                p.reason
            );
        }
        assert!(!plan.iter().any(|p| p.reason.contains("--remove-all")));
        assert_eq!(
            texts(&seen.lock().unwrap()),
            vec![
                "[info] wired mode: adb forward tcp:9943/tcp:9944 up on 1WMHH0X \
                 (a later non-wired run clears these two)"
                    .to_string()
            ]
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// `adb.calls` — one line per non-`devices` invocation of [`fake_adb`].
    fn adb_calls(root: &Path) -> Vec<String> {
        std::fs::read_to_string(root.join("adb.calls"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// A real (non-dry) ctx over the scratch root, so the fake adb actually
    /// runs and its exit status decides the branch.
    fn real_ctx(root: &Path, opts: StageOptions) -> (StageCtx, Arc<Mutex<Vec<StageEvent>>>) {
        let (mut ctx, seen) = dry_ctx(root, opts);
        let run_id = ctx.run_id;
        ctx.executor = Arc::new(crate::executor::RealExecutor::new(
            run_id,
            ctx.sink.clone(),
            ctx.cancel.clone(),
        ));
        (ctx, seen)
    }

    /// run.sh:108 — a nonzero `adb forward` removes BOTH ports before dying.
    #[tokio::test]
    async fn a_failed_forward_removes_both_ports_and_dies_with_run_shs_text() {
        let root = scratch("wired-fail");
        let (mut ctx, _) = real_ctx(
            &root,
            StageOptions {
                wired: true,
                ..Default::default()
            },
        );
        ctx.paths.adb = Some(fake_adb(
            &root,
            "List of devices attached\n1WMHH0X\tdevice\n",
            1,
        ));

        let err = adb_forward_hygiene(&ctx).await.unwrap_err();
        assert_eq!(
            err.to_string(),
            "adb forward tcp:9943 tcp:9943 failed on 1WMHH0X — check the USB connection \
             (adb devices)"
        );
        let calls = adb_calls(&root);
        assert!(
            calls
                .iter()
                .any(|c| c.contains("forward --remove tcp:9943")),
            "{calls:?}"
        );
        assert!(
            calls
                .iter()
                .any(|c| c.contains("forward --remove tcp:9944")),
            "{calls:?}"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    /// A7-4: a cancellation between the two ports used to return through `?`
    /// and skip the rollback entirely — leaving `tcp:9943` on the device with
    /// nothing on disk naming it, which is exactly the stale forward that
    /// silently breaks the next WiFi run.
    #[tokio::test]
    async fn a_cancellation_mid_loop_still_rolls_the_first_forward_back() {
        let root = scratch("wired-cancel");
        let (mut ctx, _) = real_ctx(
            &root,
            StageOptions {
                wired: true,
                ..Default::default()
            },
        );
        ctx.paths.adb = Some(slow_second_port_adb(
            &root,
            "List of devices attached\n1WMHH0X\tdevice\n",
        ));

        let cancel = ctx.cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            cancel.cancel();
        });

        let err = adb_forward_hygiene(&ctx).await.unwrap_err();
        assert!(matches!(err, SabrageError::Cancelled), "{err:?}");
        let calls = adb_calls(&root);
        assert!(
            calls
                .iter()
                .any(|c| c.contains("forward --remove tcp:9943")),
            "the rollback must run on a fresh executor, not the cancelled one: {calls:?}"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    /// A cancel landing on the `adb devices` probe reads as a cancellation,
    /// not as "no Quest over adb" — and the probe itself is interruptible.
    #[tokio::test]
    async fn a_cancel_during_the_device_probe_is_a_cancellation() {
        let root = scratch("wired-probe-cancel");
        let (mut ctx, _) = real_ctx(
            &root,
            StageOptions {
                wired: true,
                ..Default::default()
            },
        );
        ctx.paths.adb = Some(slow_devices_adb(&root));

        let cancel = ctx.cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            cancel.cancel();
        });

        let err = adb_forward_hygiene(&ctx).await.unwrap_err();
        assert!(matches!(err, SabrageError::Cancelled), "{err:?}");
        std::fs::remove_dir_all(&root).ok();
    }

    /// An `adb` whose `devices` never answers.
    fn slow_devices_adb(dir: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("adb");
        std::fs::write(&path, "#!/bin/sh\nsleep 30\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// [`fake_adb`], but `tcp:9944` hangs — so a cancellation can land between
    /// the two forwards.
    fn slow_second_port_adb(dir: &Path, devices_stdout: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("adb");
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\n\
                 for a in \"$@\"; do\n\
                 \x20 case \"$a\" in devices) printf '%s' '{devices_stdout}'; exit 0;; esac\n\
                 done\n\
                 echo \"$@\" >> \"$(dirname \"$0\")/adb.calls\"\n\
                 case \"$*\" in *--remove*) ;; *9944*) sleep 30;; esac\n\
                 exit 0\n"
            ),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    // ── goldberg ─────────────────────────────────────────────────────────────

    /// `bs_dir` with a `steam_api64.dll` under the Plugins path, plus the
    /// Goldberg dll at `third_party/gbe/`.
    fn goldberg_fixture(root: &Path, api_bytes: &[u8], gbe_bytes: &[u8]) -> PathBuf {
        let bs_dir = root.join("Beat Saber 1294");
        let plugins = bs_dir.join("Beat Saber_Data/Plugins/x86_64");
        std::fs::create_dir_all(&plugins).unwrap();
        std::fs::write(plugins.join("steam_api64.dll"), api_bytes).unwrap();
        let gbe = root.join("third_party/gbe");
        std::fs::create_dir_all(&gbe).unwrap();
        std::fs::write(gbe.join("steam_api64.dll"), gbe_bytes).unwrap();
        bs_dir
    }

    fn goldberg_ctx(root: &Path, bs_dir: PathBuf) -> (StageCtx, Arc<Mutex<Vec<StageEvent>>>) {
        let (mut ctx, seen) = dry_ctx(
            root,
            StageOptions {
                bs_dir_override: Some(bs_dir),
                ..Default::default()
            },
        );
        // `Paths::new(root)` already points gbe_dll at <root>/third_party/gbe.
        ctx.bs_dir = ctx.opts.bs_dir_override.clone().unwrap();
        (ctx, seen)
    }

    #[tokio::test]
    async fn goldberg_dies_when_no_steam_api_dll_exists() {
        let root = scratch("gbe-missing");
        let bs_dir = root.join("empty");
        std::fs::create_dir_all(&bs_dir).unwrap();
        let (ctx, _) = goldberg_ctx(&root, bs_dir.clone());
        let err = goldberg_stage(&ctx).await.unwrap_err();
        assert_eq!(
            err.to_string(),
            format!(
                "steam_api64.dll not found under {} — is this a complete Beat Saber install?",
                bs_dir.display()
            )
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[tokio::test]
    async fn goldberg_backs_up_once_installs_and_writes_the_exact_artifacts() {
        let root = scratch("gbe-install");
        let bs_dir = goldberg_fixture(&root, b"REAL-STEAM", b"GOLDBERG");
        let (ctx, seen) = goldberg_ctx(&root, bs_dir.clone());

        goldberg_stage(&ctx).await.unwrap();

        let api_dir = bs_dir.join("Beat Saber_Data/Plugins/x86_64");
        let plan = ctx.executor.planned();
        let describe: Vec<String> = plan.iter().map(|p| p.describe()).collect();

        // 1. backup, 2. install, 3. appid, 4. mkdir, 5-7. flag files.
        assert_eq!(
            plan.iter().map(|p| p.kind).collect::<Vec<_>>(),
            vec![
                PlannedKind::Copy,
                PlannedKind::Copy,
                PlannedKind::Write,
                PlannedKind::CreateDir,
                PlannedKind::Write,
                PlannedKind::Write,
                PlannedKind::Write,
            ],
            "{describe:#?}"
        );
        assert_eq!(
            plan[0].dst.as_deref(),
            Some(api_dir.join("steam_api64.dll.orig-steam").as_path())
        );
        assert_eq!(
            plan[1].src.as_deref(),
            Some(root.join("third_party/gbe/steam_api64.dll").as_path())
        );
        assert_eq!(
            plan[2].dst.as_deref(),
            Some(api_dir.join("steam_appid.txt").as_path())
        );
        assert_eq!(
            plan[3].dst.as_deref(),
            Some(api_dir.join("steam_settings").as_path())
        );
        for (p, name) in plan[4..].iter().zip(GOLDBERG_FLAG_FILES) {
            assert_eq!(
                p.dst.as_deref(),
                Some(api_dir.join("steam_settings").join(name).as_path())
            );
        }

        assert_eq!(
            texts(&seen.lock().unwrap()),
            vec![
                "-- Goldberg".to_string(),
                format!(
                    "[ok] installed goldberg -> {}",
                    api_dir.join("steam_api64.dll").display()
                ),
            ]
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[tokio::test]
    async fn goldberg_skips_the_backup_when_one_exists_and_reports_already_installed() {
        let root = scratch("gbe-idempotent");
        let bs_dir = goldberg_fixture(&root, b"GOLDBERG", b"GOLDBERG");
        let api_dir = bs_dir.join("Beat Saber_Data/Plugins/x86_64");
        std::fs::write(api_dir.join("steam_api64.dll.orig-steam"), b"REAL-STEAM").unwrap();
        let (ctx, seen) = goldberg_ctx(&root, bs_dir);

        goldberg_stage(&ctx).await.unwrap();

        // No backup copy, no install copy — only the appid + settings writes.
        let plan = ctx.executor.planned();
        assert_eq!(
            plan.iter().map(|p| p.kind).collect::<Vec<_>>(),
            vec![
                PlannedKind::Write,
                PlannedKind::CreateDir,
                PlannedKind::Write,
                PlannedKind::Write,
                PlannedKind::Write,
            ]
        );
        assert_eq!(
            texts(&seen.lock().unwrap()),
            vec![
                "-- Goldberg".to_string(),
                "[info] goldberg already installed".to_string()
            ]
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[tokio::test]
    async fn goldberg_writes_the_appid_digits_with_no_trailing_newline() {
        // The bytes are the parity-critical part, so this one runs for real —
        // against a fixture tree under the temp dir, never the user's game.
        let root = scratch("gbe-bytes");
        let bs_dir = goldberg_fixture(&root, b"REAL-STEAM", b"GOLDBERG");
        let api_dir = bs_dir.join("Beat Saber_Data/Plugins/x86_64");
        let (mut ctx, _) = goldberg_ctx(&root, bs_dir.clone());
        let run_id = ctx.run_id;
        ctx.executor = Arc::new(crate::executor::RealExecutor::new(
            run_id,
            crate::stages::null_sink(),
            CancellationToken::new(),
        ));

        goldberg_stage(&ctx).await.unwrap();

        assert_eq!(
            std::fs::read(api_dir.join("steam_appid.txt")).unwrap(),
            contract().game.appid.to_string().into_bytes(),
            "printf '%s' — no trailing newline"
        );
        assert_eq!(
            std::fs::read(api_dir.join("steam_api64.dll.orig-steam")).unwrap(),
            b"REAL-STEAM",
            "the backup holds the ORIGINAL dll"
        );
        assert_eq!(
            std::fs::read(api_dir.join("steam_api64.dll")).unwrap(),
            b"GOLDBERG"
        );
        for name in GOLDBERG_FLAG_FILES {
            let p = api_dir.join("steam_settings").join(name);
            assert!(p.is_file(), "{name} missing");
            assert_eq!(std::fs::read(&p).unwrap(), b"", "{name} must be empty");
        }

        // Second pass: the backup is not refreshed with the Goldberg dll.
        goldberg_stage(&ctx).await.unwrap();
        assert_eq!(
            std::fs::read(api_dir.join("steam_api64.dll.orig-steam")).unwrap(),
            b"REAL-STEAM"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// A7-5: the live dll is already Goldberg and there is no `.orig-steam`.
    /// run.sh's bytes are unchanged (the backup is still minted — artifact
    /// parity), but the row says what that backup actually holds, so nothing
    /// downstream can call copying it back a restore.
    #[tokio::test]
    async fn goldberg_says_so_when_the_backup_it_mints_is_itself_goldberg() {
        let root = scratch("gbe-already");
        let bs_dir = goldberg_fixture(&root, b"GOLDBERG", b"GOLDBERG");
        let (ctx, seen) = goldberg_ctx(&root, bs_dir.clone());

        goldberg_stage(&ctx).await.unwrap();

        let api_dir = bs_dir.join("Beat Saber_Data/Plugins/x86_64");
        let backup = api_dir.join("steam_api64.dll.orig-steam");
        assert_eq!(
            texts(&seen.lock().unwrap()),
            vec![
                "-- Goldberg".to_string(),
                format!(
                    "[warn] steam_api64.dll was already the Goldberg build, so {} is a copy of \
                     Goldberg — the real Steam dll was never seen here and cannot be restored",
                    backup.display()
                ),
                "[info] goldberg already installed".to_string(),
            ]
        );
        // The backup is still planned — run.sh mints it here too.
        let plan = ctx.executor.planned();
        assert_eq!(plan[0].kind, PlannedKind::Copy);
        assert_eq!(plan[0].dst.as_deref(), Some(backup.as_path()));
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn goldberg_falls_back_to_the_game_root_dll() {
        let root = scratch("gbe-root-dll");
        let bs_dir = root.join("Beat Saber 1294");
        std::fs::create_dir_all(&bs_dir).unwrap();
        std::fs::write(bs_dir.join("steam_api64.dll"), b"REAL").unwrap();
        std::fs::create_dir_all(root.join("third_party/gbe")).unwrap();
        std::fs::write(root.join("third_party/gbe/steam_api64.dll"), b"GBE").unwrap();
        let (ctx, _) = goldberg_ctx(&root, bs_dir.clone());

        goldberg_stage(&ctx).await.unwrap();
        let plan = ctx.executor.planned();
        assert_eq!(
            plan[2].dst.as_deref(),
            Some(bs_dir.join("steam_appid.txt").as_path()),
            "steam_appid.txt lands next to the dll that was found"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    // ── wine env / spec / banner ─────────────────────────────────────────────

    #[test]
    fn wine_env_reproduces_run_shs_exports_including_caller_precedence() {
        let rt = Path::new("/repo/ext/oxrsys/build-x64/runtime/oxrsys-runtime.json");
        let quiet = wine_env(false, None, 620980, rt);
        assert_eq!(
            quiet,
            vec![
                (
                    "XR_RUNTIME_JSON".to_string(),
                    "/repo/ext/oxrsys/build-x64/runtime/oxrsys-runtime.json".to_string()
                ),
                ("CX_GRAPHICS_BACKEND".to_string(), "dxmt".to_string()),
                ("WINEDEBUG".to_string(), "-all".to_string()),
                ("SteamAppId".to_string(), "620980".to_string()),
                ("SteamGameId".to_string(), "620980".to_string()),
            ]
        );

        let verbose = wine_env(true, None, 620980, rt);
        assert_eq!(verbose[2].1, "fixme-all,+openxr");

        // The caller's preset wins in BOTH branches (parity decision 21).
        assert_eq!(wine_env(false, Some("+d3d11"), 1, rt)[2].1, "+d3d11");
        assert_eq!(wine_env(true, Some("+d3d11"), 1, rt)[2].1, "+d3d11");
        // zsh's `${WINEDEBUG:-…}` treats empty exactly like unset.
        assert_eq!(wine_env(false, Some(""), 1, rt)[2].1, "-all");
        assert_eq!(wine_env(true, Some(""), 1, rt)[2].1, "fixme-all,+openxr");
    }

    #[test]
    fn wine_spec_is_run_shs_argv() {
        let root = scratch("wine-spec");
        let b = bottle(&root);
        let bs_dir = b
            .prefix
            .join("drive_c/Program Files (x86)/Steam/steamapps/common/BS");
        let (mut ctx, _) = dry_ctx(
            &root,
            StageOptions {
                bottle_name: Some("Steam".into()),
                bs_dir_override: Some(bs_dir.clone()),
                ..Default::default()
            },
        );
        ctx.bs_dir = bs_dir;
        ctx.paths.wine = Some(PathBuf::from("/Applications/CrossOver.app/x/bin/wine"));

        let spec = wine_spec(&ctx, &b);
        assert_eq!(
            spec.display(),
            "/Applications/CrossOver.app/x/bin/wine --bottle Steam --no-update --cx-app \
             C:\\Program Files (x86)\\Steam\\steamapps\\common\\BS\\Beat Saber.exe"
        );
        assert_eq!(spec.step, step::RUN_LAUNCH);
        let keys: Vec<&str> = spec.env.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            keys,
            vec![
                "XR_RUNTIME_JSON",
                "CX_GRAPHICS_BACKEND",
                "WINEDEBUG",
                "SteamAppId",
                "SteamGameId"
            ]
        );
        assert!(spec.env_path.is_some(), "a Finder-launched .app needs PATH");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_banner_is_run_shs_nine_lines_in_order() {
        let evs = banner_events(
            Uuid::nil(),
            "Steam",
            "Z:\\games\\Beat Saber.exe",
            Path::new("/repo/logs/beatsaber-20260829-101112.log"),
        );
        assert_eq!(
            texts(&evs),
            vec![
                "",
                "-- launching Beat Saber through the bridge",
                "   put the headset ON and open the ALVR client; first frame can take ~30s.",
                "   pause in-game = X/A button or the Quest system button",
                "   (the left-menu-button pause is a Beat Saber/Unity limitation on every OpenXR runtime)",
                "   stop: Ctrl-C here, or ./demo.sh stop --bottle Steam from another shell",
                "   exe: Z:\\games\\Beat Saber.exe",
                "   log: /repo/logs/beatsaber-20260829-101112.log",
                "",
            ]
        );
        // Only the banner headline is a Section; everything else is verbatim Text.
        assert_eq!(
            evs.iter()
                .filter(|e| matches!(e, StageEvent::Section { .. }))
                .count(),
            1
        );
        // Every Text row is attributed to the launch step.
        for ev in &evs {
            if matches!(ev, StageEvent::Text { .. }) {
                assert_eq!(ev.step(), Some(step::RUN_LAUNCH));
            }
        }
    }

    #[test]
    fn eexist_is_the_only_retryable_spawn_error() {
        assert!(is_already_exists(&SabrageError::io(
            "/x",
            std::io::Error::new(std::io::ErrorKind::AlreadyExists, "exists")
        )));
        assert!(!is_already_exists(&SabrageError::io(
            "/x",
            std::io::Error::new(std::io::ErrorKind::PermissionDenied, "nope")
        )));
        assert!(!is_already_exists(&SabrageError::Cancelled));
    }

    #[test]
    fn orig_steam_suffixes_the_whole_name() {
        assert_eq!(
            orig_steam_path(Path::new("/g/steam_api64.dll")),
            PathBuf::from("/g/steam_api64.dll.orig-steam")
        );
    }

    #[test]
    fn survivors_render_as_pid_basename_pairs_with_a_trailing_space() {
        let procs = vec![
            ProcInfo {
                pid: 12,
                start_time: 1,
                exe: PathBuf::from("/x/bin/wineserver"),
            },
            ProcInfo {
                pid: 34,
                start_time: 1,
                exe: PathBuf::new(),
            },
        ];
        assert_eq!(
            format_survivors(&procs, "wineserver"),
            "12 wineserver 34 wineserver "
        );
        assert_eq!(format_survivors(&[], "wineserver"), "");
    }

    // ── wineserver reset ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn wineserver_reset_plans_k_then_w_and_reports_down() {
        let root = scratch("ws-reset");
        let b = bottle(&root);
        let (mut ctx, seen) = dry_ctx(&root, StageOptions::default());
        ctx.paths.wineserver = Some(PathBuf::from("/cx/bin/wineserver"));

        wineserver_reset(&ctx, &b).await.unwrap();

        let plan = ctx.executor.planned();
        assert_eq!(
            plan.iter().map(|p| p.reason.clone()).collect::<Vec<_>>(),
            vec![
                "/cx/bin/wineserver -k".to_string(),
                "/cx/bin/wineserver -w".to_string()
            ]
        );
        assert_eq!(
            texts(&seen.lock().unwrap()),
            vec![
                "-- resetting wineserver for bottle 'Steam'".to_string(),
                "[ok] wineserver down".to_string()
            ]
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[tokio::test]
    async fn wineserver_reset_without_crossover_still_reports_down() {
        let root = scratch("ws-none");
        let b = bottle(&root);
        let (ctx, seen) = dry_ctx(&root, StageOptions::default());
        wineserver_reset(&ctx, &b).await.unwrap();
        assert!(ctx.executor.planned().is_empty());
        assert_eq!(
            texts(&seen.lock().unwrap()).last().map(String::as_str),
            Some("[ok] wineserver down")
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    // ── adb reverse cleanup ──────────────────────────────────────────────────

    #[tokio::test]
    async fn adb_reverse_cleanup_is_silent_without_adb_or_on_the_legacy_protocol() {
        let root = scratch("reverse");
        let (ctx, seen) = dry_ctx(&root, StageOptions::default());
        let alvr = PreflightFacts {
            protocol: "alvr".into(),
            encoder_process: "auto".into(),
        };
        adb_reverse_cleanup(&ctx, &alvr).await.unwrap();
        assert!(seen.lock().unwrap().is_empty());

        let legacy = PreflightFacts {
            protocol: "oxrsys".into(),
            encoder_process: "auto".into(),
        };
        adb_reverse_cleanup(&ctx, &legacy).await.unwrap();
        assert!(seen.lock().unwrap().is_empty());
        assert!(ctx.executor.planned().is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[tokio::test]
    async fn adb_reverse_cleanup_removes_all_reverse_tunnels_on_the_alvr_path() {
        let root = scratch("reverse-alvr");
        let (mut ctx, seen) = dry_ctx(&root, StageOptions::default());
        ctx.paths.adb = Some(fake_adb(
            &root,
            "List of devices attached\n1WMHH0X\tdevice\n",
            0,
        ));
        adb_reverse_cleanup(
            &ctx,
            &PreflightFacts {
                protocol: "alvr".into(),
                encoder_process: "auto".into(),
            },
        )
        .await
        .unwrap();

        let plan = ctx.executor.planned();
        assert_eq!(plan.len(), 1);
        assert!(
            plan[0].reason.ends_with("-s 1WMHH0X reverse --remove-all"),
            "{}",
            plan[0].reason
        );
        assert_eq!(
            texts(&seen.lock().unwrap()),
            vec![
                "[info] Quest 1WMHH0X: cleared adb reverse tunnels (ALVR manages its own)"
                    .to_string()
            ]
        );
        assert_eq!(
            seen.lock().unwrap()[0].step(),
            Some(step::RUN_ADB_REVERSE),
            "rows are attributed to their launch action's step"
        );
        // Severity is `info`, exactly as run.sh:227.
        assert!(matches!(
            &seen.lock().unwrap()[0],
            StageEvent::Line {
                severity: Severity::Info,
                ..
            }
        ));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
