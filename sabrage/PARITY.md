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
| Launch blocked when `protocol=oxrsys` (`cfg.protocol.legacy-oxrsys`: shell warn / native block) | v1 trim: the legacy USB path stays `./demo.sh run` territory. |

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

## Planned for later phases (declared now)

- Log filename `-2` suffix on same-second collision + file-fd wine I/O instead of tee (Phase 3).
- Persisted audio-device restore + Revert-original-`steam_api64.dll` action (Phase 3/4).
- CLI help text includes `--wired`/`--verbose` (zsh's `sed 2,20p` truncates them).

## Invariants that must NOT change (byte/behavior parity)

Write-once `oxrsys-runtime.toml` creation from the shared template; host-manifest bytes +
skip-when-current; the host manifest's bytes end in install.sh's trailing newline
(`}\n}\n`, never `}\n}`); the `.sha256` marker file is the pin string plus exactly one
trailing newline; run's permanent-vs-guarded mutation boundary; wineserver budgets
(5 s fatal / 4 s soft); adb `forward --remove` per-serial for exactly tcp:9943+9944 (never
`--remove-all`) vs `reverse --remove-all`; legacy reverse ports exactly `9944 9945 9946 9948`;
Goldberg hash-tolerance at run; `WINEDEBUG` caller-precedence; `win_path` semantics
(trailing-slash prefix rule); `helper_is_arm64` word-match (`arm64e` must NOT satisfy);
system.reg lazy-flush re-probe is Warn, never Fail.
