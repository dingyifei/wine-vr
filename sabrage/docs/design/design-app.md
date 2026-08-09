# Sabrage — Tauri 2 Application Shell & Frontend Design Plan

Grounded in: `/Users/yifeiding/orca/workspaces/wine-vr/gui/CLAUDE.md`, `demo.sh`, `scripts/demo/*` (all read), `.gitignore`, `.gitmodules`, the full pipeline inventory, and live probes of this machine (node v26.5.1 via Homebrew, cargo 1.97.1 via Homebrew, **no rustup/volta/mise on PATH in a non-login shell**, branch `dingyifei/gui`). The mockup files (`Sabrage.dc.html`, `styles.css`) are not in the repo; this plan assumes they are copied in during Phase 0 with `styles.css` landing verbatim.

---

## 1. Repo layout

Keep the wine-vr root exactly as it is (the repo root *is* the demo pipeline; `WINEVR_ROOT` derivation, `scripts/demo/`, `logs/`, `evidence/` are all load-bearing). Sabrage gets one top-level directory that is simultaneously the npm root and the Cargo workspace root — **no `Cargo.toml` at the repo root**, so `demo.sh` users and the submodule workflow never see Rust tooling.

```
wine-vr/
├── demo.sh                      # untouched, stays the reference implementation
├── scripts/demo/                # untouched
├── ext/ …                       # untouched
└── sabrage/
    ├── Cargo.toml               # [workspace] members = ["src-tauri", "crates/sabrage-core"]
    ├── Cargo.lock               # TRACKED
    ├── package.json             # frontend deps + "tauri" scripts (@tauri-apps/cli pinned)
    ├── package-lock.json        # TRACKED
    ├── .mise.toml               # pins node LTS (see §7)
    ├── vite.config.ts
    ├── index.html
    ├── src/                     # Svelte 5 frontend
    │   ├── styles.css           # the mockup design system, copied verbatim
    │   ├── main.ts
    │   ├── ipc.ts               # hand-mirrored types + typed invoke/listen wrappers (§3)
    │   ├── stores/              # appState, session, doctor, logs (runes-based)
    │   ├── screens/             # About, Library, EditGame, Session, Doctor, Logs, Settings
    │   ├── components/          # Sidebar, GateModal, SetupWizard, AddGameWizard, charts/
    │   └── charts/              # LineChart, AreaChart, StackedBar (hand-rolled SVG, §5)
    ├── crates/
    │   └── sabrage-core/        # the native pipeline reimplementation — ZERO tauri deps
    │       ├── src/paths.rs     # the lib.sh port: Paths struct built once per op; Option<PathBuf> for CX_APP/ADB
    │       ├── src/pins.rs      # DEPS_URL, both sha256 pins, BS_APPID, depot triple, DXMT_FILES — single source
    │       ├── src/config.rs    # oxrsys-runtime.toml read (lenient) + write (toml_edit, §4 Settings)
    │       ├── src/checks.rs    # ONE check registry with blocking_for_launch flags (doctor + run preflight)
    │       ├── src/stages/      # setup.rs, build.rs, install.rs, run.rs, stop.rs
    │       ├── src/process.rs   # child spawn + merged line streaming + exec-path-matched reaping (libproc/sysinfo)
    │       ├── src/telemetry.rs # runtime_status.json watcher, session_log.txt GRAPH/STATS/enc1s parsers
    │       ├── src/audio.rs     # SwitchAudioSource wrapper + persisted previous-device
    │       ├── src/adb.rs       # devices/forward/reverse parsing (never forward --remove-all)
    │       ├── src/admin.rs     # osascript privileged write of the host manifest (byte-parity with install.sh)
    │       └── src/bin/sabrage-cli.rs  # thin CLI: `sabrage-cli doctor|paths|parity` for diffing against demo.sh
    └── src-tauri/
        ├── Cargo.toml           # depends on sabrage-core
        ├── tauri.conf.json
        ├── capabilities/main.json
        ├── icons/
        └── src/ { main.rs, commands.rs, events.rs, state.rs, menu.rs }
```

**Why a separate `sabrage-core` crate:** the Rust pipeline logic must be testable and debuggable without a webview (the whole point vs demo.sh), and `sabrage-cli` gives a terminal-runnable parity surface — `sabrage-cli doctor --bottle Steam` can be diffed against `WINEVR_DOCTOR_SOFT=1 ./demo.sh doctor --bottle Steam` while both stay maintained. A unit test in `pins.rs` greps `../../scripts/demo/lib.sh` at test time and asserts the Rust constants equal the shell's `DEPS_URL`/`DXMT_TGZ_SHA256`/`GBE_DLL_SHA256`/`BS_APPID` — a zero-cost drift guard that keeps `lib.sh` the human-readable source of truth.

**App-owned state** goes to `~/Library/Application Support/Sabrage/state.json` (library entries, per-game overrides, previous-audio-device persistence, last-session summaries, UI prefs). Never into `~/Library/Application Support/OXRSys/` — that dir is owned by the runtime, and `oxrsys-runtime.toml` keeps its write-once contract (§4 Settings).

**.gitignore additions** (append; the existing blanket `build/`, `*.log`, `*.dylib` rules don't conflict since Vite outputs `dist/` and all Rust binaries live under `target/`):

```gitignore
# Sabrage (Tauri app)
sabrage/node_modules/
sabrage/dist/
sabrage/src-tauri/target/
sabrage/src-tauri/gen/
```

`Cargo.lock` and `package-lock.json` are deliberately tracked (longevity: reproducible rebuild after a toolchain wipe). Runtime logs Sabrage writes go to the existing gitignored `logs/` with the demo.sh filename format plus a uniquifying suffix (`beatsaber-YYYYmmdd-HHMMSS-sabrage.log`) so the two front-ends' artifacts interleave naturally.

---

## 2. Frontend framework: **Svelte 5 + TypeScript + Vite** (plain Vite, not SvelteKit)

Decision rationale against the two alternatives:

- **vs vanilla TS:** the app has real reactive surfaces — doctor rows streaming in, a gate modal checklist, 1 Hz chart ticks, log tailing, a session state machine feeding both the sidebar footer and the Session screen. In vanilla you end up hand-writing a mini store/render framework, which becomes the least-maintained dependency in the repo. Svelte's compiler output *is* vanilla JS with no framework runtime to churn under you.
- **vs React:** JSX diverges from the approved template-based HTML mockup; React + its ecosystem is the highest-churn option and the heaviest runtime for zero benefit at this scale.
- **Why Svelte specifically fits this mockup:** Svelte components are HTML templates with sprinkled `{expressions}` — the `Sabrage.dc.html` markup ports nearly mechanically, and the design system stays a single **global** `styles.css` imported once in `main.ts` (do *not* use Svelte scoped styles for design-system classes; components use the mockup's class names as-is).

**Churn containment** (the machine repeatedly loses brew toolchains):
- Exactly two meaningful dev dependencies: `vite` + `@sveltejs/vite-plugin-svelte` (plus `svelte`, `typescript`, `@tauri-apps/api`, `@tauri-apps/cli`). All exact-pinned in `package-lock.json`; restore with `npm ci`, never `npm install`.
- Node comes from **mise**, not brew (§7) — a brew wipe cannot take the frontend toolchain with it.
- The *built* app needs no node at all: `frontendDist` is a static `dist/` folder; a compiled `Sabrage.app` keeps working through any toolchain loss.
- No SvelteKit: no SSR/router machinery to migrate. Routing is a `$state` enum over seven screens — the mockup's "small state graph" literally.

---

## 3. IPC design

Rule of thumb: **Tauri commands** for request/response, **`tauri::ipc::Channel<T>`** for per-operation streams (ordered, scoped to one invocation, auto-cleaned), **global events** only for app-wide broadcast state that multiple screens consume. Never expose shell/fs to the webview — all process and file work happens inside `sabrage-core` behind commands (this is also what keeps the capability file tiny, §6).

### Command surface (src-tauri/src/commands.rs)

| Command | Kind | Notes |
|---|---|---|
| `get_app_state()` | req/resp | Paths resolution snapshot: repo root, CrossOver path+version, bottle list, adb path, config summary, sidebar footer strings. Recomputed on window focus. |
| `run_doctor(on_event: Channel<DoctorEvent>) -> DoctorReport` | streaming | Checks run concurrently with per-check timeouts; rows stream in as they resolve. adb-touching checks gated by a setting (they start the adb daemon / can prompt on-headset). |
| `run_stage(stage, opts, on_event: Channel<StageEvent>) -> StageResult` | streaming | setup/build/install; long-running; cancellable except the marked non-interruptible windows (tar swap, stock backup — done as temp-dir + atomic rename, strictly better than the shell). |
| `launch(game_id, opts, on_event: Channel<StageEvent>) -> LaunchResult` | streaming | Runs the ordered run.sh preflight set (with its auto-fixes surfaced as `StepFixed` rows) then spawns wine; session lifecycle continues via global events. |
| `stop_session() -> StopReport` | req/resp | stop.sh port: wineserver -k + 4s bounded wait, survivor report, exec-path-matched reaping, audio restore **from persisted state** (the genuine improvement over demo.sh). |
| `fix(action: FixAction) -> FixResult` | req/resp | Doctor "Fix" buttons: enum, not strings — `SetBackendDxmt`, `RestageHelper`, `RemoveAdbForwards`, `DeleteSessionJson`, `RunInstall`, `RestoreAudio`, `CreateZDrive`… each idempotent, each refuses while a session is live. |
| `start_log_tail(source, on_event: Channel<LogBatch>) -> TailId` / `stop_log_tail(id)` | streaming | Sources: current wine console, `oxrsys-runtime.log` (rotation/inode-aware), `alvr/session_log.txt` (tail-from-end only), past `logs/beatsaber-*.log`. Batched ~50 ms. |
| `get_library() / save_game / remove_game` | req/resp | `Sabrage/state.json` CRUD. |
| `read_runtime_config() -> RuntimeConfig` / `write_runtime_config(patch)` | req/resp | toml_edit-backed (§4 Settings policy). |
| `get_session_status()` | req/resp | Poll fallback for the event below. |
| `copy_equivalent_command(op) -> String` | req/resp | The "copy the demo.sh command" affordance — kept everywhere for parity culture. |

### Typed payloads (the load-bearing part)

```rust
#[derive(Serialize, Clone)] #[serde(tag = "kind", rename_all = "camelCase")]
pub enum StageEvent {
  SectionStarted { title: String },                       // "-- global DXMT overlay …"
  Line { severity: Severity, text: String, remedy: Option<String> }, // info/ok/warn/fail — remedy is a FIRST-CLASS field
  Progress { label: String, done: u64, total: Option<u64> }, // downloads (reqwest byte counts), ninja [n/m], git %
  StepFixed { label: String },                            // "bottle graphics backend forced to dxmt", "helper restaged"
  NeedsAdmin { reason: String },                          // precedes the osascript prompt
  Fatal { message: String, remedy: Option<String> },      // die() equivalent — message text preserved verbatim
  Finished { ok: bool },
}

#[derive(Serialize, Clone)] #[serde(rename_all = "camelCase")]
pub struct DoctorEvent { pub group: GroupId, pub check: CheckId,
  pub status: CheckStatus /* Ok|Warn|Fail|Info|Skipped|Running */,
  pub title: String, pub detail: Option<String>,
  pub remedy: Option<String>, pub fix: Option<FixAction> }
```

Global events (broadcast, `app.emit`): `session://status` (`{ phase: Idle|Preflight|Launching|Running|Stalled|Stopping|Exited, encoder: Option<{codec, path, pid}>, bottle, startedAt, exitCode? }` — drives the sidebar dot, the Session pill, and the Stop button everywhere) and `doctor://badge` (`{ fails: u32 }` — the sidebar nav dot). Everything else is Channels.

The session watcher task derives `phase` from **multiple signals**, per the inventory: wine child liveness, `runtime_status.json` `updated_at_unix_ms` staleness (never file existence), the `encoder ready …` log line (HEVC-native vs H.264-inproc chip), `enc1s` freshness vs battery-line freshness (the standby-freeze `Stalled` heuristic), and the `lsof` port probe.

### Type mirroring: **hand-written, single-file pair** — not tauri-specta (v1)

The surface is small (~13 commands, ~8 payload types) and this project's culture is already "two parallel implementations kept honest by discipline + parity checks" (demo.sh ↔ sabrage-core). `sabrage/src/ipc.ts` mirrors `src-tauri/src/events.rs` 1:1 with `#[serde(rename_all = "camelCase", tag = "kind")]` conventions, and a Rust test serializes one fixture of every payload into `sabrage/src/ipc.fixtures.json`, which a vitest (or a plain `tsc` typecheck of the imported fixtures) consumes — drift fails a test, not a user. tauri-specta is a fine later upgrade but historically lags Tauri releases; on a churn-averse machine, a proc-macro codegen stack is the wrong first dependency.

---

## 4. Screen-by-screen build plan

Global shell: `App.svelte` = sidebar + `<main>` switching on a `screen` rune; sidebar footer subscribes to `session://status` + `get_app_state()`; Doctor nav dot subscribes to `doctor://badge`. Doctor auto-runs on app launch and on window focus (it's the one read-only stage — safe on a timer), with adb probes behind a toggle.

| Screen | Components | Backed by | Empty / loading / error states (not in mockup) | Honest v1 stub |
|---|---|---|---|---|
| **About** | Static template: h1, pipeline chip flow, credits grid, two buttons | Static content + `get_app_state()` for the version line (bridge date from git, "ALVR v20.14.1") | None needed (fully static) | Fully real in Phase 0 |
| **Doctor** | `DoctorScreen` → `CheckGroup` → `CheckRow` (ok/warn/fail/waiting/spinner already in the design system), summary bar, Run button | `run_doctor` channel; Fix buttons → `fix(FixAction)`; groups mirror doctor.sh's 18 sections incl. conditional rows (z: drive, adb forwards — decide: render green rows where the CLI is silent, labeled "not applicable", a deliberate divergence) | Initial: all rows `waiting`, auto-run on mount. No bottle selected: bottle-scoped groups greyed with "select a bottle", not FAIL rows. Check timeout: row shows `warn` + "timed out". | Fully real in Phase 1 — this is the walking skeleton's proof |
| **Pre-launch gate modal** | `GateModal`: sequential checklist + progress counter, auto-fix rows render as "Fixed automatically", fatal row shows remedy + deep-link (e.g. "Run install") | `launch()` channel — the run.sh preflight set with `die` semantics, NOT the doctor set | Fatal → modal stays with red row + action button; Cancel allowed between steps only | Phase 2 shows setup/build/install through the same modal component before launch exists |
| **Session** | Status tag + Stop; 6 stat cards; `LineChart` (m2p/encoder/vsync + dashed p95), `AreaChart` (drops + burst threshold), `StackedBar` (pipeline ms); footer client info | `session://status`; stats from a `subscribe_session_stats` channel fed by telemetry.rs (GRAPH seconds→ms conversion, STATS ms, `enc1s` drops); client info from `session.json` `client_connections` + views-config log line | Empty state (mockup has it). **Degraded state**: session live but no stats yet → status pill + encoder chip + live console tail, charts show "waiting for ALVR stats (first frame ~30 s)". `Stalled` → amber "Stream stalled — wake the headset" banner. Exited non-zero → "wine exited with status N" card + log link. | Before Phase 5: status pill + encoder chip + console tail are real; chart area shows a labeled placeholder "Telemetry parsing lands in a later milestone — raw stream in Logs". Never fake data. |
| **Library** | Table + expandable row; Run / Edit / Doctor buttons; Add game; footer explainer | `get_library()`; per-row validity re-checked on expand (`Beat Saber.exe` present, `bs_version()` port, bottle cxbottle.conf, DXMT/backend state); Run → gate modal | Empty: "No games yet" + Add game + DepotDownloader pointer. Invalid path: red badge + Edit deep-link. Last-session column: "—" until summaries exist. | Real from Phase 4 (persistence); Phase 0–3 shows a single implicit entry derived from settings (bottle + bs-dir) so Run works before the library exists |
| **Edit game** | Identity&paths card (Browse → dialog plugin), Patches card (Goldberg toggle + sub-flags + AppID), per-game streaming overrides (use-global checkbox) | `save_game`; folder picker via dialog plugin; overrides live ONLY in state.json — the runtime toml stays global (overrides applied at launch time where demo.sh parity allows, i.e. env/flags; toml-only knobs show "global setting" lock icons) | Validation inline: exe missing, version ≠1.29.4 warn (glob `1.29.4*`), outside-drive_c → z:-drive check row | Phase 4 |
| **Logs** | Filter segmented control (console/runtime/alvr/past runs), monospace tail, footer path + verbose hint | `start_log_tail` channels; past-run list = `logs/` dir listing sorted by mtime | No logs yet: placeholder showing the logs dir path. Rotation mid-tail: handled in telemetry.rs, seamless. Huge files: tail-from-end only, never full reads. | Phase 3 (console + runtime log); alvr tail joins in Phase 5 |
| **Settings** | Streaming card (bitrate slider, codec seg, encoder seg, protocol seg, res scale, refresh), Audio&launch toggles (no-audio, no-dashboard, wired, verbose → WINEDEBUG input honoring user precedence), Paths card (bottle dropdown from Bottles dir, bs-dir Browse, write-once note) | `read_runtime_config` / `write_runtime_config`; toggles → state.json; a "copy equivalent demo.sh flags" line | toml missing → "Run setup first" state + button. Protocol=oxrsys → persistent legacy banner, audio+dashboard toggles greyed. Parse-vs-runtime mismatch: diff the on-disk toml against the newest startup config-dump line, flag discrepancies. | Phase 4 |
| **Setup / Add-game wizards** | Modal step flows reusing `GateModal` internals; Add-game: DepotDownloader copy-command panel (never automated — needs the user's Steam login), destination picker, validation checklist, "Add to library" | `run_stage(Setup)` channel; validation = same library checks | Network failure mid-setup → Fatal row with retry; tmp files cleaned on failure (improvement over fetch_pinned) | Phase 6 (raw stage buttons exist from Phase 2) |

**Settings write policy for `oxrsys-runtime.toml`** (the deliberate divergence from write-once, called out in-app): create-if-absent writes the byte-identical setup.sh template; edits go through `toml_edit` mutating values in place, preserving all comments and unknown keys, emitting comments **on their own lines only** (pre-2026-08 runtime parser bug); first-ever edit shows a one-time confirm ("demo.sh treats this file as write-once; Sabrage will edit values in place") and snapshots a backup to `Sabrage/backups/`. Keys whose names are only known from the runtime's display labels (`encoder_10bit` etc.) stay hidden until verified against `ext/oxrsys` source (checkout required — flagged UNVERIFIED in the inventory).

---

## 5. Charts: **hand-rolled SVG**, no library

The data is 1 Hz over a 48-sample window (≤ ~150 points on screen across all charts). Three tiny Svelte components — `LineChart` (polylines + dashed p95 twin), `AreaChart` (filled path + threshold rule), `StackedBar` — each ~50 lines: a ring-buffer `$state` array in the session store, `$derived` points-string computation, fixed `viewBox` with CSS-token colors from the verbatim `styles.css` (the mockup already draws exactly these). No axes engine needed (the mockup uses sparse tick labels). A chart lib (uPlot et al.) buys nothing at this rate and adds the exact kind of dependency this project is trying not to carry. Revisit only if a future feature needs >10 Hz or thousands of points.

One unit rule enforced in `telemetry.rs`, not the frontend: GRAPH fields are **seconds**, STATS are **milliseconds** — everything crossing IPC is milliseconds, converted once in Rust.

---

## 6. Tauri specifics

**Identity & config** — `identifier: "dev.dingyifei.sabrage"` and never change it (TCC grants key off bundle id + signing identity). `tauri.conf.json` sketch:

```jsonc
{
  "productName": "Sabrage",
  "identifier": "dev.dingyifei.sabrage",
  "app": {
    "windows": [{
      "title": "Sabrage", "width": 1280, "height": 840,
      "minWidth": 1080, "minHeight": 720,
      "titleBarStyle": "Overlay", "hiddenTitle": true
    }],
    "security": { "csp": "default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:" }
  },
  "bundle": {
    "macOS": { "signingIdentity": "Sabrage Dev", "minimumSystemVersion": "14.0" },
    "icon": ["icons/icon.icns"]
  },
  "build": { "frontendDist": "../dist", "devUrl": "http://localhost:5173",
             "beforeDevCommand": "npm run dev:vite", "beforeBuildCommand": "npm run build:vite" }
}
```

**Window chrome:** the real window uses `titleBarStyle: Overlay` — native traffic lights float over the sidebar's top-left; the mockup's `MacWindow` fake chrome is **not ported**. Add `data-tauri-drag-region` to the sidebar header band and give the wordmark ~52 px top padding so the lights don't collide. Fonts (Barlow/Barlow Condensed) are bundled locally in `src/fonts/` — no Google Fonts at runtime (offline app, CSP `'self'`).

**Capabilities** (`capabilities/main.json`) — deliberately tiny because *all* fs/process work lives in Rust commands (custom commands are not gated by the plugin ACL):

```json
{ "identifier": "main", "windows": ["main"], "permissions": [
    "core:default",
    "dialog:allow-open",
    "opener:allow-open-url",
    "opener:allow-reveal-item-in-dir",
    "clipboard-manager:allow-write-text",
    "notification:default"
] }
```

No `fs` plugin, no `shell` plugin (opener covers "reveal in Finder" + upstream-credit links; DepotDownloader command copy uses clipboard-manager). Plugins registered in Rust: `dialog`, `opener`, `clipboard-manager`, `notification`, `single-instance` (registered **first**, focuses the existing window), optionally `window-state`.

**Menu bar** (`menu.rs`): App menu (About Sabrage, Settings… ⌘,, Quit); **Edit menu with the predefined clipboard items — required or ⌘C/⌘V dies in webview inputs on macOS**; Pipeline menu (Run Doctor ⌘D, Setup…, Build, Install…, Launch ⌘R, Stop ⌘., ─, Open Logs Folder ⇧⌘L, Open Config Folder); Window; Help (Troubleshooting doc).

**Icon:** 1024 px source art (champagne saber slicing a cork — Beat Saber wink) run through `cargo tauri icon`; keep the source PNG at `src-tauri/icons/source.png`.

**Signing + App Management TCC (the reason a real .app exists):**
- **Do not ship ad-hoc for daily use.** Ad-hoc signatures (`-`) change per build; macOS then treats each build as a new app and the App Management grant won't stick. Instead create a persistent self-signed code-signing certificate once (Keychain Access → Certificate Assistant → Create a Certificate → "Sabrage Dev", Code Signing) and set it as `signingIdentity`. Free and stable; grants persist across rebuilds.
- **Grant flow to document (in-app panel + README):** install layers 1–2 write inside `CrossOver.app` → first attempt hits TCC EPERM (`PermissionDenied` on a path under `*.app/`) → Sabrage detects that exact signature, shows "Sabrage needs App Management permission" with an Open System Settings button (`x-apple.systempreferences:com.apple.preference.security?Privacy_AppBundles`), notes the **relaunch requirement**, and offers Retry plus the fallback "copy `./demo.sh install --bottle <n>` for a permitted terminal".
- **Dev caveat:** `cargo tauri dev` runs an unbundled binary — TCC treats it separately. Test TCC-touching flows against `cargo tauri build` bundles; day-to-day dev works fine because everything else is unprivileged.
- **The one sudo step** (host manifest) uses `osascript … with administrator privileges` in `admin.rs`: write the JSON to a scratch file, elevate only `mkdir -p` + `install -m 0644` — never embed the JSON in the shell string; compare **parsed** `library_path` first (like doctor) *and* fall back to demo.sh's byte-compare so an already-current file produces zero prompts and both front-ends agree byte-for-byte.

---

## 7. Dev workflow

- **Rust:** install rustup via its curl installer (the pipeline's `build` stage already hard-requires rustup for the x86_64 target, so this is not a new dependency — note this shell currently doesn't even see rustup, only Homebrew cargo). Sabrage builds with rustup's stable arm64 toolchain; `~/.cargo/bin` survives brew wipes. Homebrew's cargo becomes irrelevant.
- **Node:** **mise** (`curl https://mise.run | sh`), `mise use node@lts` pinned by the committed `sabrage/.mise.toml`. Installs under `~/.local/share/mise` — a brew wipe cannot touch it, and restoring after any loss is `mise install` + `npm ci` (lockfile-exact). Volta is an acceptable substitute; mise wins on being one tool for any future runtime.
- **Tauri CLI:** `@tauri-apps/cli` as a pinned devDependency (`npm run tauri …`) — version locked in the lockfile, no global installs to lose.
- **Commands:** dev = `cd sabrage && npm run tauri dev` (vite HMR + Rust rebuild-on-change); release = `npm run tauri build` → outputs at `sabrage/src-tauri/target/release/bundle/macos/Sabrage.app` (+ `.dmg`); copy the .app to `/Applications` for the TCC-granted daily driver.
- **demo.sh users unaffected:** nothing in `sabrage/` is sourced or read by `demo.sh`; the only shared surfaces are on-disk artifacts, which Sabrage produces byte-identically (host manifest, toml template, helper staging path, registry value, logs layout), plus the `pins.rs` ↔ `lib.sh` parity test that fails loudly if either side drifts.

---

## 8. Phased milestones

**Phase 0 — Walking skeleton (window + shell + About).** Scaffold workspace, port `styles.css` verbatim, real overlay window chrome, sidebar nav across all seven screens with static/placeholder bodies, About fully real, menu bar with Edit items, single-instance. *Verify:* `npm run tauri dev` opens a native-feeling 1280×840 window; every nav item renders; ⌘C/⌘V works in a text input; second launch focuses the first.

**Phase 1 — sabrage-core foundations + Doctor real.** `paths.rs`/`pins.rs`/`config.rs` (read-only), `checks.rs` registry, `run_doctor` channel, Fix actions stubbed to "copy remedy". `sabrage-cli doctor` bin. *Verify:* pins parity test green; `sabrage-cli doctor --bottle Steam` vs `WINEVR_DOCTOR_SOFT=1 ./demo.sh doctor --bottle Steam` — identical pass/warn/fail verdicts per check (documented intentional divergences only, e.g. all-5 DXMT file comparison); GUI rows stream in live.

**Phase 2 — Stages + gate modal + fixes.** `setup`/`build`/`install` stages with streaming output/progress, atomic tar-swap improvement, `admin.rs` osascript step, TCC-denial detection panel, real Fix actions, Stop. *Verify:* full setup→build→install driven from the GUI on a scratch bottle, then `./demo.sh doctor` exits 0; host manifest byte-identical to what install.sh writes (`cmp`); re-running install from Sabrage prints all "unchanged" and prompts for nothing.

**Phase 3 — Launch + Logs.** `run.rs` port (ordered preflights incl. the three mutating auto-fixes, wineserver reset, Goldberg swap + revert action, audio guard with **persisted** previous device, wired/adb hygiene), gate modal wired to `launch`, console capture → Logs screen + `logs/` file, session watcher v1 (child + runtime_status.json freshness). *Verify:* Beat Saber launches and streams from the GUI; Ctrl-free Stop restores audio even after a `kill -9`'d previous session (the demo.sh-can't-do test); log file interchangeable with a demo.sh run.

**Phase 4 — Settings + Library + Edit game.** toml_edit read/write with the comment policy + backup, state.json library CRUD, per-game overrides, validity badges. *Verify:* edit bitrate in Sabrage → `./demo.sh doctor` still passes and the toml's comments/unknown keys survive a round-trip diff; write-once creation matches setup.sh's template byte-for-byte.

**Phase 5 — Session telemetry.** `telemetry.rs` parsers (GRAPH/STATS/enc1s, unit conversion, rotation-aware tailing), stats channel, charts, degraded/stalled states, last-session summaries into the library. *Verify:* live session shows m2p ≈ the documented ~103–107 ms WiFi baseline; drops gauge reacts to raising bitrate; standby-freeze heuristic fires when the headset sleeps.

**Phase 6 — Wizards + polish.** Setup wizard, Add-game wizard (DepotDownloader copy flow), icon, "Sabrage Dev" signing identity in config, README section for the TCC grant, `--no-dashboard` default flip (Sabrage subsumes the ALVR dashboard). *Verify:* fresh-machine dry run: clone → mise install → npm ci → build → grant TCC once → full pipeline from the .app with zero terminal use.

### Critical Files for Implementation

- /Users/yifeiding/orca/workspaces/wine-vr/gui/scripts/demo/lib.sh (single source of truth being ported: every path, pin, and helper; the parity-test target)
- /Users/yifeiding/orca/workspaces/wine-vr/gui/scripts/demo/run.sh (the launch preflight/auto-fix/trap semantics the gate modal and session lifecycle must reproduce)
- /Users/yifeiding/orca/workspaces/wine-vr/gui/scripts/demo/doctor.sh (the check registry, remedies, and Fix-action mapping for the Doctor screen)
- /Users/yifeiding/orca/workspaces/wine-vr/gui/scripts/demo/install.sh (the TCC/sudo boundary and byte-parity artifacts: host manifest, overlay layers)
- /Users/yifeiding/orca/workspaces/wine-vr/gui/CLAUDE.md (conventions the app must not violate: write-once toml, gitignored logs/, three build trees, absolute-path fragility)