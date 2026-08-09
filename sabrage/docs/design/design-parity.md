All grounding files read (CLAUDE.md, demo.sh, lib.sh, doctor.sh, setup.sh, build.sh, install.sh, run.sh, stop.sh, .gitignore; branch `dingyifei/gui` confirmed). Design follows.

---

# Parity Strategy: Sabrage (native Rust) ⇄ demo.sh (zsh), wine-vr

## 0. Recommended mechanism set (summary)

**Adopt: (a) in a split form + (b) in three tiers + (d) with verbatim rules. Reject (c).**

1. **Shared contract, split by kind of data** — `contract/` at repo top level:
   - *Scalars/pins* (URLs, sha256s, appid/depot/manifest, port lists, artifact path lists) live in `contract/pipeline.toml`, **generated** into a whole new file `scripts/demo/contract.gen.sh` (committed, sourced by lib.sh) — no jq at runtime, no BEGIN/END markers inside hand-written files.
   - *Byte-exact templates* (oxrsys-runtime.toml template, host `active_runtime.x86_64.json` shape) become **tracked template files read at runtime by both sides** — no generation, no duplication at all.
   - *Check registry* (stable slugs + `launch_blocking`/`autofix`/`volatile` flags) lives in the contract as a **manifest of IDs only**; check *logic and message text stay impl-owned*.
2. **Parity harness in three tiers** — (i) always-on pure `cargo test`s (contract regen `--check`, shell-file fingerprint tripwire, artifact golden bytes, Rust-check-set == contract set, static grep of `# preflight:` slugs in run.sh); (ii) live doctor diff via a machine-readable tap channel added to zsh doctor, run by `scripts/dev/parity.sh`; (iii) a committed pre-push hook via `git config core.hooksPath scripts/dev/hooks`, plus a CLAUDE.md rule that makes Claude run parity after touching either side.
3. **No delegation**: demo.sh never calls the sabrage binary. A headless `sabrage` **CLI** ships anyway (same crate as the Tauri backend) — for debugging and for the harness — but the two front-ends stay independent implementations meeting only at on-disk artifacts and the contract.
4. **Formalize check slugs now**, before any Rust is written, as the join key for everything.
5. **demo.sh scope: frozen vocabulary, not frozen file.** It keeps receiving every *shared pipeline requirement* forever; GUI-only features must compile down to the frozen `--bottle/--bs-dir/…` vocabulary and store their state only under Sabrage's own app-support dir.

Why this combination: the failure mode the user fears is *silent drift under stacking updates*. Templates-as-shared-files eliminate the largest byte-parity risk (install.sh does literal `$(cat)` string equality on the host JSON — /Users/yifeiding/orca/workspaces/wine-vr/gui/scripts/demo/install.sh:56); generation handles scalars where sourcing speed and `set -u` make runtime parsing unpleasant; slugs + tap give a cheap live differ; and the fingerprint tripwire converts "someone edited only lib.sh" into a failing `cargo test` on the other side. Everything runs on one machine with no CI.

---

## 1. Grounding facts that force the design

- **Stages are sourced zsh fragments sharing lib.sh globals** (demo.sh:44-48); lib.sh is already declared "the single source of truth for paths, sha256 pins, and helpers" (CLAUDE.md:49). The contract mechanism is a *promotion* of that existing rule, not a new invention.
- **Three artifacts are compared byte-for-byte at runtime**: the host XR JSON (`[ "$(cat …)" = "$WANT" ]`, install.sh:56), the cxbottle line `"CX_GRAPHICS_BACKEND" = "dxmt"` (run.sh:29, doctor.sh:36 greps it anchored), and every `install_if_changed` copy (`cmp -s`, lib.sh:91). If Sabrage writes even one different byte, the two front-ends will thrash each other (spurious sudo prompts, spurious "installed:" rows, doctor FAILs). **Byte parity of artifacts is the hard requirement; text parity of console output is not.**
- **The check surface is already dual**: doctor (~30 counted FAIL sites, exit = FAILCOUNT, doctor.sh:196) and run's hard preflight (a strict subset plus three mutations: cxbottle fix run.sh:29-42, helper restage run.sh:59-78, adb-forward hygiene run.sh:98-109). CLAUDE.md:102 already mandates "doctor section + matching run preflight" for every new requirement — the contract's `launch_blocking` flag is that rule made machine-checkable.
- **`WINEVR_DOCTOR_SOFT` exists** (doctor.sh:196) precisely so doctor can run embedded — the tap channel slots in next to it with zero behavior change.
- **A pre-push convention already exists** (the four-part arch gate, CLAUDE.md:100). Adding a parity gate to a documented pre-push checklist is consistent with how this repo already works without CI.
- **Fresh clone must work with zero Rust build artifacts**: `doctor`/`setup` run before anything is compiled. (The machine *does* have cargo — `build` requires rustup — but only after setup; so nothing on the zsh path may depend on a compiled sabrage binary, ever.)

---

## 2. Mechanism (a): the shared contract — split, not monolithic

### 2.1 What goes in, what stays out

| Data | Where | Why |
|---|---|---|
| `DEPS_URL`, asset filenames, `DXMT_TGZ_SHA256`, `GBE_DLL_SHA256` | `contract/pipeline.toml` → generated `contract.gen.sh` | Drift = broken verification; pure scalars |
| `BS_APPID` 620980, depot 620981, manifest 6291266771922375922 | contract → gen | Today the depot/manifest are inlined twice in `DEPOT_CMD` strings (lib.sh:82, doctor.sh:45) — hoist to `BS_DEPOT`/`BS_MANIFEST` vars in the refactor |
| `DXMT_FILES` (5 paths), stream ports 9943/9944, legacy reverse ports 9944-9948, dashboard addr 127.0.0.1:8082 | contract → gen | `WIRED_PORTS` currently lives in run.sh:85 — hoist into the generated file |
| Default BS dir leaf `Beat Saber 1294`, `HOST_XR_JSON` path, OXRSys app-support subpaths | contract → gen | Path literals both sides must agree on |
| oxrsys-runtime.toml first-write template | `contract/oxrsys-runtime.toml.template`, **read at runtime** by setup.sh (`cat > "$TOML"`) and by Rust (`include_str!`) | Write-once file; comments are load-bearing (pre-2026-08 parser caveat, CLAUDE.md:94) — one copy or zero drift |
| Host XR JSON shape with `@OXR_DYLIB@` placeholder | `contract/active_runtime.x86_64.json.template`, runtime-read; zsh: `WANT="${$(<template)//@OXR_DYLIB@/$OXR_DYLIB}"`, Rust: same substitution | install.sh's byte-equality check makes this the single most drift-sensitive artifact |
| Check registry: `[[check]] id/title/launch_blocking/autofix_in_run/volatile` | contract only (neither impl reads it at runtime) | The hub both implementations are *verified against*, in tests |
| Check logic, message text, remedy strings | **impl-owned, NOT in contract** | Checks are code, not data. Cross-language codegen of check logic is the rabbit hole that doubles maintenance instead of halving it. The parity harness compares slug+status, never prose. |
| Rust-only data (remedy→FixAction button mapping, GUI strings) | sabrage crates | GUI concerns |

### 2.2 Generation mechanics

- Generator: a tiny crate `sabrage-contract-gen` (`cargo run -p sabrage-contract-gen -- --write|--check`). Rejected writing the generator in awk/zsh: the fragile side must not own codegen, and the machine always has cargo by the time anyone *edits* the contract (editing is a dev action, not a fresh-clone action).
- Output: **a whole generated file** `scripts/demo/contract.gen.sh`, committed, with header:
  ```
  # GENERATED from contract/ — DO NOT EDIT. Regenerate: scripts/dev/parity.sh --regen
  # contract-sha256: <combined sha256 of contract/* files>
  ```
  lib.sh's current pins section (lib.sh:6-9) becomes `source "$ROOT/scripts/demo/contract.gen.sh"`. A whole file beats BEGIN/END markers inside lib.sh: no ambiguity about what's hand-editable, and hand edits to the generated file are caught by `--check` diffing.
- Rust side parses `contract/pipeline.toml` at **compile time** (`include_str!` + serde in `build.rs` or `LazyLock`) — no runtime file dependency for the .app.
- **zsh-only staleness tripwire**: doctor gains a new check `meta.contract-sync` — recompute the combined contract hash with `shasum` and compare against the `# contract-sha256:` header. This catches "edited contract.toml, forgot to regen" *with zero Rust available*, surfaced exactly where the user already looks (doctor), with remedy `scripts/dev/parity.sh --regen`. The inverse (hand-edited contract.gen.sh) is caught by the Rust `--check` diff in the always-on test tier.

### 2.3 Rejected alternative for (a)

- **Runtime-parsed contract in zsh (jq or a TOML-ish awk)** — rejected. Adds a dependency (jq isn't checked by doctor today) or adds *more* fragile awk parsing to the side we're trying to stabilize; also makes `set -u` sourcing order trickier. Generation keeps the zsh runtime surface exactly as dumb as it is today.
- **One giant contract that also carries check logic/remedies as data** — rejected. That's an interpreter in two languages; churn cost exceeds duplication cost. The slug manifest is the correct minimal join.

---

## 3. Check IDs: formalize now, in zsh first

**Yes, stable slugs, and they land as commit #1, before any Rust exists.** They are the join key for: the parity differ, the run-preflight static check, the divergence ledger, and the GUI's remedy buttons.

Slug scheme (dotted, stable, one per FAIL-countable row — loops get per-item slugs):

```
meta.contract-sync
sys.arch  sys.macos27
cx.present  cx.version
bottle.named  bottle.exists  bottle.template  bottle.gfx-dxmt  bottle.zdrive
tool.cmake  tool.ninja  tool.git  tool.curl  tool.mingw
rust.x64-target
src.oxrsys  src.wineopenxr  src.alvr  src.alvr-patchset
dep.dxmt  dep.goldberg
game.present  game.version
build.oxr-dylib  build.alvr-core  build.runtime-json  build.woxr-dll  build.woxr-so  build.dashboard
build.helper-staged  build.helper-arm64
overlay.dxmt-d3d11  overlay.dxmt-winemetal  overlay.woxr-dll  overlay.woxr-so
bottle.woxr-dll  bottle.manifest  bottle.registry
host.manifest
cfg.protocol  cfg.session-pins
hs.adb  hs.client
audio.loopback
net.ports  net.adb-forwards
```

**zsh mechanics (behavior-neutral):** add one function to lib.sh:

```zsh
chk() { # chk <ok|warn|fail|info> <slug> <msg> [remedy]
  local st="$1" slug="$2"; shift 2
  case "$st" in ok) ok "$@";; warn) warn "$@";; fail) fail "$@";; info) info "$@";; esac
  [ -n "${WINEVR_DOCTOR_TAP:-}" ] && print -r -- "$slug $st" >> "$WINEVR_DOCTOR_TAP"
}
```

Every doctor call site becomes `chk ok sys.arch "Apple Silicon (…)"` etc. Human output stays **byte-identical** (docs/troubleshooting.md quotes these lines; don't break them). The tap channel is opt-in via env, exactly like `WINEVR_DOCTOR_SOFT`. Skipped sections (10 when no CrossOver, 11 when no bottle, 13b/16b silence-when-clean) emit `slug skipped` to the tap so the differ can distinguish "skipped" from "forgot to implement".

**run.sh preflight slugs are comments, not code:** each preflight line gets `# preflight: bottle.woxr-dll` etc. above it, and the three mutations get `# preflight-autofix: bottle.gfx-dxmt` style tags. No dry-run mode is added to run.sh (see §4.3).

---

## 4. Mechanism (b): the parity harness — three tiers

### Tier 1 — always-on pure tests (`cargo test -p sabrage-parity`, no env gate, no machine state)

1. **Contract regen check**: regenerate `contract.gen.sh` to a temp buffer, diff against the committed file. Fails on hand edits *or* stale regen.
2. **Shell fingerprint tripwire**: a committed `sabrage/parity/shell.fingerprint` holds sha256 of `demo.sh` + every `scripts/demo/*.sh`. The test recomputes and compares. **This is the answer to "how is divergence detected when someone edits only lib.sh":** any shell edit makes `cargo test` red until the editor re-runs `scripts/dev/parity.sh --bless`, which only re-blesses after the full suite (including Tier 2 when available) passes. The coupling is mechanical, not honor-system.
3. **Check-set coverage, both directions**: Rust's implemented check slugs == contract `[[check]]` set (compile-time-ish, pure); and a static parse of doctor.sh's `chk` calls (regex over the tracked file) == contract set. Adding a check to only one place fails here.
4. **Run-preflight structural parity**: grep run.sh for `# preflight:` slugs; assert the set == `launch_blocking=true` checks in the contract; assert Rust's launch-preflight list is derived from the same flags. This is the CLAUDE.md:102 rule, enforced.
5. **Artifact golden-byte tests** (pure string functions in `sabrage-core`): host JSON rendering for a sample dylib path == template substitution; toml template bytes == contract template file; cxbottle three-branch edit against fixture files (existing-key / `[EnvironmentVariables]`-present / bare file) produces the byte-exact lines doctor greps for; `steam_appid.txt` content `620980` with **no trailing newline** (run.sh:133); `.sha256` marker content = pin + newline (setup.sh:38); `win_path()` table tests including the `drive_c` (no slash) → `Z:` edge and backslash forms.

### Tier 2 — live doctor diff (needs the real machine; env-gated)

`scripts/dev/parity.sh` (zsh, dev-only, writes only to the scratch dir):

```
WINEVR_DOCTOR_TAP=$T1 WINEVR_DOCTOR_SOFT=1 ./demo.sh doctor --bottle "$B"
sabrage doctor --bottle "$B" --tap "$T2"        # headless CLI, same WINEVR_* env contract
normalize + diff:
  - full slug set must match (incl. skipped)
  - status must match for all checks with volatile=false in the contract
  - volatile=true (hs.adb, hs.client, net.ports, net.adb-forwards) compare presence only
  - FAILCOUNT (zsh exit surrogate) == native fail count
```

Volatility flags matter: the two doctors run seconds apart and `adb devices`/`lsof` can legitimately change; without the flag the harness cries wolf and gets ignored.

Where it runs, given no CI — **three anchors**:
- `scripts/dev/parity.sh` as the one-command entry (runs Tier 1 via `cargo test`, then Tier 2 if `WINEVR_BOTTLE` is set or `--bottle` given; `--live=off` to skip).
- A **committed pre-push hook** at `scripts/dev/hooks/pre-push`, activated once via `git config core.hooksPath scripts/dev/hooks` (documented in CLAUDE.md). Escape hatch `PARITY_SKIP=1 git push` for emergencies, by convention logged in the commit message.
- A **CLAUDE.md instruction** (below) so agent-driven edits — which is how most edits happen in this repo — run the harness without being asked. On a personal repo, the agent rule is realistically the strongest anchor of the three.

### Tier 3 — periodic soak (optional, cheap)

Because `doctor` is read-only and safe on a timer, an occasional `scripts/dev/parity.sh --live-only` after CrossOver/macOS updates catches environment-driven divergence (e.g. a new CrossOver version string format breaking zsh's `sort -V` regex-quirk comparison but not Rust's semver — a known asymmetry worth *detecting*, not preventing).

### 4.3 Why no zsh run-preflight dry-run

Adding `--dry-run` to run.sh means threading a mode flag through the script whose fragility motivated this whole project — every future preflight edit would need to keep the dry path honest, which is a second parity problem *inside* the zsh side. Rejected. The combination of (i) static `# preflight:` slug parity (Tier 1.4), (ii) artifact golden tests for the three mutations (Tier 1.5), and (iii) live doctor covering the same underlying predicates gives equivalent drift coverage with zero new zsh branches.

---

## 5. Mechanism (c): demo.sh → sabrage delegation — **rejected**

Arguments, in order of weight:

1. **It destroys the recovery property.** The user's stated architecture is "native is the robust one, zsh stays alive as updates stack." The moment demo.sh delegates, a Sabrage regression takes both front-ends down; the zsh path stops being an independent escape hatch precisely when you need it (a half-broken Rust refactor mid-update). Two implementations meeting at on-disk artifacts fail independently; a delegating wrapper fails jointly.
2. **It hollows out the parity harness.** Diffing zsh-doctor against native-doctor is meaningless if zsh-doctor *is* native-doctor behind a `command -v sabrage` check. You'd be testing the fallback branch only when the binary is absent — i.e., never on the dev machine.
3. **Bootstrap ordering.** Fresh clone → `doctor`/`setup` must run before any build. A delegation branch adds a "which implementation am I actually running?" question to every debugging session — the exact cognitive load demo.sh is supposed to not have.
4. The one real benefit of delegation (users get the robust behavior from the terminal) is captured **without** it: ship the `sabrage` CLI as a first-class binary that accepts the same six flags and reads the same `WINEVR_*` env vars, so `s/demo.sh/sabrage/` in any command line works. Every Sabrage GUI action also displays its demo.sh-equivalent command (adopt the "copy equivalent command" affordance as a hard rule) — the translation layer runs in the user's head, cheaply, instead of in a fragile wrapper.

Partial-delegation variants ("only `doctor --native`", "delegate only when `SABRAGE=1`") are rejected for the same reason 2: any delegation path that exists will become the tested path.

---

## 6. Mechanism (d): the CLAUDE.md contract — proposed verbatim text

Add a section to /Users/yifeiding/orca/workspaces/wine-vr/gui/CLAUDE.md (and mirror the first three bullets in `sabrage/CLAUDE.md` if a nested one is created):

```markdown
## Sabrage ⇄ demo.sh parity (both front-ends stay alive)

The native pipeline (`sabrage/`, Rust) and the zsh pipeline (`demo.sh` + `scripts/demo/`)
are two independent implementations of ONE pipeline. They meet at on-disk artifacts and
at `contract/`. Rules:

- **Never edit `scripts/demo/contract.gen.sh`.** Pins, URLs, ports, artifact lists, and
  the check registry live in `contract/`; regenerate with `scripts/dev/parity.sh --regen`.
  The templates in `contract/` are read at runtime by BOTH sides — a byte changed there
  changes what both implementations write.
- **A pipeline behavior change lands in both implementations in the same commit**: any
  new/changed doctor check (same slug in `contract/pipeline.toml`, `chk` call in
  doctor.sh, check impl in `sabrage-core`), any run-preflight change (`# preflight:` tag
  in run.sh + contract `launch_blocking` flag + Rust preflight), any change to bytes
  either side writes (host XR JSON, oxrsys-runtime.toml template, cxbottle line,
  steam_appid.txt, `.sha256` marker) with its golden test updated.
- **Run `scripts/dev/parity.sh` after touching `scripts/demo/`, `demo.sh`, `contract/`,
  or `sabrage/` core, and before any push.** It re-blesses the shell fingerprint only
  when green. `cargo test` failing on `shell.fingerprint` means shell was edited without
  re-running parity — fix the divergence, don't bless around it.
- **Intentional divergences go in `sabrage/PARITY.md`**, keyed by slug/stage, with
  rationale (e.g. Rust deletes `.tmp` on failed download; Rust matches executable paths
  instead of `pkill -f`). Bug-for-bug parity is NOT required; artifact-byte parity IS.
- **demo.sh scope**: frozen vocabulary (6 stages, 6 flags, `WINEVR_*` env). It gains
  changes only for shared pipeline requirements, never for GUI features. GUI-only
  features (game library, dashboards, audio-restore persistence) keep ALL state under
  `~/Library/Application Support/Sabrage/` and must remain expressible as a
  `./demo.sh <stage> --bottle … --bs-dir …` command; every Sabrage pipeline action
  shows that equivalent command in the UI.
- demo.sh never invokes `sabrage`, and `sabrage` never shells out to demo.sh for core
  operations. The parity harness is the only place both run together.
```

This extends, and does not replace, the existing CLAUDE.md:102 rule (doctor section + run preflight for every new requirement) — that rule becomes machine-checked by Tier 1.3/1.4.

---

## 7. Sequencing

**Phase 0 — instrument the reference (zsh-only commits, no Rust yet):**
1. Introduce slugs + `chk()` + `WINEVR_DOCTOR_TAP` in doctor.sh/lib.sh; `# preflight:` tags in run.sh. Verify behavior-neutrality by capturing doctor's human output before/after on this machine and diffing (must be byte-identical; ANSI included).
2. Extract `contract/` (pipeline.toml + two templates), hand-write the first `contract.gen.sh` matching current lib.sh values exactly, switch lib.sh/setup.sh/install.sh to source/read them. Re-run doctor + a no-op `install` to confirm "unchanged:"/"already current" idempotency is preserved (proves byte parity of the template refactor against the live root-owned file).
3. Add the `meta.contract-sync` doctor check. Tag this state `parity-baseline-1`. This is the "freeze as reference" moment — *freeze* meaning fingerprinted and tagged, **not** frozen against future shared changes.

**Phase 1 — port 1:1 (warts documented, not silently fixed):**
4. Scaffold `sabrage/` (workspace: `sabrage-core`, `sabrage-cli`, `sabrage-app` (Tauri 2), `sabrage-contract-gen`, `sabrage-parity` tests). Wire the contract into `sabrage-core` at compile time. Note `.gitignore` interaction: `build/` and `*.dylib` blankets are fine for cargo (`target/` needs adding), but nothing under `sabrage/` may rely on the ignored `third_party/`.
5. Port **doctor first** (read-only, immediately Tier-2-testable), then the artifact writers with golden tests, then setup → build → install → run → stop in pipeline order. Start every intentional divergence in `sabrage/PARITY.md` from day one (the known zsh warts: usage text truncating `--wired/--verbose`, `fetch_pinned` leaving `.tmp` on mismatch, log-filename same-second collision, `pkill -f` overmatching, `CX` becoming `/Contents/SharedSupport/CrossOver` when CrossOver is absent). Where a wart is a one-line zsh fix (the usage `sed` range), fix zsh instead of ledgering.
6. Stand up `scripts/dev/parity.sh`, the fingerprint, and the pre-push hook; land the CLAUDE.md section.

**Phase 2 — GUI on top of a proven core:** Tauri app consumes `sabrage-core` directly; TCC/App-Management and the osascript-elevated host-JSON write are app concerns that never touch parity (the *bytes* written are already golden-tested).

**Phase 3 — evolve under the rules.** New shared requirements follow the both-sides-same-commit rule; GUI-only growth follows the scope policy.

Rationale for this order over "port first, instrument later": the instrumentation commits are the riskiest zsh edits in the whole plan (they touch every doctor call site). Doing them *before* the port means the port targets the instrumented, tagged reference, and any instrumentation regression is caught while zsh is still the only implementation — with the human-output byte-diff as the safety net.

---

## 8. Divergence-detection matrix

| Edit scenario | Caught by |
|---|---|
| Edited `contract/pipeline.toml`, no regen | zsh: doctor `meta.contract-sync` FAIL (no Rust needed); Rust: Tier 1.1 |
| Hand-edited `contract.gen.sh` | Tier 1.1 regen diff |
| Edited lib.sh/run.sh/doctor.sh logic only | Tier 1.2 fingerprint → red `cargo test` until parity re-blessed; behavior drift itself by Tier 2 diff / Tier 1.4 / Tier 1.5 |
| Added doctor check to one side only | Tier 1.3 (both directions: static zsh `chk` parse vs contract; Rust set vs contract) |
| Added run preflight to run.sh only | Tier 1.4 (`# preflight:` set vs `launch_blocking` flags) |
| Changed an artifact's bytes on one side | Tier 1.5 golden test; at runtime, the other side's `cmp -s`/`$(cat)` idempotency check visibly churns ("installed:" instead of "unchanged:") |
| Changed template files | Both sides read the same file — divergence impossible by construction; unwanted *change* caught in review + golden tests |
| Environment drift (CrossOver/macOS update changes a probe's behavior differently per impl) | Tier 3 periodic live diff |
| Edited Rust core only | Tier 1 goldens + Tier 2 live diff (fingerprint doesn't fire — correct, since zsh is unchanged; parity.sh still runs the diff) |

---

## 9. "demo.sh remains usable" — precise scope policy

**Definition adopted:** *every launch-critical pipeline capability remains reachable through demo.sh alone, on a machine with no sabrage binary, forever.* Concretely:

- **Frozen:** the 6-stage × 6-flag CLI surface, the `WINEVR_*` env mirror, exit-code conventions (2/1/FAILCOUNT/wine-rc), the sourced-stage architecture, and all shared on-disk contracts (paths in `contract/`).
- **Growing (shared requirements only):** new doctor checks, new preflights, new install layers, pin bumps — these land in demo.sh *and* Sabrage per the rules. demo.sh is not an abandoned museum piece; it tracks the pipeline.
- **Never in demo.sh:** GUI-only features. Multi-game library, session dashboards, log viewers, audio-restore persistence, CrossOver-update watchers. Test for admissibility: *can this feature's effect on the pipeline be expressed as a demo.sh command line?* A library entry is `(name, bottle, bs_dir)` in `~/Library/Application Support/Sabrage/library.json`; launching it must be behaviorally identical to `./demo.sh run --bottle B --bs-dir D`, and Sabrage displays exactly that command. If a GUI feature would need demo.sh to *read* new state to stay equivalent (e.g. persisted previous-audio-device), the state lives in Sabrage's dir and demo.sh at most gains an optional, silently-degrading read (a deliberate, per-case decision recorded in PARITY.md) — default is demo.sh unchanged, and `stop`'s existing "audio still on BlackHole" warn remains its honest answer.
- The OXRSys write-once toml stays governed by its existing contract: Sabrage's Settings editor uses `toml_edit` (comments preserved, comments **on their own lines only** per the pre-2026-08 parser caveat), and first-write creation uses the shared template byte-for-byte — so a config created by either side is indistinguishable to the other.

---

## 10. Rejected alternatives (summary)

- **(c) delegation in any form** — §5; kills independence, hollows the harness.
- **Runtime jq/TOML parsing in zsh** — new dependency on the fragile side; generation is strictly simpler.
- **Codegen of check logic/remedies from the contract** — an interpreter in two languages; the slug manifest is the minimal sufficient join.
- **zsh `run --dry-run`** — a second parity problem inside the zsh side; replaced by static slug tags + artifact goldens.
- **Screen-scraping zsh's human output as the parity channel** — ANSI/format brittleness, and docs quote those lines so they can't be normalized freely; the tap channel is additive and stable.
- **Freezing demo.sh entirely (no growth)** — contradicts "remain usable as updates stack": a frozen demo.sh silently rots the first time a new install layer ships; frozen *vocabulary* + growing *checks* is the workable line.
- **Cron/scheduled parity instead of pre-push+agent-rule** — timers on a personal machine get ignored; coupling parity to the edit event (fingerprint) and the push event (hook) puts the check where the change is.

### Critical Files for Implementation
- /Users/yifeiding/orca/workspaces/wine-vr/gui/scripts/demo/lib.sh (pins → `contract.gen.sh` sourcing, `chk()` tap helper; the current single source of truth being promoted)
- /Users/yifeiding/orca/workspaces/wine-vr/gui/scripts/demo/doctor.sh (slug instrumentation of every check row; `meta.contract-sync` check)
- /Users/yifeiding/orca/workspaces/wine-vr/gui/scripts/demo/run.sh (`# preflight:` slug tags; the three auto-fix sites whose bytes get golden tests)
- /Users/yifeiding/orca/workspaces/wine-vr/gui/scripts/demo/install.sh + /Users/yifeiding/orca/workspaces/wine-vr/gui/scripts/demo/setup.sh (switch host-JSON `WANT` and the toml heredoc to the shared `contract/` templates — the byte-parity linchpin)
- /Users/yifeiding/orca/workspaces/wine-vr/gui/CLAUDE.md (the parity rules section in §6; extends the existing line-102 doctor+preflight rule)

New files this plan introduces: `contract/pipeline.toml`, `contract/oxrsys-runtime.toml.template`, `contract/active_runtime.x86_64.json.template`, `scripts/demo/contract.gen.sh` (generated), `scripts/dev/parity.sh`, `scripts/dev/hooks/pre-push`, `sabrage/` (workspace: `sabrage-core`, `sabrage-cli`, `sabrage-app`, `sabrage-contract-gen`, `sabrage-parity`), `sabrage/PARITY.md`, `sabrage/parity/shell.fingerprint`.