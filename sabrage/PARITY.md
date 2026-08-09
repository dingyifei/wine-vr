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

## Planned for later phases (declared now)

- Exec-path process matching instead of `pkill -f` (Phase 2/3 stop/reap).
- `.tmp` cleanup on failed pinned download (Phase 2 setup).
- Log filename `-2` suffix on same-second collision + file-fd wine I/O instead of tee (Phase 3).
- Persisted audio-device restore + Revert-original-`steam_api64.dll` action (Phase 3/4).
- `reg add` output captured instead of discarded (Phase 2).
- CLI help text includes `--wired`/`--verbose` (zsh's `sed 2,20p` truncates them).

## Invariants that must NOT change (byte/behavior parity)

Write-once `oxrsys-runtime.toml` creation from the shared template; host-manifest bytes +
skip-when-current; run's permanent-vs-guarded mutation boundary; wineserver budgets
(5 s fatal / 4 s soft); adb `forward --remove` per-serial for exactly tcp:9943+9944 (never
`--remove-all`) vs `reverse --remove-all`; legacy reverse ports exactly `9944 9945 9946 9948`;
Goldberg hash-tolerance at run; `WINEDEBUG` caller-precedence; `win_path` semantics
(trailing-slash prefix rule); `helper_is_arm64` word-match (`arm64e` must NOT satisfy);
system.reg lazy-flush re-probe is Warn, never Fail.
