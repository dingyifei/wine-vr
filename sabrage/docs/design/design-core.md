# Sabrage — native Rust pipeline core architecture (`sabrage-core`)

Verified against: `/Users/yifeiding/orca/workspaces/wine-vr/gui/CLAUDE.md`, `demo.sh`, `.gitignore`, branch `dingyifei/gui`, plus the five-sweep pipeline inventory. All demo.sh behavioral references below (check numbering, message text, ordering, timing budgets) come from `scripts/demo/{lib,doctor,setup,build,install,run,stop}.sh`.

---

## 0. Placement and workspace layout

Top-level directory `sabrage/` in the wine-vr repo (branch `dingyifei/gui`):

```
sabrage/
├── Cargo.toml                 # [workspace] members = ["crates/*", "src-tauri"]
├── crates/
│   ├── sabrage-core/          # UI-agnostic pipeline engine (this document)
│   └── sabrage-cli/           # thin CLI over sabrage-core (demo.sh-compatible surface)
├── src-tauri/                 # Tauri 2 app crate ("sabrage-app"), thin command layer
│   ├── tauri.conf.json
│   └── src/main.rs
└── ui/                        # frontend (Vite + whatever framework; out of scope here)
```

`.gitignore` interaction (verified): the repo blanket-ignores `build/`, `*.dll`, `*.so`, `*.dylib`. Cargo emits into `target/`, which is **not** currently ignored — add `sabrage/target/`, `sabrage/ui/node_modules/`, `sabrage/ui/dist/`, and `sabrage/src-tauri/gen/` to `.gitignore`. Do not name any tracked directory `build/`. Note the `*.dylib`/`*.dll` blanket ignores are harmless to Sabrage but mean any bundled test fixtures must not use those extensions.

Repo root discovery: unlike demo.sh (`dirname $0`), Sabrage.app's location is unrelated to the repo. `sabrage-core` takes `repo_root: PathBuf` as explicit configuration (persisted in Sabrage settings, default discovered by walking up from the executable in dev builds, prompted-for in the .app). **Changing `repo_root` must invalidate install state** (the host OpenXR manifest embeds the absolute dylib path under it — this machine is currently live proof: the manifest points at a deleted `27-beta-4` checkout).

---

## 1. `sabrage-core` module layout

```
sabrage-core/src/
├── lib.rs
├── paths.rs          # Paths: the typed port of lib.sh globals (single source of truth)
├── consts.rs         # pins: DEPS_URL, DXMT_TGZ_SHA256, GBE_DLL_SHA256, BS_APPID=620980,
│                     # DXMT_FILES[5], depot triple (620980/620981/6291266771922375922),
│                     # ports 9943/9944, "Beat Saber 1294", host manifest path, timing budgets
├── config/
│   ├── mod.rs
│   ├── runtime_toml.rs   # oxrsys-runtime.toml: typed read + toml_edit comment-preserving write
│   ├── settings.rs       # Sabrage app settings (~/Library/Application Support/Sabrage/settings.json)
│   ├── options.rs        # LaunchOptions ⇄ WINEVR_* env mirror
│   └── session_state.rs  # crash-recovery state (prev audio device, created adb forwards, run id)
├── library.rs        # game-library store (library.json)
├── checks/
│   ├── mod.rs            # registry, CheckId, CheckOutcome, policies
│   ├── ctx.rs            # CheckCtx (Paths + resolved bottle + probes cache)
│   ├── system.rs         # doctor 1, 1-macos, 2
│   ├── bottle.rs         # 3a-3d, 3-z
│   ├── toolchain.rs      # 4, 5
│   ├── sources.rs        # 6, 6-alvr-patch
│   ├── pinned.rs         # 7 (dxmt, goldberg)
│   ├── game.rs           # 8 (+ bs_version)
│   ├── build_outputs.rs  # 9, 9b
│   ├── overlay.rs        # 10, 11, 12
│   ├── runtime_cfg.rs    # 13, 13b
│   ├── headset.rs        # 14, 16b (adb)
│   ├── audio.rs          # 15
│   └── ports.rs          # 16
├── fixes.rs          # FixAction registry (Fix ids referenced by checks)
├── stages/
│   ├── mod.rs            # Stage trait, orchestrator, operation lock
│   ├── setup.rs
│   ├── build.rs
│   ├── install.rs
│   ├── run.rs            # preflight + prepare + guards + launch + supervise + teardown
│   ├── stop.rs
│   └── all.rs            # setup→build→install→run chaining
├── process/
│   ├── mod.rs            # spawn_streamed(): tokio::process + pgroup + line/CR splitting
│   ├── supervise.rs      # ChildHandle, cancellation, grace-kill escalation
│   ├── reap.rs           # exec-path-based process matching (replaces pgrep/pkill -f)
│   └── wine.rs           # CrossOver wine wrapper invocation, wineserver kill/wait budgets
├── privilege.rs      # host-manifest write via osascript admin prompt; TCC denial detection
├── events.rs         # serde event stream (Section 3)
├── logs.rs           # logs/beatsaber-*.log writer (demo.sh-compatible), JSONL event log, rotation-aware tailer
├── telemetry/
│   ├── mod.rs            # SessionMonitor
│   ├── runtime_status.rs # runtime_status.json watcher (notify + staleness)
│   ├── oxrsys_log.rs     # enc1s / encoder-ready / session-state parsers
│   ├── alvr_log.rs       # [GRAPH]/[STATS] parsers (seconds vs milliseconds!)
│   └── probes.rs         # port/lsof + process liveness polls
├── error.rs          # taxonomy (Section 8)
├── util/
│   ├── fsops.rs          # copy_if_changed (cmp semantics), atomic write, dir copy
│   ├── hash.rs           # sha256 tri-state: Missing / Mismatch{got} / Ok
│   ├── download.rs       # fetch_pinned port (reqwest, .tmp + verify + rename, cleanup on fail)
│   ├── macho.rs          # is_thin_or_fat_arm64() via `object` (arm64e must NOT satisfy)
│   ├── winpath.rs        # win_path(): C:\ vs Z:\ rule, trailing-slash prefix match preserved
│   └── versions.rs       # macOS>=27, CrossOver>=26.2 real comparisons
└── parity.rs         # tests + `parity-check` support: assert Rust consts == scripts/demo/lib.sh
```

Key struct — the lib.sh port, built once per operation, no ambient globals:

```rust
pub struct Paths {
    pub root: PathBuf,                       // WINEVR_ROOT equivalent (canonicalized)
    pub cx_app: Option<PathBuf>,             // ~/Applications first, then /Applications
    pub cx: Option<PathBuf>,                 // NEVER the bogus "/Contents/SharedSupport/CrossOver"
    pub wine: Option<PathBuf>, pub wineserver: Option<PathBuf>,
    pub oxr_appsup: PathBuf, pub toml: PathBuf,
    pub host_xr_json: PathBuf,               // /usr/local/share/openxr/1/active_runtime.x86_64.json
    pub oxrsys: PathBuf, pub woxr: PathBuf, pub alvr: PathBuf,
    pub dxmt_art: PathBuf, pub gbe_dll: PathBuf,
    pub oxr_build: PathBuf, pub oxr_dylib: PathBuf, pub oxr_alvr_dylib: PathBuf,
    pub oxr_runtime_json: PathBuf, pub oxr_helper_built: PathBuf, pub oxr_helper_staged: PathBuf,
    pub woxr_dll: PathBuf, pub woxr_so: PathBuf, pub alvr_dashboard: PathBuf,
    pub adb: Option<PathBuf>,                // SDK path wins over PATH; None (not "")
}
pub struct Bottle { pub name: String, pub prefix: PathBuf, pub sys32: PathBuf }  // validated by cxbottle.conf
```

`Option<PathBuf>` for `cx`/`adb` fixes two documented shell traps (bogus `/Contents/...` path; empty-string ADB) at the type level.

---

## 2. The check/step engine

### 2.1 Types

```rust
pub struct CheckId(pub &'static str);   // stable string ids mirroring doctor.sh numbering:
// "doctor.01.arch", "doctor.01.macos-version", "doctor.02.crossover",
// "doctor.03a.bottle-named" … "doctor.03d.graphics-backend", "doctor.03z.z-drive",
// "doctor.04.tool.cmake" … "doctor.05.rustup-target",
// "doctor.06.submodule.oxrsys|wineopenxr|alvr", "doctor.06.alvr-patchset",
// "doctor.07.dxmt-artifacts", "doctor.07.goldberg",
// "doctor.08.beatsaber", "doctor.09.output.<basename>", "doctor.09b.helper-staged",
// "doctor.09b.helper-arm64", "doctor.10.overlay.<basename>", "doctor.11.bottle-dll",
// "doctor.11.bottle-manifest", "doctor.11.active-runtime", "doctor.12.host-manifest",
// "doctor.13.protocol", "doctor.13b.session-pins", "doctor.14.adb-device",
// "doctor.14.alvr-client", "doctor.15.audio", "doctor.16.ports", "doctor.16b.adb-forwards"
// run-only preflights get "run.pre.*" ids: "run.pre.wine-exec", "run.pre.wineserver-reset",
// "run.pre.goldberg-locate", "run.pre.wired-adb"

pub enum CheckStatus { Pass, Warn, Fail, Skipped }

pub struct CheckOutcome {
    pub id: CheckId,
    pub status: CheckStatus,
    pub title: String,          // row label
    pub message: String,        // demo.sh-equivalent text, verbatim where remedies are embedded
    pub detail: Option<String>, // expected-vs-actual hash, parsed library_path, lipo archs, …
    pub remedy: Option<Remedy>,
    pub duration_ms: u64,
}
pub struct Remedy { pub text: String, pub fix: Option<FixId> }  // text = doctor's remedy string
```

### 2.2 Registry: one list, three consumers

Each check is a `CheckDef` registered in a `&'static [CheckDef]` table (order = doctor.sh order, which is load-bearing: section 3 resolves the bottle context before 3z/8/11):

```rust
pub struct CheckDef {
    pub id: CheckId,
    pub group: Group,                 // System, Bottle, Toolchain, Sources, Pinned, Game, Build, Overlay, Runtime, Headset, Audio, Ports
    pub requires: &'static [Cap],     // Cap::Bottle, Cap::CrossOver, Cap::Adb — unmet ⇒ Skipped (mirrors BOTTLE_OK/CX_APP/ADB gating)
    pub run: for<'a> fn(&'a CheckCtx) -> BoxFuture<'a, CheckOutcome>,
    pub launch_policy: LaunchPolicy,  // how run-preflight treats this check
    pub fix: Option<FixId>,
    pub cost: Cost,                   // Cheap (stat) vs Expensive (shasum, cmp, lsof, adb) for scheduling
    pub side_effect: Probe,           // Pure | StartsAdbDaemon | TouchesRustup — gate adb behind a setting
}

pub enum LaunchPolicy {
    NotGating,                        // doctor-only (arch, toolchain, submodules, …)
    Block,                            // die-equivalent (bottle dll stale, host manifest missing, …)
    WarnOnly,                         // BS version, protocol=oxrsys (run warns where doctor FAILs)
    AutoFix(FixId),                   // 3d graphics-backend rewrite, 9b helper restage, 16b forward cleanup
}
```

**How the one registry serves all three consumers:**

- **Doctor** runs the full table (subject to `requires` skipping), collects `Vec<CheckOutcome>`, returns `DoctorReport { outcomes, fail_count, warn_count }`. Never mutates anything (the registry's `run` fns are read-only by contract; auto-fixes live in `fixes.rs`, not in checks). This is exactly the `WINEVR_DOCTOR_SOFT` philosophy made structural: exit codes only exist at the CLI shim.
- **Run preflight** filters to `launch_policy != NotGating`, executes **in run.sh's order** (which differs from doctor order — preflight order is a second static slice of CheckIds), and applies the policy: `Block` ⇒ abort with the check's message as a `Fatal`, `AutoFix(fix)` ⇒ execute the fix, emit a `StepResult { status: AutoFixed }` event, re-run the check, block if still failing. The doctor/run divergences from the inventory (protocol=oxrsys: doctor Fail vs run Warn; graphics backend: doctor Fail vs run auto-fix; host manifest: doctor validates `library_path`, run only checks presence — **Sabrage upgrades run to validate library_path too**, flagged as deliberate divergence) are encoded in this one table instead of two drifting scripts. This directly implements CLAUDE.md:102's rule ("every launch-critical requirement gets both a doctor section and a run preflight") as a single `blocking_for_launch`-style flag.
- **Per-row Fix actions**: `FixId` indexes a second registry:

```rust
pub struct FixDef {
    pub id: FixId,                    // "fix.set-graphics-backend", "fix.restage-helper",
                                      // "fix.remove-adb-forwards", "fix.run-install", "fix.run-setup",
                                      // "fix.run-build", "fix.delete-session-json", "fix.restore-audio",
                                      // "fix.edit-protocol", "fix.create-z-drive"
    pub needs_admin: bool,            // only fix.run-install (layer 4)
    pub destructive: bool,            // confirm dialog (delete-session-json, create-z-drive)
    pub forbidden_while_session_live: bool,
    pub run: fn(&FixCtx) -> BoxFuture<'_, Result<FixReport>>,
}
```

Some fixes are whole stages (`fix.run-install` invokes the install stage through the normal orchestrator, so it streams events like any stage). Fixes are serialized behind the same operation lock as stages and refuse to run while `SessionMonitor` reports a live session.

Checks that must diverge from shell semantics (each flagged in Section 10): all five DXMT overlay files compared (shell checks 2 of 5); real version comparisons instead of `sort -V`/`grep -qx` regex accidents; sections 13b/16b render a green row when clean instead of printing nothing.

---

## 3. Stage orchestration, events, process supervision

### 3.1 Event stream (serde-serializable, consumed by Tauri `emit` and by the CLI renderer)

```rust
#[derive(Serialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    StageStarted  { run_id: Uuid, stage: StageId, ts_ms: u64 },
    StepStarted   { run_id: Uuid, stage: StageId, step: StepId, title: String, index: u32, total: u32 },
    StepOutput    { run_id: Uuid, step: StepId, stream: Stream, chunk: String },   // line- or CR-delimited
    Progress      { run_id: Uuid, step: StepId, current: u64, total: Option<u64>, unit: Unit }, // bytes for downloads, [n/m] for ninja
    CheckResult   { run_id: Uuid, outcome: CheckOutcome },                          // doctor + preflight rows stream in as they resolve
    AutoFixed     { run_id: Uuid, step: StepId, fix: FixId, description: String },  // "bottle graphics backend forced to dxmt"
    StepResult    { run_id: Uuid, step: StepId, status: StepStatus, error: Option<ErrorPayload> },
    StageFinished { run_id: Uuid, stage: StageId, status: StageStatus, exit_code_equiv: i32 },
    NeedsAdmin    { run_id: Uuid, step: StepId, reason: String },                   // pre-announce the auth prompt
    Session       ( SessionEvent ),                                                 // telemetry, Section 7
}
```

`StepId`s are stable strings mirroring the inventory ids (`"setup.1.submodules"`, `"install.4.host-manifest"`, `"run.17.wineserver-reset"`, `"run.18.goldberg"` …), so the JSONL event log is greppable against the shell scripts.

Transport: each operation gets a `tokio::sync::mpsc::Sender<Event>`; the Tauri layer forwards to `app_handle.emit("sabrage://event", &ev)`; the CLI renders human text (reproducing demo.sh's `OK`/`WARN`/`FAIL`/`remedy:` layout, colors gated on `isatty` + `NO_COLOR`). Every event is also appended to `~/Library/Application Support/Sabrage/runs/<run_id>/events.jsonl`.

### 3.2 Stage orchestrator

```rust
#[async_trait]
pub trait Stage {
    fn id(&self) -> StageId;
    async fn execute(&self, ctx: &StageCtx, sink: &EventSink, cancel: &CancellationToken)
        -> Result<StageOutcome, SabrageError>;
}
```

- A single `OperationLock` (tokio `Mutex`) guarantees one mutating operation at a time (stage or fix). Doctor is read-only and may run concurrently, except its adb probes share a small semaphore with everything else that touches adb.
- `all` = the four stages run **in-process, sequentially**, each with a **fresh per-stage report/ctx** (reproducing the process-boundary isolation demo.sh gets from re-exec, per the inventory's porting note), aborting on the first failure with that stage's `exit_code_equiv`; `require_bottle` validation happens first (fail-fast parity).
- `run` is an explicit state machine, because its structure is the subtle part:

```
Preflight(ordered checks + auto-fixes)          — mutations here are intentionally permanent:
  cxbottle.conf backend fix, helper restage,      never unwound (parity with run.sh lines 28–78)
  adb forward create/clear
Prepare                                          — wineserver reset (5s budget, fatal on timeout),
                                                   Goldberg swap, steam_appid.txt, steam_settings/
Guards                                           — AudioGuard + DashboardGuard acquired ONLY here
                                                   (mirrors trap installation at run.sh:161); guard
                                                   state persisted to session_state.json BEFORE the
                                                   mutation so crash recovery can restore audio
Launch                                           — spawn wine; open logs/beatsaber-<ts>.log
Supervise                                        — wait on child ∥ stream output ∥ SessionMonitor
Teardown(reason)                                 — Normal: drop guards only (wineserver stays up —
                                                   deliberate demo.sh parity); Cancelled/Signal:
                                                   stop_wine first, then guards (INT/TERM parity)
```

`AudioGuard`/`DashboardGuard` are RAII structs whose `Drop` is a best-effort sync fallback, but the orchestrator always calls their async `release()` explicitly; their acquisition writes `session_state.json` (previous audio output device name, dashboard pid, wired forwards created, run id) so app restart after SIGKILL/power-loss shows the "previous session did not shut down cleanly" reconciliation banner and can actually restore audio — the single clearest upgrade over `stop.sh`, which can only warn.

### 3.3 Child processes

`process::spawn_streamed(spec: ChildSpec) -> ChildHandle`, built on `tokio::process::Command` with:

- `.kill_on_drop(true)` and `.process_group(0)` (std `CommandExt`, tokio derefs to it) — every build tool runs in its own process group so cancellation can signal the whole tree (`nix::sys::signal::killpg(SIGTERM)`, escalate to `SIGKILL` after a per-spec grace, default 5 s).
- **Exception — the wine launch is never SIGKILLed as cancellation.** Cancelling `run` means the INT-trap path: `wineserver -k` + bounded `-w` wait (the two distinct budgets are named constants: `RUN_WINESERVER_WAIT = 5.0s` fatal, `STOP_WINESERVER_WAIT = 4.0s` non-fatal — kept deliberately distinct per the inventory), then guard teardown, then reap the wrapper if still alive.
- Stdout/stderr piped and split by a custom codec that treats **both `\n` and `\r` as delimiters** (git/curl/cargo/ninja CR progress; ninja `[n/m]` parsed into `Progress` events; download progress comes from reqwest byte counts instead of curl's stderr bar).
- Environment: children get a **constructed PATH** (`/opt/homebrew/bin:/usr/local/bin:$HOME/.cargo/bin:$HOME/Library/Android/sdk/platform-tools:` + system) because a Finder-launched .app inherits a bare PATH; plus the exact demo.sh exports where applicable (`XR_RUNTIME_JSON`, `CX_GRAPHICS_BACKEND=dxmt`, `WINEDEBUG` with user-preset-wins precedence, `SteamAppId`/`SteamGameId=620980`, `WINEPREFIX` only for wineserver/reg-add invocations, never for the launch, `CX_BOTTLE` only for reg-add). Every spawn is logged with full argv + env delta + cwd + duration + exit status (Section 6).
- The wine launch replaces `> >(tee $LOG) 2>&1 &`: sabrage owns the pipes, fans each chunk out to (a) the `StepOutput` event stream, (b) `logs/beatsaber-YYYYmmdd-HHMMSS.log` in the demo.sh location/format (with a `-2` suffix on same-second collision — divergence, flagged), with an explicit flush on exit — strictly better than the tee tail-truncation race. Exit status of the wine child = the run's `exit_code_equiv`.
- Process reaping (`reap.rs`): enumerate via `sysinfo`, match on the **resolved executable path** (equal to `oxr_helper_staged` / `alvr_dashboard`), not `pgrep -f` argv-substring — the inventory calls the shell behavior the riskiest primitive; the divergence is deliberate and the GUI shows what will be killed before killing.

Blocking work (sha256 over dlls, `cmp` byte-compares, tar extraction) runs under `tokio::task::spawn_blocking`; expensive checks run concurrently with per-check timeouts, streaming `CheckResult` rows as they resolve.

---

## 4. Config layer

### 4.1 `oxrsys-runtime.toml` (shared with demo.sh and the runtime — the delicate one)

- **Typed read** (`RuntimeConfig`): parse with `toml_edit` (one parser for read+write; a `toml`-crate deserialization of the same text gives the typed view). v1 exposes only the six **verified** keys: `protocol`, `bitrate_mbps`, `encoder_process`, `video_codec`, `resolution_scale`, `refresh_rate_hz` — everything else in the startup config-dump line is display labels, not verified key names (flagged; must be read out of `ext/oxrsys`'s parser before Settings grows).
- **Create policy = demo.sh parity**: create only if absent, byte-identical to setup.sh's heredoc template (template stored as a string constant, guarded by the parity test in Section 9). Never regenerate, never migrate.
- **Edit policy (the deliberate write-once override)**: the GUI Settings screen edits keys **in place** via `toml_edit::DocumentMut`, preserving all comments, unknown keys, ordering, and whitespace. Rules enforced by `runtime_toml.rs`:
  1. Before any write: copy the current file to `~/Library/Application Support/Sabrage/backups/oxrsys-runtime.toml.<unix-ts>` (keep last 10).
  2. Never author a same-line `#` comment; if the edited value line carries one, move it to its own line above (pre-2026-08 runtime parser bug; flagged as a policy decision — alternative "refuse to edit such lines" rejected as worse UX).
  3. Writes are atomic (temp file + rename in the same directory).
  4. A "Config health" comparison: diff on-disk values vs the last startup config-dump line parsed from `oxrsys-runtime.log`, surfacing parse discrepancies.
  5. `fix.edit-protocol` (doctor 13's real fix — "run setup" can never fix a wrong protocol because setup won't overwrite) goes through this same editor.

### 4.2 Sabrage's own store — `~/Library/Application Support/Sabrage/`

```
Sabrage/
├── settings.json         # repo_root, default bottle, default LaunchOptions, allow_adb_probes,
│                         # ui prefs; serde struct with #[serde(default)] for forward-compat
├── library.json          # game library (below)
├── session-state.json    # crash-recovery: { run_id, prev_audio_output, dashboard_pid,
│                         #   wired_forwards: [{serial, port}], started_at } — written at guard
│                         #   acquisition, cleared on clean teardown
├── backups/              # oxrsys-runtime.toml backups
├── runs/<run-id>/events.jsonl
└── logs/sabrage.log      # human tracing log (daily rotation)
```

JSON (serde_json, pretty) rather than TOML for the store: no human-comment requirement, and atomic-rewrite semantics are simpler. All writes atomic.

### 4.3 Game library

demo.sh has no registry — a "game" is just `BS_DIR`. Sabrage introduces one:

```rust
pub struct GameEntry {
    pub id: Uuid,
    pub name: String,               // "Beat Saber 1.29.4"
    pub bs_dir: PathBuf,
    pub bottle: String,
    pub appid: u32,                 // 620980
    pub detected_version: Option<String>,   // bs_version() port: marker file, else regex scan of globalgamemanagers
    pub overrides: LaunchOptionsPatch,      // per-game partial override of global defaults
}
```

Effective options = `settings.default_options ⊕ game.overrides ⊕ per-launch toggles`. The DepotDownloader command (exact pinned triple from `consts.rs`) is rendered by the Add Game wizard as a copyable block — acquisition is never automated (interactive Steam Guard).

### 4.4 Options ⇄ `WINEVR_*` mirror

```rust
pub struct LaunchOptions { pub bottle: Option<String>, pub bs_dir: Option<PathBuf>,
    pub no_audio: bool, pub no_dashboard: bool, pub wired: bool, pub verbose: bool,
    pub winedebug: Option<String> }   // user-preset WINEDEBUG wins, both branches (parity)
```

- `from_env()` reads `WINEVR_BOTTLE/BS_DIR/NO_AUDIO/NO_DASHBOARD/WIRED/VERBOSE` (any non-empty = true) so `sabrage-cli` is drop-in env-compatible with demo.sh.
- `to_env()` produces the same vars for any child that might consult them, and `to_demo_command()` renders the exact equivalent `./demo.sh run --bottle X …` string for the "copy shell command" affordance — the cheap interchangeability contract with the zsh path.
- CLI accepts the same six flags with the same `exit 2` on unknown-arg/missing-value; help text is hardcoded and **includes `--wired`/`--verbose`** (fixing demo.sh's `sed 2,20p` truncation — divergence, flagged).

---

## 5. Privilege boundary

**Exactly one privileged write in the whole pipeline** (verified across all five sweeps): install layer 4 — creating `/usr/local/share/openxr/1/` and writing `active_runtime.x86_64.json` (root:wheel 0644). Install layers 1–2 (inside CrossOver.app) need **App Management TCC**, not root — sudo does not help there; layer 3 (bottle) and everything in setup/build/run/stop are plain user writes.

**Chosen mechanism: `osascript -e 'do shell script … with administrator privileges'`.** Justification against the alternatives:

- *SMAppService/SMJobBless privileged helper*: correct for a shipping product, but requires a stable Developer ID signature, launchd plist embedding, and an XPC protocol — disproportionate for a macOS-only personal project on ad-hoc/local signing (an ad-hoc-signed helper is exactly the case SMJobBless rejects). Rejected.
- *Embedded `sudo` over a pty*: terrible GUI UX, and the app may have no TTY. Rejected.
- *Instructing the user to run a Terminal command*: kept as the **fallback path only** (render the exact `sudo mkdir -p … && sudo tee …` or `./demo.sh install --bottle <n>` command), used when osascript auth is declined/fails.

Implementation (`privilege.rs`):

1. Build the manifest content **byte-identical to install.sh** (4-space indent, key order `file_format_version` → `runtime{name, library_path}`, trailing newline) so demo.sh's `[ "$(cat …)" = "$WANT" ]` sees it as current and never re-prompts — the two front-ends must converge on identical bytes (parity decision, flagged).
2. Skip entirely when on-disk bytes already match (zero prompts on re-install, same as install.sh). For *staleness detection* elsewhere (doctor 12), compare the **parsed** `library_path` (serde_json) so cosmetic differences warn rather than fail.
3. Write the content to a temp file under the Sabrage app-support dir, then one osascript invocation: `/usr/bin/osascript -e 'do shell script "…" with administrator privileges'` where the inner command is `/bin/mkdir -p /usr/local/share/openxr/1 && /usr/bin/install -m 0644 -o root -g wheel <tmp> /usr/local/share/openxr/1/active_runtime.x86_64.json` — quoting the inner string via a dedicated AppleScript-escaper (paths contain spaces; JSON never rides on the command line). One prompt total (vs demo.sh's up-to-two sudo prompts).
4. Emit `NeedsAdmin` before invoking so the UI explains the prompt ("this writes the host OpenXR registration — one password, only when the repo path changes").

**TCC (App Management) handling** for layers 1–2: detect `io::ErrorKind::PermissionDenied` on writes under a path containing `.app/`, classify as `SabrageError::TccDenied`, and render a dedicated panel (open `x-apple.systempreferences:com.apple.preference.security?Privacy_AppBundles`, explain relaunch requirement, offer the Terminal fallback). This is also the answer to the user's original pain — the .app gets the App Management grant once, then install works from anywhere. Ad-hoc-signing caveat flagged in Section 10.

---

## 6. Debuggability upgrades over zsh (the core complaint)

1. **`tracing` spans everywhere**: one span per stage (`stage = "install"`), nested per step (`step = "install.2.dxmt-overlay"`), per spawned child (`child.argv`, `child.cwd`, `child.env_delta`, `child.exit`, `child.duration_ms`), per check. Subscriber stack (`tracing-subscriber` + `tracing-appender`): human layer → `Sabrage/logs/sabrage.log` (daily rotation) and stderr in CLI dev mode; JSON layer → the per-run `events.jsonl` alongside the Event stream. `RUST_LOG`-style env filter honored.
2. **Structured results as the primary artifact**: `DoctorReport`, `StageOutcome`, and every `CheckOutcome` are serde types; `sabrage-cli doctor --json`, `sabrage-cli run --json` print them; the human renderer is a view over the same data (no screen-scraping ever again).
3. **Dry-run mode**: all mutating primitives (`copy_if_changed`, `download`, atomic write, dir copy, `spawn_streamed`, adb mutations, privileged write) go through an `Executor` trait with `Real` and `DryRun` impls; `DryRun` records a `PlannedAction { kind, src, dst, reason }` list and the CLI prints "would install: …/would skip (unchanged): …" — an honest preview demo.sh cannot give. Read-only probes still execute so the plan is accurate.
4. **Single-check / single-step execution**: `sabrage-cli check doctor.09b.helper-arm64`, `sabrage-cli fix fix.restage-helper`, `sabrage-cli step install.4.host-manifest --dry-run`. This is the debugging story the sourced-zsh design structurally cannot have (stages can't run standalone; lib.sh globals).
5. **No swallowed diagnostics**: the two worst shell offenders are fixed — install.sh's `reg add … >/dev/null 2>&1 || die "reg add failed"` captures and attaches wine's output to the error; cmake configure stdout (suppressed in build.sh) is captured into the step log at debug level.
6. **Never fail-fast implicitly**: like `set -u`-without-`set -e`, every child exit status is checked explicitly at its call site and mapped to a typed error with the demo.sh remedy text — no silent `set -e` aborts with raw git/cmake exit codes and no message.
7. **Explainability**: every `CheckOutcome` carries `detail` (expected vs actual hash, parsed vs expected library_path, lipo arch list) instead of a bare FAIL line.

---

## 7. Session telemetry v1

`telemetry::SessionMonitor` starts with the run stage (and can attach to an externally-started session). All v1 sources are files/polls — no cooperation from oxrsys required:

| Source | Mechanism | Feeds |
|---|---|---|
| `~/Library/Application Support/OXRSys/runtime_status.json` | `notify` watcher + 1 s poll fallback; **liveness = `updated_at_unix_ms` staleness, never file existence** (persists after death); `state` treated as opaque string (enum unverified — flagged) | Status pill; Launch-vs-Stop button state |
| `~/Library/Application Support/OXRSys/oxrsys-runtime.log` | rotation-aware tailer (inode change + truncation detection) in `logs.rs`; regex parsers for `enc1s …` (per-second encoder histogram incl. `drop=` — the "bitrate too high" gauge), `encoder ready <W>x<H> @<R>Hz <B>Mbps (<codec>, <path>)` (HEVC-native vs H.264-inproc chip; mid-session downgrade detection by diffing consecutive lines), `helper pid <n> up`, `Session state -> <n>` (mapped 1..5 to XrSessionState names), `Session::EndFrame streaming enqueue` | Encoder card, degradation banner, session-state strip |
| `…/OXRSys/alvr/session_log.txt` | tail-from-end only (unbounded file); `HH:MM:SS.mmm [GRAPH] {json}` (fields in **seconds**, `bitrate_directives` members `Option<f64>`) and `[STATS] {json}` (fields in **milliseconds** — unit conversion isolated in `alvr_log.rs`; first ~3 s of samples dropped as garbage); timestamps date-anchored via file mtime; repeating `Manual IP connection attempt failed` collapsed into one "stale IP pin — Clear pins" card | Motion-to-photon stacked breakdown, bitrate/loss/battery tiles |
| Game console (own pipe from the wine child) | marker matching: `Metal graphics requirements provided` with no DXMT banner within a timeout ⇒ "graphics backend stall" explanation card | Live console pane + stall diagnosis |
| Probes | periodic `lsof -nP -iUDP:9944 -iTCP:9943` + exec-path process liveness | Port health chip, leftover-process reconciliation |
| Stall heuristic | `Session state == 5` AND no `enc1s` line for >2 s AND battery lines still arriving ⇒ "Stream stalled — wake the headset" (the documented standby-freeze; state alone is a false-healthy signal) | Amber banner |

All parsed samples are re-emitted as `Event::Session(SessionEvent::…)` on the same channel — the Session screen consumes one stream.

**What oxrsys should expose later** (file an issue in `ext/oxrsys`; do not build against it yet): (a) a documented `runtime_status.json` schema + state enum and a write-cadence guarantee; (b) a local status/control endpoint (the embedded ALVR dashboard API on `127.0.0.1:8082` exists but its routes are **unverified** in this worktree — verify in `ext/ALVR/alvr/server_core` before use; if stable it becomes the push-style primary and log tailing the fallback); (c) machine-readable encoder events (JSON lines) instead of the `enc1s` text histogram; (d) confirmation of the `OXRSYS_ENCODER_HELPER` env override semantics.

---

## 8. Error taxonomy and exit-code mapping

```rust
#[derive(thiserror::Error, Debug)]
pub enum SabrageError {
    #[error("invalid input: {0}")]        InvalidInput(String),              // CLI exit 2 (usage parity)
    #[error("{message}")]                 Fatal { message: String,           // die() parity — message text preserved
                                                  remedy: Option<Remedy> },  // CLI exit 1
    #[error("child failed: {argv0} exited {status}")]
                                          ChildFailed { argv0: String, status: i32,
                                                        tail: Vec<String> }, // last N output lines attached
    #[error("cancelled")]                 Cancelled,                          // CLI exit 130
    #[error("administrator authorization declined")] AdminDeclined,
    #[error("macOS App Management permission denied for {path}")]
                                          TccDenied { path: PathBuf },
    #[error("download failed: {url}")]    Download { url: String, source: reqwest::Error },
    #[error("sha256 mismatch for {label} (got {got})")] HashMismatch { label: String, got: String },
    #[error(transparent)]                 Io(#[from] std::io::Error),
}
```

Mapping:

| Outcome | GUI | CLI exit code |
|---|---|---|
| `InvalidInput` | inline validation / toast | **2** (demo.sh usage parity) |
| `Fatal { remedy }` | red banner; remedy rendered as an action button when it maps to a `FixId`, else text | **1** |
| Doctor report | grouped check rows; sticky footer "all passed — Run" / "N failed" | **`min(fail_count, 255)`** (parity: doctor exit = FAIL count; identical to zsh for <256, and the mod-256 wraparound is a shell bug not worth reproducing — flagged) |
| Run finished | "wine exited with status N (log: …)" + teardown summary | **wine's exit status** |
| `Cancelled` (user stop) | neutral "stopped" state | 130 (INT parity) |
| Setup/Build/Install/Stop success | green stage completion + "next:" CTA | 0 |
| `TccDenied` / `AdminDeclined` | dedicated permission panels (Section 5) | 1 with a distinct machine-readable `error.kind` in `--json` |

`ErrorPayload` (the serde projection used in events) always carries `{ kind, message, remedy_text, fix_id }` so the GUI never parses message strings. Where demo.sh embeds remedies in FATAL text (`— ./demo.sh install --bottle <name>`), the Rust error keeps the verbatim text **and** the structured `FixId`.

---

## 9. Parity mechanism with demo.sh (keeping zsh alive without doubling maintenance)

The drift risk is concentrated in a small set of shared literals and formats. Strategy:

1. **`parity.rs` tests** (run in CI-less repo via `cargo test`): parse `scripts/demo/lib.sh` textually and assert Rust `consts.rs` matches — `DEPS_URL`, both sha256 pins, `BS_APPID`, the 5-element `DXMT_FILES` array, `HOST_XR_JSON`, the default BS dir leaf `Beat Saber 1294`, the depot triple in `require_bottle`. Also assert the toml template in `runtime_toml.rs` equals setup.sh's heredoc, and the host-manifest format string equals install.sh's `WANT` construction. Cheap, read-only, and it makes "update lib.sh" automatically break the Rust build until synced (or vice versa).
2. **Shared on-disk contracts, byte-for-byte**: host manifest bytes, toml template, `.sha256` provenance marker (hash + newline), `cxbottle.conf` line format `"CX_GRAPHICS_BACKEND" = "dxmt"` with exact spacing (doctor greps it anchored), helper staging location, `logs/` naming, Goldberg artifacts (`steam_appid.txt` with **no trailing newline**). Sabrage and demo.sh must each see the other's output as "unchanged/current".
3. **`sabrage-cli parity-check`**: runs (1) at runtime plus a behavioral spot-check (doctor row count/ids vs a golden list), for use after stacking demo.sh updates.
4. demo.sh remains the reference for message text; Rust preserves verbatim message/remedy strings so docs quoting them stay true.

---

## 10. Flagged ambiguities and porting decisions (consolidated)

**Unverified — must read submodule source before depending on it** (submodules not checked out in this worktree):
1. `runtime_status.json` state enum / vocabularies / write cadence (only `"idle"` observed) → treat as opaque + staleness in v1.
2. ALVR dashboard API routes on `127.0.0.1:8082` → not used in v1.
3. `OXRSYS_ENCODER_HELPER` env semantics → not exposed in v1.
4. Full toml key list beyond the six verified keys (startup-dump labels ≠ key names) → Settings v1 limited to the six; `render_device` hidden unless protocol=oxrsys.

**Deliberate divergences from demo.sh (improvements, each noted in code with a `// DIVERGENCE:` marker):**
5. Compare all 5 DXMT overlay files (shell: 2 of 4 pairs in doctor, 1 in run).
6. Run preflight validates host-manifest `library_path`, not just presence (repo-moved detection — currently a live failure on this machine).
7. Exec-path process matching instead of `pgrep/pkill -f` argv regex.
8. Delete `.tmp` on download/hash failure (fetch_pinned leaves it).
9. Log filename collision suffix; explicit flush (fixes tee truncation race).
10. Real version comparisons (macOS ≥27, CrossOver ≥26.2) instead of `sort -V` + `grep -qx` regex accident.
11. Help text includes `--wired`/`--verbose`; doctor 13b/16b render green rows when clean.
12. Persisted audio-restore state → Stop can actually restore audio after a crash; "Restore original steam_api64.dll" offered (`.orig-steam` exists but nothing in the repo restores it).
13. Doctor exit capped at 255 (no mod-256 wraparound).
14. `reg add` output captured, not discarded.

**Parity decisions (must NOT change):**
15. Write-once creation of `oxrsys-runtime.toml`; edits only in-place, comments preserved, own-line comments only.
16. Host manifest byte-identical; skip-when-current before any admin prompt.
17. Run's permanent-vs-guarded mutation boundary (backend fix/helper restage/Goldberg = permanent; audio/dashboard = guarded); normal exit leaves wineserver alive.
18. Two distinct wineserver budgets (5 s fatal / 4 s soft); helper respawn budget facts (1/30 s, pin after 2) are runtime-side, display-only.
19. `adb forward --remove` per-serial for exactly tcp:9943/9944, never `--remove-all`; `adb reverse --remove-all` is fine. adb device = first row with state exactly `device`.
20. Goldberg hash-mismatch tolerated at run when the file exists; setup keeps a non-pinned dll.
21. WINEDEBUG precedence: caller-preset wins in both verbose branches.
22. `win_path` trailing-slash glob semantics (`$PREFIX/drive_c/` prefix; exact-`drive_c` falls to `Z:`); `Z:` + whole path with `/`→`\`.
23. `helper_is_arm64`: executable bit AND an arm64 slice, where `arm64e` alone does **not** satisfy (via `object`: cputype ARM64, subtype ≠ ARM64E) — fat x86_64+arm64 passes, matching `grep -qw`.
24. system.reg lazy-flush window: post-reg-add re-probe is Warn, never Fail.
25. cxbottle.conf three-branch edit logic and exact target line; refuse while the bottle's wineserver is live (shell races the CrossOver GUI — Sabrage adds the guard).

**Open judgment calls surfaced to the user during implementation:**
26. Ad-hoc signing vs a free-account Developer certificate: TCC App Management prompting is unreliable with ad-hoc signatures and re-prompts on identity change — needs an early empirical test; fallback (Terminal command) exists either way.
27. Whether Sabrage subsumes the ALVR dashboard by default (`no_dashboard = true` once telemetry v1 lands) — recommended, with an "open legacy dashboard" escape hatch.
28. Bundling strategy for `sabrage-cli` (separate binary vs `Sabrage.app/Contents/MacOS` sidecar) — separate binary recommended so demo.sh users never need the .app.

**Crate manifest (sabrage-core):** `tokio` (rt-multi-thread, process, io-util, sync, time, fs), `tokio-util` (CancellationToken), `serde`/`serde_json`, `toml_edit` (+ `toml` for typed reads), `tracing`/`tracing-subscriber`/`tracing-appender`, `thiserror`, `reqwest` (stream), `sha2` + `hex`, `flate2` + `tar`, `object` (Mach-O arch), `sysinfo` (exec-path process matching), `nix` (killpg/signals), `notify` (file watching), `regex`, `plist` (CrossOver Info.plist version), `uuid`, `time`, `dirs`, `async-trait`, `futures`. App: `tauri` 2.x. CLI: `clap` 4, `owo-colors` (isatty-gated).

**Suggested implementation order:** `paths`/`consts`/`util` + parity tests → checks registry + doctor (read-only, immediately useful) → events + CLI renderer → stop → run (preflight → guards → launch → supervise) → setup/build (long-running streaming) → install + privilege → config editor + library → telemetry v1 → Tauri shell.

### Critical Files for Implementation
- /Users/yifeiding/orca/workspaces/wine-vr/gui/scripts/demo/lib.sh — single source of truth for every path, pin, and helper the Rust `paths.rs`/`consts.rs`/`util/` must mirror (and the parity-test target)
- /Users/yifeiding/orca/workspaces/wine-vr/gui/scripts/demo/run.sh — the run state machine, preflight order, trap/guard boundary, and launch env that `stages/run.rs` must reproduce
- /Users/yifeiding/orca/workspaces/wine-vr/gui/scripts/demo/doctor.sh — check numbering, message/remedy text, and skip-gating for the `checks/` registry
- /Users/yifeiding/orca/workspaces/wine-vr/gui/scripts/demo/install.sh — the 4-layer install, byte-exact host-manifest format, and the sole privileged write for `privilege.rs`
- /Users/yifeiding/orca/workspaces/wine-vr/gui/demo.sh — CLI surface, `WINEVR_*` env mirror, and exit-code conventions `sabrage-cli` must stay compatible with