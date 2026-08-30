# Sabrage ⇄ demo.sh — intentional divergence ledger

The native pipeline (`sabrage/`, Rust) and the zsh pipeline (`demo.sh` + `scripts/demo/`)
are two independent implementations of ONE pipeline. Machine-checkable policy
(per-side launch gates, volatile flags) lives in `contract/pipeline.toml`; this file is
the human rationale for every place the implementations deliberately differ.
Artifact-byte parity IS required; bug-for-bug parity is NOT.

## Doctor / checks

| Divergence | Rationale |
|---|---|
| Console colors gated on isatty + `NO_COLOR` | zsh bakes ANSI constants into every row even when piped; the native CLI strips them for non-terminals. Content is byte-identical (verified: strip-ANSI diff of both outputs). |
| `net.adb-forwards` renders a green `OK   no stale adb port forwards` row when clean | zsh's 16b prints nothing when clean (silence ≙ pass, declared in the differ). 13b (`cfg.session-pins`) already prints its own OK row in zsh. |
| Silent-when-clean tap-only slugs (`cx.present`, `bottle.named`, `game.present`, the quiet branch of each `cfg.protocol.*`) carry `CheckOutcome::silent_pass` | The CLI console suppresses them (byte-compat); the tap channel and the GUI (which deliberately shows every row) keep them. |
| Real version comparisons (macOS ≥ 27, CrossOver ≥ 26.2) | zsh uses `sort -n`/`sort -V` + `grep -qx` string accidents. Same verdicts on real version strings. |
| Doctor exit code capped at 255 | zsh would wrap mod 256 at 256 FAILs — a shell artifact not worth reproducing. |
| `host.manifest` / `cfg.session-pins` parse JSON natively (serde) | The shell's `python3` failure branches ("broken python3?") can never fire natively — strictly better. `serde_json/preserve_order` keeps multi-pin ordering identical to CPython's insertion order. |
| `helper_is_arm64` currently shells out to `lipo` (like zsh) | design-core §10.23 wants the `object` crate to shed the Xcode-CLT dependency — deferred; behavior verified identical (`arm64e` alone rejected, fat `x86_64 arm64` accepted). |
| `is_executable` tests mode bits `0o111`, not effective access | A root-owned `0700` helper reads executable natively but not to `[ -x ]`. Cosmetic in practice. |
| Bottle listing / lsof-token ordering byte-sorted, not locale-collated | Remedy-string cosmetics only. |

## Run preflight (encoded in the contract's per-side gates)

| Divergence | Rationale |
|---|---|
| Native preflight blocks on ALL four overlay files | run.sh cmp-checks only `d3d11.dll`; a stale winemetal.so/wineopenxr.* still black-windows. |
| Native preflight validates `host.manifest`'s `library_path` | run.sh checks presence only; this machine's manifest pointing at a deleted checkout is the live failure it catches. |
| Launch refuses `protocol=oxrsys` outright (`cfg.protocol.legacy-oxrsys`: shell `warn` and launches the legacy USB path anyway / native `block`) | v1 trim: Sabrage does not implement the legacy USB/adb-reverse client at all, so where run.sh warns and proceeds, `stages::run::preflight` dies with run.sh's own warn text plus a second line naming the shell equivalent (`./demo.sh run --bottle <name>`) — refusing to launch a path it cannot supervise, rather than launching something and hoping. Already in the check table above; this row exists to spell out *why* the gate differs rather than merely that it does. |
| A `Skipped` outcome that reaches a gate is a Fatal ("cannot verify \<slug\>: \<reason\>"), never a pass | Applies to every native-gated check, not one slug: an `overlay.*` row with no CrossOver.app to probe, or `run.wired-adb` with adb probing switched off, reports the row and then refuses to launch on an unverified gate — the row is emitted before the die so the reason is visible, but it never resolves to Pass. run.sh has no "could not verify" state at all — its file tests are binary. |
| With CrossOver absent, the shell dies on `run.wine-exec` (`[ -x "$WINE" ]`, run.sh line 17); the native walk dies earlier, on `overlay.dxmt-d3d11` being unverifiable (no `CrossOver.app` to probe — the Skipped-is-Fatal row above) | Both sides evaluate the same check set and abort on the same underlying condition (no CrossOver install); only *which* check fires first differs, because the native walk follows contract order — bottle → goldberg → game → helper → overlay → bottle-bridge → host → protocol → run-only — while run.sh's own sequence is game → wine → bridge → host → bottle → overlay → backend → goldberg → protocol → helper, and `overlay.dxmt-d3d11` sits ahead of `run.wine-exec` in contract order but behind `run.wine-exec` in run.sh's. One contract-ordered walk, no bespoke reordering to make the two die on the same line — both are die-either-way situations, only the message differs. |
| With `oxrsys-runtime.toml` missing *and* the arm64 helper unstaged, the shell dies on the missing toml (`[ -f "$TOML" ]`, run.sh line 56); the native walk dies on the helper checks (`build.helper-staged`/`build.helper-arm64`) first | A missing toml file yields the shell's own `${ENCODER_PROC:-auto}` default (run.sh line 71), which requires the helper — so the shell would have died on the helper too, one line later, had it gotten past line 56. The native walk reads the same default (`read_toml_facts`) but reaches the helper slugs before `cfg.protocol.*` in contract order, so it dies there first instead. Same rationale as the row above: one contract-ordered walk, both sides die either way, only the message differs. |

## Setup

| Divergence | Rationale |
|---|---|
| A pinned download's `.tmp` file is removed when curl or the sha256 check fails | lib.sh's `fetch_pinned` leaves `<dest>.tmp` behind on failure, which confuses the next run into treating a partial file as real; `RealExecutor::download` cleans it up on both the curl-failure and hash-mismatch paths. |

## Install (the one privileged write)

| Divergence | Rationale |
|---|---|
| The elevated write is `/usr/bin/install -m 0644 -o root -g wheel <tmp> <dest>` from a `0600` staging file, not `sudo tee` | `tee` can't set mode/owner atomically; staging the exact host-manifest bytes to a private temp file first and letting `install` set permissions in the same elevated call keeps the privileged step to one atomic operation instead of write-then-chown. |
| A GUI launch (no tty) elevates via a single `osascript -e '… with administrator privileges'`, which has no shell counterpart | install.sh's prompt needs a tty to write to, which a Finder-launched `.app` never has. `AdminMethod::detect()` still picks `sudo` — and install.sh's exact `sudo mkdir failed`/`sudo write failed` die text — whenever a controlling terminal is reachable (stdin is a tty, **or** `/dev/tty` opens); stdout is deliberately not consulted, since `sudo` reads its password from `/dev/tty`, so `sabrage install \| tee log` prompts in the terminal exactly like `./demo.sh install \| tee`. |
| `wine … reg add`'s output is captured into the event stream instead of discarded | install.sh redirects it to `>/dev/null 2>&1`; every Sabrage child feeds the same `StageEvent::Line` sink the GUI/CLI render from, so there is no "discard" primitive to route through — capturing it is strictly more information, not a byte contract either side depends on. |
| A copy failure prints the OS error as one stderr-shaped output line (`<dst>: Permission denied`) before the verbatim `FATAL copy failed: <src> -> <dst>` | The shell shows `cp`'s own stderr and then dies; Sabrage has no `cp` child, so it emits the io cause itself (as `StageEvent::Output`/stderr, without the `cp:` prefix) rather than swallowing it behind the die text — a plain PermissionDenied, a read-only volume and ENOSPC stay distinguishable. |

## Stop

| Divergence | Rationale |
|---|---|
| Each reap (leftover encoder helper, leftover ALVR dashboard) matches by **exact resolved executable path** (`find_processes_by_exe`), never `pkill -f`'s argv substring | `pkill -f` would happily kill an unrelated process that merely mentions the path on its command line (a `tail -f` of the log, an editor with it open, doctor's own shell); a mutating kill can't tolerate that false-positive risk. |
| The Beat Saber survivor probe scans live processes' **argv** for the substring `Beat Saber.exe`, matching `pgrep -f`'s own semantics, unlike the reaps above | under Wine the resolved executable is CrossOver's `wine`/preloader, never `Beat Saber.exe` — that string appears only as a `Z:\…` path in argv — so this one read-only safety check has to match argv or it can never fire true. A read-only report can tolerate what a live kill cannot. |
| Each reap sends `/bin/kill -TERM <pid>` once per matched process, not one `pkill -f` call | the `Executor` trait's only "mutate the machine" primitive is "run a child"; there is no bespoke "signal a pid" method, and one `kill` per pid gives each kill its own `StageEvent` instead of one opaque exit code covering however many processes `pkill -f` happened to hit. |
| The survivor warning lists `"<pid> <exe-basename>"` pairs, not `pgrep -lf`'s full argv text | the probe reads argv to *find* the survivors (row above), but what it carries away is a `ProcInfo` — pid plus resolved exe, not the command line it matched on; pid+basename is cheap and unambiguous, and stop.sh's warn text only needs "something survived," not the invoking command line verbatim. |

## Build

| Divergence | Rationale |
|---|---|
| The `cmake` **configure** calls (`build-x64`, `build-helper-arm64`, wineopenxr) stream their output as `StageEvent::Line` | build.sh redirects configure to `>/dev/null`; every Sabrage child feeds the same live sink the GUI/CLI render progress from, so there is no "quiet" mode to route configure noise through short of a bespoke suppression this stage doesn't add. |

## CLI / GUI

| Divergence | Rationale |
|---|---|
| `--dry-run` / `--quiet` flags | sabrage-only: preview a stage's writes/spawns without touching anything, or suppress a child's own output passthrough. demo.sh has neither mode, so there is no shell text either flag needs to match. |
| Dry-run rows swap the verb to "would install" / "would clear" / "would back up and delete" | none of the stage scripts have a dry-run mode at all; these are additive language for a capability the shell doesn't have, not a divergence from any existing shell text. |
| A dry run ends with a trailing `-- plan (dry run)` section, in **both** front-ends (the CLI prints it; the Tauri stage command emits it as a section + `info` rows) | the narrative rows say "install: `<path>`" either way — only the recorded plan distinguishes *would copy* from *would skip because the bytes already match*, which is the whole point of a preview. The section title and its body lines come from one place (`sabrage_core::DRY_RUN_PLAN_TITLE` / `dry_run_plan_body`) precisely so the two front-ends cannot drift apart. Printed even when empty (`(nothing planned)`, e.g. after an early die) so "no actions" and "never rendered" stay distinguishable. |
| A dry run's layer 4 prints `info "would prompt for administrator authorization"` instead of emitting `NeedsAdmin` | `NeedsAdmin` is what the GUI renders as "macOS will ask for your password", which would be a lie on a run that never prompts. The shell has no dry-run mode to diverge from. |
| A `FATAL` row may be followed by a `       remedy: <r>` continuation line | lib.sh's `die` has no remedy slot at all, so this adds text where the shell prints none rather than changing text it prints. Needed because two `Fatal`s come from `privilege` rather than from a `die`-shaped call site — the App Management refusal and the declined authorization dialog — and both carry the only actionable instruction the user gets (the `x-apple.systempreferences:` deep link, the relaunch requirement) in `remedy`. Same seven-space indent `fail`'s own remedy line uses. |
| `error: <e>` is suppressed for every error whose condition already emitted a `Fatal` (`Fatal`, `TccDenied`, `AdminDeclined`), not just for `Fatal` | `upgrade_write_error` and `elevate_osascript` both emit the `Fatal` themselves and return a different variant, documented as "the caller must propagate, not re-emit"; without this the user reads the FATAL row and then a second, differently-worded line for one condition. |
| SIGTERM is trapped like SIGINT; a cancelled run exits 130 whichever signal started it | run.sh re-raises TERM and exits 143 — the shell's own convention of 128+signal, which differs by signal number. Sabrage cancels through one `CancellationToken` regardless of which of the two signals fired, so the exit code names the cancellation path (130, SIGINT's number) rather than the delivered signal. |

## Run (launch)

`stages::run` ports run.sh's launch (lines 6–270) natively: the preflight
table above, then the seven `[[launch_action]]`s in order
(`LAUNCH_ACTION_IDS`), then supervision until exit. Divergences not already
covered by a preflight-table row:

| Divergence | Rationale |
|---|---|
| The wine console log is a plain file the child's stdout/stderr are redirected into, not a `tee` pipeline — and a same-second name collision gets a `-2`, `-3`, … suffix instead of silently truncating the earlier run's log | run.sh's `"$WINE" … > >(tee "$LOG") 2>&1 &` can lose the last buffer when the pipeline is torn down, and two launches in the same wall-clock second overwrite one path with `tee`, so the first run's log is simply gone. `logs::wine_log_candidate` opens the file `create_new` (via `Executor::spawn_detached`) so the collision is *detected*, never assumed; `attempt == 0` still produces the byte-identical `beatsaber-<ts>.log` name for the same instant. |
| The wine child is spawned in its **own process group**, not the launching process's | `stages::run::actions::launch_wine` uses `Executor::spawn_detached` (`process_group(0)`, `kill_on_drop(false)`) — never `process::spawn_streamed`'s `kill_on_drop(true)`, which would SIGKILL the game the instant Sabrage itself exits. run.sh's wine runs as a background job in the *script's* process group, so a Ctrl-C at the terminal (SIGINT to the whole foreground group) reaches wine directly; a Ctrl-C aimed at Sabrage reaches only Sabrage, which then tears wine down deliberately through `stages::run::guards`/`teardown` rather than by accident of process-group membership. |
| `SwitchAudioSource -t output -s …` and `osascript -e 'set volume output volume 100'` have their own stdout/stderr forwarded as `StageEvent::Output` (the CLI prints `output audio device set to "BlackHole 2ch"` around run.sh's own `audio: …` line) | run.sh redirects both to `/dev/null`; every Sabrage child feeds the same `Output` sink the GUI's console pane and the CLI's passthrough render from, and — as with install's `reg add` row — capturing is strictly more information than discarding. `--quiet` suppresses the passthrough for a byte-clean console. |
| The CLI renders nothing for a `StageEvent::AutoFixed` | `stage_event_lines` maps it to `vec![]` — the two auto-fixing preflight rows (`bottle.gfx-dxmt`, the helper pair) already print their post-fix `Check` outcome; `AutoFixed` is a structured signal for the GUI's "here's what got fixed" affordance, not a second console line duplicating what the row already said. run.sh has no such structured event to omit — `ok "bottle graphics backend forced to dxmt …"` and `ensure_helper_staged`'s own `ok`/`warn` lines are the only text either side prints for an auto-fix. |
| `sabrage all` chains `setup → build → install → run` **in-process**, each stage over a fresh `StageCtx` (fresh `run_id`, fresh executor) sharing one `paths`/`opts`/cancellation token | demo.sh's `all` re-executes `"$ROOT/demo.sh" <stage>` once per stage — a fresh *process* each, `WINEVR_*` travelling via the environment, `|| exit $?` fail-fast after a single up-front `require_bottle`. The native chain keeps the isolation that matters (per-stage identity, executor and event stream) without a re-exec, checks `require_bottle` once up front the same way, stops at the first non-zero stage with that stage's exit code, and lets a single Ctrl-C reach whichever stage is running through the one shared `CancellationToken`. The `\n##### demo.sh: <stage> #####` separator demo.sh prints before each re-exec is not reproduced: every stage already announces itself on the same sink with its own `== wine-vr demo <stage> ==` banner, and printing both would name the starting stage twice. |

## Session (detach / reconcile)

Crash/detach recovery is a Sabrage-only capability layered on top of the run
state machine — see `docs/design/design-core.md` §3.2–3.3 and
`session/reconcile.rs`'s own doc comment for the row texts, none of which have
a shell counterpart at all (run.sh's guards are shell traps; a `SIGKILL` or a
power loss skips them entirely).

| Divergence | Rationale |
|---|---|
| A recorded **Live** session (same pid, same start time, still running) refuses a new Launch outright | run.sh has no session record and no such refusal: a second `./demo.sh run` on top of a running one just resets wineserver — killing the running game out from under the user — and relaunches. `stages::run` calls `session::reconcile::reconcile` before doing anything permanent, and a `Live` classification returns a `Fatal` naming the pid, the start time, and both stop routes (`Stop` in Sabrage, or `./demo.sh stop --bottle <name>`) instead. |
| A **Dead** or **IdentityMismatch** recorded session has its leftover guards *restored*, not merely reported | The audio-device switch, the ALVR dashboard, and any `--wired` adb forwards a previous session left behind are put back (`RestoreMode::Full` for Dead, `RestoreMode::SafeOnly` — no pid signalling — for IdentityMismatch, a recycled pid). `stop.sh` has no record to restore *from* at all: it can only warn that the Mac's output is still `BlackHole 2ch`, with nothing on the machine saying what it was before. `stages::stop::run` calls the same `session::reconcile::finish_stopped_session` between its reap step and its audio report, so a `stop` after an unclean exit reports the *restored* device, not just the stale one. |

## Planned for later phases (declared now)

- Revert-original-`steam_api64.dll` action (no shell counterpart either — run.sh never restores `.orig-steam` — Phase 4+, if ever).
- CLI help text includes `--wired`/`--verbose` (zsh's `sed 2,20p` truncates them).

## Invariants that must NOT change (byte/behavior parity)

Write-once `oxrsys-runtime.toml` creation from the shared template; host-manifest bytes +
skip-when-current; the host manifest's bytes end in install.sh's trailing newline
(`}\n}\n`, never `}\n}`); the `.sha256` marker file is the pin string plus exactly one
trailing newline; run's permanent-vs-guarded mutation boundary; wineserver budgets
(5 s fatal / 4 s soft); adb `forward --remove` per-serial for exactly tcp:9943+9944 (never
`--remove-all`) vs `reverse --remove-all`; legacy reverse ports exactly `9944 9945 9946 9948`;
Goldberg hash-tolerance at run (a Goldberg dll present but not matching the pinned
build warns, never blocks — only a missing file dies); `steam_appid.txt`'s bytes
(exactly the contract appid's digits, `printf '%s'`, never a trailing newline);
`WINEDEBUG` caller-precedence (the caller's preset wins whether or not `--verbose`
was passed, in both the verbose and non-verbose branches); the six-line launch
banner text (`banner_events`, verbatim, printed before the wine spawn); `win_path`
semantics (trailing-slash prefix rule); `helper_is_arm64` word-match (`arm64e` must
NOT satisfy); system.reg lazy-flush re-probe is Warn, never Fail.
