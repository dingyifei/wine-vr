# Sabrage — test-redundancy scan

*2026-09-01 · seventeen vertical and five horizontal Codex `gpt-5.6-sol` reviewers at `xhigh`, every non-keep verdict re-read assertion by assertion by an opus verifier, every deletion checked against a whole-crate `cargo-mutants` kill matrix (2,248 mutants), disagreements adjudicated by a critic. **Report only — nothing under `sabrage/` was changed; the branch is unpushed.***

---

## 1. The answer

**Is a large portion of the tests redundant? No. About one in seven test functions is, and most of those are literal-variant siblings that become tables, not defensive junk.** Of 907 test functions the reviewers proposed 262 non-keep verdicts (29%); after an opus verifier re-read each one assertion by assertion, the kill matrix vetoed five deletions and the critic adjudicated 64 disagreements, **133 functions (14.7%) and 1,888 lines (8.1%) can go under the standard**, landing the suite at 774 functions and 21,374 lines. A further 68 functions stay but lose one assertion that another test already pins. Under the strict policy (no finding-pinned or golden test is touched at all) the figure is 91 functions and 1,190 lines. The already-programmed 08-31 scaffolding work owns 34 of the 133; **99 functions and 1,575 lines are new to this scan**.

**What the redundancy actually is.** 81 rows are literal-variant siblings (bucket V and its P/G-labelled cousins) that collapse into 29 labelled tables, one function each; 48 are outright deletions, of which 24 are same-unit duplicates (D), 11 are private-seam tests whose fact a public test already implies (I), 8 test std/serde/the harness (F) and 3 cannot fail (X); 34 merge their assertions into a named existing test. 25 of the cuts resolve a core-side copy of a byte-fact toward the `sabrage-parity` test that CI actually runs. Only 17 rows in the whole suite are the change-detector kind the owner suspected (X and F). The 66 fix-added tests among the cuts are all merges, tables or single-assertion drops that keep their round-qualified label; not one finding loses its pin.

**What the verifiers refused.** 8 proposed cuts were rejected outright, every one for the same two reasons: the loser and its named carrier test *different units* (`reconcile_with` versus `finish_stopped_session_inner`, a shell byte versus a Rust resolver), or an assertion had no home anywhere else (the serde wire spellings the UI mirrors by hand; the launch path naming its log `beatsaber-*`). 26 were downgraded, mostly `delete` to `drop_assertion`. 5 deletions that read as safe lost a mutant only that test catches and were kept: the kill matrix earned its place, and two of the five (`wine_env`, `registry_binds_in_contract_order`) had a parity carrier that pins the *text* but never calls the Rust function.

**What the real test cases are.** The 17 vertical reviewers wrote 752 behaviour statements for the 907 tests (the standard would need about 799 test functions or table rows to cover them once each, so the surviving 774 are close to that target); 183 are protected by a review finding, 135 by a golden byte, 80 by the contract or parity, and 354 by nothing but the test itself. The architect's top-down inventory found four behaviours nothing tests (§2). The suite is large because it is a second implementation of a shell pipeline that must stay byte-identical to it and because a third of it pins 175 review findings, not because it is padded.

| | tests | lines |
|---|---:|---:|
| now | 907 | 23,262 |
| proposed non-keep by the reviewers | 262 | 4,792 |
| **after verification and the mutation gate (standard policy)** | **−133 → 774** | **−1,888 → 21,374** |
| strict policy (every P/G-sensitive row kept) | −91 → 816 | −1,190 → 22,072 |
| of which already inside the 08-31 program | −34 | −313 |
| of which beyond the 08-31 program | −99 | −1,575 |

Lines are the test functions themselves — body plus doc comment, 23,262 in total; module-level helpers and fixtures are not counted (the verifiers re-measured 268 rows; 268 agree with the index within 3 lines). A `drop_assertion` row is priced at 2 lines, an estimate; the line totals without that estimate are 1,752 (standard) and 1,114 (strict).

By final verdict:

| verdict | rows | meaning |
|---|---:|---|
| delete | 48 | the test goes; a named survivor carries every assertion |
| merge | 34 | its assertions move into a named existing test |
| table | 81 | literal-variant siblings become one labelled table |
| drop_assertion | 68 | the test stays; one duplicated assertion goes |
| new tables created | 29 | each replaces its group with one function |

By bucket of the rows that change (the verified bucket where a verifier corrected it):

| bucket | rows | of all tests in that bucket |
|---|---:|---:|
| D | 32 | 73% of 44 |
| V | 64 | 96% of 67 |
| I | 18 | 78% of 23 |
| X | 5 | 83% of 6 |
| F | 12 | 67% of 18 |
| B | 43 | 10% of 414 |
| P | 32 | 17% of 190 |
| G | 25 | 17% of 145 |

Where the verifiers pushed back:

- 8 proposed cuts **rejected** (§4), 26 **downgraded** to a weaker verdict, 0 left unverified (counted as keep), 0 reset to keep because their carrier is itself removed.
- 5 deletions **vetoed by the kill matrix** (the deleted test was the only catcher of a mutant; §3 names them).
- 7 keeps flagged as **missed duplicates** by the keep verifiers; 6 of them confirmed by a second cut-verifier pass and included above; the rest stay keep.
- 11 rows re-bucketed (verdict unchanged).

---

## 2. What the real test cases are

The owner's second question was "what are really the test cases". Each vertical reviewer wrote a behaviour inventory for its module *before* filling in verdicts: one entry per guarantee the code makes, with the layer it lives at, what protects it (a golden byte, a review finding, a parity test, the contract, or nothing), the tests that assert it today and how many the standard needs. The horizontal architect (X3) did the same top-down for the whole workspace, from `contract/pipeline.toml`, `PARITY.md` and the shell scripts, before opening any test body. The two views agree on the shape: the suite protects roughly one behaviour per 1.2 tests, and the excess is concentrated in a few modules rather than spread evenly.

The crate and file tables copy the aggregator's numbers; the behaviour tables copy the reviewers' inventories verbatim. Every completeness critic (one per reviewer) confirmed that no `#[test]` in the scoped files is missing from the index, and that every behaviour marked as protected keeps at least one surviving carrier after the program below.

| crate | tests now | tests target | lines now | lines target |
|---|---:|---:|---:|---:|
| sabrage-core | 769 | 642 | 19,103 | 17,450 |
| sabrage-cli | 57 | 38 | 1,076 | 938 |
| sabrage-parity | 47 | 40 | 1,195 | 1,174 |
| src-tauri | 26 | 21 | 603 | 550 |
| sabrage-contract-gen | 8 | 4 | 231 | 208 |

Per file (files with no change omitted):

| file | tests now → target | lines now → target | mutants caught / total (kill matrix) |
|---|---:|---:|---:|
| `sabrage-cli:main.rs` | 57 → 38 | 1,076 → 938 | - |
| `sabrage-core:stages/stop.rs` | 34 → 22 | 764 → 634 | 39 / 44 |
| `sabrage-core:stages/install.rs` | 23 → 19 | 862 → 738 | 23 / 27 |
| `sabrage-core:stages/run/preflight.rs` | 34 → 27 | 857 → 751 | 74 / 80 |
| `sabrage-core:checks/config.rs` | 17 → 8 | 305 → 206 | 15 / 17 |
| `sabrage-core:session/reconcile.rs` | 42 → 37 | 1,325 → 1,228 | 79 / 88 |
| `sabrage-core:fixes/backend.rs` | 22 → 8 | 303 → 212 | 24 / 24 |
| `sabrage-core:stages/mod.rs` | 22 → 18 | 561 → 480 | 43 / 50 |
| `sabrage-core:logs.rs` | 28 → 25 | 632 → 558 | 102 / 118 |
| `sabrage-core:session/watcher.rs` | 25 → 17 | 1,237 → 1,168 | 55 / 60 |
| `sabrage-core:session/mod.rs` | 14 → 10 | 448 → 382 | 47 / 48 |
| `sabrage-core:config/runtime_toml.rs` | 70 → 63 | 1,449 → 1,388 | 213 / 236 |
| `sabrage-core:stages/run/mod.rs` | 34 → 32 | 1,285 → 1,225 | 41 / 45 |
| `sabrage-core:stages/build.rs` | 25 → 18 | 555 → 501 | 45 / 48 |
| `sabrage/src-tauri:commands.rs` | 26 → 21 | 603 → 550 | - |
| `sabrage-core:checks/run_only.rs` | 13 → 9 | 230 → 182 | 26 / 27 |
| `sabrage-core:store/settings.rs` | 14 → 10 | 218 → 172 | 10 / 10 |
| `sabrage-core:util/winpath.rs` | 1 → 0 | 51 → 8 | 3 / 3 |
| `sabrage-core:util/mod.rs` | 9 → 7 | 134 → 98 | 89 / 103 |
| `sabrage-core:contract.rs` | 7 → 6 | 89 → 56 | 12 / 14 |
| `sabrage-core:stages/run/actions.rs` | 33 → 30 | 963 → 931 | 65 / 78 |
| `sabrage-core:stages/run/guards.rs` | 16 → 14 | 377 → 345 | 35 / 50 |
| `sabrage-core:paths.rs` | 9 → 8 | 189 → 162 | 45 / 51 |
| `sabrage-core:checks/host.rs` | 6 → 4 | 103 → 78 | 9 / 9 |
| `sabrage-contract-gen:lib.rs` | 8 → 4 | 231 → 208 | - |
| `sabrage-core:process.rs` | 23 → 21 | 471 → 450 | 56 / 68 |
| `sabrage-parity:lib.rs` | 47 → 40 | 1,195 → 1,174 | - |
| `sabrage-core:error.rs` | 2 → 1 | 54 → 34 | 13 / 17 |
| `sabrage-core:fixes/mod.rs` | 16 → 15 | 401 → 381 | 21 / 21 |
| `sabrage-core:session/state.rs` | 11 → 10 | 232 → 214 | 48 / 51 |
| `sabrage-core:privilege.rs` | 26 → 25 | 702 → 686 | 64 / 70 |
| `sabrage-core:store/library.rs` | 28 → 28 | 638 → 622 | 51 / 52 |
| `sabrage-core:checks/toolchain.rs` | 4 → 3 | 65 → 51 | 2 / 4 |
| `sabrage-core:checks/bottle.rs` | 7 → 6 | 130 → 117 | 9 / 9 |
| `sabrage-core:checks/network.rs` | 8 → 6 | 82 → 71 | 7 / 11 |
| `sabrage-core:checks/headset.rs` | 6 → 2 | 45 → 35 | 12 / 14 |
| `sabrage-core:checks/meta.rs` | 7 → 6 | 125 → 116 | 5 / 5 |
| `sabrage-core:fixes/adb.rs` | 14 → 11 | 364 → 356 | 20 / 20 |
| `sabrage-core:checks/system.rs` | 5 → 4 | 92 → 85 | 14 / 24 |
| `sabrage-core:executor.rs` | 22 → 22 | 595 → 589 | 42 / 60 |
| `sabrage-core:fixes/session_json.rs` | 6 → 6 | 204 → 198 | 3 / 5 |
| `sabrage-core:stages/setup.rs` | 16 → 15 | 470 → 466 | 18 / 18 |
| `sabrage-core:checks/audio.rs` | 2 → 1 | 42 → 38 | 2 / 2 |
| `sabrage-core:checks/game.rs` | 4 → 4 | 77 → 73 | 6 / 6 |
| `sabrage-core:checks/overlay.rs` | 4 → 4 | 79 → 75 | 3 / 3 |
| `sabrage-core:store/goldberg.rs` | 12 → 12 | 367 → 363 | 4 / 4 |
| `sabrage-core:checks/bridge.rs` | 6 → 6 | 110 → 108 | 7 / 7 |
| `sabrage-core:checks/mod.rs` | 5 → 5 | 78 → 76 | 31 / 44 |

### The architect's top-down inventory (X3)

X3 wrote 36 workspace-level behaviours and mapped the 907 tests onto them; the four it found uncovered are listed after the table. They are gaps, not redundancy, and this scan does not propose filling them.

| id | behaviour | layer | protected by | tests now | target |
|---|---|---|---|---:|---:|
| ARCH-01 | The compiled pipeline contract is parsed consistently, carries a valid digest, matches its checkout, and reports a different checkout as stale. | unit | contract | 9 | 8 |
| ARCH-02 | Contract generation reproduces the committed shell artifact exactly while safely quoting contract-controlled values. | parity | golden | 10 | 9 |
| ARCH-03 | The assembled check registry is complete, unique, contract-ordered, and partitions doctor, preflight, and run-only gates correctly. | parity | contract | 16 | 8 |
| ARCH-04 | Host-runtime manifests are valid JSON, preserve escaping, and match the shared on-disk template through every write path. | parity | golden | 6 | 5 |
| ARCH-05 | Windows-path conversion implements drive_c mapping and Z-drive fallback for boundary, whitespace, empty-prefix, and sibling-prefix cases. | pure | golden | 2 | 2 |
| ARCH-06 | Errors and progress events retain the externally consumed kind, message, remedy, severity, and already-reported semantics. | unit | finding | 8 | 7 |
| ARCH-07 | Executor implementations distinguish dry-run planning from mutation and preserve atomic write, copy, subprocess, and privilege-boundary results. | unit | finding | 6 | 5 |
| ARCH-08 | Log candidates, stamped attempts, resolution, tailing, and rotation produce deterministic observable paths and events. | unit | finding | 9 | 8 |
| ARCH-09 | Repository, bottle, game, cache, manifest, and runtime paths derive from explicit roots without consulting ambient HOME. | unit | finding | 8 | 8 |
| ARCH-10 | Privileged writes stage safely, preserve cancellation and reported failures, and do not mutate during dry runs. | unit | finding | 7 | 7 |
| ARCH-11 | Process execution captures streams, supports cancellation, identifies owned processes, and applies pgrep-compatible command-line matching. | unit | finding | 6 | 5 |
| ARCH-12 | Fix planning and application transform ADB, backend, helper, session, and registry artifacts for every supported literal form without touching unrelated text. | unit | finding | 12 | 5 |
| ARCH-13 | Runtime TOML reads, edits, backups, compare-and-swap writes, and live-session refusal preserve comments and required field semantics. | unit | finding | 8 | 8 |
| ARCH-14 | Build detects toolchain states, constructs the supported artifacts, and reports missing rustup targets deterministically. | stage | finding | 8 | 6 |
| ARCH-15 | Install plans only necessary copies, handles DXMT backups, waits for registry flush completion, and rejects stale or unsafe state. | stage | finding | 8 | 6 |
| ARCH-16 | Stage orchestration brackets execution with locks and events, and setup installs contract-pinned runtime artifacts with its deliberate first-match protocol semantics. | stage | finding | 8 | 8 |
| ARCH-17 | Stop identifies owned Wine processes, probes wedged resources, reaps spawned children, and reports partial cleanup without killing unrelated processes. | stage | finding | 8 | 8 |
| ARCH-18 | Launch action order, environment, argv, banner, app IDs, and caller-precedence match the shell pipeline while retaining Rust-only action invariants. | parity | golden | 8 | 7 |
| ARCH-19 | Run guards and lifecycle supervision sequence audio, dashboard, wineserver, launch, detach, teardown, and cancellation according to observed state. | stage | finding | 8 | 8 |
| ARCH-20 | Preflight evaluates the contract gating set in order, distinguishes blocking/warning/autofix gates, and retains run-only exclusions. | stage | contract | 8 | 8 |
| ARCH-21 | Doctor checks classify filesystem, configuration, headset, system, network, and run-only states into stable status/remedy outcomes. | unit | finding | 12 | 7 |
| ARCH-22 | Goldberg installation and revert preserve user data, backups, validation, and rollback safety. | unit | finding | 6 | 6 |
| ARCH-23 | Library CRUD and transaction behavior validates inputs, preserves ordering, and rolls back failed mutations. | unit | none | 6 | 6 |
| ARCH-24 | Settings load, defaulting, validation, migration, and atomic save preserve supported user configuration. | unit | finding | 6 | 6 |
| ARCH-25 | Session and persisted state distinguish idle, launching, running, detached, stale, and foreign ownership states. | unit | finding | 7 | 7 |
| ARCH-26 | Reconciliation restores guards, attributes owned processes, handles detachment, and never adopts or destroys foreign sessions. | stage | finding | 6 | 6 |
| ARCH-27 | The status watcher recognizes supported encoder-ready messages, rejects near misses, respects log freshness, and attributes events to the active session. | unit | finding | 7 | 5 |
| ARCH-28 | The CLI parses commands, renders labelled results/errors, merges chained outcomes, honors dry run, and maps cancellation to stable exit behavior. | cli | finding | 17 | 4 |
| ARCH-29 | Parity shell scanners correctly determine slug coverage and tagged block semantics before those scanners are trusted as CI gates. | parity | parity | 16 | 16 |
| ARCH-30 | The CI parity crate owns hermetic contract, artifact, registry, launch, shell-text, and fingerprint byte facts shared with demo.sh. | parity | golden | 16 | 16 |
| ARCH-31 | Tauri IPC helpers resolve roots, serialize progress, coordinate quit, and expose launch/repository behavior without testing struct derives or fixture-owned fields. | ipc | finding | 9 | 6 |
| ARCH-32 | Setup's shell recipe deliberately selects the first protocol assignment while doctor/config evaluation deliberately selects the last assignment. | unit | parity | 2 | 2 |
| ARCH-33 | The contract-generator executable validates its argument grammar, selects check versus write mode, and returns stable failure status. | cli | contract | 0 | 1 |
| ARCH-34 | Tauri write_runtime_config composes live-session refusal, idle-state validation, operation locking, and the core atomic write as one IPC behavior. | ipc | finding | 0 | 1 |
| ARCH-35 | Concurrent settings saves are serialized so an older operation cannot overwrite a later accepted value. | ipc | none | 0 | 1 |
| ARCH-36 | get_repo_info returns a coherent repository hash, source classification, and host-manifest snapshot from one resolved root. | ipc | none | 0 | 1 |

**Uncovered** — behaviours the architect found that nothing tests (4):

- ARCH-33: contract-generator CLI argument, mode-selection, and exit-status behavior has no test.
- ARCH-34: write_runtime_config's complete IPC refusal-lock-write composition has no command-level test.
- ARCH-35: SETTINGS_LOCK ordering under concurrent saves is not exercised.
- ARCH-36: get_repo_info's combined hash/source/manifest snapshot is not tested as one IPC result.

### Per-module behaviour inventories (the 17 vertical reviewers)

Each row is one guarantee the module makes; `now → target` is how many test functions assert it today and how many the standard needs (a table counts as one). Carriers are the tests that remain.

#### V01 — `sabrage-core:config/runtime_toml.rs`

*This module protects four load-bearing areas: Config.cpp-compatible reading, byte-preserving TOML edits, transactional writes/backups, and live-session-safe protocol fixes. Reading supports reducing 70 functions to 62 through three merges, two labelled tables, and two deletions, plus five assertion-only cuts; most apparent repetition exercises different public states or deliberately different parsers. The raw-last and last-accepted readers must remain separate, as must view-level round-trip detection and write-level refusal. Because this was a read-only scan with no mutant run, the standard requires owner sign-off before applying the deletions.*

| id | behaviour | layer | protected by | now → target | carriers |
|---|---|---|---|---:|---|
| V01-01 | An empty patch preserves every input byte and reports no changed or shadowed keys, including deployed, template, CRLF, BOM, and unterminated inputs. | pure | golden | 3 → 1 | T0208 |
| V01-02 | A populated patch whose values already match the file is byte-identical and reports no changed keys, including non-normalized text. | pure | golden | 2 → 2 | T0207, T0209 |
| V01-03 | A literal-quoted runtime string is reported invalid and is rewritten to a runtime-readable basic string. | pure | finding | 1 → 1 | T0210 |
| V01-04 | Quoted key spellings are invisible to the runtime; edits target the live bare occurrence instead. | pure | golden | 1 → 1 | T0211 |
| V01-05 | A document containing only a quoted spelling of an editable key is refused rather than silently editing an invisible key. | pure | none | 1 → 1 | T0212 |
| V01-06 | Editing an existing value changes only that value while preserving surrounding bytes, comments, key decor, and equals spacing. | pure | golden | 2 → 2 | T0213, T0215 |
| V01-07 | Resolution-scale values retain a fractional TOML spelling such as 1.0. | pure | golden | 1 → 1 | T0214 |
| V01-08 | An absent editable key is appended under the existing streaming table after its existing content. | pure | golden | 1 → 1 | T0216 |
| V01-09 | When streaming is absent it is created at the end with exactly the required blank-line separation, including no leading blank on an empty document. | pure | golden | 2 → 1 | NEW-V01-streaming-table-creation |
| V01-10 | A trailing comment on an edited key moves above it with its indentation, while comments on untouched keys remain byte-stable. | pure | golden | 3 → 3 | T0219, T0220, T0221 |
| V01-11 | For duplicate keys across tables, apply_patch edits only the last physical occurrence, reports the shadow, and preserves dead assignments. | pure | golden | 1 → 1 | T0222 |
| V01-12 | The Settings view reports the last effective accepted value and identifies a shadowed duplicate. | pure | none | 1 → 1 | T0223 |
| V01-13 | Dotted keys are not runtime occurrences, and an ambiguous dotted streaming group is refused for insertion. | pure | none | 1 → 1 | T0224 |
| V01-14 | Invalid TOML is refused by apply_patch, and the write path propagates that refusal without altering the file. | unit | none | 2 → 2 | T0225, T0260 |
| V01-15 | Numeric patches outside runtime ranges are rejected with the affected key named, while inclusive endpoints are accepted. | pure | none | 1 → 1 | T0226 |
| V01-16 | A disk read of the deployed file returns all six runtime-effective values, defaults, clean diagnostics, and modification metadata. | unit | none | 1 → 1 | T0227 |
| V01-17 | An absent config file is represented as a non-error view state with no values or mtime. | unit | none | 1 → 1 | T0228 |
| V01-18 | Rejected assignments with no accepted predecessor produce absent effective values and UI-visible InvalidValue records. | pure | none | 1 → 1 | T0229 |
| V01-19 | Even when TOML parsing fails, read returns the values Config.cpp would obtain and exposes the parse error. | unit | none | 1 → 1 | T0230 |
| V01-20 | The runtime reader is table-blind, comment-aware, compatible with Config.cpp numeric/string parsing, last-valid-wins, and reports shadows. | pure | finding | 2 → 2 | T0231, T0234 |
| V01-21 | Comment stripping tracks only double-quote state and performs no escape handling, matching Config.cpp. | pure | none | 1 → 1 | T0232 |
| V01-22 | A later rejected assignment does not erase an earlier accepted value, every rejection and shadow is reported, and edits still target the last physical line. | pure | finding | 1 → 1 | T0233 |
| V01-23 | Assignments physically inside multiline TOML strings remain live to Config.cpp, make the view non-round-trippable, and are refused by apply_patch, write, and edit_protocol without side effects. | unit | finding | 2 → 1 | T0236 |
| V01-24 | A BOM immediately before a root editable key makes that key invisible to Config.cpp and the whole file unsafe to rewrite. | pure | finding | 1 → 1 | T0237 |
| V01-25 | A BOM before a table header is harmless and does not prevent an otherwise safe edit. | pure | none | 1 → 1 | T0238 |
| V01-26 | Resolution-scale acceptance uses the runtime's f32 precision at both range boundaries. | pure | finding | 1 → 1 | T0239 |
| V01-27 | Two distinct rejected occurrences of one key retain distinct UI identities. | pure | finding | 1 → 1 | T0240 |
| V01-28 | effective_string returns the raw last assignment across tables, strips only double quotes/comments as Config.cpp does, and returns None when absent. | pure | none | 1 → 1 | T0241 |
| V01-29 | effective_accepted returns the last assignment the modeled key's runtime whitelist accepts and None when none qualifies. | pure | finding | 1 → 1 | T0242 |
| V01-30 | effective_accepted and the full runtime line reader resolve maintained shadow fixtures identically. | pure | none | 1 → 1 | T0243 |
| V01-31 | A real edit preserves uniform line endings, leading BOM, and final-newline state byte-for-byte outside the changed value. | pure | finding | 1 → 1 | T0244 |
| V01-32 | Writing an absent file creates from the shared template and then changes exactly the requested value. | unit | golden | 1 → 1 | T0245 |
| V01-33 | Executor create_new leaves an already-created destination's bytes untouched. | unit | none | 1 → 1 | T0019 |
| V01-34 | Writing a pre-existing config reports it as pre-existing and patches its bytes rather than treating it as a template creation. | unit | none | 2 → 1 | T0250 |
| V01-35 | Two same-second backup reservations receive distinct paths and retain their respective bytes. | unit | finding | 1 → 1 | T0247 |
| V01-36 | Write uses the documented cross-process lock path and waits for a contending flock before its best-effort timeout. | unit | finding | 1 → 1 | T0248 |
| V01-37 | An empty-patch write to an absent path creates the shared template verbatim and reports template creation without a backup. | unit | golden | 1 → 1 | T0249 |
| V01-38 | Before replacing an existing file, write creates a byte-identical backup under the backup naming contract. | unit | none | 1 → 1 | T0250 |
| V01-39 | A no-op write neither rewrites the config nor changes its mtime nor creates a backup, including non-normalized files and empty/populated patches. | unit | none | 2 → 1 | NEW-V01-write-noop |
| V01-40 | Same-second backup-name collisions receive monotonically increasing numeric suffixes. | pure | none | 1 → 1 | T0253 |
| V01-41 | After a successful write, the backup ring retains the newest ten entries and prunes the oldest. | unit | none | 1 → 1 | T0254 |
| V01-42 | A failed config commit leaves config bytes, prior backup history, and backup reservations unchanged. | unit | finding | 1 → 1 | T0255 |
| V01-43 | A prune failure after commit does not turn a committed save into an error and does not prevent pruning other removable stale entries. | unit | none | 1 → 1 | T0256 |
| V01-44 | list_backups filters unrelated/malformed entries and sorts valid backups newest-first with same-second suffix ordering and metadata. | unit | none | 1 → 1 | T0257 |
| V01-45 | An existing-file dry run plans backup and replacement writes in order while touching neither config nor backup directory. | unit | none | 1 → 1 | T0258 |
| V01-46 | An absent-file dry run plans template creation and patched replacement without creating the destination. | unit | none | 1 → 1 | T0259 |
| V01-47 | Write returns an error for a non-round-trippable file and leaves its bytes unchanged. | unit | none | 1 → 1 | T0260 |
| V01-48 | Write rejects an out-of-range patch before any template creation or other disk mutation. | unit | none | 1 → 1 | T0261 |
| V01-49 | Write refuses live sessions detected from same-process records, foreign-owner records, or live runtime status, but permits a stale dead-process record. | unit | finding | 4 → 4 | T0262, T0263, T0265, T0266 |
| V01-50 | A live-session write refusal names the bottle and actionable demo.sh stop command and contains no formatting craters. | unit | finding | 2 → 1 | T0262 |
| V01-51 | Replacement refuses changed underlying bytes with an actionable path/message, while dry-run comparison is exempt. | unit | finding | 1 → 1 | T0267 |
| V01-52 | edit_protocol changes only protocol to alvr, returns a changed FixReport, and preserves the prior file in Sabrage's backup directory. | stage | none | 1 → 1 | T0268 |
| V01-53 | edit_protocol reports unchanged and creates no backup when the effective file already says alvr. | stage | none | 1 → 1 | T0269 |
| V01-54 | When the file is absent, edit_protocol reports a change and explains that creation came from the shared template. | stage | golden | 1 → 1 | T0270, T0249 |
| V01-55 | Dry-run edit_protocol reports what would change, plans work, and writes neither config nor backup. | stage | none | 1 → 1 | T0271 |
| V01-56 | Each runtime enum's accepted spelling round-trips through its production parser, and spellings are case-sensitive. | pure | none | 1 → 1 | T0272 |
| V01-57 | Every EDITABLE_KEYS entry maps to a populated patch value, while a default patch maps none of them. | pure | none | 1 → 1 | T0274 |

#### V02 — `sabrage-core:session/reconcile.rs`, `sabrage-core:session/state.rs`

*The 53 roster functions protect 44 distinct guarantees; I found a safe target of 44 surviving functions, a nine-function reduction rather than evidence that most of the suite is redundant. The strongest reductions are four stop-tail entry-point mirrors, one private schema predicate, one audio-helper merge, and two literal tables. Finding-pinned ownership, forward-progress, schema, and detach tests remain separate because they exercise different branches or layers. The dedicated row-text golden remains, but its unrelated serde round-trip assertion should go.*

| id | behaviour | layer | protected by | now → target | carriers |
|---|---|---|---|---:|---|
| V02-01 | Absent or dead wine identities classify Dead, a matching identity classifies Live, and a recycled identity classifies IdentityMismatch. | pure | none | 1 → 1 | T0692 |
| V02-02 | PID zero, dead, recycled, and unverifiable identities are never signalable; only a verified live identity may be signalled. | pure | none | 2 → 1 | T0693 |
| V02-03 | Live and alive-but-unverifiable wine identities are adopted as Live without rows, mutations, or record deletion. | unit | finding | 3 → 1 | NEW-V02-live-reconcile |
| V02-04 | With no state file, reconciliation returns NoSession silently and performs no action. | unit | none | 2 → 1 | T0695 |
| V02-05 | A dead session restores audio, dashboard, and recorded forwards in order, persists each release, emits one banner, and clears its record. | unit | contract | 2 → 1 | T0697 |
| V02-06 | An identity mismatch restores only PID-free guards and never signals or marks the recorded dashboard. | unit | contract | 1 → 1 | T0698 |
| V02-07 | A record matching this process's Preflight, Launching, or Stopping phase is silent Busy, untouched, and retained. | unit | finding | 1 → 1 | T0699 |
| V02-08 | An in-flight phase for another run does not shield a stale record from recovery. | unit | none | 1 → 1 | T0700 |
| V02-09 | A record owned by another live process is reported as non-silent Busy and left byte- and machine-untouched. | unit | finding | 1 → 1 | T0701 |
| V02-10 | A newer-schema record is reported as non-silent Busy and is never restored, rewritten, or cleared. | unit | finding | 1 → 1 | T0702 |
| V02-11 | Only non-silent Busy outcomes expose a launch-refusal reason; self-in-flight and non-Busy outcomes do not. | unit | finding | 1 → 1 | T0703 |
| V02-12 | Stop keeps verified-live and unverifiable-live records without mutation and names the appropriate reason. | unit | finding | 2 → 1 | NEW-V02-stop-live-identity |
| V02-13 | A failed forward removal remains recorded and pending, while successful siblings are removed and the record is retained. | unit | finding | 1 → 1 | T0706 |
| V02-14 | Each successful forward removal is persisted before the next removal is attempted. | unit | finding | 1 → 1 | T0707 |
| V02-15 | A record this process still supervises is left to its supervisor without rows, mutations, or deletion. | unit | none | 2 → 1 | T0708 |
| V02-16 | An unreadable record emits warning and remedy rows, does not block launch, and is not deleted. | unit | none | 1 → 1 | T0709 |
| V02-17 | If the recorded output is disconnected, reconciliation tries it first, falls back to the built-in output, reports the fallback, and completes recovery. | unit | contract | 1 → 1 | T0710 |
| V02-18 | If neither the recorded output nor an audible fallback can be selected, reconciliation reports the remedy and preserves a distinguishable pending record. | unit | finding | 2 → 1 | T0711 |
| V02-19 | A stale record with no recorded guards clears silently without unnecessary writes or commands. | unit | none | 1 → 1 | T0713 |
| V02-20 | If the user already restored the output, reconciliation issues no switch or row but marks and persists the audio guard released. | unit | none | 2 → 1 | T0719 |
| V02-21 | An unavailable audio probe leaves the audio guard pending and performs no mutation. | unit | none | 1 → 1 | T0715 |
| V02-22 | Already-released recorded guards are not restored, reported, or planned again. | unit | none | 1 → 1 | T0716 |
| V02-23 | A dead or recycled dashboard identity is marked complete without a signal or false close report. | unit | none | 1 → 1 | T0717 |
| V02-24 | Without adb, recorded forwards remain pending and no removal is planned. | unit | none | 1 → 1 | T0718 |
| V02-25 | Reconciliation row text, dry-run verbs, section text, retry text, record-kept text, and step id remain byte-stable. | unit | golden | 1 → 1 | T0720 |
| V02-26 | Classification, restore-mode, and Reconciled values serialize with the exact camelCase discriminants and fields consumed by the UI. | ipc | contract | 1 → 1 | T0721 |
| V02-27 | Stop ignores a state record for a different bottle without rows, mutations, or deletion. | unit | none | 1 → 1 | T0724 |
| V02-28 | A non-cancellation restore failure becomes warn and retry rows, does not fail Stop, and leaves the unreleased guard on disk. | unit | none | 1 → 1 | T0726 |
| V02-29 | Cancellation during reconciliation propagates to the caller without being rendered as a partial-restore report. | unit | none | 1 → 1 | T0727 |
| V02-30 | Normal detach fires only the detach token, marks the record detached, and preserves every guard and restoration value. | unit | contract | 1 → 1 | T0729 |
| V02-31 | Once Stop has fired, detach cannot fire its token or relabel the record. | unit | finding | 1 → 1 | T0730 |
| V02-32 | If Stop wins while detach is waiting, detach does not relabel the retained record. | unit | finding | 1 → 1 | T0731 |
| V02-33 | A detach timeout may fire the detach token but must not write or alter the supervised record. | unit | finding | 1 → 1 | T0732 |
| V02-34 | Detach never recreates a record that the supervisor already cleared. | unit | none | 1 → 1 | T0733 |
| V02-35 | Session state has absent-file semantics and round-trips through atomic file persistence with its expected field names and newline. | unit | contract | 1 → 1 | T0734 |
| V02-36 | A minimal older session record loads with safe defaults for ownership, processes, guards, forwards, and detach state. | unit | contract | 1 → 1 | T0735 |
| V02-37 | A corrupt session-state file is an I/O error rather than an absent record. | unit | none | 1 → 1 | T0736 |
| V02-38 | Fresh state uses the current schema and owner, and pending-guard status remains true until every recorded guard is individually released. | unit | none | 1 → 1 | T0737 |
| V02-39 | A dry-run state save plans directory creation and writing without creating the file. | unit | none | 1 → 1 | T0738 |
| V02-40 | Foreign ownership requires a live matching owner and a session that can still be running; self, recycled, exited, and legacy-zero owners do not qualify. | unit | none | 1 → 1 | T0739 |
| V02-41 | Save and clear refuse a different run's live foreign-owned record, while owner writes and replacement after owner exit remain allowed. | unit | finding | 1 → 1 | T0740 |
| V02-42 | State mutators reject newer-schema records across save, clear, and clear_run without changing their unknown bytes. | unit | finding | 2 → 1 | T0742 |
| V02-43 | clear_run removes only its named run, preserves a superseding run, and succeeds when the file is already absent. | unit | finding | 1 → 1 | T0743 |
| V02-44 | Unconditional clear is idempotent. | unit | none | 1 → 1 | T0744 |

#### V03 — `sabrage-core:stages/run/mod.rs`

*These 34 functions protect 33 guarantees spanning rendered text, run phases, teardown outcomes, three distinct launch-refusal authorities, and nine finding-pinned tests. Only T0461 is fully redundant; T0455 can merge into T0459 while retaining its round-qualified label, leaving 32 functions. Seven other rows should lose duplicated or vacuous assertions, especially the core copy of parity-owned helper text and claims made with empty fixtures. The large-looking refusal, cancellation, and phase families remain load-bearing because they exercise different production branches or state transitions.*

| id | behaviour | layer | protected by | now → target | carriers |
|---|---|---|---|---:|---|
| V03-01 | The shell-owned helper-reaped line remains byte-identical to scripts/demo/run.sh. | parity | parity | 1 → 1 | T0877 |
| V03-02 | Normal-exit and detach closing renderers interpolate the supplied status and log path into their exact observable rows. | pure | golden | 1 → 1 | T0444 |
| V03-03 | A recorded live-session refusal names its pid, civil start time, bottle, both stop routes, emits Fatal, and exits 1. | unit | parity | 1 → 1 | T0445 |
| V03-04 | Local timestamps render as civil RFC3339-like text, with out-of-range values falling back to the raw number without panic. | pure | none | 1 → 1 | T0446 |
| V03-05 | Dry-run teardown preserves its recorder, while real teardown receives a fresh uncancelled executor context with the same identity and paths. | unit | none | 1 → 1 | T0447 |
| V03-06 | A normal no-guard teardown returns wine's status, emits a blank row then the status row, and plans no guard or record work. | unit | parity | 1 → 1 | T0448 |
| V03-07 | On normal exit, the status row precedes audio restoration, and the audio guard is consumed and recorded restored. | unit | parity | 1 → 1 | T0449 |
| V03-08 | A normal teardown preserves a nonzero wine status even when saving released-guard state fails, warns, and clears the live handle. | unit | none | 2 → 1 | T0450 |
| V03-09 | A teardown with an unreleased recorded guard re-saves and retains the session record, explains why, and preserves wine's status. | unit | finding | 1 → 1 | T0451 |
| V03-10 | Outstanding wired forwards alone do not make run teardown retain a record after the guards teardown owns are restored. | unit | finding | 1 → 1 | T0452 |
| V03-11 | Cancelled teardown remains exit 130 and completes guard restoration, warning, guard consumption, and live-handle cleanup despite a state-save failure. | unit | finding | 1 → 1 | T0453 |
| V03-12 | Detach persists its record before disarming; if persistence fails, it warns, keeps guards armed, and clears the live handle. | unit | finding | 1 → 1 | T0454 |
| V03-13 | A kept recovery record carries every unfinished audio and wired-forward guard into the next launch, excludes completed guards, and deduplicates serial-port pairs. | pure | finding | 2 → 1 | T0459 |
| V03-14 | A recorded Live classification refuses public run before any preflight check or permanent executor action and clears the run phase. | stage | finding | 1 → 1 | T0456 |
| V03-15 | A shell-started session visible only through fresh runtime_status refuses public run before preflight or permanent work. | stage | finding | 1 → 1 | T0457 |
| V03-16 | A foreign live owner's Busy record causes a public-run refusal before preflight, remains on disk, and is never restored, removed, or reset over. | stage | finding | 1 → 1 | T0458 |
| V03-17 | Late run teardown never removes a newer run's record, while it still removes the record whose run id it expects. | unit | finding | 1 → 1 | T0460 |
| V03-18 | Cancellation emits the interrupt announcement, plans teardown wineserver kill then wait, and returns exit 130. | unit | parity | 1 → 1 | T0462 |
| V03-19 | Successful detach marks and persists the session, retains pending guards and the state record, emits its announcement, and returns zero. | unit | parity | 1 → 1 | T0463 |
| V03-20 | Dry-run teardown returns zero without emitting rows or planning actions. | unit | none | 1 → 1 | T0464 |
| V03-21 | Failed teardown propagates the original stage error rather than replacing it with teardown output. | unit | none | 1 → 1 | T0465 |
| V03-22 | Composite Guards release consumes both held guard slots and a second release is a no-op. | unit | none | 1 → 1 | T0466 |
| V03-23 | Composite Guards disarm consumes both guard-owner slots. | unit | none | 1 → 1 | T0467 |
| V03-24 | A preparation checkpoint succeeds while cancellation is live and immediately returns Cancelled after the token fires. | pure | none | 1 → 1 | T0468 |
| V03-25 | Cancellation after the audio guard is armed ends guarded supervision before dashboard, launch, or live-session publication while retaining the guard for teardown. | unit | none | 1 → 1 | T0469 |
| V03-26 | A guarded dry run plans the wine spawn but supervises nothing, publishes no live session, records no wine identity, and creates no log directory. | unit | none | 1 → 1 | T0470 |
| V03-27 | RunPhaseScope publishes phase identity, clears only its own run on Drop, and deliberately preserves a finalized Exited phase and code. | unit | none | 1 → 1 | T0471 |
| V03-28 | Public run publishes Preflight before its first emitted check row and clears that phase when preflight fails. | stage | none | 1 → 1 | T0472 |
| V03-29 | Bottle resolution failure occurs before any run phase is published. | stage | none | 1 → 1 | T0473 |
| V03-30 | Normal teardown exposes Stopping for its blank row, then a surviving Exited phase carrying code, bottle, and run identity. | unit | none | 1 → 1 | T0474 |
| V03-31 | Cancelled and failed teardown publish Stopping while detached and dry-run teardown do not; all four non-normal outcomes finish with no retained phase. | unit | none | 1 → 1 | T0475 |
| V03-32 | The detach announcement closes RUN_SUPERVISE rather than being attributed to RUN_TEARDOWN. | unit | none | 1 → 1 | T0476 |
| V03-33 | Failure to persist an already-running session is best effort: supervision continues and one actionable warning explains the lost restart discoverability. | unit | none | 1 → 1 | T0477 |

#### V04 — `sabrage-core:stages/run/actions.rs`, `sabrage-core:stages/run/guards.rs`

*This slice protects 50 distinct guarantees across launch preparation, Goldberg staging, guarded teardown, and shell parity. Seven of 49 test functions are removable: three core copies should move entirely to parity, two private-seam tests are implied by public-stage tests, and two Drop tests are vacuous. Five surviving tests should lose one duplicated or unobservable assertion, leaving 42 functions. The rollback, precondition, and guard-state families are mostly load-bearing despite sharing literals because their triggers and resulting persisted states differ.*

| id | behaviour | layer | protected by | now → target | carriers |
|---|---|---|---|---:|---|
| V04-01 | The native launch-action IDs equal the contract's seven IDs in order. | parity | parity | 1 → 1 | T0869 |
| V04-02 | The Run stage has one step per launch action plus preflight, supervise, and teardown. | stage | contract | 1 → 1 | T0396 |
| V04-03 | The wired-device parser skips the header and non-device states and returns the first exact device row. | pure | none | 1 → 1 | T0397 |
| V04-04 | Wired forwarding derives tcp:9943 and tcp:9944 from the contract. | stage | contract | 2 → 1 | T0403 |
| V04-05 | A non-wired run with no adb performs no mutation and emits no event. | stage | none | 1 → 1 | T0399 |
| V04-06 | Non-wired forward cleanup is attributed to the Run stage's adb-forward step. | stage | none | 1 → 1 | T0400 |
| V04-07 | Wired mode without adb fails with run.sh's exact fatal text. | stage | golden | 1 → 1 | T0401 |
| V04-08 | Wired mode without a connected device fails with run.sh's exact fatal text. | stage | golden | 1 → 1 | T0402 |
| V04-09 | A successful wired run records and plans both forwards, never remove-all, and reports the exact summary. | stage | contract | 1 → 1 | T0403 |
| V04-10 | An ordinary failed forward removes both ports, clears the record, and returns the shell fatal. | stage | golden | 1 → 1 | T0404 |
| V04-11 | A failed rollback removal retains the indeterminate forward in memory and on disk. | stage | finding | 1 → 1 | T0405 |
| V04-12 | Cancellation between forwards rolls the first forward back using a fresh executor and persists the result. | stage | finding | 1 → 1 | T0406 |
| V04-13 | Failure saving the second forward's record rolls back the first and never attempts the second. | stage | none | 1 → 1 | T0407 |
| V04-14 | Cancellation during the adb device probe is reported as cancellation, not no-device failure. | stage | none | 1 → 1 | T0408 |
| V04-15 | Goldberg staging without either steam_api64.dll location fails with the exact path-bearing fatal. | stage | golden | 1 → 1 | T0409 |
| V04-16 | A fresh Goldberg dry run plans backup, install, appid, settings directory, and three flags in order at exact destinations. | stage | golden | 1 → 1 | T0410 |
| V04-17 | Real Goldberg staging preserves the original backup, installs Goldberg, creates empty flags, and never refreshes the backup. | stage | golden | 2 → 1 | T0412 |
| V04-18 | steam_appid.txt lands on disk as exactly the contract appid digits with no trailing newline. | parity | parity | 1 → 1 | T0871 |
| V04-19 | An existing usable backup and installed Goldberg cause no copies while settings are refreshed and already-installed is reported. | stage | golden | 1 → 1 | T0411 |
| V04-20 | When the newly minted backup is itself Goldberg, staging warns truthfully while preserving shell artifact parity. | stage | finding | 1 → 1 | T0413 |
| V04-21 | A non-file object at the reserved backup name fails closed before the live Steam DLL changes. | stage | finding | 1 → 1 | T0414 |
| V04-22 | A backup known to be Goldberg receives durable provenance under Sabrage's application-support store only. | stage | finding | 1 → 1 | T0415 |
| V04-23 | An ordinary Steam backup receives no Goldberg provenance marker. | stage | none | 1 → 1 | T0416 |
| V04-24 | Goldberg staging falls back to the game-root DLL and places companion artifacts beside it. | stage | none | 1 → 1 | T0417 |
| V04-25 | The launch environment matches run.sh, including verbose defaults, caller precedence, and empty-as-unset semantics. | parity | parity | 1 → 1 | T0872 |
| V04-26 | The Wine program and argv match run.sh's exact launch command. | parity | parity | 1 → 1 | T0873 |
| V04-27 | The Wine ChildSpec is attributed to RUN_LAUNCH, carries all required environment keys, and supplies Finder-safe PATH. | unit | none | 1 → 1 | T0419 |
| V04-28 | The launch banner's nine rendered entries, including blank lines and interpolated paths, match the shell ordering. | parity | parity | 1 → 1 | T0879 |
| V04-29 | The banner headline is a Section and every Text event is attributed to RUN_LAUNCH. | unit | none | 1 → 1 | T0420 |
| V04-30 | Only an AlreadyExists I/O error is retryable during detached launch. | pure | none | 1 → 1 | T0421 |
| V04-31 | The Steam backup path appends .orig-steam to the complete DLL filename. | stage | none | 2 → 1 | T0410 |
| V04-32 | Wineserver survivor lists render pid-basename pairs with a trailing space and a fallback basename. | pure | golden | 1 → 1 | T0423 |
| V04-33 | Launch-time wineserver reset plans -k then -w and reports the bottle section and down result. | stage | golden | 1 → 1 | T0424 |
| V04-34 | Without CrossOver's wineserver, reset plans nothing but still reports wineserver down. | stage | golden | 1 → 1 | T0425 |
| V04-35 | ADB reverse cleanup is silent and inert without adb or for a non-ALVR protocol. | stage | none | 1 → 1 | T0426 |
| V04-36 | On ALVR with a device, reverse cleanup plans `reverse --remove-all` and emits the exact attributed info row. | stage | golden | 1 → 1 | T0427 |
| V04-37 | Audio eligibility follows run.sh's no-audio, protocol, and binary-presence precedence. | pure | golden | 1 → 1 | T0428 |
| V04-38 | BlackHole is recognized only as an exact complete output-device line. | pure | golden | 1 → 1 | T0429 |
| V04-39 | Dashboard eligibility follows run.sh's disabled, protocol, executable, and not-built precedence. | pure | golden | 1 → 1 | T0430 |
| V04-40 | Audio switch/restore and dashboard opening/closed renderers retain their exact shell text. | parity | parity | 1 → 1 | T0878 |
| V04-41 | No-audio returns an inert guard, emits one info row, and performs no save or restore work. | stage | golden | 1 → 1 | T0432 |
| V04-42 | A cancelled audio switch leaves an armed guard for normal async teardown, which records exactly one successful restore. | stage | finding | 1 → 1 | T0433 |
| V04-43 | A non-ALVR protocol causes AudioGuard acquisition and release to touch audio not at all. | stage | golden | 1 → 1 | T0434 |
| V04-44 | If the recorded output vanished, release tries it first, falls back to built-in speakers, and marks restoration complete. | stage | none | 1 → 1 | T0437 |
| V04-45 | If no audible output can be restored, release warns, attempts no virtual fallback, and leaves the guard pending. | stage | none | 1 → 1 | T0438 |
| V04-46 | No-dashboard returns an inert guard, emits one info row, and performs no work. | stage | golden | 1 → 1 | T0439 |
| V04-47 | A missing dashboard binary emits the exact attributed warning and continues. | stage | golden | 1 → 1 | T0440 |
| V04-48 | An eligible dashboard dry run plans a detached null-stdio spawn, emits opening, and records no identity. | stage | golden | 1 → 1 | T0441 |
| V04-49 | Dashboard release never signals a mismatched process identity but still completes persisted cleanup state. | stage | none | 1 → 1 | T0442 |
| V04-50 | The output-device listing probe honors cancellation promptly and yields no devices. | unit | finding | 1 → 1 | T0443 |

#### V05 — `sabrage-core:stages/run/preflight.rs`

*These tests primarily protect preflight ordering, runtime-compatible TOML interpretation, gate outcomes, autofix event/state guarantees, and exact run.sh-compatible diagnostics. The real redundancy is concentrated in private-seam matrices, one protocol literal-variant pair, a duplicated helper-failure fixture, and renderer bytes that belong in the CI-running parity test. The proposed target is 27 functions from 34: seven functions collapse while one additional test merely drops its parity-owned equality assertion. All roster functions exist, and the module contains no unlisted tests.*

| id | behaviour | layer | protected by | now → target | carriers |
|---|---|---|---|---:|---|
| V05-01 | The native launch preflight slug list equals the contract's native-gating checks in contract order. | parity | parity | 1 → 1 | T0868 |
| V05-02 | The preflight slug list is non-empty and unique, excludes Gate::None, and includes both run-only wine and bridge checks. | pure | contract | 1 → 1 | T0478 |
| V05-03 | Every current Autofix gate names a supported FixAction, and game.version is the only Warn gate. | pure | contract | 1 → 1 | T0479 |
| V05-04 | Raw effective-string parsing is table-blind, last-assignment-wins, quote/comment aware, key-specific, and returns empty for an absent key at the preflight wrapper. | pure | none | 1 → 1 | T0241, T0482 |
| V05-05 | Modeled preflight facts agree with the runtime and Settings last-accepted interpretation when assignments are shadowed. | unit | finding | 2 → 1 | T0504 |
| V05-06 | encoder_process defaults to auto for a missing file or missing key and preserves an explicit accepted value. | unit | none | 1 → 1 | T0482 |
| V05-07 | auto/native require the helper, inproc disables it, and unknown values warn while still requiring it. | stage | none | 5 → 4 | T0487, T0496, T0497, T0504 |
| V05-08 | Non-wired adb and inproc helper checks are inapplicable with explicit reasons; wired, helper-requiring and unrelated rows remain applicable. | pure | none | 1 → 1 | T0484 |
| V05-09 | A pre-cancelled token aborts preflight before the first Check row. | stage | none | 1 → 1 | T0485 |
| V05-10 | Bottle resolution failure occurs before any registry Check row. | stage | none | 1 → 1 | T0486 |
| V05-11 | A clean preflight returns its config facts and emits exactly one final Check per slug in order, with no autofix. | stage | contract | 1 → 1 | T0487 |
| V05-12 | A stale generated-contract header blocks on row zero, carries the regen remedy, emits one Fatal and permits no later autofix. | stage | contract | 1 → 1 | T0488 |
| V05-13 | An existing Goldberg DLL with a non-pinned hash is reported as Warn with an explanation and does not block launch. | stage | none | 1 → 1 | T0489 |
| V05-14 | A missing Goldberg DLL emits a failed row and aborts with run.sh's setup text. | stage | golden | 1 → 1 | T0490 |
| V05-15 | A wrong Beat Saber version emits run.sh's interpolated warning and launch continues. | stage | golden | 1 → 1 | T0491 |
| V05-16 | A missing game executable aborts with run.sh's three-line path and depot-command diagnostic. | stage | golden | 1 → 1 | T0492 |
| V05-17 | A missing runtime TOML aborts with the exact setup remedy. | stage | golden | 1 → 1 | T0493 |
| V05-18 | An effective oxrsys protocol passes the supported row but blocks at the native legacy row with the declared two-line divergence text. | stage | finding | 2 → 1 | NEW-V05-PROTOCOL-OXRSYS |
| V05-19 | An unknown protocol aborts at the supported row with run.sh's two-line diagnostic and never evaluates the legacy row. | stage | golden | 1 → 1 | T0495 |
| V05-20 | inproc emits its notice once, skips both helper rows with reasons and performs no helper autofix. | stage | golden | 1 → 1 | T0496 |
| V05-21 | An unknown encoder value is returned raw, warned once and treated as helper-requiring auto. | stage | golden | 1 → 1 | T0497 |
| V05-22 | An applicable but unverifiable check remains visible as Skipped and is fatal rather than a pass. | stage | none | 1 → 1 | T0498 |
| V05-23 | Wired preflight without adb aborts with run.sh's exact diagnostic. | stage | golden | 1 → 1 | T0499 |
| V05-24 | A real backend autofix writes dxmt, emits one AutoFixed, rechecks to Pass and emits only the final Check. | stage | none | 1 → 1 | T0500 |
| V05-25 | A dry-run backend autofix performs no write and reports the planned change as Info rather than a failed recheck. | stage | none | 1 → 1 | T0501 |
| V05-26 | An unfixable helper aborts with the exact ensure-helper text while emitting exactly one helper Check and one Fatal. | stage | golden | 2 → 1 | T0502 |
| V05-27 | The shipped shadowed-invalid fixture launches using accepted alvr/native facts matching Settings and passes both helper checks. | stage | finding | 1 → 1 | T0504 |
| V05-28 | Trailing invalid protocol and encoder assignments preserve alvr/inproc, skip helpers, emit no false warning and explain the disagreeing doctor row. | stage | finding | 2 → 1 | T0505 |
| V05-29 | Cancellation interrupts a wired adb child probe promptly instead of waiting for the probe to finish. | stage | finding | 1 → 1 | T0506 |
| V05-30 | A later native encoder assignment overrides an earlier inproc value, requires the helper and suppresses the inproc notice. | stage | finding | 1 → 1 | T0507 |
| V05-31 | A backend autofix write failure emits one failed Check with cause, one shell-shaped Fatal and stderr detail without changing state. | stage | finding | 1 → 1 | T0508 |
| V05-32 | block_die renders the exact shell-backed and declared native-only messages and preserves run-only evaluator messages. | parity | parity | 1 → 1 | T0876 |
| V05-33 | post_fix_die renders the exact backend and helper validation diagnostics. | parity | parity | 1 → 1 | T0876 |

#### V06 — `sabrage-core:session/mod.rs`, `sabrage-core:session/watcher.rs`

*These modules protect the single live-session policy, stop routing, audio fallback, runtime/log parsing, and the watcher’s temporal attribution and source-precedence rules. Most of the watcher mass is not redundant: similar fixtures distinguish newly started versus adopted sessions, phase versus identity selection, and freshness within versus across runs. The safe reduction is three platform/fixture-only deletions, four two-function tables, three merges into named carriers, and two partial assertion trims. That takes the 39 roster functions to 29 carriers, including the already-programmed H3-2 encoder-fixture table.*

| id | behaviour | layer | protected by | now → target | carriers |
|---|---|---|---|---:|---|
| V06-01 | `SessionStatus` and nested `EncoderInfo` expose their stable camelCase field names on the Rust-to-UI IPC wire. | ipc | contract | 1 → 1 | T0679 |
| V06-02 | Every `SessionPhase` has its stable nine-word camelCase IPC representation. | ipc | contract | 2 → 1 | T0680 |
| V06-03 | The unified idle gate refuses authoritative in-process and persisted session signals, accepts dead/finished signals, and returns the shared stop remedy. | unit | none | 1 → 1 | T0681 |
| V06-04 | A running Beat Saber process with no Sabrage files both blocks mutation and renders as an unowned External session. | unit | finding | 2 → 2 | T0682, T0763 |
| V06-05 | Runtime status establishes an external live session only when its timestamp is fresh and its named pid is alive; the predicate, gate, and snapshot agree. | unit | finding | 3 → 3 | T0681, T0752, T0762 |
| V06-06 | Stop routing cancels an identifiable pre-spawn run, fires the owned live token for a supervised session, and otherwise invokes the bottle-scoped stop stage. | pure | none | 1 → 1 | T0683 |
| V06-07 | The live-session slot reports its run identity and only the owning run may clear it; stale and repeated clears are harmless. | unit | none | 2 → 1 | T0685 |
| V06-08 | The run-phase slot publishes phase, run id, bottle, and exit code as one value and only its owning run may clear it. | unit | none | 1 → 1 | T0687 |
| V06-09 | Audio fallback prefers built-in speakers, otherwise selects the first non-virtual output, and returns none when no real output exists. | pure | none | 2 → 1 | NEW-V06-audio-fallback |
| V06-10 | Normal, dry-run, and unrestorable audio-fallback diagnostics retain their exact text. | unit | golden | 1 → 1 | T0691 |
| V06-11 | The monitor derives both runtime-status and runtime-log paths from the configured OXRSys application-support root. | unit | none | 1 → 1 | T0745 |
| V06-12 | The runtime-status parser accepts compatible documents with unknown or absent optional fields and returns none for incomplete input. | unit | none | 2 → 1 | T0747 |
| V06-13 | The runtime-status maximum age, startup grace, and post-fresh stall grace retain their configured durations. | pure | none | 2 → 1 | NEW-V06-watcher-budgets |
| V06-14 | Freshness accepts timestamps through the past-age and small future-skew boundaries but rejects values outside either window. | pure | finding | 2 → 1 | NEW-V06-freshness-boundaries |
| V06-15 | An oxrsys spdlog prefix is parsed as local wall-clock milliseconds, while undated or malformed lines carry no timestamp. | pure | finding | 1 → 1 | T0753 |
| V06-16 | Encoder-ready fixture lines for HEVC/native-helper and H.264/in-process are parsed field-for-field. | pure | none | 2 → 1 | NEW-H3-2 |
| V06-17 | Unrelated runtime-log fixture lines and empty input do not produce encoder information. | pure | none | 1 → 1 | T0756 |
| V06-18 | The encoder-ready marker parses without requiring an spdlog timestamp prefix. | pure | none | 1 → 1 | T0757 |
| V06-19 | The oxrsys encoder-ready source format remains byte-compatible with Sabrage's parser. | parity | golden | 1 → 1 | T0758 |
| V06-20 | A newly started run never inherits a preloaded encoder line, accepts a line appended during that run, and does not pass its chip to the next run. | unit | finding | 1 → 1 | T0759 |
| V06-21 | An adopted session accepts only timestamped preloaded encoder lines written after that session started. | unit | finding | 2 → 1 | T0761 |
| V06-22 | An externally started runtime snapshot carries only its verified pid, remains unowned with no invented run or bottle, and loses to this process's published preflight. | unit | finding | 1 → 1 | T0762 |
| V06-23 | Freshness and last-fresh history are reset on run identity changes without disabling stall detection for history belonging to the current run. | unit | finding | 1 → 1 | T0764 |
| V06-24 | A live pid with unverifiable start time is rendered Running for both live-handle and persisted-record sources, consistently with the mutation door. | unit | finding | 1 → 1 | T0765 |
| V06-25 | A status written before the current session started is not fresh, although its opaque state remains available for display. | unit | finding | 1 → 1 | T0766 |
| V06-26 | When live, persisted, and published phase sources conflict, the documented phase precedence is applied. | unit | none | 2 → 2 | T0767, T0768 |
| V06-27 | Snapshot identity and ownership come from the strongest winning source, displaced-run fields are cleared, and a compatible published Exited code is retained. | unit | none | 1 → 1 | T0768 |
| V06-28 | Across sequential snapshots the monitor reports Idle, Detached, Exited, transient published phases, Running and Stalled correctly, attributes encoder chips, and clears them when the session ends. | unit | none | 1 → 1 | T0769 |

#### V07 — `sabrage-core:stages/mod.rs`, `sabrage-core:stages/stop.rs`

*These 56 tests protect 44 distinct guarantees, concentrated in stage dispatch policy, Stop reporting, process reaping, cancellation, and shell-visible text. The standard target is 44 test functions: four two-test families become tables, two private-seam tests merge into existing owner tests, six functions delete, and eight surviving tests lose redundant or tautological assertions. The largest true redundancy is the four-test mirror of the session layer in stages/mod.rs; the apparent cancellation and wedged-probe twins mostly exercise different control-flow boundaries and remain load-bearing. This was a read-only assessment; no builds or tests were run.*

| id | behaviour | layer | protected by | now → target | carriers |
|---|---|---|---|---:|---|
| V07-01 | StageCtx::new selects RealExecutor normally and DryRunExecutor when dry_run is set. | unit | none | 1 → 1 | T0323 |
| V07-02 | require_bottle emits the shell-compatible Fatal text for both an absent bottle name and a named-but-missing bottle. | unit | golden | 1 → 1 | T0324 |
| V07-03 | run_stage brackets a failed dispatch with StageStarted and failed StageFinished carrying the equivalent exit code. | stage | none | 1 → 1 | T0325 |
| V07-04 | StageOutcome is successful only for exit code zero and preserves a completed nonzero Wine status. | pure | none | 1 → 1 | T0326 |
| V07-05 | Run uses a 5-second fatal wineserver budget while Stop uses a distinct 4-second soft budget. | pure | parity | 1 → 1 | T0327 |
| V07-06 | StageCtx::check_ctx forwards the bottle and every launch flag while retaining normal ADB probes. | unit | none | 1 → 1 | T0328 |
| V07-07 | The in-process operation mutex admits one holder and reports itself held. | unit | none | 1 → 1 | T0329 |
| V07-08 | The advisory lock excludes a second file description, records its holder pid, and releases on drop. | unit | none | 1 → 1 | T0330 |
| V07-09 | The public operation guard acquires the advisory file lock as well as the process mutex. | unit | finding | 1 → 1 | T0331 |
| V07-10 | An advisory-lock wait terminates for both an already-fired token and a token fired during polling. | unit | none | 1 → 1 | T0332 |
| V07-11 | A queued stage announces its run id and wait, cancels with exit 130, and never dispatches. | stage | finding | 1 → 1 | T0333 |
| V07-12 | operation_in_progress_anywhere detects an independently held advisory file lock. | unit | none | 1 → 1 | T0334 |
| V07-13 | The stage live-session alias follows the session owner's policy for idle, persisted-live, runtime freshness, foreign ownership, and unreadable records. | unit | none | 4 → 1 | T0681 |
| V07-14 | Setup, Build, and Install refuse a live session, while Run and Stop remain available. | stage | finding | 1 → 1 | T0340 |
| V07-15 | A queued mutating stage rechecks liveness after acquiring the lock and refuses a session that started during the wait. | stage | finding | 1 → 1 | T0341 |
| V07-16 | Both dispatch doors reject contract skew for Setup, Build, Install, and Run before events or mutation; Stop remains open. | stage | finding | 1 → 1 | T0342 |
| V07-17 | The contract-skew stage refusal preserves the meta.contract-sync message and remedy verbatim. | unit | contract | 1 → 1 | T0343 |
| V07-18 | StepEmitter rows carry the bound step while direct StageCtx rows carry no step. | unit | none | 1 → 1 | T0344 |
| V07-19 | Stop's lsof parser renders sorted CMD(PID) tokens with the shell's trailing-space shape. | unit | golden | 1 → 1 | T0361 |
| V07-20 | Stop maps a successful lsof probe to the correct step, severity, and free-or-held shell text. | stage | golden | 1 → 1 | T0362 |
| V07-21 | A cancelled lsof probe returns Cancelled promptly instead of awaiting the child. | unit | finding | 1 → 1 | T0363 |
| V07-22 | A cancelled port report emits no machine-state row. | unit | finding | 1 → 1 | T0364 |
| V07-23 | A timed-out lsof probe warns and never reports free ports. | unit | finding | 1 → 1 | T0365 |
| V07-24 | A timed-out SwitchAudioSource probe warns and never names an empty device. | unit | finding | 1 → 1 | T0366 |
| V07-25 | Audio-probe cancellation returns Cancelled promptly and the audio reporter remains silent. | unit | finding | 1 → 1 | T0367 |
| V07-26 | A missing lsof binary follows the shell's failed-command substitution and reports streaming ports free. | stage | golden | 1 → 1 | T0368 |
| V07-27 | Survivor formatting covers empty input, basename pairs, trailing spaces, and the no-filename fallback. | pure | golden | 2 → 1 | NEW-format_survivors_cases |
| V07-28 | Command-line matching follows pgrep -f semantics for Windows paths, split arguments, negatives, and empty argv. | pure | none | 1 → 1 | T0123 |
| V07-29 | The process scanner finds the current test process by an argv substring and rejects an absent needle. | unit | none | 1 → 1 | T0123 |
| V07-30 | Stop reports an empty survivor scan as game-and-wineserver-down and a nonempty scan as a warning on the wineserver step. | stage | golden | 1 → 1 | T0373 |
| V07-31 | Only the exact BlackHole 2ch device name selects Stop's still-BlackHole branch. | pure | none | 1 → 1 | T0374 |
| V07-32 | Stop's still-BlackHole warning and restoration hint remain shell-verbatim. | pure | golden | 1 → 1 | T0375 |
| V07-33 | Dry-run stop_wine plans -k and -w when CrossOver exists and plans nothing when no wineserver exists. | unit | none | 2 → 1 | NEW-stop_wine_wineserver_presence |
| V07-34 | A reap with no matches returns false, emits its not-found row once, and plans no signal. | unit | none | 1 → 1 | T0378 |
| V07-35 | A dry-run reap uses exact executable matches, plans TERM children, and emits one future-tense Info row. | unit | finding | 1 → 1 | T0379 |
| V07-36 | A real reap reports success only after exit and warns with the surviving pid when SIGTERM is ignored. | unit | finding | 3 → 2 | T0381, T0382 |
| V07-37 | The exit waiter rejects a stale pid/start-time identity as no longer the same live process. | unit | finding | 1 → 1 | T0384 |
| V07-38 | A foreign checkout's helper is reported whether or not the local reap matched, and the not-found row is suppressed. | unit | finding | 2 → 1 | NEW-foreign_helper_local_match |
| V07-39 | With no foreign helper, the shell not-found row appears only when the local reap also found nothing. | unit | golden | 2 → 1 | NEW-no_foreign_helper_local_match |
| V07-40 | stop_wine propagates a pre-cancelled token after its first planned child rather than swallowing it. | unit | none | 1 → 1 | T0390 |
| V07-41 | reap propagates a pre-cancelled token and suppresses its closing message. | unit | none | 1 → 1 | T0391 |
| V07-42 | A pre-cancelled Stop finishes failed with exit 130 and never reports StageFinished ok=true, with or without a wineserver path. | stage | none | 1 → 1 | T0392 |
| V07-43 | A failed previous-session reconciliation is warned, does not fail Stop, and does not prevent the later audio report. | stage | none | 1 → 1 | T0393 |
| V07-44 | Cancellation during reporting is caught by the between-step checkpoint, prevents later reports, and fails the stage. | stage | none | 1 → 1 | T0394 |

#### V08 — `sabrage-core:stages/build.rs`, `sabrage-core:stages/install.rs`, `sabrage-core:stages/setup.rs`

*This vertical protects 57 distinct guarantees, especially build-tool execution, install's four-layer state machine, setup's write-once behavior, and review-added cancellation/backup/dry-run fixes. Of 64 tests, 51 remain unchanged; six rows become four labelled tables, three merge into stronger carriers, three delete, and one drops redundant helper assertions, leaving 56 test functions. The clear redundancies are T0309, T0346, the finding-safe merges T0288/T0314, the harness-only T0302, and tautological T0303. Most apparent overlaps are load-bearing because they exercise different entry points, opposite shell recipes, or the stage write path rather than a pure/parity renderer.*

| id | behaviour | layer | protected by | now → target | carriers |
|---|---|---|---|---:|---|
| V08-01 | Ninja's default `[current/total]` prefix produces progress values; malformed and non-Ninja lines do not. | pure | none | 1 → 1 | NEW-v08-ninja-progress-cases |
| V08-02 | Build tool resolution accepts executable files, rejects absent/non-executable candidates, and searches PATH entries in order. | pure | none | 1 → 1 | T0276 |
| V08-03 | The build tool gate reports the first absent required tool in shell order and passes only when all are present. | unit | none | 1 → 1 | T0277 |
| V08-04 | Each missing required tool renders build.sh's exact die string. | pure | golden | 1 → 1 | NEW-v08-missing-tool-message-cases |
| V08-05 | The fixed rustup, uninitialized-submodule, and dashboard-build fatal texts remain byte-exact. | pure | golden | 1 → 1 | NEW-v08-fixed-build-die-texts |
| V08-06 | The rustup gate fails for an absent binary or missing x64 target and passes when that target is listed. | unit | none | 3 → 1 | NEW-v08-rustup-gate |
| V08-07 | Cancellation interrupts the direct rustup subprocess path promptly. | unit | none | 1 → 1 | T0283 |
| V08-08 | A missing built encoder helper produces build.sh's exact path-bearing fatal text. | pure | golden | 1 → 1 | T0284 |
| V08-09 | A wrong-architecture encoder helper fatal embeds lipo output and the helper build directory exactly. | pure | golden | 1 → 1 | T0285 |
| V08-10 | The build-output presence sweep's missing-file message is byte-exact. | pure | golden | 1 → 1 | T0286 |
| V08-11 | CMake configure/build ChildSpecs preserve argv order, `-j8`, PATH, and step attribution. | unit | none | 1 → 1 | T0287 |
| V08-12 | The x64 oxrsys configure invocation explicitly ends with `OXRSYS_BUILD_ENCODER_HELPER=OFF`. | unit | finding | 2 → 1 | T0289 |
| V08-13 | Build completion narration is Ok/past-tense on real runs and Info/future-tense on dry runs. | unit | finding | 1 → 1 | T0290 |
| V08-14 | A byte-identical staged helper that lost execute permission is repaired and never called unchanged. | unit | finding | 1 → 1 | T0291 |
| V08-15 | A healthy staged helper is reported unchanged and is not rewritten. | unit | none | 1 → 1 | T0292 |
| V08-16 | An absent staged helper is installed executable without an unnecessary repair pass. | unit | none | 1 → 1 | T0293 |
| V08-17 | Encoder-helper staging performs no filesystem write during a fresh-checkout dry run. | unit | none | 1 → 1 | T0294 |
| V08-18 | An unusable staged-helper fatal names the staged destination in its remedy. | pure | none | 1 → 1 | T0295 |
| V08-19 | Build's ordinary child wrapper delegates dry runs to Executor and records one Spawn. | unit | none | 1 → 1 | T0296 |
| V08-20 | Build's Ninja child wrapper also delegates dry runs to Executor without spawning. | unit | none | 1 → 1 | T0297 |
| V08-21 | Build's ordinary child wrapper maps a real nonzero exit to ChildFailed with status and an empty tail. | unit | none | 1 → 1 | T0298 |
| V08-22 | A real Ninja build derives progress while forwarding the original output events unchanged. | unit | none | 1 → 1 | T0299 |
| V08-23 | Host-manifest currency matches install.sh cat semantics, ignoring all trailing newlines but rejecting missing/stale content. | pure | golden | 1 → 1 | T0300 |
| V08-24 | The install registry predicate requires ActiveRuntime, openxr, and wineopenxr64.json in order on one line. | pure | none | 1 → 1 | T0301 |
| V08-25 | An install dry run traverses all four layers in order, plans their mutations, uses the current-host branch, and touches no machine state. | stage | none | 2 → 1 | T0304 |
| V08-26 | No install dry-run row claims an unperformed mutation completed. | stage | finding | 1 → 1 | T0305 |
| V08-27 | Install fails with the exact shell-compatible message when a required build output is missing. | stage | golden | 1 → 1 | T0306 |
| V08-28 | Install fails with `CrossOver.app not found` when CrossOver is absent. | stage | golden | 1 → 1 | T0307 |
| V08-29 | Install layer four stages the exact host-manifest file form, including one final newline, and reports a planned dry-run write honestly. | stage | golden | 1 → 1 | T0308 |
| V08-30 | A PermissionDenied copy inside CrossOver.app becomes one TccDenied fatal with an App Management remedy. | stage | none | 1 → 1 | T0310 |
| V08-31 | A non-TCC copy failure emits its OS cause before lib.sh's exact `copy failed` Fatal and has no remedy. | stage | golden | 1 → 1 | T0311 |
| V08-32 | An empty committed stock backup is warned about, not called current, and never re-captured. | stage | finding | 1 → 1 | T0312 |
| V08-33 | A refused stock-backup cp is classified as TCC and removes only its uncommitted partial capture. | stage | finding | 1 → 1 | T0313 |
| V08-34 | A stale ActiveRuntime does not end registry polling; a correct late value completes without a warning. | stage | finding | 2 → 1 | T0319 |
| V08-35 | A registry value that never appears warns exactly once but leaves install successful. | stage | finding | 1 → 1 | T0315 |
| V08-36 | An interrupted non-empty stock capture is never committed, is cleaned up, and the retry re-captures stock. | stage | finding | 1 → 1 | T0316 |
| V08-37 | A partial capture inherited from an earlier killed run is swept, never promoted, and does not suppress stock capture. | stage | finding | 1 → 1 | T0317 |
| V08-38 | Cancellation during registry work returns Cancelled before warnings, success claims, authorization, or layer four. | stage | finding | 1 → 1 | T0318 |
| V08-39 | A stale ActiveRuntime that never becomes launch-qualified still produces the lazy-flush warning. | stage | finding | 1 → 1 | T0320 |
| V08-40 | A control character in the dylib path refuses layer four before comparison, staging, or authorization. | stage | finding | 1 → 1 | T0321 |
| V08-41 | Install refuses an incomplete DXMT artifact set with the exact never-half-apply fatal. | stage | golden | 1 → 1 | T0322 |
| V08-42 | Setup's awk-compatible protocol parser is comment/key-shape aware and returns the first matching assignment. | pure | none | 1 → 1 | T0345 |
| V08-43 | A DXMT provenance marker is the contract pin followed by exactly one newline. | pure | golden | 1 → 1 | T0201 |
| V08-44 | Setup skips the game check with one advice row when neither bottle nor Beat Saber directory is supplied. | unit | none | 1 → 1 | T0347 |
| V08-45 | Setup reports Beat Saber found through a directory override. | unit | none | 1 → 1 | T0348 |
| V08-46 | Setup reports a missing game with the shell warning and contract-derived DepotDownloader command. | unit | golden | 1 → 1 | T0349 |
| V08-47 | A named missing bottle takes require_bottle's fatal path rather than the directory-override warning path. | unit | golden | 1 → 1 | T0350 |
| V08-48 | Setup never overwrites an existing runtime config and reports an existing alvr config. | unit | none | 1 → 1 | T0351 |
| V08-49 | Setup preserves a non-alvr runtime config and emits the exact not-overwriting warning. | unit | golden | 1 → 1 | T0352 |
| V08-50 | When absent, setup creates the runtime config from the exact shared TOML template and reports the write. | unit | golden | 1 → 1 | T0353 |
| V08-51 | If another writer wins runtime-config creation, setup preserves and reports the winner's bytes without claiming a write. | unit | finding | 1 → 1 | T0354 |
| V08-52 | A present Goldberg DLL with the wrong hash is retained, warned about, and not downloaded again. | unit | none | 1 → 1 | T0355 |
| V08-53 | A complete DXMT set with a current marker skips DXMT download, removal, and extraction. | unit | none | 1 → 1 | T0356 |
| V08-54 | A fresh-checkout setup dry run emits future-tense postconditions, no false Ok state, while retaining its marker-write plan. | stage | finding | 1 → 1 | T0357 |
| V08-55 | A setup dry run retains truthful Ok rows for existing submodules but cannot claim a missing provenance marker was written. | stage | finding | 1 → 1 | T0358 |
| V08-56 | A setup-config dry run reports a future write, plans it, and leaves no TOML on disk. | unit | finding | 1 → 1 | T0359 |
| V08-57 | A full fresh-checkout setup dry run records every mutation and exact git argv in order while changing no target state. | stage | none | 1 → 1 | T0360 |

#### V09 — `sabrage-core:fixes/adb.rs`, `sabrage-core:fixes/backend.rs`, `sabrage-core:fixes/helper.rs`, `sabrage-core:fixes/mod.rs`, `sabrage-core:fixes/session_json.rs`

*These 67 tests protect 52 distinct behaviours across ADB cleanup, backend rewriting, helper staging, fix dispatch, and guarded session-json deletion. The defensible target is 52 test functions: three labelled tables replace 14 literal-variant functions, two launch-entry mirrors are deleted, one private-seam test is subsumed, and one finding-pinned test is merged without losing its label. Seven additional tests should survive with only tautological fixture assertions removed. Golden and finding coverage remains intact; no files were changed and no builds or tests were run.*

| id | behaviour | layer | protected by | now → target | carriers |
|---|---|---|---|---:|---|
| V09-01 | ADB forward-list parsing returns serial/local pairs for rows with at least two fields and ignores blank or short rows. | pure | none | 2 → 1 | NEW-adb-forward-parse-cases |
| V09-02 | Removing stale forwards without an adb path is an unchanged no-op. | unit | none | 1 → 1 | T0129 |
| V09-03 | A successfully queried table containing no stale ports produces no removal and an unchanged report. | unit | none | 1 → 1 | T0130 |
| V09-04 | The ADB fix removes exactly the contract stream ports per serial and never removes unrelated forwards or uses remove-all. | unit | contract | 2 → 1 | T0131 |
| V09-05 | Each forward-list row is removed using that row's own device serial. | unit | none | 1 → 1 | T0139 |
| V09-06 | Successful stale-forward removals report changed and emit Info rows with run.sh's exact cleared-forward text. | unit | golden | 1 → 1 | T0131 |
| V09-07 | ADB preview reports what would be cleared while performing no removal. | unit | none | 1 → 1 | T0132 |
| V09-08 | Standalone ADB-fix rows use the fix contract step while the caller-supplied entry point uses the launch step. | unit | contract | 1 → 1 | T0133 |
| V09-09 | Failed per-forward removals never claim a clean table and name and warn for the surviving forwards. | unit | finding | 1 → 1 | T0134 |
| V09-10 | Non-zero and unspawnable ADB queries return an explicit unchanged failure report, emit warnings, and remove nothing. | unit | finding | 2 → 2 | T0135, T0136 |
| V09-11 | The native cleared-forward renderer produces the exact cleared and preview prose expected by the shell contract. | pure | golden | 1 → 1 | T0137 |
| V09-12 | The standalone ADB fix refuses during a live session, while the launch-specific entry point remains usable. | unit | finding | 1 → 1 | T0138 |
| V09-13 | Graphics-backend rewriting selects the correct branch and preserves exact cxbottle bytes across rewrite, insertion, append, malformed, and newline cases. | pure | golden | 8 → 1 | NEW-backend-rewrite-cases |
| V09-14 | Bottle liveness is true for an exact WINEPREFIX or an unreadable prefix, and false for no process or only known-different prefixes. | pure | none | 4 → 1 | NEW-backend-wineprefix-cases |
| V09-15 | Wineserver scan wrappers do not report liveness for an executable path no process can resolve to. | unit | none | 1 → 1 | T0152 |
| V09-16 | An already canonical graphics-backend file is returned unchanged and is not rewritten. | unit | none | 2 → 1 | T0153 |
| V09-17 | A real backend fix writes a doctor-matching target line and returns and emits the forced-backend description. | unit | golden | 1 → 1 | T0154 |
| V09-18 | A noncanonical key line that cannot produce the target postcondition fails without writing or claiming success. | unit | finding | 1 → 1 | T0155 |
| V09-19 | Backend preview reports a change and records work while leaving cxbottle.conf byte-identical. | unit | none | 1 → 1 | T0156 |
| V09-20 | Both backend doors require a selected bottle before accessing cxbottle.conf. | unit | none | 2 → 1 | T0157 |
| V09-21 | The standalone backend fix refuses and preserves bytes while the bottle's wineserver is live. | unit | none | 1 → 1 | T0158 |
| V09-22 | The launch backend door deliberately edits even while the bottle's wineserver is live. | unit | none | 1 → 1 | T0159 |
| V09-23 | The helper fix resolves encoder_process with the runtime's table-blind, last-valid-assignment semantics. | pure | finding | 1 → 1 | T0162 |
| V09-24 | Missing, empty, or ignored encoder_process values default to auto while an accepted value is retained. | pure | none | 1 → 1 | T0163 |
| V09-25 | An already staged executable arm64 helper is a silent unchanged no-op. | unit | none | 1 → 1 | T0164 |
| V09-26 | A missing staged helper is copied from a valid build output, remains executable, and emits the expected restage rows. | unit | golden | 1 → 1 | T0165 |
| V09-27 | Previewing a missing-helper restage reports the planned outcome without creating or revalidating the destination. | unit | none | 1 → 1 | T0166 |
| V09-28 | A byte-identical staged helper missing execute permission is repaired and reported as installed work. | unit | finding | 1 → 1 | T0167 |
| V09-29 | Previewing a mode-only helper repair preserves mode while recording Copy rather than Skip. | unit | finding | 1 → 1 | T0168 |
| V09-30 | When neither staged nor built helper is arm64-executable, the fix emits run.sh's exact fatal message and remedy. | unit | golden | 1 → 1 | T0169 |
| V09-31 | A missing runtime TOML is represented as encoder_process=auto in the helper fatal text. | unit | none | 1 → 1 | T0170 |
| V09-32 | The public fix entry point waits for the operation lock and dispatches once it becomes available. | stage | none | 1 → 1 | T0171 |
| V09-33 | A caller already holding the operation lock can dispatch a whole-stage fix without reacquiring and deadlocking. | stage | none | 1 → 1 | T0172 |
| V09-34 | Public apply refuses every registry action marked forbidden during a live session before its mutation runs. | stage | finding | 2 → 1 | T0173 |
| V09-35 | The holding-lock launch-preflight door remains exempt from the public live-session gate. | stage | finding | 1 → 1 | T0174 |
| V09-36 | Every FixAction has stable bare, contract, CLI-parse, and wire spellings, with the documented deferred conversion exception. | pure | contract | 1 → 1 | T0176 |
| V09-37 | The modelled known-bad session-json deletion is exactly the withheld/no-button FixAction. | pure | finding | 2 → 1 | T0185 |
| V09-38 | The known-bad deletion is destructive and uniquely carries pre-mutation black-screen, backup, and in-place-recovery disclosure. | pure | finding | 1 → 1 | T0177 |
| V09-39 | Every fix id referenced by the contract is either modelled or explicitly deferred. | pure | contract | 1 → 1 | T0178 |
| V09-40 | Every native or shell Autofix gate references an offered modelled action. | pure | contract | 1 → 1 | T0179 |
| V09-41 | The fix registry covers every action exactly once and preserves the unique admin/destructive sets and universal live-session policy. | pure | contract | 1 → 1 | T0180 |
| V09-42 | A fix queued while idle rechecks liveness after acquiring the lock and refuses if a session started meanwhile. | stage | finding | 1 → 1 | T0181 |
| V09-43 | A queued fix announces its run id and wait before blocking and can be cancelled without applying. | stage | finding | 1 → 1 | T0182 |
| V09-44 | Every fix action refuses to mutate a checkout whose contract differs from the binary's compiled contract. | stage | finding | 1 → 1 | T0183 |
| V09-45 | Every fix event step id is identical to that action's full contract id. | pure | contract | 1 → 1 | T0184 |
| V09-46 | RunSetup, RunBuild, and RunInstall map to their corresponding stages while non-stage fixes do not. | pure | none | 1 → 1 | T0186 |
| V09-47 | Deleting an absent session.json is an unchanged DeleteSessionJson result. | unit | none | 1 → 1 | T0187 |
| V09-48 | A real deletion creates one byte-identical backup before removal and exposes its location and recovery caveat. | unit | none | 1 → 1 | T0188 |
| V09-49 | Session-json preview plans directory creation, backup write, and removal in order while touching no disk state. | unit | none | 1 → 1 | T0189 |
| V09-50 | Two rapid deletions reserve distinct backup names, retain both byte sets, and report the paths actually written. | unit | finding | 1 → 1 | T0190 |
| V09-51 | The injected Executor's preview mode controls destructive behaviour even when StageOptions says real run. | unit | none | 1 → 1 | T0191 |
| V09-52 | Because session.json is machine-global, any observed wineserver causes deletion refusal and preserves the file. | unit | none | 1 → 1 | T0192 |

#### V10 — `sabrage-core:store/goldberg.rs`, `sabrage-core:store/library.rs`, `sabrage-core:store/settings.rs`

*These tests primarily protect Goldberg revert safety, lossless library transactions, install-health classification, and settings downgrade compatibility. The 54 functions reduce to 49: one settings load-only test merges into its stronger round-trip regression, while four direct Serde/derive tests are deleted or moved behind public loaders. Fourteen surviving tests can also lose redundant, setup-derived, or strictly implied assertions. Same-named tests across library.rs and settings.rs, and the three Goldberg liveness refusals, remain load-bearing because they exercise different production units or independent liveness signals.*

| id | behaviour | layer | protected by | now → target | carriers |
|---|---|---|---|---:|---|
| V10-01 | Reverting without a backup reports restored=false with an explanation and leaves the live DLL untouched. | unit | none | 1 → 1 | T0624 |
| V10-02 | A valid backup replaces the live DLL while the backup and other Goldberg artifacts remain in place. | unit | none | 1 → 1 | T0625 |
| V10-03 | Repeating a successful revert remains successful and leaves the restored bytes stable. | unit | none | 1 → 1 | T0626 |
| V10-04 | A backup known to be Goldberg by pin, configured-payload bytes, or recorded provenance is refused without changing either DLL. | unit | finding | 3 → 3 | T0627, T0628, T0629 |
| V10-05 | A backup unlike both the pin and configured Goldberg payload remains eligible for restoration. | unit | none | 1 → 1 | T0630 |
| V10-06 | Revert refuses a live session detected through a matching process, fresh runtime telemetry with a live PID, or persisted Wine-process identity, leaving the DLL untouched. | unit | finding | 3 → 3 | T0631, T0633, T0634 |
| V10-07 | A successful revert describes the .orig-steam backup without claiming authenticated original-Steam provenance. | unit | finding | 1 → 1 | T0632 |
| V10-08 | Revert cannot touch the DLL while another operation holds the operation lock and proceeds after release. | unit | finding | 1 → 1 | T0635 |
| V10-09 | The library store path is <app-support>/library.json. | pure | none | 1 → 1 | T0636 |
| V10-10 | A missing library file loads as the version-one empty library. | unit | none | 1 → 1 | T0637 |
| V10-11 | A corrupt library file is an error rather than a silent reset. | unit | none | 1 → 1 | T0638 |
| V10-12 | A library schema newer than the binary is refused without modifying its bytes. | unit | finding | 1 → 1 | T0639 |
| V10-13 | Unknown fields in a current-version library file do not prevent loading its known contents. | unit | none | 1 → 1 | T0640 |
| V10-14 | Library save/load creates parent directories, writes newline-terminated camelCase JSON, and round-trips nested entry state. | unit | none | 1 → 1 | T0641 |
| V10-15 | Dry-run library save plans directory creation and writing without creating the file. | unit | none | 1 → 1 | T0642 |
| V10-16 | Library transactions do not write for an unchanged value and persist an actual mutation. | unit | none | 1 → 1 | T0643 |
| V10-17 | Serialized library transactions cannot resurrect a removed game or discard an unrelated entry. | unit | finding | 1 → 1 | T0644 |
| V10-18 | A stale editable snapshot and a newly recorded session both survive their store transactions. | unit | finding | 1 → 1 | T0645 |
| V10-19 | Library upsert inserts a new ID and replaces rather than appends when the ID already exists. | pure | none | 1 → 1 | T0646 |
| V10-20 | Library remove deletes a matching entry and reports whether an entry was found. | pure | none | 1 → 1 | T0647 |
| V10-21 | Library get returns the matching ID and None for a different ID. | pure | none | 1 → 1 | T0648 |
| V10-22 | Editable upsert accepts editable fields, preserves last_session, added_at, and appid for stored IDs, and inserts unknown IDs. | pure | finding | 1 → 1 | T0649 |
| V10-23 | Recording a session updates only the matching entry and reports false for an unknown ID. | pure | none | 1 → 1 | T0650 |
| V10-24 | A new-entry template carries the fixed display name, contract appid, empty history, and default launch overrides. | pure | contract | 1 → 1 | T0651 |
| V10-25 | Template bottle selection prefers the settings default, otherwise the first discovered bottle, otherwise an empty string. | pure | none | 2 → 2 | T0651, T0652 |
| V10-26 | Template Beat Saber directory selection prefers settings, then environment, then the resolved bottle default. | pure | none | 1 → 1 | T0653 |
| V10-27 | Effective per-game options take identity from the entry, let Some overrides beat global settings, and never store dry-run. | pure | none | 1 → 1 | T0654 |
| V10-28 | launch_options_for applies the merge to a known game ID and returns None for an unknown ID. | pure | none | 1 → 1 | T0655 |
| V10-29 | A missing executable yields NotFound ahead of other validation failures and supplies an explanatory problem. | unit | none | 1 → 1 | T0656 |
| V10-30 | A valid executable and version with a missing bottle yields NeedsSetup and names the missing bottle. | unit | none | 1 → 1 | T0657 |
| V10-31 | A non-1.29.4 detected version yields NeedsAttention and a version-specific problem. | unit | none | 1 → 1 | T0658 |
| V10-32 | A Beat Saber directory outside drive_c without a z: drive yields NeedsAttention and explains the missing mapping. | unit | none | 1 → 1 | T0659 |
| V10-33 | A complete healthy install is Ready, reports all healthy facets, and has no problems. | unit | none | 1 → 1 | T0660 |
| V10-34 | Bottle template and backend mismatches appear as problems but do not independently change an otherwise Ready status. | unit | none | 1 → 1 | T0661 |
| V10-35 | Goldberg classification distinguishes NoDll, Original, AppliedUnverified, Modified, and Applied using pin, payload, and backup presence. | unit | finding | 1 → 1 | T0662 |
| V10-36 | An otherwise healthy install without steam_api64.dll is NoDll and NeedsAttention rather than Ready, with an explanatory problem. | unit | finding | 1 → 1 | T0663 |
| V10-37 | The settings store path is <app-support>/settings.json. | pure | none | 1 → 1 | T0664 |
| V10-38 | A missing settings file loads the complete application defaults. | unit | none | 2 → 1 | T0665 |
| V10-39 | A corrupt settings file is an error rather than a silent reset. | unit | none | 1 → 1 | T0666 |
| V10-40 | Unknown top-level settings fields are captured and survive a known-field load-modify-save cycle unchanged. | unit | finding | 2 → 1 | T0668 |
| V10-41 | Unknown keys nested inside launch survive load-modify-save while known launch fields remain usable. | unit | finding | 1 → 1 | T0669 |
| V10-42 | A settings version newer than the binary is refused with an update remedy and unchanged file bytes. | unit | finding | 1 → 1 | T0670 |
| V10-43 | A legacy settings file without a version is interpreted as the current settings version through the public loader. | unit | none | 1 → 1 | T0668 |
| V10-44 | A current-version settings file is accepted through the public save/load boundary. | unit | none | 2 → 1 | T0674 |
| V10-45 | Settings save/load writes newline-terminated camelCase JSON without spurious extra keys and round-trips all known fields. | unit | none | 2 → 1 | T0674 |
| V10-46 | A present but minimal empty-object settings file loads application defaults through the public loader. | unit | none | 1 → 1 | T0665 |
| V10-47 | Settings effective_stage_options carries global bottle, directory, and flags while forcing dry_run=false. | pure | none | 1 → 1 | T0676 |
| V10-48 | Dry-run settings save plans directory creation and writing without creating the file. | unit | none | 1 → 1 | T0677 |

#### V11 — `sabrage-core:checks/config.rs`, `sabrage-core:checks/meta.rs`, `sabrage-core:checks/mod.rs`, `sabrage-core:checks/pinned.rs`, `sabrage-core:checks/run_only.rs`, `sabrage-core:checks/sources.rs`

*The 51 roster functions protect 39 distinct guarantees; under the standard, the scoped files need about 37 test functions after consolidation. Most reduction comes from two labelled tables: five protocol-outcome variants become one table, and four session.json shape variants become one table. Seven more functions can disappear because existing core or parity tests carry their facts; one registry test should retain only its unique lenient-mode assertion. Finding regressions, private parsers with distinct semantics, filesystem-form cases, and observable run-preflight behaviours remain load-bearing.*

| id | behaviour | layer | protected by | now → target | carriers |
|---|---|---|---|---:|---|
| V11-01 | The Doctor protocol parser emulates its last-match, table-blind, double-quote-field awk recipe while ignoring comments and longer key names. | pure | parity | 1 → 1 | T0536 |
| V11-02 | Missing, alvr, oxrsys, and unsupported protocol states produce the specified paired outcomes for the supported and legacy slugs. | unit | parity | 4 → 1 | NEW-config_protocol_state_matrix |
| V11-03 | Public protocol checks resolve duplicate assignments from the last matching line in either assignment order. | unit | finding | 2 → 2 | T0541, NEW-config_protocol_state_matrix |
| V11-04 | An absent ALVR session.json makes cfg.session-pins Skipped rather than clean, corrupt, or pinned. | unit | parity | 1 → 1 | T0543 |
| V11-05 | Malformed and unreadable session state warn with accurate native errors and never blame python3. | unit | finding | 2 → 2 | T0544, T0545 |
| V11-06 | Parsed session JSON maps absent or empty pin containers to Pass and invalid container shapes to the corrupt Warn. | unit | golden | 4 → 1 | NEW-session_json_shape_matrix |
| V11-07 | A pinned client warns with the shell-compatible trailing-space join and the in-place-edit remedy. | unit | golden | 1 → 1 | T0548 |
| V11-08 | Multiple manual IPs for one client are comma-joined in the warning entry. | unit | none | 1 → 1 | T0549 |
| V11-09 | A live, internally consistent checkout matching the compiled contract produces the successful meta.contract-sync outcome. | unit | contract | 1 → 1 | T0569 |
| V11-10 | An unreadable or internally inconsistent checkout fails meta.contract-sync with the exact regeneration diagnosis. | unit | golden | 1 → 1 | T0570 |
| V11-11 | A self-consistent checkout foreign to the compiled binary fails the Doctor evaluator with checkout and binary hashes. | unit | finding | 1 → 1 | T0571 |
| V11-12 | The compiled contract digest equals the checkout from which the binary was built. | unit | finding | 1 → 1 | T0005 |
| V11-13 | The reusable mutation guard accepts the matching live checkout. | unit | finding | 1 → 1 | T0573 |
| V11-14 | The reusable mutation guard fails closed when contract files cannot be read. | unit | finding | 1 → 1 | T0574 |
| V11-15 | The reusable mutation guard rejects a readable but foreign contract checkout. | unit | finding | 1 → 1 | T0575 |
| V11-16 | The strict registry binds every declared slug exactly once and exposes it in contract order. | parity | parity | 4 → 1 | T0867 |
| V11-17 | A complete registry also builds successfully through the lenient entry point. | unit | none | 1 → 1 | T0577 |
| V11-18 | Doctor evaluates and emits only doctor-visible checks, preserving their count and excluding run-only rows. | stage | contract | 1 → 1 | T0578 |
| V11-19 | Registry native_preflight returns exactly the native-gating contract subset in order. | stage | contract | 1 → 1 | T0579 |
| V11-20 | CheckCtx distinguishes no bottle request from a named but unresolved bottle while producing the correct labels and empty prefix. | unit | none | 1 → 1 | T0580 |
| V11-21 | An incomplete DXMT artifact set fails with the setup remedy. | unit | none | 1 → 1 | T0593 |
| V11-22 | A complete DXMT file set without a current provenance marker warns. | unit | golden | 1 → 1 | T0594 |
| V11-23 | A complete DXMT set with the current newline-terminated contract marker passes. | unit | golden | 1 → 1 | T0595 |
| V11-24 | A missing Goldberg DLL fails with the setup remedy. | unit | none | 1 → 1 | T0596 |
| V11-25 | A present Goldberg DLL with a wrong hash warns and reports diagnostic hash detail. | unit | none | 1 → 1 | T0597 |
| V11-26 | An executable configured CrossOver wine path passes the run-only gate. | stage | none | 1 → 1 | T0598 |
| V11-27 | A configured wine path lacking execute bits fails with run.sh's exact interpolated die text. | stage | golden | 1 → 1 | T0599 |
| V11-28 | When CrossOver supplies no wine path, the native gate reports CrossOver.app absent without fabricating lib.sh's path. | stage | parity | 1 → 1 | T0600 |
| V11-29 | The bridge gate passes only when both native and Wine bridge outputs exist and identifies missing halves. | stage | parity | 1 → 1 | T0601 |
| V11-30 | The wired-adb gate is Skipped with a reason when wired mode is not requested. | stage | contract | 1 → 1 | T0602 |
| V11-31 | Wired mode without any adb path fails with run.sh's first wired die sentence. | stage | parity | 1 → 1 | T0875 |
| V11-32 | Wired mode with probing disabled is explicitly unverifiable rather than passing. | stage | parity | 1 → 1 | T0604 |
| V11-33 | A healthy adb probe ignores the header and non-device states, then passes with the first connected serial as detail. | stage | none | 3 → 1 | T0605 |
| V11-34 | An adb device list with no connected device fails with run.sh's second wired die sentence. | stage | golden | 2 → 1 | T0606 |
| V11-35 | A wedged adb devices child is killed and reaped within a bounded probe interval. | stage | finding | 1 → 1 | T0607 |
| V11-36 | A source checkout without a file-or-directory .git marker fails with its submodule basename and setup remedy. | unit | none | 1 → 1 | T0611 |
| V11-37 | A nested clone represented by a .git directory is accepted as a present source checkout. | unit | none | 1 → 1 | T0612 |
| V11-38 | A real submodule represented by a .git gitlink file is accepted as a present source checkout. | unit | none | 1 → 1 | T0613 |
| V11-39 | The ALVR patchset check fails without the connection marker and passes when connection.rs contains is_streaming_nonblocking. | unit | none | 1 → 1 | T0614 |

#### V12 — `sabrage-core:checks/audio.rs`, `sabrage-core:checks/bottle.rs`, `sabrage-core:checks/bridge.rs`, `sabrage-core:checks/build.rs`, `sabrage-core:checks/game.rs`, `sabrage-core:checks/headset.rs`, `sabrage-core:checks/host.rs`, `sabrage-core:checks/network.rs`, `sabrage-core:checks/overlay.rs`, `sabrage-core:checks/system.rs`, `sabrage-core:checks/toolchain.rs`

*These 61 tests protect 51 distinct target behaviours, chiefly doctor state transitions, exact shell-compatible prose, filesystem probes, and machine-tool mappings. Under the standard policy, the suite can fall to 51 functions: six already-programmed H3-4 registry mirrors, two parser-table reductions, and one private-seam merge account for the removals. Several surviving tests should also lose assertions that merely restate setup or statuses already covered by the six-output wiring test. The remaining textual similarities are mostly different evaluators or independent implementations, so they are not redundant assertion-for-assertion.*

| id | behaviour | layer | protected by | now → target | carriers |
|---|---|---|---|---:|---|
| V12-01 | audio.loopback maps SwitchAudioSource absence, exact BlackHole presence, and missing BlackHole to the corresponding doctor outcomes and prose. | unit | golden | 1 → 1 | T0512 |
| V12-02 | With no bottle name, bottle.named fails with the shell-compatible remedy and every downstream bottle check skips. | unit | golden | 1 → 1 | T0514 |
| V12-03 | A requested but unresolved bottle reports its prefix and remedy, then skips template, graphics, and z-drive checks. | unit | golden | 1 → 1 | T0515 |
| V12-04 | A matching win11_64 and exact DXMT cxbottle.conf satisfies both configuration checks. | unit | none | 1 → 1 | T0516 |
| V12-05 | A mismatching template warns with the first key line while a non-DXMT backend fails with the install/run remedy. | unit | golden | 1 → 1 | T0517 |
| V12-06 | An absent Template key renders the warning's parenthetical as empty. | unit | golden | 1 → 1 | T0518 |
| V12-07 | bottle.zdrive skips inside drive_c, fails outside without z:, and passes outside when z: exists. | unit | golden | 1 → 1 | T0519 |
| V12-08 | The registry scanner requires ActiveRuntime, openxr, and wineopenxr64.json in that order on one line. | pure | none | 1 → 1 | T0521 |
| V12-09 | Without a resolved bottle, all three bottle-bridge checks skip with the section's verbatim reason. | unit | golden | 1 → 1 | T0522 |
| V12-10 | Byte-identical built and bottle wineopenxr DLLs make bottle.woxr-dll pass. | unit | none | 1 → 1 | T0523 |
| V12-11 | A stale or missing bottle wineopenxr DLL fails with the exact install remedy. | unit | golden | 1 → 1 | T0524 |
| V12-12 | Bottle OpenXR manifest absence fails with the install remedy and presence passes with the Windows path text. | unit | golden | 1 → 1 | T0525 |
| V12-13 | The required ActiveRuntime registry line controls bottle.registry failure versus pass and its prose. | unit | golden | 1 → 1 | T0526 |
| V12-14 | A missing build output reports its relative path and the build remedy. | unit | golden | 1 → 1 | T0527 |
| V12-15 | A present build output passes with its repository-relative path. | unit | golden | 1 → 1 | T0528 |
| V12-16 | Each of the six build-output evaluators probes its own Paths field and flips between Pass and Fail with file presence. | unit | none | 3 → 1 | T0529 |
| V12-17 | A missing staged helper fails build.helper-staged and causes build.helper-arm64 to skip with a reason. | unit | golden | 1 → 1 | T0530 |
| V12-18 | A present staged helper passes with the staged-next-to-runtime suffix. | unit | none | 1 → 1 | T0531 |
| V12-19 | An executable carrying a plain arm64 slice satisfies helper_is_arm64. | pure | none | 1 → 1 | T0532 |
| V12-20 | An arm64e-only binary does not satisfy the plain-arm64 whole-word requirement. | pure | none | 1 → 1 | T0533 |
| V12-21 | A plain-arm64 file without execute permission does not satisfy helper_is_arm64. | pure | none | 1 → 1 | T0534 |
| V12-22 | A staged executable with unusable architecture fails and embeds lipo stdout plus the restaging remedy. | unit | golden | 1 → 1 | T0535 |
| V12-23 | With neither bottle nor BS directory override, both game checks skip with the verbatim section reason. | unit | golden | 1 → 1 | T0553 |
| V12-24 | A BS directory override runs the game section without a bottle; a missing executable fails present and skips version. | unit | golden | 1 → 1 | T0554 |
| V12-25 | A Beat Saber executable with a matching 1.29.4 marker passes both game checks and reports the detected version. | unit | none | 1 → 1 | T0555 |
| V12-26 | An installed executable with another version warns using the Meta-gate wording. | unit | golden | 1 → 1 | T0556 |
| V12-27 | The headset adb parser skips the header and non-device states, returns the first device serial, and returns None when none qualifies. | pure | none | 3 → 1 | NEW-headset-first-connected-serial-cases |
| V12-28 | Disabling adb probes skips both headset checks without spawning adb. | unit | none | 1 → 1 | T0560 |
| V12-29 | Without an adb binary, hs.adb warns with the WiFi-safe message and hs.client skips. | unit | golden | 1 → 1 | T0561 |
| V12-30 | A missing host runtime manifest fails with the path-specific privileged-install remedy. | unit | golden | 1 → 1 | T0563 |
| V12-31 | Malformed JSON or a missing runtime.library_path is a host-manifest parse failure with the parse remedy. | unit | golden | 2 → 1 | NEW-host-invalid-manifest-cases |
| V12-32 | A manifest naming the expected existing dylib passes without a remedy. | unit | none | 1 → 1 | T0566 |
| V12-33 | A manifest naming a different existing dylib warns and reports actual versus expected paths. | unit | golden | 1 → 1 | T0567 |
| V12-34 | A manifest naming a missing dylib fails with the path-specific rewrite remedy. | unit | golden | 1 → 1 | T0568 |
| V12-35 | net.ports maps the direct lsof listener set to exact free or busy doctor prose. | unit | golden | 1 → 1 | T0581 |
| V12-36 | Disabling adb probes skips net.adb-forwards. | unit | none | 1 → 1 | T0582 |
| V12-37 | An absent adb binary skips net.adb-forwards. | unit | none | 1 → 1 | T0583 |
| V12-38 | A successful adb forward query warns when tcp:9943 or tcp:9944 is present and passes otherwise. | unit | none | 1 → 1 | T0584 |
| V12-39 | An unspawnable adb forward query is an error and the public doctor row Warns rather than claiming no stale forwards. | unit | finding | 2 → 1 | T0586 |
| V12-40 | A successfully spawned adb forward query that exits nonzero is returned as an error. | unit | none | 1 → 1 | T0587 |
| V12-41 | Without CrossOver.app, all four global overlay checks skip. | unit | none | 1 → 1 | T0589 |
| V12-42 | Byte-identical source and destination overlay files pass and render the destination basename. | unit | none | 1 → 1 | T0590 |
| V12-43 | A stale named-bottle overlay fails with the exact install remedy. | unit | golden | 1 → 1 | T0591 |
| V12-44 | A stale overlay without a resolved bottle still fails and uses the name placeholder in its remedy. | unit | golden | 1 → 1 | T0592 |
| V12-45 | Dotted version comparison is numeric, zero-fills missing components, and treats nonnumeric probe fallbacks as zero. | pure | none | 1 → 1 | T0615 |
| V12-46 | sys.arch maps the real uname result to the Apple-Silicon pass or exact unsupported-machine failure. | unit | golden | 1 → 1 | T0616 |
| V12-47 | sys.macos27 applies dotted major-version comparison and emits the correct pass or upgrade result. | unit | golden | 1 → 1 | T0617 |
| V12-48 | CrossOver discovery jointly controls cx.present and whether cx.version runs or skips. | unit | golden | 1 → 1 | T0618 |
| V12-49 | The shared tool checker renders deterministic pass and fail shapes with the common toolchain remedy. | unit | golden | 1 → 1 | T0620 |
| V12-50 | Each of the five tool evaluators probes its corresponding binary and shares the shell remedy on failure. | unit | golden | 1 → 1 | T0621 |
| V12-51 | rust.x64-target requires rustup plus the x86_64-apple-darwin installed target and emits the corresponding prose. | unit | golden | 1 → 1 | T0622 |

#### V13 — `sabrage-cli:main.rs`

*The CLI suite protects distinct parser entry points, shell-compatible output bytes, event-to-stream projection, orchestration, and signal handling. Four functions can disappear outright: T0802 is subsumed by T0801, T0810 repeats core test T0008 through a one-line wrapper, and T0808/T0809 test private fatal-format seams already covered through stage_event_lines. Sixteen functions become five labelled tables under the already-programmed V7-2 work; two more variants merge into existing carriers, and T0824 drops only its duplicated header assertion. Most doctor-versus-stage similarities remain load-bearing because the outer parsers and projections are separate units that can drift independently.*

| id | behaviour | layer | protected by | now → target | carriers |
|---|---|---|---|---:|---|
| V13-01 | Doctor parses --bottle and --bs-dir values and leaves unrelated doctor options absent. | pure | none | 1 → 1 | T0778 |
| V13-02 | Doctor accepts --tap and forwards every shared boolean flag. | pure | none | 1 → 1 | T0779 |
| V13-03 | Doctor parsing emits the exact demo.sh missing/unknown diagnostics and stops at the first bad token. | cli | golden | 3 → 1 | NEW-V13-doctor-parse-errors |
| V13-04 | For an unknown command, remaining tokens are parsed first: the first bad tail token wins, while a clean tail falls through to usage. | cli | finding | 2 → 1 | NEW-V13-unknown-command-outcomes |
| V13-05 | Doctor outcomes render the shell-compatible Pass, Warn, Fail, Info, silent-status, and ANSI label shapes. | cli | golden | 7 → 1 | NEW-V13-doctor-outcome-rendering |
| V13-06 | Outcome detail is absent by default, appears only in verbose mode, and verbose adds nothing when detail is absent. | cli | finding | 2 → 1 | T0792 |
| V13-07 | Both doctor footer branches, including colored success, match doctor.sh bytes. | cli | golden | 1 → 1 | T0794 |
| V13-08 | The stage parser accepts shared value/boolean flags and the stage-only --dry-run and --quiet flags. | pure | none | 2 → 2 | T0795, T0796 |
| V13-09 | The stage parser propagates exact shared missing-value diagnostics, rejects stage-invalid tokens, and stops at its first bad argument. | cli | golden | 3 → 3 | T0797, T0798, T0799 |
| V13-10 | All six parsed CLI options plus dry-run are forwarded into StageOptions. | pure | none | 1 → 1 | T0800 |
| V13-11 | Absent CLI flags preserve environment-derived stage booleans, while dry_run comes directly from parsed CLI state. | pure | none | 2 → 1 | T0801 |
| V13-12 | Explicit empty stage CLI bottle and bs-dir values collapse to None, clearing any environment preset. | pure | finding | 2 → 1 | NEW-V13-stage-empty-option-merge |
| V13-13 | Explicit empty doctor CLI bottle and bs-dir values collapse to None, clearing any environment preset. | pure | finding | 2 → 1 | NEW-V13-doctor-empty-option-merge |
| V13-14 | Every stage Severity maps to the corresponding shell-compatible shared row shape. | pure | golden | 1 → 1 | T0807 |
| V13-15 | A Fatal event with a remedy emits the fatal line and seven-space remedy continuation on stderr. | cli | none | 2 → 1 | T0819 |
| V13-16 | Fatal output has no leading indent and consumes stderr color state rather than stdout color state. | cli | finding | 2 → 1 | T0822 |
| V13-17 | Errors whose prose was already emitted are classified as already reported, while ordinary IO errors are not. | unit | none | 1 → 1 | T0008 |
| V13-18 | Setup, build, and install closing lines match their stage scripts exactly; stop and run add none. | cli | golden | 1 → 1 | T0811 |
| V13-19 | Text events preserve empty strings and caller-owned leading whitespace verbatim. | pure | none | 1 → 1 | T0812 |
| V13-20 | Structured Check, Launched, AutoFixed, and Progress events produce no console row. | pure | none | 2 → 1 | T0813 |
| V13-21 | NeedsAdmin renders as an indented stdout info row. | pure | none | 1 → 1 | T0815 |
| V13-22 | StageStarted, Section, and Line events compose into their banner, section, and shared-row stdout shapes. | pure | none | 1 → 1 | T0816 |
| V13-23 | Ordinary Output events route by stream and --quiet suppresses child output. | pure | none | 1 → 1 | T0817 |
| V13-24 | CR chunks repaint only on the destination TTY; off-TTY CR and EOF use ordinary lines, and quiet still suppresses them. | cli | finding | 1 → 1 | T0818 |
| V13-25 | StageFinished emits a closing line only for successful stages that define one; run and failed stages add nothing. | pure | none | 1 → 1 | T0819 |
| V13-26 | NO_COLOR disables both stream colors without erasing TTY state, and otherwise stream color eligibility is independent. | pure | finding | 2 → 2 | T0820, T0821 |
| V13-27 | Event projection applies stderr colors to Fatal and stdout colors to Line in both mirrored terminal configurations. | cli | finding | 1 → 1 | T0822 |
| V13-28 | Dry-run plans have one ordered line per action, while an empty plan explicitly says nothing is planned. | cli | none | 2 → 2 | T0823, T0824 |
| V13-29 | Run and all commands reject malformed arguments with code 2 before repository, path, or stage work. | cli | none | 2 → 2 | T0825, T0826 |
| V13-30 | The all-stage chain stops immediately and returns the first nonzero stage code. | stage | none | 1 → 1 | T0827 |
| V13-31 | The all-stage chain visits every stage in order and returns zero when all succeed. | stage | none | 1 → 1 | T0828 |
| V13-32 | run_all requires a usable bottle before starting any stage, for both absent and nonexistent bottle names. | stage | none | 2 → 2 | T0829, T0830 |
| V13-33 | The signal watcher invokes its first callback after the first observed delivery, not before. | cli | none | 1 → 1 | T0831 |
| V13-34 | After the first callback, the watcher remains active so a later second signal reaches the fatal action exactly once. | cli | none | 1 → 1 | T0832 |
| V13-35 | Two signal deliveries before one poll remain two events rather than collapsing into a flag. | cli | none | 1 → 1 | T0833 |
| V13-36 | SIGINT and SIGTERM share one delivery count, and a mixed-kind second delivery reraises the most recently delivered kind. | cli | none | 1 → 1 | T0834 |

#### V14 — `sabrage-contract-gen:lib.rs`, `sabrage-parity:lib.rs`

*These 55 functions protect 40 distinct guarantees concentrated in the CI parity gate: generated contract bytes, scanner soundness, contract and registry ordering, launch artifacts, argv/environment/text parity, and repository-root spelling. The target is 40 test carriers: delete four contract-generator mirrors in favor of parity carriers, replace seven scanner fixtures with two labelled tables, and merge six parity functions into existing parity tests. Most apparent core-versus-parity duplication remains because sabrage-core tests do not run in the Ubuntu parity gate. The real redundancy is fixture fragmentation and repeated entry-point assertions, not a broadly over-defensive suite.*

| id | behaviour | layer | protected by | now → target | carriers |
|---|---|---|---|---:|---|
| V14-01 | The generator reproduces committed contract.gen.sh exactly, including its current hash header and single final newline. | pure | golden | 4 → 1 | T0835 |
| V14-02 | Changing any shell-consumed contract field changes a generated body line, not only the header hash. | pure | finding | 1 → 1 | T0773 |
| V14-03 | Scalar and word encoders preserve safe historical spellings and quote shell-active literals correctly. | pure | finding | 1 → 1 | T0774 |
| V14-04 | generate_from emits hostile scalar and array values as non-expandable zsh literals. | unit | finding | 1 → 1 | T0775 |
| V14-05 | A real zsh source round-trips hostile generated values without execution, expansion, splitting, or array-cardinality drift. | stage | finding | 1 → 1 | T0776 |
| V14-06 | The public disk check reports the working checkout's generated contract file in sync. | parity | golden | 2 → 1 | T0837 |
| V14-07 | Shell setup/lib/doctor consume emitted asset and install-leaf variables and do not hard-code their current values. | parity | finding | 1 → 1 | T0836 |
| V14-08 | The shell fingerprint rows are sorted, cover exactly the tracked shell files, and contain their current hashes. | parity | golden | 1 → 1 | T0838 |
| V14-09 | Doctor emissions have no header-only loops or cross-section slug reuse and match contract slugs in first-emission order. | parity | parity | 1 → 1 | T0839 |
| V14-10 | Contract check slugs are unique. | parity | contract | 2 → 1 | T0839 |
| V14-11 | The doctor scanner distinguishes executable text from full-line, indented, and trailing comments without treating quoted or expansion hashes as comments. | pure | finding | 3 → 1 | NEW-V14-SLUG-COMMENTS |
| V14-12 | Loop headers are credited only when a chk/tap call carries that loop item's slug, directly or through its assigned carrier variable. | pure | finding | 4 → 1 | NEW-V14-SLUG-LOOPS |
| V14-13 | The doctor scanner records first-emission order and does not reduce emissions to a set. | pure | finding | 1 → 1 | T0847 |
| V14-14 | run.sh preflight tags form a conflict-free slug-to-gate map exactly equal to the contract's shell gates. | parity | finding | 2 → 1 | T0848 |
| V14-15 | Every real run.sh preflight tag group uses the executable verb its declared gate requires. | parity | finding | 1 → 1 | T0849 |
| V14-16 | The tag scanner reports both halves of a warn/block verb swap in a mixed protocol group. | pure | finding | 1 → 1 | T0850 |
| V14-17 | run.sh launch-action tags equal the contract launch-action IDs in order. | parity | contract | 1 → 1 | T0851 |
| V14-18 | Ordinary host-manifest comparison and file forms match the on-disk JSON template exactly. | parity | golden | 1 → 1 | T0856 |
| V14-19 | All host-manifest byte producers JSON-escape quote and backslash path spellings so decoded library_path equals the input. | parity | finding | 1 → 1 | T0857 |
| V14-20 | The privileged host-manifest boundary stages file-form bytes with exactly one newline beyond comparison form. | parity | golden | 1 → 1 | T0858 |
| V14-21 | The core TOML template bytes equal the working checkout's contract template. | parity | golden | 1 → 1 | T0859 |
| V14-22 | win_path implements the shared trailing-slash drive_c rule and Z: fallbacks for outside, bare drive_c, and missing-prefix paths. | parity | parity | 1 → 1 | T0860 |
| V14-23 | The contract appid renders as exactly 620980 with no newline. | parity | golden | 1 → 1 | T0861 |
| V14-24 | Every shell or native Autofix gate declares a fix action. | parity | contract | 1 → 1 | T0863 |
| V14-25 | The nonempty run-only check set is blocking on both shell and native sides. | parity | contract | 1 → 1 | T0864 |
| V14-26 | The protocol contract contains supported and legacy-oxrsys slugs and no obsolete unsplit cfg.protocol slug. | parity | contract | 1 → 1 | T0865 |
| V14-27 | Legacy reverse ports are exactly 9944, 9945, 9946, and 9948, excluding 9947. | parity | golden | 1 → 1 | T0866 |
| V14-28 | The strict native registry builds, follows contract order, and binds an evaluator for every contract slug. | parity | contract | 1 → 1 | T0867 |
| V14-29 | Native launch preflight slugs equal all and only native-gating contract checks in contract order. | parity | parity | 1 → 1 | T0868 |
| V14-30 | Native launch-action IDs equal the contract order and the contract contains exactly seven actions. | parity | parity | 1 → 1 | T0869 |
| V14-31 | A dry-run Goldberg stage plans the steam_appid.txt write at the right destination with the appid payload length. | stage | parity | 1 → 1 | T0870 |
| V14-32 | A real-writing Goldberg stage places exactly the contract appid bytes on disk and performs the DLL replacement. | stage | finding | 1 → 1 | T0871 |
| V14-33 | wine_env reproduces fixed exports and WINEDEBUG default, caller-precedence, verbose, and empty-value semantics. | pure | parity | 1 → 1 | T0872 |
| V14-34 | wine_spec selects the configured wine program and exact run.sh-compatible ordered argv. | unit | parity | 1 → 1 | T0873 |
| V14-35 | Wine log candidates use run.sh's timestamp spelling and the declared -2 collision suffix. | pure | parity | 1 → 1 | T0874 |
| V14-36 | Native run-only and preflight die/warn renderers retain run.sh's verbatim text in the CI parity gate. | parity | finding | 2 → 1 | T0876 |
| V14-37 | Launch-action status, warning, and failure text retained by the native implementation remains verbatim in run.sh. | parity | golden | 1 → 1 | T0877 |
| V14-38 | Native audio and dashboard guard constants/renderers remain verbatim in run.sh. | parity | finding | 1 → 1 | T0878 |
| V14-39 | Native banner events, interpolated banner fields, exit text, and interrupt text retain run.sh's wording. | parity | finding | 1 → 1 | T0879 |
| V14-40 | Both frontends use logical repository-root spelling: dot segments fold lexically, symlink spelling survives, and equivalent spellings render identical manifest bytes. | parity | finding | 2 → 1 | T0881 |

#### V15 — `sabrage-core:contract.rs`, `sabrage-core:error.rs`, `sabrage-core:events.rs`, `sabrage-core:executor.rs`, `sabrage-core:paths.rs`, `sabrage-core:tap.rs`, `sabrage-core:util/hash.rs`, `sabrage-core:util/mod.rs`, `sabrage-core:util/winpath.rs`

*This roster mostly protects real contracts: executor safety, path spelling, event-wire stability, and shell-byte parity. Under the standard, 47 tests stay, five surviving tests lose duplicated assertions, five tests delete, and three core tests merge into parity carriers. The clearest redundancy is the duplicated compiled-contract fixtures, the dead ErrorPayload self-test, a private plan-sharing seam already exercised through StageCtx, and core golden copies already held by CI-running parity tests. The 112-line PlannedAction renderer and the aggregate dry-run mutation test are long but load-bearing rather than redundant.*

| id | behaviour | layer | protected by | now → target | carriers |
|---|---|---|---|---:|---|
| V15-01 | The compiled contract exposes the current game pins, paths, ports, DXMT cardinality, check presence, and launch-action cardinality. | parity | contract | 1 → 3 | T0001, T0866, T0869 |
| V15-02 | Contract check slugs are unique and meta.contract-sync is first. | parity | contract | 1 → 2 | T0002, T0862 |
| V15-03 | Representative check entries decode shell/native gates, volatility, gating participation, and fix bindings exactly. | parity | contract | 1 → 1 | T0003 |
| V15-04 | The DepotDownloader remedy renders the pinned triple and quoted destination exactly like lib.sh. | parity | golden | 1 → 1 | T0004 |
| V15-05 | The compiled contract identity equals its source checkout and differs from a self-consistent checkout with different contract bytes. | unit | finding | 2 → 2 | T0572, T0571 |
| V15-06 | Compiled templates retain the required runtime protocol, host placeholder, and source trailing newline. | parity | golden | 1 → 1 | T0007 |
| V15-07 | Errors already emitted as rows, plus cancellation, are classified as already reported; other errors are not. | ipc | none | 1 → 1 | T0008 |
| V15-08 | Stage values round-trip through demo.sh words and JSON, while an unknown stage is a usage error. | parity | contract | 1 → 1 | T0010 |
| V15-09 | Only install, run, and stop require a CrossOver bottle. | pure | none | 1 → 1 | T0011 |
| V15-10 | Step ids are unique, stage-prefixed, and preserve the declared run sequence boundaries. | ipc | contract | 1 → 1 | T0012 |
| V15-11 | Generic stage events use stable internal kind tags and camelCase wire fields with verbatim row text. | ipc | contract | 1 → 1 | T0013 |
| V15-12 | Run Text, Check, and Launched events retain their distinct stable wire shapes. | ipc | contract | 1 → 1 | T0014 |
| V15-13 | Every event returns its run id, exposes a step only where applicable, and round-trips through JSON unchanged. | ipc | none | 1 → 1 | T0015 |
| V15-14 | copy_if_changed copies absent or differing destinations and skips byte-identical destinations. | unit | none | 1 → 1 | T0016 |
| V15-15 | A failed copy preserves the previous destination, reports that destination, and cleans temporary files. | unit | finding | 1 → 1 | T0017 |
| V15-16 | Byte-identical destinations with mode drift are repaired by real execution and truthfully planned without mutation by dry-run. | unit | none | 1 → 1 | T0018 |
| V15-17 | create_new publishes only when absent, never clobbers existing bytes, and reports both dry-run branches truthfully. | unit | none | 1 → 1 | T0019 |
| V15-18 | write_atomic creates fresh files at 0644 and preserves a pre-existing destination's mode. | unit | finding | 1 → 1 | T0020 |
| V15-19 | Failure to open or sync a write's parent directory is reported rather than treated as durable success. | unit | finding | 1 → 1 | T0021 |
| V15-20 | create_new publishes complete bytes through a temporary inode, leaves no temporary name, and never replaces an existing empty file. | unit | finding | 1 → 1 | T0022 |
| V15-21 | hard_link captures the currently named bytes and refuses to replace an existing destination name. | unit | finding | 1 → 1 | T0023 |
| V15-22 | write_atomic replacement leaves no sibling temporary files. | unit | none | 1 → 1 | T0024 |
| V15-23 | Dry-run copy performs real probes, records truthful Skip/Copy reasons, and writes nothing. | unit | none | 1 → 1 | T0025 |
| V15-24 | Dry-run child execution records Spawn and returns simulated success without invoking the program. | unit | none | 1 → 1 | T0026 |
| V15-25 | Executor views narrowed to a step share the same accumulated dry-run plan. | stage | none | 1 → 1 | T0356 |
| V15-26 | remove_file deletes an existing file and treats a repeated removal as success. | unit | none | 1 → 1 | T0028 |
| V15-27 | Dry-run remove_file preserves the file and records the exact removal target and reason. | unit | none | 1 → 1 | T0029 |
| V15-28 | A cancelled real executor rejects filesystem mutations, except that rollback removal remains allowed and idempotent. | unit | none | 1 → 1 | T0030 |
| V15-29 | Across every Executor mutation primitive, a dry-run changes no filesystem state and records one action per call. | unit | none | 1 → 1 | T0031 |
| V15-30 | Every PlannedKind renders its exact readable line and Display equals describe(). | pure | none | 1 → 1 | T0032 |
| V15-31 | Download temporary paths append .tmp to the complete destination filename. | pure | none | 1 → 1 | T0033 |
| V15-32 | A real detached child writes both output pipes to one log and returns its process identity. | unit | none | 1 → 1 | T0034 |
| V15-33 | Detached launch refuses an existing log without truncating its previous bytes. | unit | none | 1 → 1 | T0035 |
| V15-34 | Dry detached launch creates neither process nor log and renders log and null stdio plans correctly. | unit | none | 1 → 1 | T0036 |
| V15-35 | A pre-cancelled real executor refuses detached launch. | unit | none | 1 → 1 | T0037 |
| V15-36 | Repository discovery walks ancestors to the demo.sh plus scripts/demo/lib.sh marker pair and otherwise returns none. | unit | none | 1 → 1 | T0066 |
| V15-37 | Explicit repository roots honor precedence and become logical absolute paths with lexical dot folding and symlink spelling preserved. | parity | finding | 3 → 3 | T0067, T0881, T0069 |
| V15-38 | Mutating path construction rejects missing, empty, or relative HOME and accepts an absolute home. | unit | finding | 1 → 1 | T0070 |
| V15-39 | Paths derives the complete root-relative artifact set, Sabrage state and lock paths, logs, and relative display names correctly. | unit | none | 1 → 1 | T0071 |
| V15-40 | CrossOver-derived helpers are present exactly when CrossOver is found and never fabricate the root-based bogus CX path. | unit | none | 1 → 1 | T0072 |
| V15-41 | Bottle helper paths match lib.sh's prefix, system32, conf, z-drive, and OpenXR manifest layout. | unit | none | 1 → 1 | T0073 |
| V15-42 | Beat Saber directory resolution prefers an override, otherwise uses the bottle and contract leaf, including the no-bottle shell quirk. | unit | none | 1 → 1 | T0074 |
| V15-43 | Tap output uses the fixed zsh status words and emits one ordered newline-terminated slug/status line per outcome. | parity | golden | 2 → 2 | T0124, T0125 |
| V15-44 | SHA helpers produce lowercase SHA-256 and treat a missing file as a non-match. | pure | none | 2 → 2 | T0193, T0194 |
| V15-45 | cmp_files returns true only for two readable byte-identical files and false for differences or either missing side. | unit | none | 1 → 1 | T0195 |
| V15-46 | Host-manifest comparison and file forms derive exactly from the on-disk template and differ by one trailing newline. | parity | parity | 1 → 1 | T0856 |
| V15-47 | Host-manifest renderers JSON-escape quote and backslash path bytes while decoding back to the original dylib path. | parity | finding | 1 → 1 | T0857 |
| V15-48 | json_escape_string performs exactly install.sh's backslash-then-quote substitutions and deliberately no full JSON escaping. | parity | golden | 1 → 1 | T0198 |
| V15-49 | The core runtime hash of the three contract files equals the hash recorded in contract.gen.sh. | parity | golden | 1 → 1 | T0199 |
| V15-50 | DXMT is current only when every contracted artifact exists and the provenance marker matches the pin under command-substitution newline semantics. | unit | none | 1 → 1 | T0200 |
| V15-51 | The DXMT provenance marker's write form is the pin plus exactly one newline. | parity | golden | 1 → 1 | T0201 |
| V15-52 | Beat Saber version detection returns ? when absent and otherwise mirrors grep's bounded stamp and first-matching-line semantics. | pure | none | 2 → 2 | T0202, T0203 |
| V15-53 | win_path exactly mirrors the shell table for drive_c, Z-drive, empty-prefix, spaces, and lookalike-directory cases. | parity | parity | 1 → 1 | T0860 |

#### V16 — `sabrage-core:privilege.rs`, `sabrage-core:process.rs`

*These modules are not broadly over-tested: 44 of 49 roster functions survive, protecting the privileged-write security boundary and distinct child-process lifecycle states. Five functions can go: one core byte-pin already owned by parity, two private/entry-point mirrors, one platform smoke test, and one serde self-test. Three surviving tests should lose redundant or change-detector assertions. The cancellation, quoting, splitter, TCC, and process-identity groups look similar syntactically but protect different inputs, boundaries, or failure states.*

| id | behaviour | layer | protected by | now → target | carriers |
|---|---|---|---|---:|---|
| V16-01 | AppleScript string literals escape backslash, quote, newline, carriage return, and tab without altering unrelated characters. | pure | none | 1 → 1 | T0075 |
| V16-02 | Every shell argv element is single-quoted so spaces, apostrophes, substitutions, and metacharacters remain data. | pure | none | 1 → 1 | T0076 |
| V16-03 | Hostile and Unicode paths survive the composed AppleScript and shell quoting layers as the intended argv. | unit | none | 1 → 1 | T0077 |
| V16-04 | Elevation creates the destination directory and installs root:wheel 0644 using one osascript child or the shell-compatible two sudo children. | unit | none | 2 → 2 | T0078, T0079 |
| V16-05 | A staged manifest has the intended bytes and 0600 mode and is removed when an armed staging guard drops. | unit | none | 1 → 1 | T0080 |
| V16-06 | The staging directory is 0700 and distinct writes receive distinct unpredictable names. | unit | none | 1 → 1 | T0081 |
| V16-07 | A shell-equivalent current host manifest is skipped without authorization, including files with extra trailing newlines. | stage | none | 1 → 1 | T0082 |
| V16-08 | A privileged dry run reports Planned, emits only preview wording, records staging and child actions, mutates nothing, and never places manifest content in argv. | stage | none | 1 → 1 | T0083 |
| V16-09 | Cancellation reaps an ordinary elevated child before return and preserves a possibly in-use staging source until a later age-qualified sweep. | unit | finding | 2 → 2 | T0084, T0088 |
| V16-10 | An already-cancelled token prevents both raw elevation helpers from spawning any child. | unit | none | 1 → 1 | T0085 |
| V16-11 | A pre-cancelled public privileged write returns before announcing authorization, staging bytes, or touching the destination. | stage | finding | 1 → 1 | T0086 |
| V16-12 | Control characters that would make the host manifest invalid are rejected before rendering or authorization while ordinary paths remain accepted. | stage | finding | 1 → 1 | T0087 |
| V16-13 | The administrator announcement accurately distinguishes a terminal sudo prompt from an osascript dialog while explaining the host registration. | pure | finding | 1 → 1 | T0089 |
| V16-14 | A child write failure is upgraded to TCC denial only for an app-bundle destination and a recognized permission tail, emitting one diagnostic event. | unit | none | 2 → 2 | T0090, T0091 |
| V16-15 | The osascript -128 stderr marker denotes declined authorization; other failures and empty stderr do not. | pure | none | 1 → 1 | T0092 |
| V16-16 | Only PermissionDenied inside an app bundle is classified and upgraded as likely App Management; other paths, errnos, and errors pass through. | unit | none | 2 → 2 | T0093, T0094 |
| V16-17 | App Management diagnostics remain explicitly hypothetical and provide settings, relaunch, and terminal fallback remedies. | pure | none | 1 → 1 | T0095 |
| V16-18 | Declined host authorization renders doctor's exact bottle-qualified terminal fallback text. | pure | golden | 1 → 1 | T0096 |
| V16-19 | A path is inside an app bundle when any whole component has the `.app` extension, not merely a similar suffix. | pure | none | 1 → 1 | T0097 |
| V16-20 | Privileged staging uses Sabrage's Application Support tmp subdirectory and never the world-writable `/tmp` root. | pure | none | 1 → 1 | T0098 |
| V16-21 | Either stdin or a controlling terminal selects sudo; only absence of both selects osascript, independent of stdout. | pure | none | 1 → 1 | T0099 |
| V16-22 | The privileged host-manifest byte source is the on-disk template's file form with exactly one trailing newline beyond the comparison form. | parity | parity | 1 → 1 | T0858, T0856 |
| V16-23 | The terminator-blind splitter handles LF, CR, CRLF, blank chunks, read boundaries, EOF partials, and empty input. | pure | none | 1 → 1 | T0101 |
| V16-24 | The terminator-aware splitter reports LF for LF/CRLF, CR for repaint chunks, and EOF for an unterminated final chunk. | pure | none | 1 → 1 | T0102 |
| V16-25 | `spawn_streamed` returns the child's status and emits attributed stdout and stderr chunks. | unit | none | 2 → 1 | T0103 |
| V16-26 | Output events preserve each streamed chunk's original terminator through the core event boundary. | unit | finding | 1 → 1 | T0104 |
| V16-27 | `run_ok` converts a nonzero child exit into ChildFailed with argv0, status, and the captured output tail. | unit | none | 1 → 1 | T0105 |
| V16-28 | Cancelling an ordinary streamed child terminates its process group and returns Cancelled with public exit code 130. | unit | none | 1 → 1 | T0107 |
| V16-29 | Cancellation escalates against a TERM-ignoring descendant that outlives its leader and retains the output pipes. | unit | finding | 1 → 1 | T0108 |
| V16-30 | Cancellation also kills a TERM-ignoring descendant that releases the pipes, using process-group liveness rather than EOF. | unit | finding | 1 → 1 | T0109 |
| V16-31 | Without cancellation, an intentionally backgrounded descendant cannot wedge the stage and output written before leader exit is retained. | unit | none | 1 → 1 | T0110 |
| V16-32 | Read-only probes are bounded by both deadline and cancellation, returning the distinct timeout and Cancelled outcomes promptly. | unit | finding | 2 → 2 | T0111, T0112 |
| V16-33 | Sabrage's synthetic ExitStatus conversion and exit-code extraction preserve ordinary success and failure codes. | pure | none | 1 → 1 | T0113 |
| V16-34 | Executable and command-line process scans return the same concrete matches whether used through convenience entry points or shared snapshots. | unit | none | 2 → 2 | T0114, T0372 |
| V16-35 | GUI child PATH places Homebrew, `/usr/local`, cargo, and Android tools first and removes duplicate entries. | pure | none | 1 → 1 | T0117 |
| V16-36 | ChildSpec builders retain spawn configuration and render the intended human-readable command line. | pure | none | 1 → 1 | T0118 |
| V16-37 | Targeted process observation returns PID, executable, and a nonzero start time consistent with a full executable scan. | unit | none | 1 → 1 | T0119 |
| V16-38 | Process identity accepts the live observed process and rejects recycled start times, zero-time fallbacks, and dead PIDs. | unit | none | 1 → 1 | T0120 |
| V16-39 | The capture API collects both pipes and status, trims shell-style trailing newlines on request, and reports missing programs as io errors. | unit | none | 1 → 1 | T0122 |
| V16-40 | Command-line matching follows pgrep-f substring semantics across individual argv elements and the joined command line. | pure | none | 1 → 1 | T0123 |

#### V17 — `sabrage-core:logs.rs`, `sabrage/src-tauri:commands.rs`

*The 54 roster tests protect 47 distinct behaviours; most of the apparent repetition is load-bearing state-transition coverage in Tailer and Tauri lifecycle policy. Nine functions can disappear safely: four direct deletions and five assertion-preserving merges, reducing this roster to 45 functions. One cleanup assertion should move to its existing session-slot carrier, and the truncate/regrow test should become a branch-free labelled table. The strongest redundancy is in duplicated wine-name goldens, private predicates covered through public decisions, a misplaced contract pin, and Rust struct/derive self-tests.*

| id | behaviour | layer | protected by | now → target | carriers |
|---|---|---|---|---:|---|
| V17-01 | wine_log_candidate produces the shell-compatible local timestamp for attempt zero and appends -{attempt+1} on collisions. | pure | parity | 3 → 1 | T0874 |
| V17-02 | wine_log_candidate_stamped produces the same exact filename rule from a caller-supplied stamp without a chrono-typed input. | pure | golden | 2 → 1 | T0041 |
| V17-03 | LogSource uses the stable internally tagged camelCase IPC shape, including File as a struct variant with path. | ipc | contract | 1 → 1 | T0042 |
| V17-04 | The fixed OXRSys sources resolve under oxr_appsup and an explicit File resolves to its supplied path. | pure | none | 1 → 1 | T0043 |
| V17-05 | WineConsole resolves to None when there is no live session, usable persisted log, or past run. | unit | none | 1 → 1 | T0044 |
| V17-06 | A persisted session log that still exists outranks directory recency for WineConsole resolution. | unit | none | 1 → 1 | T0045 |
| V17-07 | A persisted session whose log disappeared is ignored and resolution continues to real past runs. | unit | none | 1 → 1 | T0046 |
| V17-08 | With no live or usable persisted session, WineConsole resolves to the newest matching past run. | unit | none | 1 → 1 | T0047 |
| V17-09 | The current in-process live session log outranks persisted state and newer past-run files. | unit | none | 1 → 1 | T0048 |
| V17-10 | Past runs include only matching regular files, newest first, with filename and size metadata. | unit | none | 1 → 1 | T0049 |
| V17-11 | Listing a missing logs directory returns an empty collection rather than failing. | unit | none | 1 → 1 | T0050 |
| V17-12 | An ordinary append produces only the newly completed lines and is never reported as rotation. | unit | none | 2 → 1 | T0051 |
| V17-13 | Rename-and-recreate rotation reopens from the beginning and marks the first new batch rotated. | unit | none | 1 → 1 | T0052 |
| V17-14 | Same-inode truncation below the cursor reopens from the beginning and reports rotation. | unit | none | 1 → 1 | T0053 |
| V17-15 | A trailing partial line is withheld until a later poll supplies its newline, then delivered joined. | unit | none | 1 → 1 | T0054 |
| V17-16 | Opening from the end preloads exactly the requested last N lines and remains positioned for later appends. | unit | none | 1 → 1 | T0055 |
| V17-17 | Opening from the end with zero preload starts at EOF and emits no existing content. | unit | none | 1 → 1 | T0056 |
| V17-18 | A missing source can be opened and polled safely; its later first appearance is a fresh rotated batch. | unit | none | 1 → 1 | T0057 |
| V17-19 | A line burst over MAX_LINES_PER_POLL is delivered in ordered capped batches without dropping the remainder. | unit | none | 1 → 1 | T0058 |
| V17-20 | One poll consumes at most POLL_BYTE_BUDGET while repeated polls eventually deliver the entire large file. | unit | finding | 1 → 1 | T0059 |
| V17-21 | A newline-free input cannot grow the splitter indefinitely and is synthetically broken at the configured bound. | unit | finding | 1 → 1 | T0060 |
| V17-22 | Same-inode truncate-and-regrow to equal or larger length is detected between polls and restarts at the new first line. | unit | finding | 1 → 1 | T0061 |
| V17-23 | Queued old-file lines survive disappearance and reappearance, remain in their old epoch, and precede the deferred new-file rotation marker. | unit | none | 1 → 1 | T0063 |
| V17-24 | Applying the UI clear-on-rotated contract leaves only lines from the new file incarnation after rotation. | unit | finding | 1 → 1 | T0064 |
| V17-25 | A rewrite inside the read window causes the straddled read to be discarded and the next poll to reopen from byte zero. | unit | finding | 1 → 1 | T0065 |
| V17-26 | Beat Saber browsing starts at the current field's nearest directory, then the bottle-derived directory, then HOME; blank bottles produce no derived path. | pure | none | 1 → 1 | T0882 |
| V17-27 | Persisted settings fill only option fields left unset by higher-precedence sources, and blank defaults count as unset. | pure | none | 1 → 1 | T0883 |
| V17-28 | Stage options inherit WINEVR_BOTTLE when the GUI supplies none, while an explicit GUI bottle wins. | pure | none | 1 → 1 | T0884 |
| V17-29 | Launch-only flags inherit their base values field-by-field, GUI Some values override, and dry-run defaults false. | pure | none | 1 → 1 | T0885 |
| V17-30 | Stop targets the live session only when no bottle was requested or the requested bottle matches it. | pure | none | 1 → 1 | T0886 |
| V17-31 | The bounded live-slot wait distinguishes a slot that clears during polling from one still occupied at its deadline. | pure | finding | 1 → 1 | T0887 |
| V17-32 | Quit refusal text is silent for completed teardown and accurately distinguishes timeout from a real detach without claiming actions that did not occur. | pure | finding | 1 → 1 | T0888 |
| V17-33 | Every deferred fix is removed from Doctor IPC and refused by the direct GUI fix door, while offered fixes remain reachable. | ipc | finding | 1 → 1 | T0889 |
| V17-34 | Quit interception asks while a live unapproved session has a responsive window, gives up after the unanswered deadline, and otherwise passes through. | pure | finding | 2 → 1 | T0890 |
| V17-35 | PendingQuit records the first request instant, does not refresh it on repeats, and clears after resolution. | ipc | none | 1 → 1 | T0891 |
| V17-36 | A log-tail registry entry disappears whenever its task guard ends, while stopping a live entry signals its task. | ipc | finding | 1 → 1 | T0892 |
| V17-37 | Page-level stop_all drains every tracked tail and signals every corresponding task. | ipc | none | 1 → 1 | T0893 |
| V17-38 | Session-status broadcasting emits the first snapshot and every change but suppresses consecutive duplicates. | ipc | none | 1 → 1 | T0894 |
| V17-39 | A GUI dry run emits the shared plan section and rows, while a real run emits no plan rows. | ipc | none | 1 → 1 | T0896 |
| V17-40 | The contract's run-only slugs use the no-Doctor-row group and remain part of native launch gating, unlike a Doctor-only control slug. | parity | parity | 1 → 1 | T0868 |
| V17-41 | RunRegistry cancellation reports whether an entry existed, fires its canceller once, and is idempotent afterward. | ipc | none | 1 → 1 | T0898 |
| V17-42 | RunRegistry forget removes an entry without firing its cancellation callback. | ipc | none | 1 → 1 | T0899 |
| V17-43 | Repository-source classification follows settings over environment over executable walk, otherwise unresolved. | pure | none | 1 → 1 | T0902 |
| V17-44 | A returned game row preserves its entry while recomputing current installation validity from Paths. | ipc | none | 1 → 1 | T0904 |
| V17-45 | A populated SettingsPathsCache returns the stored settings/paths pair without reloading or probing. | ipc | none | 1 → 1 | T0905 |
| V17-46 | Cache invalidation removes the stored settings/paths pair so the next access must reload. | ipc | none | 1 → 1 | T0906 |
| V17-47 | A last-session record is produced only when game id, Launched information, and a settled outcome all exist, with the launch and outcome fields preserved. | pure | none | 1 → 1 | T0907 |

---

## 3. The reduction program, by module

Every row below survived three gates: a Codex reviewer proposed it against the standard, an opus verifier re-read both tests and mapped each assertion of the loser to a line of the carrier (the mapping is in the verifier's JSON), and the kill matrix confirmed that no mutant is caught only by the deleted test. Rows a verifier downgraded appear with the weaker verdict; rows the critic changed carry its reason. `+carrier` is the verifier's measured growth of the surviving test (a merged assertion, a table row with its label).

Apply order inside a module: `delete` (D/X/F) → `merge` (I into the public test) → `table` (V) → `drop_assertion`; cross-layer removals whose carrier is a parity test go last. `mutants lost` is `0` when the kill matrix shows every mutant the loser catches is also caught by a surviving test; a non-empty list names what the carrier must keep killing (these rows were vetoed to keep when the verdict was `delete`).

### `sabrage-cli:main.rs` — 19 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0802 `merge_stage_options_flags_only_ever_add_never_clear` | delete (verifier: CONFIRM_WITH_NOTE) | merge_stage_options_env_base_survives_when_no_flag_overrides_it |  | 14 | 0 | 0 | Delete only with T0801 named as the carrier in the commit message (3.4b), and note there that `StageArgs.wired` is a plain bool (main.rs:640) so T0801's `StageArgs::default()` is byte-identical to T0802's explicit `wired: false`. |
| T0808 `fatal_line_has_no_leading_spaces_unlike_the_other_rows` | delete | fatal_uses_stderr_colors_while_a_line_event_uses_stdout_colors |  | 10 | 0 | 0 | Delete outright; no comment or label needs moving, since fatal_line's die-shape rationale already lives on the production doc at main.rs:483-489. |
| T0809 `a_fatal_with_a_remedy_gets_the_same_continuation_line_a_fail_row_does` | delete (verifier: CONFIRM_WITH_NOTE) | fatal_and_stage_finished_events_compose_through_the_shared_helpers |  | 27 | 4 | 0 | Move the four-line 'Finding #4 / finding #6 upgrade_write_error' comment from main.rs:1861-1863 onto T0819's remedy assertion before deleting, so the pre-review regression label keeps a home. |
| T0810 `errors_that_already_emitted_a_fatal_are_not_reported_a_second_time` | delete | already_reported_covers_the_variants_that_emit_their_own_row (`sabrage-core:error.rs`) |  | 25 | 0 | 0 | Delete outright; the Cancelled rationale the loser's comment carries is already on the production doc at sabrage-core/src/error.rs:139-144, so nothing needs re-hosting. |
| T0814 `progress_event_renders_to_nothing` | merge (verifier: CONFIRM_WITH_NOTE) | check_launched_and_auto_fixed_events_render_to_nothing |  | 10 | 10 | 0 | Rename the carrier when merging (e.g. structured_only_events_render_no_console_line) so its name still enumerates what it covers, and keep the AutoFixed 'would double the console row' comment at main.rs:2000-2001 attached to its own case. |
| T0780 `missing_value_messages_match_demo_sh_verbatim` | table (verifier: CONFIRM_WITH_NOTE) | NEW-V13-doctor-parse-errors |  | 14 | 12 | 0 | Keep the --tap row inside the doctor-only table and never fold it into the stage-parser table (T0798 pins the opposite answer for the same token), and charge the ~9-line table scaffold to this row rather than the reviewer's rows-only 6. |
| T0781 `unknown_argument_message_matches_demo_sh_verbatim` | table (verifier: CONFIRM_WITH_NOTE) | NEW-V13-doctor-parse-errors |  | 12 | 2 | 0 | Turn the :1452-1453 comment into the positional row's label string so the "same `*)` branch as demo.sh:40" fact is not dropped when the two asserts become rows. |
| T0782 `first_bad_argument_wins_no_aggregation` | table (verifier: CONFIRM_WITH_NOTE) | NEW-V13-doctor-parse-errors |  | 7 | 1 | 0 | Put this row in the doctor table only — its jaccard-1.0 twin T0799 (main.rs:1669-1676) tests parse_stage_args, a different unit that can drift, so the two must not be merged into one shared table. |
| T0783 `unknown_command_outcome_reports_the_first_bad_remaining_token` | table (verifier: CONFIRM_WITH_NOTE) | NEW-V13-unknown-command-outcomes | r1:A14-4 | 11 | 12 | 0 | The A14-4 label must land on this row's label string (bare `A14-4`, since the id exists in round 1 only) and the section comment at main.rs:1469 must not be deleted without it moving there. |
| T0784 `unknown_command_outcome_is_ok_when_the_remaining_tokens_parse_clean` | table (verifier: CONFIRM_WITH_NOTE) | NEW-V13-unknown-command-outcomes |  | 9 | 2 | 0 | Keep this Ok(()) row in the same A14-4-labelled table as T0783 — round-1's fix sketch (2026-08-30-codex-round1.md:1668) names both cases as that finding's regression test, so it is not an unlabelled variant. |
| T0785 `pass_row_has_three_space_gap_like_ok` | table (verifier: CONFIRM_WITH_NOTE) | NEW-V13-doctor-outcome-rendering |  | 7 | 12 | 0 | This row creates the rendering table, so charge its ~10-line scaffold here, and keep the expected column typed Option<&str> compared against `.as_deref()` so the None rows (T0790) join without a branch. |
| T0786 `warn_row_has_one_space_gap_like_warn` | table | NEW-V13-doctor-outcome-rendering |  | 7 | 2 | 0 | Row moves verbatim; its only index partner T0807 (main.rs:1811-1845) exercises format_line_event, a different unit, and must stay a separate test. |
| T0787 `fail_row_with_remedy_aligns_at_column_seven` | table (verifier: CONFIRM_WITH_NOTE) | NEW-V13-doctor-outcome-rendering |  | 11 | 4 | 0 | Copy the two-line expected string with its embedded \n and seven-space indent byte-for-byte into the row — nothing in sabrage-parity pins these doctor row bytes, so this table is their last copy in the tree. |
| T0788 `fail_row_without_remedy_has_no_remedy_line` | table | NEW-V13-doctor-outcome-rendering |  | 7 | 2 | 0 | Row must keep the whole expected string including the absence of a remedy continuation, since it is the only place the fail_bare-vs-fail branch of format_outcome (main.rs:456) is distinguished from T0787. |
| T0789 `info_row_is_two_space_indent_no_label` | table | NEW-V13-doctor-outcome-rendering |  | 10 | 2 | 0 | Keep the row's expected string byte-exact (two leading spaces, no status label) — sabrage-parity pins no doctor row text, so this table is the last home for those bytes. |
| T0790 `skipped_and_not_implemented_print_nothing` | table | NEW-V13-doctor-outcome-rendering |  | 6 | 4 | 0 | Must become two separately labelled rows, not one — Skipped and NotImplemented share a single match arm (main.rs:458) and a single row would let a split of that arm pass unnoticed. |
| T0791 `colors_wrap_only_the_label_text` | table (verifier: CONFIRM_WITH_NOTE) | NEW-V13-doctor-outcome-rendering |  | 7 | 2 | 0 | This is the only row with colors=true, so the table must carry a `colors` column and keep the full \x1b[32mOK\x1b[0m literal with its three trailing spaces — it is the only assertion in the tree that the ANSI wrapper covers the label and not the message. |
| T0793 `verbose_with_no_detail_prints_nothing_extra` | table (verifier: DOWNGRADE) | detail_is_hidden_by_default_and_shown_only_when_verbose |  | 7 | 2 | 0 | Move the assertion into the NEW-V13-doctor-outcome-rendering table as the verbose=true row for T0785's outcome (2 lines) instead of appending a second fixture to T0792, so the r2:A3b-3 regression test keeps exactly one behaviour and one fixture. |
| T0821 `colors_from_is_independent_per_stream` | table (verifier: MISSED_DUP) | no_color_forces_both_streams_off_regardless_of_tty |  | 24 | 26 | 0 | Fold T0820 + T0821 into one three-row `(label, (no_color, stdout_tty, stderr_tty), Colors)` table under the verbatim `── A14-5: color gating is per-stream ──` header (main.rs:2233), carrying T0821's 'other half of the same bug' prose into its row label — and budget it as line-neutral (~+26 to the ca |

### `sabrage-contract-gen:lib.rs` — 4 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0770 `generate_reproduces_the_committed_file_byte_for_byte` | delete (verifier: CONFIRM_WITH_NOTE) | generate_matches_the_committed_contract_gen_sh (`sabrage-parity:lib.rs`) |  | 8 | 0 | 0 | Delete the COMMITTED const at sabrage/crates/sabrage-contract-gen/src/lib.rs:374-375 in the same edit — it is used only by T0770 (:381,:383) and T0771 (:392), so cutting both leaves an unused const (dead_code) behind. |
| T0771 `header_hash_is_the_documented_recipe` | delete (verifier: CONFIRM_WITH_NOTE) | generate_matches_the_committed_contract_gen_sh (`sabrage-parity:lib.rs`) |  | 6 | 0 | 0 | After this cut `contract_sha256()` (sabrage/crates/sabrage-contract-gen/src/lib.rs:158) has zero callers left in the workspace (grep: its only reference was the loser at :390), so delete the function with the test or accept an untested, unused public API. |
| T0772 `generated_file_ends_with_exactly_one_newline` | delete (verifier: CONFIRM_WITH_NOTE) | generate_matches_the_committed_contract_gen_sh (`sabrage-parity:lib.rs`) |  | 5 | 0 | 0 | Both assertions hold only *through* the golden, so if you want the shape invariant to survive a future co-regeneration of generate() + contract.gen.sh, fold `assert!(!generated.ends_with("\n\n"))` into T0835 as one extra line instead of dropping it outright. |
| T0777 `check_against_the_working_checkout_is_in_sync` | merge (verifier: DOWNGRADE) | check_reports_in_sync_against_the_live_checkout (`sabrage-parity:lib.rs`) |  | 7 | 3 | 0 |  |

### `sabrage-core:checks/audio.rs` — 1 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0513 `defs_binds_the_one_slug` | delete | registry_binds_in_contract_order_and_covers_every_slug (`sabrage-core:checks/mod.rs`) |  | 4 | 0 | 0 | Delete audio.rs:113-117 as part of the already-programmed H3-4 batch, and keep checks/mod.rs:616-621 named in the commit message as the carrier (the 08-31 acceptance criterion already names it). |

### `sabrage-core:checks/bottle.rs` — 1 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0520 `defs_binds_all_five_slugs_in_contract_order` | delete | registry_binds_in_contract_order_and_covers_every_slug (`sabrage-core:checks/mod.rs`) |  | 13 | 0 | 0 | Delete bottle.rs:358-371 with the rest of H3-4 and name checks/mod.rs:616-621 as the carrier in the commit message; the golden-list item 'registry order' is the Windows registry key order in parity, not this vector, so nothing golden moves. |

### `sabrage-core:checks/bridge.rs` — 1 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0522 `no_bottle_skips_all_three_with_the_verbatim_reason` | drop_assertion | no_bottle_skips_all_three_with_the_verbatim_reason |  | 13 | 0 | 0 | Delete only bridge.rs:180 and keep the loop at :181-189 verbatim — the carrier chain terminates here, because the reviewer's 'carrier' is this same test surviving as a keep minus one line. |

### `sabrage-core:checks/config.rs` — 9 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0552 `defs_binds_all_three_slugs_in_contract_order` | delete (verifier: CONFIRM_WITH_NOTE) | strict_registry_builds_and_covers_the_contract_in_order (`sabrage-parity:lib.rs`) |  | 11 | 0 | 0 | Land this together with the H4-2 comment replacement for checks/config.rs:1 (2026-08-31-simplification.md:507), which currently points the module doc at `defs_binds_all_three_slugs_in_contract_order` and would become a dangling pointer the moment this test goes. |
| T0538 `alvr_protocol_passes_both_slugs` | table (verifier: CONFIRM_WITH_NOTE) | NEW-config_protocol_state_matrix |  | 11 | 5 | 0 | Because the table must assert message and remedy uniformly (an Option message column would need an if-let, and comparing None to the real message is an assertion that cannot fail, 3.7), this row has to add the two Pass message literals from config.rs:146 and :179 that the current test never pins — b |
| T0539 `oxrsys_protocol_passes_supported_and_fails_legacy` | table (verifier: CONFIRM_WITH_NOTE) | NEW-config_protocol_state_matrix |  | 21 | 6 | 0 | The expected remedy embeds this row's own scratch path, so the table must store `set protocol = "alvr" in {toml}` and substitute the row's path uniformly (a per-row closure would be logic in the test). |
| T0540 `garbage_protocol_fails_supported_and_skips_legacy` | table (verifier: CONFIRM_WITH_NOTE) | NEW-config_protocol_state_matrix |  | 24 | 6 | 0 | Label the row for the Other/unsupported arm (e.g. `legacy_usb-unsupported`) and keep the legacy Skipped expectation distinct from T0537's Missing Skipped, since those are two different match arms that would otherwise read as one fact. |
| T0542 `shadowed_protocol_oxrsys_then_alvr_resolves_to_the_last_assignment` | table (verifier: CONFIRM_WITH_NOTE) | NEW-config_protocol_state_matrix |  | 17 | 5 | 0 | Give the row a label that names the reverse order (`shadowed-oxrsys-then-alvr`) because it is the second of the two assignment-order fixtures r1 A3b-1 asked for, and leave T0541 with its `A3b-1 regression` doc label as a standalone function. |
| T0546 `no_client_connections_key_is_clean` | table (verifier: CONFIRM_WITH_NOTE) | NEW-session_json_shape_matrix |  | 9 | 3 | 0 | Pin the Clean Pass message "ALVR session state has no stale manual-IP pins" (config.rs:314) in this row so the table can assert message uniformly on every row instead of needing an Option column for the two Warn rows. |
| T0547 `empty_manual_ips_is_clean` | table (verifier: CONFIRM_WITH_NOTE) | NEW-session_json_shape_matrix |  | 13 | 4 | 1: replace match guard json_falsy(v) with false in inspect_sess | Keep this row's input bytes exactly as written (present-but-empty `manual_ips`) — it is the only case pinning json_falsy's empty-list branch at config.rs:228, and collapsing it into T0546's `{}` would silently drop that path. |
| T0550 `non_object_top_level_is_corrupt` | table (verifier: CONFIRM_WITH_NOTE) | NEW-session_json_shape_matrix |  | 14 | 4 | 0 | The row must assert the full `could not inspect <path> (broken python3?)` string, not a starts_with prefix: config.rs:731 is the only test-side copy of that doctor.sh:209 warn text anywhere in the tree (sabrage-parity pins none of it, PARITY.md:18 only declares the divergence). |
| T0551 `non_dict_client_connections_entry_is_corrupt` | table (verifier: CONFIRM_WITH_NOTE) | NEW-session_json_shape_matrix |  | 15 | 3 | 0 | Keep the non-object-*entry* JSON as its own labelled row (it is the only input reaching config.rs:271) and interpolate the per-row session.json path into the expected message uniformly for every row, so no row needs a branch (3.3). |

### `sabrage-core:checks/game.rs` — 2 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0553 `no_bottle_no_override_skips_both_slugs_with_the_verbatim_reason` | drop_assertion | no_bottle_no_override_skips_both_slugs_with_the_verbatim_reason |  | 12 | 0 | 1: replace section_skipped -> bool with false | Delete only game.rs:119 (and the blank line it leaves), keeping :121-127 verbatim so SECTION_SKIP_REASON's bytes (game.rs:34) stay pinned; the carrier chain terminates here, in a keep of this same test. |
| T0554 `bs_dir_override_without_a_bottle_still_runs_the_section` | drop_assertion | bs_dir_override_without_a_bottle_still_runs_the_section |  | 24 | 0 | 0 | Delete only game.rs:138 (and the blank line at :139), keeping the Fail/message/remedy trio at :140-150 and the game_version Skipped at :152-153 — those are the behaviour, and section_skipped keeps its production callers at game.rs:41 and :66. |

### `sabrage-core:checks/headset.rs` — 4 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0562 `defs_binds_both_slugs_in_contract_order` | delete | registry_binds_in_contract_order_and_covers_every_slug (`sabrage-core:checks/mod.rs`) |  | 4 | 0 | 0 | Land this with the other seven H3-4 defs() mirrors in one commit and name mod.rs:615-621 plus sabrage-parity's strict_registry_builds_and_covers_the_contract_in_order as the carriers in the message, since §3.4(b) requires the commit to name a carrier per fact. |
| T0557 `first_connected_serial_skips_the_header_and_non_device_states` | table (verifier: CONFIRM_WITH_NOTE) | NEW-headset-first-connected-serial-cases |  | 4 | 9 | 0 | Build the table only from headset.rs:135-155 — it must not absorb run_only.rs:559-565, which tests a second, separately-defined first_connected_serial (headset.rs:45 vs run_only.rs:199), and budget ~7 lines of scaffold on whichever sibling row creates the carrier, so the group's real saving is ~5 li |
| T0558 `first_connected_serial_none_when_nothing_qualifies` | table | NEW-headset-first-connected-serial-cases |  | 8 | 4 | 0 | Build the table as &[(label, input, expected: Option<&str>)] and assert with first_connected_serial(input).as_deref() so T0558's Option<String>-vs-None comparisons and T0557/T0559's .as_deref()-vs-Some comparisons collapse into one branchless row shape. |
| T0559 `first_connected_serial_takes_the_first_qualifying_row` | table | NEW-headset-first-connected-serial-cases |  | 4 | 2 | 0 | Keep the 'first qualifying row wins' wording in this row's label — it is the only place the ordering semantics of first_connected_serial is stated once the three functions collapse. |

### `sabrage-core:checks/host.rs` — 2 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0564 `malformed_json_fails_with_the_parse_remedy` | table (verifier: CONFIRM_WITH_NOTE) | NEW-host-invalid-manifest-cases |  | 21 | 5 | 0 | Give each row its own scratch tag (derive it from the row label) so the two rows do not share one temp directory, and keep the exact-message and exact-remedy assertions per row rather than hoisting them to a shared literal. |
| T0565 `missing_runtime_key_is_a_parse_failure` | table (verifier: CONFIRM_WITH_NOTE) | NEW-host-invalid-manifest-cases |  | 11 | 2 | 0 | The row must keep an input literal that is valid JSON but missing the runtime key (b"{}") distinct from the malformed-bytes row, because those two exercise different early-returns inside host_manifest_library_path (serde parse vs. the .get("runtime")? hop) even though they converge on one outcome. |

### `sabrage-core:checks/meta.rs` — 1 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0572 `compiled_hash_matches_the_live_checkout` | delete (verifier: CONFIRM_WITH_NOTE) | compiled_contract_sha256_matches_the_checkout_it_was_built_from (`sabrage-core:contract.rs`) | r1:A1-1 | 9 | 0 | 0 | checks/meta.rs:280-284 asserts only contract_hash(repo_root()) == *COMPILED_CONTRACT_SHA256, character-identical to contract.rs:375 inside T0005 (which additionally pins .len()==64 at :376), so the carrier grows by 0 lines and the action is a plain deletion, not a merge; no r1:A1-1 label moves to co |

### `sabrage-core:checks/mod.rs` — 1 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0577 `unknown_and_duplicate_registrations_are_errors_even_leniently` | drop_assertion (verifier: CONFIRM_WITH_NOTE) | unknown_and_duplicate_registrations_are_errors_even_leniently |  | 11 | 0 | 2: replace Registry::unbound -> Vec<&'static str> with vec![""]; replace Registry::unbound -> Vec<&'static str> with vec!["xy | Dropping `registry().unbound()` leaves `Registry::unbound` (checks/mod.rs:416-422) with zero remaining callers anywhere in the workspace, so delete that method in the same commit (it is inside the 08-31 H2-3 'dead and test-only API tail' item) and rewrite the now-false inline comment at mod.rs:624-6 |

### `sabrage-core:checks/network.rs` — 2 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0588 `defs_binds_both_slugs_in_contract_order` | delete | registry_binds_in_contract_order_and_covers_every_slug (`sabrage-core:checks/mod.rs`) |  | 4 | 0 | 0 | Delete it together with T0562 and the other six defs() mirrors so the `cargo test -p sabrage-core -- --list` diff H3-4's acceptance asks for shows exactly the eight intended names gone. |
| T0585 `adb_forward_local_specs_reports_spawn_failure_as_err` | merge (verifier: CONFIRM_WITH_NOTE) | net_adb_forwards_warns_not_passes_when_the_probe_cannot_spawn_adb |  | 8 | 1 | 0 | In the same commit rewrite the two comments that back-reference the deleted test — network.rs:282-283 ("distinctly from the spawn-failure branch above") and network.rs:286 ("the spawn-failure test covers the Result plumbing") — to point at net_adb_forwards_warns_not_passes_when_the_probe_cannot_spaw |

### `sabrage-core:checks/overlay.rs` — 2 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0589 `without_crossover_all_four_are_skipped` | drop_assertion (verifier: CONFIRM_WITH_NOTE) | without_crossover_all_four_are_skipped |  | 21 | 0 | 0 | Delete the now-unused #[cfg(test)] fn crossover_absent at overlay.rs:30-33 together with its doc comment at overlay.rs:23-29 (whose last sentence, 'this standalone predicate exists only so the tests can assert the precondition explicitly', becomes false the moment the assertion goes), otherwise the  |
| T0592 `stale_overlay_remedy_uses_the_name_placeholder_without_a_bottle` | drop_assertion (verifier: MISSED_DUP) | stale_overlay_fails_with_install_remedy |  | 17 | 0 | 0 | Apply only the one-line trim — delete `assert_eq!(o.status, CheckStatus::Fail);` at overlay.rs:206 (implied by the remedy assert, since only CheckOutcome::fail sets a remedy) and keep the rest of T0592 including the comment at overlay.rs:204. |

### `sabrage-core:checks/run_only.rs` — 4 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0608 `the_devices_probe_still_returns_stdout_within_the_deadline` | delete (verifier: CONFIRM_WITH_NOTE) | wired_adb_passes_with_the_serial_as_detail |  | 18 | 0 | 0 | Delete only while T0607 stays exactly as it is — it is the A7-4 label home and, after this cut, T0605/T0606 are the sole exercise of the healthy read path and of the production ADB_PROBE_TIMEOUT constant. |
| T0609 `first_connected_serial_skips_the_header_and_non_device_states` | delete (verifier: CONFIRM_WITH_NOTE) | wired_adb_passes_with_the_serial_as_detail |  | 7 | 0 | 0 | Delete only run_only.rs's copy — the identically named test in checks/headset.rs (T0557) covers headset's *separate* duplicate parser (run_only.rs:143-145 says the probe is 'duplicated rather than shared') and must not be swept along or merged with it. |
| T0610 `defs_cover_exactly_the_contracts_run_only_group` | delete (verifier: CONFIRM_WITH_NOTE) | strict_registry_builds_and_covers_the_contract_in_order (`sabrage-parity:lib.rs`) |  | 10 | 0 | 0 | Land this only together with H4-2's comment edit retargeted: the 08-31 program proposes a `checks/run_only.rs:1` header citing `tests::defs_cover_exactly_the_contracts_run_only_group` (sabrage/docs/reviews/2026-08-31-simplification.md:514), which this deletion turns into a dangling pointer — repoint |
| T0603 `wired_adb_fails_verbatim_when_adb_is_absent` | merge | native_run_only_die_text_is_verbatim_in_run_sh (`sabrage-parity:lib.rs`) |  | 17 | 4 | 0 | The merge is only complete once the exact `assert_eq!(wired.message, "--wired needs adb (Android platform-tools) on PATH or under ~/Library/Android/sdk")` actually lands next to sabrage-parity/src/lib.rs:1954 — assert_verbatim alone is a substring test and would not pin which die fired. |

### `sabrage-core:checks/system.rs` — 1 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0619 `defs_binds_all_four_slugs_in_contract_order` | delete | registry_binds_in_contract_order_and_covers_every_slug (`sabrage-core:checks/mod.rs`) |  | 7 | 0 | 0 | Delete T0619 only in the same commit that keeps T0576 (checks/mod.rs:615-621) intact, since T0576's registry() call is the sole remaining pin that system::defs() binds all four slugs. |

### `sabrage-core:checks/toolchain.rs` — 1 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0623 `defs_binds_all_six_slugs_in_contract_order` | delete | registry_binds_in_contract_order_and_covers_every_slug (`sabrage-core:checks/mod.rs`) |  | 14 | 0 | 0 | Delete T0623 only in the same commit that keeps T0576 (checks/mod.rs:615-621) intact, since T0576's registry() call is the sole remaining pin that toolchain::defs() binds all six slugs. |

### `sabrage-core:config/runtime_toml.rs` — 11 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0205 `an_empty_patch_is_the_identity_on_the_deployed_file` | merge (verifier: CONFIRM_WITH_NOTE) | an_empty_patch_is_the_identity_on_text_toml_edit_would_normalise |  | 7 | 3 | 0 | Rename T0208 and rewrite its doc comment when the deployed row lands — `an_empty_patch_is_the_identity_on_text_toml_edit_would_normalise` (runtime_toml.rs:1805-1811) becomes a false name once a plain LF fixture joins it — and add `assert!(out.shadowed.is_empty())` as a common assertion, which T0208  |
| T0206 `the_shared_template_round_trips_unchanged` | merge (verifier: CONFIRM_WITH_NOTE) | an_empty_patch_is_the_identity_on_text_toml_edit_would_normalise |  | 5 | 1 | 0 | Label the new row `shared-template` so a failure still names the template case, and keep parity T0859 (sabrage-parity/src/lib.rs:1196) as the pin of the template's actual bytes — the merged row only pins round-trip identity. |
| T0264 `the_live_session_refusal_reads_as_prose` | merge (verifier: CONFIRM_WITH_NOTE) | write_refuses_while_a_session_is_live_and_touches_nothing |  | 14 | 2 | 0 | Do the merge, but also strengthen T0262's runtime_toml.rs:3266 from `err.contains("./demo.sh stop")` to `err.contains("./demo.sh stop --bottle beatsaber")`, or the bottle-substitution branch of `live_session_refusal` survives only as the `<name>` fallback in T0263. |
| T0217 `a_missing_streaming_table_is_created_at_the_end_with_one_blank_line` | table | NEW-V01-streaming-table-creation |  | 8 | 5 | 0 | The two row labels must carry the behaviour statements the function names carry today — `blank line before an appended [streaming] table` and `no leading blank line on an empty document` — or the table loses what the names were pinning. |
| T0218 `an_empty_document_gains_no_leading_blank_line` | table | NEW-V01-streaming-table-creation |  | 4 | 2 | 1: replace > with >= in ByteShape::of | Count the table scaffold once, on the T0217 row; this row adds only its `(label, input, expected)` tuple. |
| T0251 `an_unchanged_patch_writes_nothing_and_takes_no_backup` | table (verifier: CONFIRM_WITH_NOTE) | NEW-V01-write-noop |  | 23 | 24 | 0 | When building NEW-V01-write-noop, keep this row's `!report.created_from_template` in the shared assertion block (T0252 never asserted it) and give each row its own `scratch(...)` dir, and note the ~22-line table scaffold the reviewer's 4/6 line counts never charged to either row. |
| T0252 `a_no_op_write_leaves_an_unnormalised_file_alone` | table (verifier: CONFIRM_WITH_NOTE) | NEW-V01-write-noop |  | 34 | 6 | 1: replace append_tap -> std::io::Result<()> with Ok(()) | Carry the bare `/// Regression:` block at runtime_toml.rs:2938-2943 onto NEW-V01-write-noop and label its two unnormalised rows (e.g. "unnormalised file, empty patch" / "…, same protocol") so the regression fact keeps a named home after the table-drive. |
| T0216 `an_absent_key_is_inserted_under_streaming_after_its_last_key` | drop_assertion | an_empty_patch_is_the_identity_on_text_toml_edit_would_normalise |  | 32 | 0 | 1: replace match guard !REFRESH_RATES.contains(&hz) with true i | Land this in the same commit as the T0205/T0206 merge: dropping lines 2006-2015 leans on the empty-patch-on-deployed fact surviving at runtime_toml.rs:1782 (or on T0208's new `deployed` row), so the two edits must not be sequenced apart. |
| T0225 `a_parse_failure_refuses_to_rewrite` | drop_assertion | a_parse_failure_refuses_to_rewrite |  | 9 | 0 | 0 | Delete only runtime_toml.rs:2145-2148; the `refusing to rewrite` assertion at :2150 is the module's only apply_patch-level pin of the unparseable-file refusal and must stay. |
| T0235 `a_key_inside_a_multiline_string_reads_live_and_refuses_the_write` | drop_assertion (verifier: DOWNGRADE) | the_multiline_shadow_is_refused_by_apply_patch_write_and_edit_protocol | r1:A10-3 | 21 | 0 | 0 | Verdict stands, but no test carries the dropped assertion: the fixture-validity guard at sabrage/crates/sabrage-core/src/config/runtime_toml.rs:2407-2410 is a toml_edit parse check that 3.7 forbids outright, and T0236 (runtime_toml.rs:2422-2440) never parses that literal — naming T0236 as carrier wo |
| T0254 `backups_are_pruned_to_the_newest_ten` | drop_assertion (verifier: MISSED_DUP) | list_backups_sorts_newest_first_and_ignores_strangers |  | 28 | 0 | 1: replace match guard is_already_exists(&e) && attempt + 1 < L | Drop only runtime_toml.rs:3017-3020 and trim the comment on runtime_toml.rs:3012 to "The three oldest went." — its "the new one is newest" clause is backed by no other assertion in the test once the ordering assert goes. |

### `sabrage-core:contract.rs` — 3 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0006 `compiled_contract_sha256_differs_from_another_checkouts_contract` | delete (verifier: CONFIRM_WITH_NOTE) | fails_when_the_binary_was_compiled_from_a_different_contract (`sabrage-core:checks/meta.rs`) | r1:A1-1 | 29 | 0 | 0 | Name T0571 (checks/meta.rs:216-273), T0001 contract.rs:315 and sabrage-contract-gen/src/lib.rs:469-489 as the three carriers in the deletion commit, since the round-1 A1-1 sketch explicitly asked for the mutated-scalar fixture this test provides. |
| T0001 `contract_parses_and_include_path_resolves` | drop_assertion | legacy_reverse_ports_are_the_explicit_four_element_list (`sabrage-parity:lib.rs`) |  | 19 | 0 | 0 | Drop only contract.rs:316 and contract.rs:320; leave the other nine assertions (host XR path, stream ports, dashboard address, DXMT cardinality) exactly as they are. |
| T0002 `check_slugs_are_unique_and_ordered_meta_first` | drop_assertion (verifier: CONFIRM_WITH_NOTE) | slugs_are_unique (`sabrage-parity:lib.rs`) |  | 8 | 0 | 0 | Delete lines 326-329 and rename the survivor (the name check_slugs_are_unique_and_ordered_meta_first becomes false the moment the uniqueness loop goes — e.g. meta_contract_sync_is_the_first_compiled_check). |

### `sabrage-core:error.rs` — 1 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0009 `payload_carries_kind_message_remedy_and_the_reported_flag` | delete (verifier: CONFIRM_WITH_NOTE) | already_reported_covers_the_variants_that_emit_their_own_row |  | 20 | 0 | 0 | Delete this test only in the same commit that removes ErrorPayload (error.rs:206-217), SabrageError::payload (error.rs:160-166) and the lib.rs:84 re-export — if the DTO stays, the camelCase wire spelling loses its only assertion and the verdict becomes keep. |

### `sabrage-core:executor.rs` — 3 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0018 `a_byte_identical_destination_with_a_lost_execute_bit_is_repaired` | drop_assertion (verifier: CONFIRM_WITH_NOTE) | copy_if_changed_matches_install_if_changed |  | 46 | 0 | 0 | Keep both copy_if_changed CALLS at executor.rs:1362 and :1367 (drop only the assert_eq! wrappers) — the first creates dst for the 0o755 assertion at :1366 and the second must run before the mode is drifted at :1373. |
| T0022 `create_new_publishes_finished_bytes_and_leaves_no_temp` | drop_assertion (verifier: CONFIRM_WITH_NOTE) | create_new_never_clobbers_an_existing_file | r2:A2-4 | 38 | 0 | 0 | Drop executor.rs:1487 only, and while editing the doc comment ADD the missing `/// r2:A2-4 regression: …` label — it is absent today, so 'keep the label' means writing it for the first time. |
| T0024 `write_atomic_replaces_and_leaves_no_temp_files` | drop_assertion (verifier: CONFIRM_WITH_NOTE) | write_atomic_keeps_an_existing_files_mode |  | 17 | 0 | 0 | Drop executor.rs:1552 and rename to write_atomic_leaves_no_temp_files as the reviewer says; keep both write_atomic calls so the survivor still exercises a replacement, and be aware T0020 becomes the only killer of a no-op write_atomic mutant. |

### `sabrage-core:fixes/adb.rs` — 4 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0128 `stale_local_specs_come_from_the_contract_ports` | delete (verifier: CONFIRM_WITH_NOTE) | removes_exactly_the_two_stale_ports_per_serial_never_remove_all |  | 4 | 0 | 0 | Delete it, but name `an_unspawnable_adb_warns_once_and_never_reports_a_clean_table` (adb.rs:638-663) — not only T0131 — as the carrier in the commit message, and repoint the planned adb.rs:1 header replacement at 2026-08-31-simplification.md:513, which still cites this test by name. |
| T0126 `parse_forward_list_splits_serial_and_local` | table (verifier: CONFIRM_WITH_NOTE) | NEW-adb-forward-parse-cases |  | 12 | 14 | 0 | Land it as §3.3 style, not as a line saving: the table scaffold I measure (~11 lines) plus this row (~3) is ~14 against the 13 the test costs today, so T0126+T0127 together net roughly -2 lines, not the -14 the reviewer's 8+6 implies. |
| T0127 `parse_forward_list_ignores_blank_and_short_lines` | table | NEW-adb-forward-parse-cases |  | 8 | 6 | 0 | Give the short-line case its own row label naming the skip (`short line skipped, next row still parses`) so a mutation of the `fields.next()?` guard at adb.rs:71-73 reports that row, not just the table fn. |
| T0132 `dry_run_reports_would_clear_and_still_invokes_the_planned_spawn` | drop_assertion | dry_run_reports_would_clear_and_still_invokes_the_planned_spawn |  | 18 | 0 | 0 | Drop only adb.rs:488 and leave the three behavioural assertions untouched — the `would clear` prefix at adb.rs:492-494 is the tree's only pin of the dry-run verb branch at adb.rs:220. |

### `sabrage-core:fixes/backend.rs` — 15 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0160 `for_launch_is_a_noop_when_already_current` | delete (verifier: CONFIRM_WITH_NOTE) | set_graphics_backend_is_a_noop_when_already_current |  | 17 | 0 | 0 | Delete T0160 only if T0159 (backend.rs:738-755) survives the same pass, since it becomes the sole pin that the launch door reaches the shared rewrite body at all. |
| T0161 `for_launch_still_requires_a_bottle` | delete (verifier: CONFIRM_WITH_NOTE) | set_graphics_backend_requires_a_bottle |  | 16 | 0 | 0 | Safe to delete; the shell die text 'CrossOver bottle name required' keeps two independent homes (backend.rs:699-701 and stages/mod.rs's require_bottle_reproduces_lib_sh_die_text), so nothing verbatim is lost. |
| T0140 `branch_rewrites_an_existing_key_line_in_place` | table (verifier: CONFIRM_WITH_NOTE) | NEW-backend-rewrite-cases |  | 11 | 18 | 0 | Land H2-3 first and give the table columns (label, input, expected_out, expected_anchor) with no Branch column — H2-3 deletes the Branch enum (2026-08-31-simplification.md:80, :85), and a Branch column would reintroduce exactly what it removes. |
| T0141 `branch_rewrite_handles_an_empty_existing_value` | table (verifier: CONFIRM_WITH_NOTE) | NEW-backend-rewrite-cases |  | 7 | 6 | 0 | Label the row for the fact, not the shape — `empty existing value is still rewritten` — so a mutation of the quoted-value match at backend.rs:92-101 names this case; and expect ~6 added lines, not 3. |
| T0142 `branch_rewrite_preserves_absence_of_a_trailing_newline` | table (verifier: CONFIRM_WITH_NOTE) | NEW-backend-rewrite-cases |  | 7 | 6 | 0 | Set this row's `expected_anchor` column to true, not false — the newline-less output's single line is exactly TARGET_LINE (backend.rs:30), so a false there would be a fabricated assertion the loser never made. |
| T0143 `branch_rewrite_leaves_a_malformed_key_line_untouched_like_sed_would` | table (verifier: CONFIRM_WITH_NOTE) | NEW-backend-rewrite-cases |  | 11 | 6 | 0 | In the same commit, repoint the production comment at sabrage/crates/sabrage-core/src/fixes/backend.rs:318, which names this test function verbatim, at the new table row label — otherwise the table-drive turns a load-bearing pointer into a dangling name. |
| T0144 `branch_inserts_immediately_after_the_environment_variables_header` | table (verifier: CONFIRM_WITH_NOTE) | NEW-backend-rewrite-cases |  | 10 | 6 | 0 | Write the row as (label, input, expected, anchor) with NO Branch column and land it after wave-1 H2-3, which deletes the Branch return (2026-08-31-simplification.md:80 warns V2-7 must not reintroduce it); budget ~6 lines per row, not 3, because the byte-exact literals force rustfmt to one element pe |
| T0145 `branch_insert_preserves_absence_of_a_trailing_newline` | table (verifier: CONFIRM_WITH_NOTE) | NEW-backend-rewrite-cases |  | 30 | 8 | 0 | Land this after wave-2's comment pass and carry its surviving one-line BSD-sed rationale plus both assert message strings into the row label, and re-point the production doc at backend.rs:85, which names this test function by name and would otherwise become a dangling pointer. |
| T0146 `branch_appends_a_new_section_when_neither_exists` | table (verifier: CONFIRM_WITH_NOTE) | NEW-backend-rewrite-cases |  | 10 | 6 | 0 | Same as its siblings: (label, input, expected, anchor) with no Branch column after H2-3, and keep the blank line before [EnvironmentVariables] in the expected literal byte-for-byte since that is the shell's printf shape. |
| T0147 `branch_append_does_not_care_whether_the_original_had_a_trailing_newline` | table (verifier: CONFIRM_WITH_NOTE) | NEW-backend-rewrite-cases |  | 9 | 6 | 0 | This is the only sibling with no anchor assertion today, so give its row anchor=true (verified: the expected output contains TARGET_LINE verbatim) and fold its '// no trailing newline' clarifier into the row label. |
| T0148 `wineservers_indicate_live_matches_by_exact_wineprefix` | table (verifier: CONFIRM_WITH_NOTE) | NEW-backend-wineprefix-cases |  | 5 | 8 | 0 | Name the new table function so it still matches `wineservers_indicate_live_*` (e.g. wineservers_indicate_live_decides_by_wineprefix), because wave-2's programmed replacement comment for backend.rs:186 points at that glob and wave-2 acceptance requires every named test symbol to resolve. |
| T0149 `wineservers_indicate_live_refuses_when_a_match_cannot_be_ruled_out` | table | NEW-backend-wineprefix-cases |  | 11 | 2 | 0 | Keep both scenarios as two separately labelled rows (unknown alone, different-plus-unknown) so a mutant that only handles the single-None case still names the row it broke. |
| T0150 `wineservers_indicate_live_is_false_when_nothing_is_running` | table | NEW-backend-wineprefix-cases |  | 3 | 1 | 0 | Type the table's observation column as &[Option<&str>] so the empty-observation row can be written as &[] without a turbofish, then map to Vec<Option<String>> once inside the loop. |
| T0151 `wineservers_indicate_live_is_false_when_every_match_is_a_different_bottle` | table | NEW-backend-wineprefix-cases |  | 7 | 2 | 0 | Keep two distinct known-different prefixes in the row literal rather than collapsing to one, since the multi-element case is what separates this row from T0148's single-observation false row. |
| T0156 `set_graphics_backend_under_dry_run_does_not_touch_the_file` | drop_assertion | set_graphics_backend_under_dry_run_does_not_touch_the_file |  | 18 | 0 | 0 | Delete exactly backend.rs:673 and nothing else — the cxbottle byte assertion at 678-682 is the last core-side pin of the untouched-file bytes on the dry-run path. |

### `sabrage-core:fixes/mod.rs` — 3 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0175 `a_live_session_stops_the_adb_forward_removal_before_it_spawns_adb` | merge (verifier: CONFIRM_WITH_NOTE) | apply_refuses_every_session_forbidden_fix_while_a_session_is_live | r1:A4-1 | 37 | 21 | 0 | Merge is sound but costs 21 lines on T0173, not 14: move mod.rs:689-707 (the fake adb, minus the now-redundant create_dir_all) plus the 4-line !log.exists() assertion, change T0173's `let ctx` to `let mut ctx`, and rename/extend its doc comment so the before-it-spawns-adb ordering stays a named beha |
| T0171 `apply_waits_for_the_operation_lock_then_proceeds` | drop_assertion | apply_waits_for_the_operation_lock_then_proceeds |  | 33 | 0 | 2: replace >= with < in detach_with; delete ! in reap | Delete exactly mod.rs:552-555 and keep the 250 ms timeout assertion at 557-562 — it is the sole pin that apply serializes on the operation lock; the identical fixture assertion at mod.rs:670 in apply_holding_lock_is_not_gated_by_a_live_session is the same defect and should go in the same edit. |
| T0177 `the_known_bad_session_json_deletion_is_withheld_but_documented` | drop_assertion (verifier: CONFIRM_WITH_NOTE) | is_deferred_is_exactly_the_withheld_set | r1:A4-2 | 30 | 0 | 1: replace == with != in FixAction::def | Drop only mod.rs:752-757 and, in the same edit, add `/// r1:A4-2 regression: a known-broken destructive remedy renders no Fix button` to T0185 and `/// r1:A12-1 regression: ...` to T0177 — neither label exists today, so 'retaining' them is not a no-op. |

### `sabrage-core:fixes/session_json.rs` — 3 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0187 `missing_file_is_a_noop` | drop_assertion | missing_file_is_a_noop |  | 12 | 0 | 0 | Delete exactly session_json.rs:214; keep the unwrap at 217, which is the only pin that the missing-file branch returns Ok rather than an io error. |
| T0189 `dry_run_neither_backs_up_nor_deletes` | drop_assertion | dry_run_neither_backs_up_nor_deletes |  | 41 | 0 | 0 | Delete exactly session_json.rs:278 and leave the !backups_dir(&ctx).exists() assertion at 289-292 alone — it looks like the same kind of fixture fact but is a genuine observation, since the non-dry-run path does create that directory (session_json.rs:246-252). |
| T0191 `a_preview_executor_beats_opts_dry_run_false` | drop_assertion (verifier: CONFIRM_WITH_NOTE) | a_preview_executor_beats_opts_dry_run_false |  | 50 | 0 | 0 | When dropping the assertion, make the premise literal instead — construct the options as `StageOptions { dry_run: false, ..Default::default() }` at session_json.rs:374 — so the test cannot silently go vacuous if `StageOptions`'s derived `Default` is ever replaced; keep all five remaining assertions  |

### `sabrage-core:logs.rs` — 5 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0038 `attempt_zero_matches_the_shells_date_stamp` | delete | wine_log_candidate_matches_run_shs_date_stamp_and_the_dash_two_collision_suffix (`sabrage-parity:lib.rs`) |  | 7 | 0 | 0 | Delete logs.rs:743-750 as proposed; the run.sh date-stamp byte-fact then lives only in parity T0874, which both CI (.github/workflows/parity.yml) and tier 1 (TIER1_PKGS, scripts/dev/parity.sh:75) run. |
| T0039 `collisions_get_a_dash_n_plus_one_suffix` | merge | wine_log_candidate_matches_run_shs_date_stamp_and_the_dash_two_collision_suffix (`sabrage-parity:lib.rs`) |  | 13 | 2 | 0 | Add the attempt-3 `-4.log` assertion to parity lib.rs after line 1836 before deleting logs.rs:752-765, since that generalisation is the only fact the parity carrier does not already hold. |
| T0041 `wine_log_candidate_stamped_takes_no_date_time_type_at_all` | merge (verifier: MISSED_DUP) | wine_log_candidate_delegates_to_the_stamped_form_byte_for_byte |  | 12 | 6 | 0 | Apply this as the real fold the carrier_note describes, not as a bare delete: move T0041's two absolute literals into T0040 (turn `for attempt in [0, 1, 3]` at logs.rs:776 into a labelled `(attempt, expected_full_path)` table asserting stamped == literal AND wrapper == stamped, ~+6 lines, not the ro |
| T0061 `truncate_and_regrow_past_the_cursor_between_polls_reports_rotation` | table (verifier: CONFIRM_WITH_NOTE) | truncate_and_regrow_past_the_cursor_between_polls_reports_rotation | r1:A8-7 | 48 | 0 | 1: replace - with + in Tailer::open | Rewrite the `match regrow` at logs.rs:1235-1242 as a [(label, line_count)] table row pair and leave the doc label bare `A8-7` — §3.6 says existing bare labels are not rewritten, so do not restyle it to `r1:A8-7`. |
| T0048 `resolve_source_wine_console_prefers_the_live_session_over_everything` | drop_assertion (verifier: CONFIRM_WITH_NOTE) | the_live_slot_is_set_and_cleared_by_run_id (`sabrage-core:session/mod.rs`) |  | 39 | 0 | 0 | Drop only logs.rs:960 and also drop `live_session` from the `use crate::session::{…}` list at logs.rs:929-931, which becomes unused; keep the `clear_live_session(run_id)` call at :959 as cleanup. |

### `sabrage-core:paths.rs` — 1 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0068 `a_symlinked_root_keeps_the_shells_logical_spelling` | merge (verifier: CONFIRM_WITH_NOTE) | the_native_resolver_preserves_a_symlinked_spelling_and_folds_dotdot (`sabrage-parity:lib.rs`) | r1:A2-6 | 30 | 3 | 0 | Move BOTH paths.rs:668 (assert_ne! against the physical path) and paths.rs:670 (Paths::new(&spelled).oxr_dylib.starts_with(&link)) into T0881 and add the `/// r1:A2-6 regression:` label there — that label does not exist anywhere in the tree today, and the starts_with assertion is genuinely not impli |

### `sabrage-core:privilege.rs` — 3 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0100 `the_write_paths_bytes_are_the_file_form_not_the_comparison_form` | delete (verifier: CONFIRM_WITH_NOTE) | the_privileged_write_stages_the_file_form_of_the_host_manifest (`sabrage-parity:lib.rs`) |  | 12 | 0 | 0 | Delete privilege.rs:1860-1874 — the orphaned section header `// ── the byte source (the one artifact that must not drift) ──` at 1860 must go with the test since it is the last item in the module, and if the 08-31 H4-3 comment-cut lands its replacement text for privilege.rs:382 must stop naming `the |
| T0098 `support_dir_is_under_application_support` | drop_assertion | paths_are_derived_from_the_explicit_root (`sabrage-core:paths.rs`) |  | 8 | 0 | 2: replace sabrage_temp_dir -> PathBuf with Default::default(); replace sabrage_support_dir -> PathBuf with Default::default | Drop only privilege.rs:1817; it is doubly carried (paths.rs:757-759 and the kept privilege.rs:1818, which contains the same suffix), so the applier's commit message should name paths.rs:757-759 as the carrier and nothing else changes. |
| T0099 `admin_method_is_decided_by_the_controlling_terminal_never_by_stdout` | drop_assertion (verifier: CONFIRM_WITH_NOTE) | admin_method_is_decided_by_the_controlling_terminal_never_by_stdout |  | 33 | 0 | 1: replace \|\| with && in AdminMethod::choose | Drop privilege.rs:1851-1857 (the explanatory comment at 1851-1853 goes with the assertion), leave the module-level `use std::io::{IsTerminal, Write}` at privilege.rs:84 alone since production line 208 still uses it. |

### `sabrage-core:process.rs` — 4 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0116 `liveness_probe_agrees_with_reality` | delete (verifier: CONFIRM_WITH_NOTE) | identity_rejects_a_recycled_pid_and_the_unobservable_fallback |  | 6 | 0 | 0 | Delete it, but the commit message must name T0109 (process.rs:1250-1253) and reconcile.rs:1536 as the carriers of the negative half — the reviewer's named carrier T0120 carries only the positive half, because its dead-pid assertion is already satisfied by ProcInfo::observe returning None. |
| T0121 `proc_info_round_trips_as_camel_case_json` | delete | round_trips_through_the_file (`sabrage-core:session/state.rs`) |  | 11 | 0 | 0 | Delete it and name state.rs:588-589 as the carrier — the sample() fixture there already uses the loser's exact literals (pid 4242, start_time 1786300214), so the commit message can point at identical values. |
| T0101 `splits_on_lf_cr_and_crlf` | drop_assertion (verifier: MISSED_DUP) | chunks_carry_their_terminator |  | 13 | 0 | 0 | Drop only process.rs:954, 958 and 962 (and their inline comments at 957 and 961); keep at least the 956/960/963/964 rows so the terminator-blind push/finish delegations that logs.rs:331,499,511 actually call retain a mutant kill. |
| T0123 `cmdline_matching_is_the_pgrep_f_shape` | drop_assertion (verifier: CONFIRM_WITH_NOTE) | finds_by_cmdline_using_this_test_binarys_own_argv (`sabrage-core:stages/stop.rs`) |  | 23 | 0 | 1: replace \|\| with && in cmdline_contains | The mechanical reset to keep was wrong: T0372 (final merge -> T0123) moves its unconditional scanner assertions from crates/sabrage-core/src/stages/stop.rs:1048-1065 into process.rs, so the vacuous `if !filter.is_empty()` block at crates/sabrage-core/src/process.rs:1513-1522 is still subsumed inside |

### `sabrage-core:session/mod.rs` — 5 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0678 `the_idle_status_is_the_default` | delete | snapshot_phase_transitions (`sabrage-core:session/watcher.rs`) |  | 6 | 0 | 0 | Straight deletion of mod.rs:755-761; no assertion moves, because watcher.rs:1828-1829/1853/1877/1963/1980 already pin every default through snapshot(). |
| T0684 `live_session_run_id_agrees_with_the_full_handle_clone` | merge (verifier: CONFIRM_WITH_NOTE) | the_live_slot_is_set_and_cleared_by_run_id |  | 20 | 4 | 2: replace live_session_run_id -> Option<RunId> with Some(Defau; replace live_session_is -> bool with false | Merge into T0685 but keep the mirror line (or an equivalent assert_eq!(live_session().map(\|h\| h.run_id), Some(a)) after set_live_session) and carry mod.rs:1071-1074's rationale onto T0685 — live_session_run_id does not delegate to live_session, so that comparison is a two-implementation cross-chec |
| T0689 `the_fallback_prefers_the_built_in_speakers` | table | NEW-V06-audio-fallback |  | 34 | 9 | 0 | Legal §3.3 table — all nine rows share one call to fallback_output_device (mod.rs:648-654) and one assert_eq! with no branch; carry mod.rs:1179-1180's "observed 2026-08-29 list" provenance onto that row's label. |
| T0690 `a_real_device_beats_no_device_but_a_virtual_one_never_wins` | table | NEW-V06-audio-fallback |  | 23 | 6 | 1: replace is_virtual_output -> bool with true | Fold these four rows into the same NEW-V06-audio-fallback table and keep labels for the two policy rows ("every candidate is virtual" and "empty list"), which are the None half of the selector contract. |
| T0679 `status_serializes_camel_case_for_the_ipc_mirror` | drop_assertion (verifier: CONFIRM_WITH_NOTE) | every_phase_has_a_camel_case_wire_word |  | 32 | 0 | 0 | Drop only the serde round-trip at mod.rs:794 and keep assert_eq!(j["phase"], "stalled") — T0680 pins the phase *word* on a bare SessionPhase but nothing else pins that SessionStatus's own field serializes under the key `phase`. |

### `sabrage-core:session/reconcile.rs` — 7 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0714 `a_device_the_user_already_switched_back_is_flagged_not_switched` | merge (verifier: CONFIRM_WITH_NOTE) | each_flag_reaches_disk_as_its_guard_is_released |  | 27 | 4 | 0 | Do NOT add the reviewer's 'no child-spawn' assertion as spawns(ctx.executor.planned()) in T0719 — RealExecutor::planned() is always empty (executor.rs:234-237), so it could never fail; carry the no-switch fact with restored.is_empty() and sections().is_empty() instead. |
| T0696 `a_live_session_is_adopted_without_touching_anything` | table | NEW-V02-live-reconcile |  | 18 | 12 | 0 | Give the table a dashboard-identity column so this row keeps its `pending(Some(me()), Some(me()))` fixture — T0704's row passes None there, and collapsing both rows onto one dashboard literal would silently drop the 'a live record with a dashboard guard is still untouched' case. |
| T0704 `a_live_pid_with_no_observed_start_time_is_never_dismantled` | table (verifier: CONFIRM_WITH_NOTE) | NEW-V02-live-reconcile | r1:A9-5 | 27 | 5 | 0 | The row must be labelled `unverifiable [r1:A9-5]` verbatim (round-qualified, because r2:A9-5 is a different watcher finding), and T0705's doc comment at reconcile.rs:2036 opens with '…and `stop` says so' — its antecedent is this test, so rewrite that sentence when this fixture moves. |
| T0705 `stop_names_an_unverifiable_pid_in_its_own_words` | table (verifier: CONFIRM_WITH_NOTE) | NEW-V02-stop-live-identity |  | 33 | 12 | 0 | Keep both warning sentences as per-row literal templates with the pid interpolated (no branching on Classification inside the table, which §3.3 forbids), and rewrite this test's doc comment at reconcile.rs:2036 — its '…and `stop` says so' antecedent (T0704) is being moved away in the same program. |
| T0722 `stop_warns_and_keeps_the_record_when_the_wine_pid_survived` | table | NEW-V02-stop-live-identity |  | 26 | 5 | 0 | Label this row `live` and keep its warn sentence a verbatim per-row literal — it and T0705's sentence are the two arms of the match at reconcile.rs:610-618, and that match is exactly what the table is there to pin. |
| T0721 `the_reconcile_types_serialize_camel_case` | drop_assertion | the_reconcile_types_serialize_camel_case |  | 24 | 0 | 0 | Delete only reconcile.rs:2673 and keep reconcile.rs:2652-2672 exactly as they are — those camelCase literals are the sole Rust-side mirror of sabrage/ui/src/ipc.ts:666-677, and `j` is consumed by the dropped line, so nothing after it needs re-ordering. |
| T0727 `a_cancelled_reconcile_still_reaches_the_caller` | drop_assertion (verifier: CONFIRM_WITH_NOTE) | a_cancelled_reconcile_still_reaches_the_caller |  | 27 | 0 | 0 | Before dropping reconcile.rs:2848-2859, tighten T0726's row check (reconcile.rs:2810-2816) so it asserts the underlying error text is appended after "previous session not fully restored: " — today only the prefix is pinned — and rewrite the now-false '…while any other error is absorbed into the two  |

### `sabrage-core:session/state.rs` — 1 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0741 `a_newer_schema_is_recognised_and_never_downgraded` | delete | a_newer_schema_record_is_never_overwritten_or_deleted |  | 18 | 0 | 0 | Delete is safe as written; the commit message must name T0742 for the v2 side and T0740 (state.rs:755, 760) for the supported-v1 side, since the reviewer's row names T0740 only in prose. |

### `sabrage-core:session/watcher.rs` — 9 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0746 `runtime_status_ignores_unknown_fields_and_optional_ones` | merge (verifier: CONFIRM_WITH_NOTE) | parse_runtime_status_returns_none_for_a_half_written_file |  | 15 | 10 | 0 | Merge is right and strictly improves §3.7 compliance (the loser bypasses the module parser at watcher.rs:96-98); when renaming T0747 keep its two existing None assertions at watcher.rs:708-709 and keep the "observed file, verbatim" comment (watcher.rs:690) on the moved document. |
| T0760 `a_session_that_predates_the_monitor_keeps_its_encoder_chip` | merge (verifier: CONFIRM_WITH_NOTE) | an_adopted_session_only_inherits_lines_written_after_it_started | r1:A9-6 | 46 | 8 | 0 | Label the new row `r1:A9-6` (round prefix required — A9-6 exists in both rounds) and keep its expected value Some("HEVC") so the table holds one positive and one negative adopted-session outcome. |
| T0748 `the_staleness_budget_is_three_seconds` | table (verifier: CONFIRM_WITH_NOTE) | NEW-V06-watcher-budgets |  | 3 | 7 | 0 | Legal under 3.3 but book the real cost: the table scaffold (fn + for + assert + braces) is ~6 lines, so folding T0748+T0749 into NEW-V06-watcher-budgets nets roughly zero lines — do it for the row labels, not for the saving. |
| T0749 `the_startup_and_stall_grace_budgets` | table | NEW-V06-watcher-budgets |  | 4 | 2 | 0 | Two rows, two lines added — and the merge also fixes T0749's own 3.3 defect of bundling two unrelated budgets in one unlabelled function. |
| T0750 `is_fresh_tolerates_a_clock_skewed_slightly_into_the_future` | table (verifier: CONFIRM_WITH_NOTE) | NEW-V06-freshness-boundaries |  | 6 | 11 | 0 | All four rows must survive including the two behind-the-budget ones (3_000 / 3_001), because T0751 tests only the ahead direction and nothing else in the tree calls is_fresh. |
| T0751 `a_stamp_far_in_the_future_is_wrong_not_fresh` | table (verifier: CONFIRM_WITH_NOTE) | NEW-V06-freshness-boundaries | r1:A9-7 | 17 | 6 | 0 | Label the hour-ahead row `r1:A9-7` (the id collides across rounds, so the bare form is not allowed) and keep the allowance row expressed as MAX_FUTURE_SKEW rather than a flattened 2_000 literal. |
| T0754 `parse_encoder_ready_reads_the_hevc_native_helper_form_from_the_fixture` | table | NEW-H3-2 |  | 14 | 15 | 0 | Consistent with the accepted H3-2 item, whose whole remaining scope is watcher.rs:818-873 at -12 lines; keep parse_encoder_ready_ignores_unrelated_fixture_lines (watcher.rs:850+) out of the table since it has different setup and assertion shape. |
| T0755 `parse_encoder_ready_reads_the_h264_in_process_downgrade_form_from_the_fixture` | table | NEW-H3-2 |  | 14 | 3 | 0 | The row label must name the downgrade form explicitly (e.g. "the H.264 in-process downgrade"), since the H3-2 acceptance requires a deliberate one-case mutation to report the row label rather than the table function. |
| T0767 `snapshot_phase_precedence_table` | drop_assertion (verifier: CONFIRM_WITH_NOTE) | snapshot_phase_transitions |  | 124 | 0 | 6: replace home_dir_checked -> Result<PathBuf, SabrageError> wi; replace owned_elsewhere_row -> String with "xyzzy".into(); delete match arm SessionPhase::Stopping in SessionMonitor::s … | Delete exactly three rows — 'published Preflight beats the Idle fallthrough' (watcher.rs:1590-1596), 'published Exited beats the Idle fallthrough' (:1597-1603, carried by snapshot_identity_and_exit_code_sources:1685, NOT by T0769) and 'nothing at all is Idle' (:1604-1610) — and leave rows 6 and 7 al |

### `sabrage-core:stages/build.rs` — 7 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0288 `the_x64_configure_forces_the_encoder_helper_option_off` | merge (verifier: CONFIRM_WITH_NOTE) | the_x64_configure_spec_renders_the_helper_off_flag | r1:A5-2 | 18 | 1 | 2: replace oxrsys_x64_configure_args -> Vec<&'static str> with ; replace oxrsys_x64_configure_args -> Vec<&'static str> with | Before deleting T0288, upgrade T0289's `ends_with` (build.rs:1025-1028) to a full-string `assert_eq!` on `spec.display()` — or move T0288's exact six-element `assert_eq!` into T0289 — and ADD the `/// r1:A5-2 regression:` label, which neither test carries today. |
| T0275 `parse_ninja_progress_matches_the_default_status_prefix` | table (verifier: CONFIRM_WITH_NOTE) | NEW-v08-ninja-progress-cases |  | 30 | 20 | 2: replace parse_ninja_progress -> Option<(u64, u64)> with None; replace parse_ninja_progress -> Option<(u64, u64)> with Some | Legal §3.3 table (one pure parser, eleven literal-only cases, zero branches), but measure it as ~20 carrier lines against 31 today (net ~-11), not the reviewer's 5, and turn the three explanatory comments at build.rs:703-704, 709-710 and 722 into row labels rather than dropping them. |
| T0278 `missing_tool_message_is_verbatim` | table (verifier: CONFIRM_WITH_NOTE) | NEW-v08-missing-tool-message-cases |  | 14 | 11 | 2: replace missing_tool_message -> String with "xyzzy".into(); replace missing_tool_message -> String with String::new() | Safe as a table, but T0278 is the only place in the tree that pins the literal bytes `… missing — brew install cmake ninja mingw-w64` (T0277 at build.rs:761-786 compares against `missing_tool_message(..)`, not the string), so copy all three expected strings verbatim — em dash included — into the row |
| T0279 `fixed_die_texts_are_verbatim` | table (verifier: CONFIRM_WITH_NOTE) | NEW-v08-fixed-build-die-texts |  | 17 | 17 | 0 | This one is a wash — 18 lines today versus ~17 as a table, because two of the three expected literals are multi-line `\`-continued strings — so do it only for the row labels and copy every byte (em dashes, the `\` continuations and their leading spaces, `https://rustup.rs`) unchanged, since this tes |
| T0280 `rustup_gate_missing_binary_dies` | table (verifier: CONFIRM_WITH_NOTE) | NEW-v08-rustup-gate |  | 11 | 11 | 0 | Encode the row as (label, fake-tool filename, script, expected) so the absent-binary case is produced by writing a differently-named tool rather than by an `if let Some(script)` branch — a branch here would be logic in a test under §3.3 — and count this row as the ~11-line scaffold, not 3. |
| T0281 `rustup_gate_missing_target_dies` | table | NEW-v08-rustup-gate |  | 12 | 2 | 0 | Apply as one row of the V2-7 rustup matrix and keep the label distinct from T0280's even though both expect RUSTUP_TARGET_MISSING_MESSAGE, since the two rows pin different paths (binary absent vs target absent). |
| T0282 `rustup_gate_passes_when_the_target_is_installed` | table | NEW-v08-rustup-gate |  | 16 | 2 | 0 | Keep both `echo` lines of the fake script in the row literal — the second-line placement of x86_64-apple-darwin is what proves the gate scans the whole output, not just the first line. |

### `sabrage-core:stages/install.rs` — 5 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0303 `dxmt_backup_present_is_reported_not_replanned` | delete (verifier: CONFIRM_WITH_NOTE) | run_dry_runs_all_four_layers_in_order_without_touching_the_machine |  | 13 | 0 | 0 | Delete it, but do not book the deletion as coverage-neutral in substance: install.rs:161-177's positive branch (a non-empty existing backup → info "stock DXMT backup already exists" and no DirCopy) has no production-driven test at all — the four surviving assertions (install.rs:1562, 1801, 1830, 186 |
| T0309 `layer_four_reports_a_skipped_write_as_already_current` | delete (verifier: CONFIRM_WITH_NOTE) | run_dry_runs_all_four_layers_in_order_without_touching_the_machine |  | 46 | 0 | 0 | Delete it, but drop the doc comment's claim with it and, if the `PrivilegedWrite::Skipped` arm at install.rs:403-405 is wanted under test, write a new test that makes the destination current *between* install.rs:383's check and privilege.rs:419's re-check (a TestExecutor hook), because T0309 never r |
| T0302 `dxmt_backup_is_planned_once_then_skipped_when_present` | merge (verifier: CONFIRM_WITH_NOTE) | a_dry_run_mutates_nothing_at_all (`sabrage-core:executor.rs`) |  | 24 | 4 | 0 | When moving the DirCopy kind/src/dst assertions into T0031 (executor.rs:1720), assert them against T0031's own `sub`→`sub-copy` paths via `ex.planned().last()` right after the call, and do not re-use install's `dxmt.stock-backup` literal — production's dir_copy destination is the `dxmt.stock-backup. |
| T0314 `a_late_system_reg_flush_is_waited_for_instead_of_warned_about` | merge (verifier: CONFIRM_WITH_NOTE) | a_stale_active_runtime_value_does_not_end_the_flush_wait | r1:A6-4 | 48 | 5 | 1: replace >= with < in wait_for_registry_flush | Before deleting install.rs:1683-1731, add `/// r1:A6-4 regression: a late system.reg flush is waited for, not warned about` to T0319 and move the 1683-1686 rationale prose onto it (the 08-31 V2-7 precaution, 2026-08-31-simplification.md:1061) — that is 5 added lines, not the 1 the reviewer booked. |
| T0301 `registry_current_requires_all_three_literals_in_order_on_one_line` | drop_assertion (verifier: CONFIRM_WITH_NOTE) | registry_current_requires_all_three_literals_in_order_on_one_line |  | 23 | 0 | 1: replace += with *= in registry_current | Drop install.rs:963-964 only, keep all three `registry_current` assertions, and confirm `system_reg_contains` stays referenced by install.rs:1957 and :2014 so the `#[cfg(test)]` helper (install.rs:529-536) does not become dead code. |

### `sabrage-core:stages/mod.rs` — 7 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0336 `live_session_block_is_none_on_a_scratch_machine` | delete (verifier: CONFIRM_WITH_NOTE) | ensure_idle_refuses_for_every_source_that_can_know_about_a_session (`sabrage-core:session/mod.rs`) |  | 7 | 0 | 0 | Delete it, but cite T0341 (stages/mod.rs:1497-1501) as the carrier rather than T0681 — T0341 pins the identical None over the identical stages alias, whereas T0681 reaches the fact through ensure_idle_at. |
| T0337 `live_session_block_sees_a_running_session_recorded_on_disk` | delete | ensure_idle_refuses_for_every_source_that_can_know_about_a_session (`sabrage-core:session/mod.rs`) |  | 9 | 0 | 0 | Safe to delete outright: write_live_session_state stays in use by T0340 at stages/mod.rs:1442, so no helper is orphaned by this row. |
| T0338 `live_session_block_sees_a_fresh_runtime_status` | delete (verifier: CONFIRM_WITH_NOTE) | ensure_idle_refuses_for_every_source_that_can_know_about_a_session (`sabrage-core:session/mod.rs`) |  | 19 | 0 | 0 | Delete the helper write_live_runtime_status (stages/mod.rs:1351-1364) in the same edit — T0338 is its only caller, so leaving it behind ships dead test code. |
| T0339 `live_session_block_sees_every_signal_the_session_layer_sees` | merge (verifier: CONFIRM_WITH_NOTE) | ensure_idle_refuses_for_every_source_that_can_know_about_a_session (`sabrage-core:session/mod.rs`) |  | 45 | 5 | 0 | Insert the malformed-record row into T0681 immediately after session/mod.rs:909 (the case-4 remove_file) and delete the file again before case 5 begins, or the unreadable record short-circuits every runtime-status assertion at 927-942. |
| T0325 `run_stage_brackets_the_stage_with_events_even_when_it_fails` | drop_assertion (verifier: CONFIRM_WITH_NOTE) | require_bottle_reproduces_lib_sh_die_text |  | 28 | 0 | 0 | When dropping the message assertion, also drop the `let err =` binding at stages/mod.rs:1033 (call `run_stage(Stage::Stop, &ctx).await.unwrap_err();` unbound) or the test stops compiling clean on an unused variable, and keep the 1030-1031 comment that explains why stop-with-no-bottle is the chosen f |
| T0327 `the_two_wineserver_budgets_stay_distinct` | drop_assertion | the_two_wineserver_budgets_stay_distinct |  | 8 | 0 | 0 | Delete line 1078 together with its explanatory comment at line 1077 ('The re-export from `stop` is the same constant'), which is false once the assertion is gone and is already stated at stop.rs:161. |
| T0335 `the_test_lock_file_is_not_in_the_user_support_directory` | drop_assertion (verifier: CONFIRM_WITH_NOTE) | the_test_lock_file_is_not_in_the_user_support_directory |  | 14 | 0 | 0 | Confirm the drop, but record it as a change-detector removal (§3.4), not a subsumption: no other test pins the cfg(test) lock filename, and `"operation.lock"` is Sabrage-only (PARITY.md: demo.sh does not take this lock), so no golden byte is at stake. |

### `sabrage-core:stages/run/actions.rs` — 7 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0398 `the_wired_ports_come_from_the_contract` | delete | wired_plans_both_forwards_and_reports_them |  | 6 | 0 | 0 | Straight deletion of actions.rs:991-997; T0403 keeps both port literals through the spawned argv, so no port byte goes unasserted. |
| T0422 `orig_steam_suffixes_the_whole_name` | delete | goldberg_backs_up_once_installs_and_writes_the_exact_artifacts |  | 6 | 0 | 0 | Straight deletion of actions.rs:2045-2051; T0410's plan[0].dst assertion is the surviving pin and should be named in the commit message. |
| T0418 `wine_env_reproduces_run_shs_exports_including_caller_precedence` | merge (verifier: CONFIRM_WITH_NOTE) | wine_env_table (`sabrage-parity:lib.rs`) |  | 27 | 15 | 5: replace wine_env -> Vec<(String, String)> with vec![("xyzzy"; replace wine_env -> Vec<(String, String)> with vec![(String:; delete ! in wine_env … | The `keep` was a mutation-veto artifact, not a coverage fact: every entry of program.json mutation.lost_mutants is a `crates/sabrage-core/...` path, so the run was scoped to -p sabrage-core and never executed sabrage-parity; both vetoing mutants (sabrage/crates/sabrage-core/src/stages/run/actions.rs |
| T0395 `the_action_ids_match_the_contract_in_order` | drop_assertion (verifier: DOWNGRADE) | launch_action_ids_equal_the_contracts_order_and_there_are_exactly_seven (`sabrage-parity:lib.rs`) |  | 13 | 0 | 0 | Delete only the contract-equality assertion at actions.rs:947 (T0869 owns it in CI) and keep lines 949-952, renaming the test to something like `the_guarded_actions_are_listed_and_launch_is_last`. |
| T0412 `goldberg_writes_the_appid_digits_with_no_trailing_newline` | drop_assertion (verifier: CONFIRM_WITH_NOTE) | steam_appid_txt_lands_on_disk_as_exactly_the_appid_digits (`sabrage-parity:lib.rs`) |  | 44 | 0 | 0 | Rename the test when the appid assertion goes — `goldberg_writes_the_appid_digits_with_no_trailing_newline` becomes a false name for what is left (backup/dll/flag-file bytes), and code-standards forbids leaving a false comment or name at the source. |
| T0416 `goldberg_records_nothing_for_an_ordinary_steam_backup` | drop_assertion (verifier: CONFIRM_WITH_NOTE) | goldberg_writes_the_appid_digits_with_no_trailing_newline |  | 21 | 0 | 0 | Replace the dropped byte comparison with `assert!(backup.is_file(), "the stage minted the backup")` so the surviving negative marker assertion cannot pass on a stage that never wrote a backup at all. |
| T0420 `the_banner_is_run_shs_nine_lines_in_order` | drop_assertion (verifier: CONFIRM_WITH_NOTE) | the_launch_banner_lines_are_verbatim_in_run_sh (`sabrage-parity:lib.rs`) |  | 35 | 15 | 1: replace banner_events -> Vec<StageEvent> with vec![] | The assertion added to T0879 must render Section plus ALL Text rows (drop the `!text.is_empty()` filter at lib.rs:2186-2188) and compare to a nine-entry ordered vector including both blank endpoints, or banner order and the leading/trailing blank lines lose their only pin. |

### `sabrage-core:stages/run/guards.rs` — 3 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0436 `a_released_or_disarmed_guard_drops_silently` | delete (verifier: CONFIRM_WITH_NOTE) | a_cancelled_switch_leaves_the_guard_armed_for_the_teardown |  | 17 | 0 | 0 | Name T0432 (guards.rs:902-904), not T0433, as the surviving carrier in the deletion commit message — T0433 releases an ARMED guard and never exercises the no-previous_output early return this test's only non-vacuous assertion touches. |
| T0435 `a_dry_run_guard_drop_restores_nothing` | merge (verifier: DOWNGRADE) | a_cancelled_switch_leaves_the_guard_armed_for_the_teardown |  | 14 | 1 | 0 | Before deleting T0435, add one line to T0432 near guards.rs:899 — `assert!(guard.dry_run, "a dry run's guard never restores from Drop");` — or AudioGuard::inert's propagation of the executor's dry-run flag (guards.rs:232) loses its only assertion. |
| T0432 `no_audio_yields_an_inert_guard_and_one_info_row` | drop_assertion | no_audio_yields_an_inert_guard_and_one_info_row |  | 26 | 0 | 0 | Delete exactly guards.rs:905 and nothing else — 901/904 (empty plan before and after release) and 903 (audio_restored stays false) are the load-bearing observations and must both stay. |

### `sabrage-core:stages/run/mod.rs` — 9 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0461 `a_nonzero_wine_status_is_propagated_verbatim` | delete (verifier: CONFIRM_WITH_NOTE) | a_normal_exit_survives_a_failed_state_save |  | 24 | 0 | 0 | Delete T0461 naming T0450 (mod.rs:1484) as the carrier in the commit message, and since `cargo mutants` cannot be run from this read-only pass the deletion needs the owner's sign-off per §3.4(a); sequence it clear of the 08-31 H3-1 rewrite of the DenyWriteTo double at mod.rs:1328-1429 so T0450's rc= |
| T0455 `only_an_unfinished_restore_is_carried_into_the_next_launch` | merge (verifier: CONFIRM_WITH_NOTE) | a_kept_records_forwards_travel_into_the_next_launch | r1:A9-2 | 26 | 4 | 1: replace unfinished_audio_restore -> Option<String> with Some | Land the merge only together with the added `assert_eq!(carry_forward(&fresh(&root)), Carried::default())` row carrying a new `r1:A9-2` label, and leave T0459's existing bare `A9-2` doc line (mod.rs:1947) alone — §3.6 forbids rewriting pre-existing bare labels. |
| T0444 `the_closing_lines_are_run_shs_verbatim` | drop_assertion | launch_action_text_is_verbatim_in_run_sh (`sabrage-parity:lib.rs`) |  | 19 | 0 | 4: replace wine_exit_line -> String with "xyzzy".into(); replace wine_exit_line -> String with String::new(); replace detached_line -> String with "xyzzy".into() … | Drop only mod.rs:1154-1157 and keep the other three assert_eq!s; the byte survives in CI via sabrage-parity/src/lib.rs:2103-2108, which pins the real constant against scripts/demo/run.sh:177. |
| T0448 `a_normal_exit_prints_the_blank_line_then_the_status_and_clears_the_state` | drop_assertion (verifier: CONFIRM_WITH_NOTE) | a_normal_exit_prints_the_blank_line_then_the_status_and_clears_the_state |  | 35 | 0 | 0 | Drop mod.rs:1265 and rename the test to drop the "and clears the state" clause, since the surviving assertions no longer claim it — the live-handle clear stays pinned by T0450 at mod.rs:1458-1466/1498-1501. |
| T0449 `a_normal_exits_guards_come_off_after_the_status_line_not_before_it` | drop_assertion (verifier: CONFIRM_WITH_NOTE) | a_normal_exits_guards_come_off_after_the_status_line_not_before_it |  | 57 | 0 | 0 | Either drop mod.rs:1319-1324 as proposed, or better, spend one line — `ctx.paths.wineserver = Some(PathBuf::from("/cx/bin/wineserver"))`, as mod.rs:1610/2065 already do — to make the negative discriminating, because after a plain drop nothing in the tree pins that a Normal exit leaves the bottle's w |
| T0465 `a_failure_releases_the_guards_and_propagates_the_original_error` | drop_assertion (verifier: CONFIRM_WITH_NOTE) | a_failure_releases_the_guards_and_propagates_the_original_error |  | 22 | 0 | 0 | Drop mod.rs:2186 and rename to a_failure_propagates_the_original_error as proposed; if the Failed arm's guard release is worth pinning, arm a guard in this fixture the way mod.rs:1612-1619 does rather than keep the empty-Guards assertion, because after the drop only the Cancelled arm (mod.rs:1662) p |
| T0466 `guards_are_released_in_run_shs_trap_order` | drop_assertion (verifier: DOWNGRADE) | no_audio_yields_an_inert_guard_and_one_info_row (`sabrage-core:stages/run/guards.rs`) |  | 34 | 0 | 0 | Drop only mod.rs:2221 (planned-empty, fully carried by guards.rs:904, guards.rs:1351 and mod.rs:1264) and keep mod.rs:2220, then rename the test since the fixture observes no ordering. |
| T0467 `disarming_forgets_both_guards_without_undoing_either` | drop_assertion (verifier: CONFIRM_WITH_NOTE) | disarming_forgets_both_guards_without_undoing_either |  | 26 | 0 | 1: replace Guards::disarm with () | Drop mod.rs:2251-2252 and rename to disarming_consumes_both_guard_slots as proposed; if dashboard disarm-silence is wanted, it belongs beside guards.rs:1086-1102 with dry_run = false, not in this dry-run fixture where it cannot fail. |
| T0470 `a_dry_run_plans_the_launch_supervises_nothing_and_owns_no_session` | drop_assertion (verifier: DOWNGRADE) | the_banner_is_run_shs_nine_lines_in_order (`sabrage-core:stages/run/actions.rs`) |  | 43 | 0 | 4: replace banner_events -> Vec<StageEvent> with vec![]; replace pick_log_path -> (PathBuf, u32) with (Default::defau; replace + with - in pick_log_path … | Drop only the headline assertion at mod.rs:2348; keep the `   log: `/`beatsaber-` assertion at 2349-2351 (and the `let printed` binding it needs), since it is the sole pin on pick_log_path naming the real log and on launch_wine emitting the banner at all. |

### `sabrage-core:stages/run/preflight.rs` — 9 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0480 `effective_string_reads_the_key_the_way_the_runtime_does` | merge (verifier: DOWNGRADE) | effective_string_is_last_wins_table_blind_and_double_quote_only (`sabrage-core:config/runtime_toml.rs`) |  | 33 | 16 | 2: delete match arm '"' in strip_comment; replace match guard !in_string with true in strip_comment | Merge is right, but the carrier must also gain the unquoted-value row and the indented/no-space-`=` row (four moved cases, not two): the reviewer's claim that T0242 covers unquoted values points at effective_accepted, a different function. |
| T0483 `encoder_mode_table` | merge (verifier: DOWNGRADE) | inproc_prints_the_info_row_once_and_skips_both_helper_slugs |  | 15 | 5 | 1: delete match arm "native" \| "auto" in encoder_mode | Before deleting, add one assertion to T0487 (and/or T0504) that a clean `auto`/`native` run emits no line containing "unrecognized" — otherwise the auto/native-are-recognized fact, and the arm-swap mutant it kills, leaves the tree with the private matrix. |
| T0509 `a_helper_autofix_fatal_is_reported_once_with_its_check_row` | merge | an_unfixable_helper_dies_with_run_shs_ensure_helper_text |  | 31 | 14 | 0 | Move both cardinality assertions into T0502 as-is, keep T0502's full-text assert_eq (not T0509's prefix), and rename T0502 to name the combined behaviour. |
| T0510 `block_die_texts_are_run_shs_strings` | merge (verifier: CONFIRM_WITH_NOTE) | preflight_die_and_warn_text_is_verbatim_in_run_sh (`sabrage-parity:lib.rs`) |  | 44 | 22 | 7: delete match arm "dep.goldberg" in block_die; delete match arm "overlay.dxmt-d3d11" \| "overlay.dxmt-wineme; delete match arm "overlay.woxr-dll" \| "overlay.woxr-so" in b … | In the parity test assert the exact strings by equality on `block_die`'s return (they render `--bottle <name>` from the StageCtx default at preflight.rs:892 and would fail assert_verbatim's run.sh substring check), and carry all three groups across — the two overlay variants, the remedy tails, and t |
| T0511 `post_fix_die_texts_are_run_shs_strings` | merge (verifier: CONFIRM_WITH_NOTE) | preflight_die_and_warn_text_is_verbatim_in_run_sh (`sabrage-parity:lib.rs`) |  | 17 | 11 | 4: replace post_fix_die -> (String, Option<String>) with ("xyzz; replace post_fix_die -> (String, Option<String>) with ("xyzz; replace post_fix_die -> (String, Option<String>) with ("xyzz … | Before adding the exact-equality asserts to T0876, make its ctx carry a bottle (`let mut c = ctx("preflight-die"); c.bottle = Some(Bottle::unvalidated("FixtureBottle"));`) — parity's ctx() leaves bottle None, so an assert_eq! without it pins post_fix_die's empty unwrap_or_default path instead of the |
| T0494 `protocol_oxrsys_blocks_natively_with_both_lines` | table | NEW-V05-PROTOCOL-OXRSYS |  | 21 | 6 | 0 | Build the new table on T0494's body and derive each row's scratch tag from its label so the two rows keep distinct fixture directories. |
| T0503 `a_shadowed_protocol_is_judged_on_the_value_the_runtime_will_use` | table (verifier: CONFIRM_WITH_NOTE) | NEW-V05-PROTOCOL-OXRSYS | r1:A7-2 | 19 | 4 | 0 | Label the row exactly `r1:A7-2 shadowed alvr then oxrsys` — the round prefix is mandatory because r2:A7-2 is a different (rollback) finding, and the doc's bare "A2" is not an id to carry over. |
| T0478 `the_slug_list_is_the_contracts_gating_set_in_order` | drop_assertion (verifier: CONFIRM_WITH_NOTE) | preflight_slugs_equal_the_contracts_native_gating_checks_in_order (`sabrage-parity:lib.rs`) |  | 24 | 0 | 0 | When the applier removes the assert_eq! at preflight.rs:922 it must also delete the now-unused `expected` binding (preflight.rs:916-921) and rename the function, whose current name (`..._is_the_contracts_gating_set_in_order`) becomes false once the order/equality assertion lives only in sabrage-pari |
| T0479 `every_gated_slug_is_accounted_for` | drop_assertion (verifier: MISSED_DUP) | every_autofix_gate_maps_to_a_modelled_action (`sabrage-core:fixes/mod.rs`) |  | 31 | 0 | 0 | When dropping preflight.rs:953-959 also drop the explanatory comment at preflight.rs:950-952 (it only explains that assertion) and collapse the now single-assert `Gate::Autofix => { … }` block; leave the `use crate::fixes::{… FixAction …}` import at preflight.rs:66 alone — production code at 409/838 |

### `sabrage-core:stages/setup.rs` — 1 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0346 `dxmt_marker_bytes_are_the_pin_plus_one_newline` | delete | marker_bytes_are_the_pin_plus_exactly_one_newline (`sabrage-core:util/mod.rs`) |  | 4 | 0 | 0 | Straight deletion of setup.rs:500-504; the `.sha256` golden keeps two homes — its bytes at util/mod.rs:521-529 and setup's write of them at setup.rs:952-958 and setup.rs:1080-1083 — so nothing else needs touching. |

### `sabrage-core:stages/stop.rs` — 17 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0371 `cmdline_contains_matches_the_pgrep_f_shape_including_the_windows_path_form` | delete (verifier: CONFIRM_WITH_NOTE) | cmdline_matching_is_the_pgrep_f_shape (`sabrage-core:process.rs`) |  | 26 | 0 | 0 | Delete the stale rationale comment at stop.rs:771-772 ('the tests below still cover it here') and the now-unused `use crate::process::cmdline_contains;` at stop.rs:776 in the same commit, or the build warns on an unused import. |
| T0383 `wait_for_exit_returns_the_survivors_and_nothing_once_they_are_gone` | delete (verifier: CONFIRM_WITH_NOTE) | a_real_reap_reports_the_kill_only_once_the_process_is_really_gone |  | 11 | 0 | 0 | Delete T0383 only together with a hard constraint on the carrier chain: the drop_assertion pass on T0381 must keep stop.rs:1373 assert_eq!(rows(&seen), vec![(Severity::Ok, "found")]) and T0382 must keep stop.rs:1398-1404 (Warn + "survived: " + pid), because those two rows are the only remaining proo |
| T0389 `dry_run_executor_is_dry_run` | delete (verifier: CONFIRM_WITH_NOTE) | dry_run_selects_the_recording_executor (`sabrage-core:stages/mod.rs`) |  | 7 | 0 | 0 | Deleting this orphans two imports — `DryRunExecutor` (stop.rs:774) and `null_sink` (stop.rs:777) have no other use in stop.rs's test module (their only uses are stop.rs:1541-1542) — so trim both in the same diff or the build warns. |
| T0372 `finds_by_cmdline_using_this_test_binarys_own_argv` | merge (verifier: CONFIRM_WITH_NOTE) | cmdline_matching_is_the_pgrep_f_shape (`sabrage-core:process.rs`) |  | 18 | 6 | 0 | Land the moved body as its own named test in process.rs (e.g. find_processes_by_cmdline_finds_this_test_binary_by_a_name_suffix) rather than appending it to T0123, since T0123 pins cmdline_contains and this pins the scanner — and carry the "nonexistent-sabrage-needle.exe" empty assertion across, as  |
| T0369 `format_survivors_matches_the_pgrep_lf_shape` | table | NEW-format_survivors_cases |  | 19 | 18 | 0 | Build the table rows as (label, &[(pid, exe-path literal)], expected) and construct ProcInfo inside the loop — a uniform map, never a per-row branch — so 3.3 holds. |
| T0370 `format_survivors_falls_back_to_the_suffix_when_a_path_has_no_file_name` | table | NEW-format_survivors_cases |  | 8 | 3 | 0 | This row is the only pin of stop.rs:352's `unwrap_or(BEAT_SABER_EXE_SUFFIX)` fallback, so the expected literal "7 Beat Saber.exe " must be copied verbatim into the new table, not paraphrased. |
| T0376 `dry_run_stop_wine_never_spawns_a_real_wineserver` | table (verifier: CONFIRM_WITH_NOTE) | NEW-stop_wine_wineserver_presence |  | 21 | 18 | 0 | Have the table compare the FULL `ctx.executor.planned()` vector mapped to (kind, reason) against the row's expected literals, not a Spawn-filtered subset, or T0377's stronger 'nothing at all was planned' fact is silently downgraded to 'no spawns'. |
| T0377 `dry_run_stop_wine_is_a_no_op_without_crossover` | table (verifier: CONFIRM_WITH_NOTE) | NEW-stop_wine_wineserver_presence |  | 12 | 3 | 0 | Write this row's expected value as the empty FULL planned vector (not 'no spawns'), because it is the tree's only pin that a machine with no CrossOver makes stop_wine record nothing whatsoever. |
| T0385 `a_helper_from_another_checkout_is_reported_instead_of_no_leftover` | table (verifier: CONFIRM_WITH_NOTE) | NEW-foreign_helper_local_match | r1:A5-7 | 29 | 22 | 3: replace report_foreign_helpers with (); replace != with == in report_foreign_helpers; delete ! in report_foreign_helpers | Legal as a table, but the applier must consciously handle stop.rs:1448-1450 — either hoist the oxr_helper_staged setup plus its precondition into the table's shared setup, or record dropping it as an intentional 3.7 removal, because the reviewer's row claims nothing is lost when that assertion actua |
| T0386 `with_no_foreign_helper_running_the_shells_not_found_row_is_unchanged` | table (verifier: CONFIRM_WITH_NOTE) | NEW-no_foreign_helper_local_match |  | 17 | 14 | 0 | Take the table, and add a third row for the filter's negative branch — a ProcInfo whose exe is not named oxrsys-encoder-helper (e.g. ProcInfo::observe(std::process::id())) expecting the NO_LEFTOVER_HELPER row — otherwise the basename filter documented at stop.rs:604-609 has no test at any layer once |
| T0387 `a_foreign_helper_is_reported_even_when_the_local_reap_matched` | table (verifier: CONFIRM_WITH_NOTE) | NEW-foreign_helper_local_match | r2:A5-2 | 32 | 3 | 3: replace report_foreign_helpers with (); replace != with == in report_foreign_helpers; delete ! in report_foreign_helpers | When the doc label at stop.rs:1491-1493 becomes the row label, write it in the tree's shape — `r2:A5-2 regression: a local helper match must not suppress the cross-checkout warn` — and keep the rationale prose (the 'scan used to run only on a local miss' sentence) somewhere in the table's doc commen |
| T0388 `a_matched_local_reap_suppresses_the_not_found_row` | table | NEW-no_foreign_helper_local_match |  | 10 | 3 | 1: delete ! in report_foreign_helpers | Straight row conversion — keep the 'other half of the same gate' rationale from stop.rs:1525-1526 as the table's doc comment so the reason the pair exists survives the merge. |
| T0365 `a_wedged_lsof_warns_instead_of_reporting_free_ports` | drop_assertion (verifier: CONFIRM_WITH_NOTE) | a_wedged_lsof_warns_instead_of_reporting_free_ports | r2:A5-5 | 19 | 0 | 1: replace match guard probe_timed_out(&e) with false in stale_ | Keep the A5-5 section banner at stop.rs:847 (it is this finding's only label home), and apply the same drop to the twin's negative row at stop.rs:942-945 in the same edit or the two probe tests stop having the same shape. |
| T0366 `a_wedged_switchaudiosource_warns_instead_of_naming_an_empty_device` | drop_assertion (verifier: CONFIRM_WITH_NOTE) | a_wedged_switchaudiosource_warns_instead_of_naming_an_empty_device | r2:A5-5 | 21 | 0 | 4: replace report_audio_with with (); replace current_output_device -> Result<Option<String>> with; replace current_output_device -> Result<Option<String>> with … | Drop only the `t == "audio output: "` scan at stop.rs:941-944, fold its wording into the assert_eq! message so the intent survives, and add the missing `/// r2:A5-5 regression:` label while editing — the identical redundant scan in the lsof twin T0365 (stop.rs:919-922) is not in this cut, so cut bot |
| T0381 `a_real_reap_reports_the_kill_only_once_the_process_is_really_gone` | drop_assertion (verifier: CONFIRM_WITH_NOTE) | dry_run_selects_the_recording_executor (`sabrage-core:stages/mod.rs`) | r1:A5-5 | 23 | 0 | 2: delete ! in reap; replace && with \|\| in reap | Drop only line 1360 and, since the file carries no `/// <id> regression:` text at all, add `/// r1:A5-5 regression:` above this test so the finding's surviving assertion is labelled per 3.6. |
| T0384 `reap_never_signals_a_pid_whose_identity_no_longer_matches` | drop_assertion (verifier: CONFIRM_WITH_NOTE) | reap_never_signals_a_pid_whose_identity_no_longer_matches | r1:A5-5 | 15 | 0 | 0 | Drop stop.rs:1436 and the now-dead `let (ctx, _seen) = test_ctx(...)` at stop.rs:1426, and when adding `/// r1:A5-5 regression: …` pick ONE assertion for it — r1:A5-5 has two halves and its termination half is asserted in T0381 (stop.rs:1374-1378), so the label belongs on the identity half here and  |
| T0393 `a_failed_reconcile_is_reported_and_the_stage_still_reaches_its_audio_row` | drop_assertion (verifier: CONFIRM_WITH_NOTE) | a_failed_restore_is_reported_and_the_record_is_kept_for_the_next_stop (`sabrage-core:session/reconcile.rs`) |  | 105 | 0 | 2: replace finish_stopped_session -> Result<()> with Ok(()); replace == with != in StageOutcome::from_code | Dropping stop.rs:1731-1734 also requires rewriting the false comment above it — stop.rs:1729-1730 says 'the two steps that come after it still ran', but report_ports (stop.rs:232) runs BEFORE reconcile (stop.rs:272), so the comment must be narrowed to the audio row alone; and drop `self` from the `u |

### `sabrage-core:store/goldberg.rs` — 2 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0631 `refuses_while_a_matching_game_process_is_running` | drop_assertion (verifier: CONFIRM_WITH_NOTE) | refuses_while_a_matching_game_process_is_running | r2:A13a-2 | 44 | 0 | 1: replace match guard e.kind() == std::io::ErrorKind::NotFound | Delete goldberg.rs:563-570 but leave the bare `A13a-2` doc label at :547 unrewritten and leave assert :583's exact substring "Beat Saber is running" alone — with the live_session_reason precondition gone it is the only thing that separates the argv-probe refusal from live_session_reason's own "Beat  |
| T0633 `refuses_while_only_the_runtime_reports_a_live_session` | drop_assertion | refuses_while_only_the_runtime_reports_a_live_session |  | 44 | 0 | 1: replace <impl Drop for StagedTemp>::drop with () | Delete only goldberg.rs:634-637 and keep the `let _g = crate::session::lock_session_globals();` guard at :626, which is what makes the freshly written runtime_status.json the only live signal the refusal can be coming from. |

### `sabrage-core:store/library.rs` — 8 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0637 `missing_file_loads_as_default_with_version_one` | drop_assertion (verifier: CONFIRM_WITH_NOTE) | missing_file_loads_as_default_with_version_one |  | 6 | 0 | 1: replace match guard e.kind() == std::io::ErrorKind::NotFound | Apply the cut but rename the test to `missing_file_loads_as_default` — after :665 goes the name's "_with_version_one" claim is no longer asserted here, and the reviewer's stated reason ("structural equality already includes both fields") is only true of games.is_empty(); the version==1 literal survi |
| T0639 `a_newer_schema_version_is_refused_not_silently_rewritten` | drop_assertion (verifier: CONFIRM_WITH_NOTE) | round_trips_camel_case_through_the_file | r1:A13a-6 | 23 | 0 | 1: replace > with < in load | Apply the cut of library.rs:697-699 but record the carrier as T0640 `unknown_fields_are_ignored_on_load` (library.rs:708-714), not T0641 as the reviewer wrote — T0641 never pins the "version":1 literal or compares to Library::default(). |
| T0645 `an_edit_racing_a_recorded_session_keeps_both` | drop_assertion (verifier: CONFIRM_WITH_NOTE) | an_edit_racing_a_recorded_session_keeps_both | r1:A13b-5 | 45 | 0 | 4: replace Library::upsert_editable -> &GameEntry with Box::lea; replace == with != in Library::record_last_session; replace Library::record_last_session -> bool with false … | Delete only line 852 and keep the comment at library.rs:849-851 ('The editor's stale clone: renamed, and still carrying no session.'), which already records in prose the precondition the assertion was stating. |
| T0646 `upsert_inserts_then_replaces_by_id` | drop_assertion | get_finds_by_id_and_nothing_else |  | 15 | 0 | 0 | Delete library.rs:887 only; keep :886's 'same id replaces, does not append' message, which is the assertion that actually distinguishes replacement from append. |
| T0647 `remove_reports_whether_it_found_something` | drop_assertion | remove_reports_whether_it_found_something |  | 13 | 0 | 2: replace Library::remove -> bool with false; replace != with == in Library::remove | Delete library.rs:902; it also removes the test's only non-literal input (Uuid::new_v4()), which 3.3 wants out of tests anyway. |
| T0655 `launch_options_for_resolves_the_merge_by_id_and_is_none_for_a_stranger` | drop_assertion | effective_options_merges_overrides_over_settings_and_takes_identity_from_the_entry |  | 26 | 0 | 1: replace Library::launch_options_for -> Option<StageOptions> | Delete library.rs:1090 and keep the equality at :1085-1089 with its "one merge, one home" message, which is what pins that launch_options_for resolves the right entry rather than merely returning something. |
| T0656 `no_bottle_no_exe_is_not_found_with_a_says_why_problem` | drop_assertion | goldberg_state_covers_all_five_variants |  | 10 | 0 | 0 | Drop only library.rs:1122; do not touch T0662's line 1279 or its 1334-1337 `validate` call, which are the two lines that carry the NoDll fact and the public-entry-point plumbing. |
| T0662 `goldberg_state_covers_all_five_variants` | drop_assertion (verifier: CONFIRM_WITH_NOTE) | goldberg_state_covers_all_five_variants | r1:A13a-1; r2:A13a-1 | 82 | 0 | 3: replace \|\| with && in validate_pinned; replace == with != in sha256_file; replace file_sha256_matches -> bool with false | Delete library.rs:1293-1297, 1319-1323 and 1338-1341 only, and keep the prose comments at 1287-1288 and 1310-1314 plus the assert_eq at 1291 and 1324 — those two equalities, not the assert_ne pair, are the r1 and r2 A13a-1 regressions. |

### `sabrage-core:store/settings.rs` — 7 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0671 `the_current_version_still_loads` | delete (verifier: CONFIRM_WITH_NOTE) | round_trips_camel_case_through_the_file |  | 5 | 0 | 0 | Name both carriers in the commit message — T0674 (settings.rs:449) for 'the current version still loads through the public API' and T0670 (settings.rs:360-363) for 'the file's version key is what the gate reads' — since T0674 alone would not catch a serde rename of the field. |
| T0667 `unknown_fields_load_into_extra_instead_of_failing` | merge (verifier: CONFIRM_WITH_NOTE) | unknown_fields_survive_a_load_save_round_trip |  | 14 | 3 | 0 | Add the three pre-save assertions to T0668 (immediately after settings.rs:300) in the SAME commit that deletes T0667, because the T0668 row's own drop of settings.rs:305-306 is only safe once they are there. |
| T0672 `a_file_without_a_version_reads_as_the_current_one` | merge (verifier: CONFIRM_WITH_NOTE) | unknown_fields_survive_a_load_save_round_trip |  | 5 | 1 | 0 | Land this in the same commit as the T0667 merge and the T0668 drop — all three edit T0668's body, and the commit message must name T0674 (settings.rs:449) as the carrier for the empty-`extra` half that is not being moved. |
| T0673 `an_ordinary_settings_file_carries_no_extra_keys` | merge (verifier: DOWNGRADE) | round_trips_camel_case_through_the_file |  | 22 | 2 | 0 | Do the cut as a merge, not a delete: before removing T0673, add `assert!(text.contains("\"version\""));` and `assert!(text.contains("\"defaultBsDir\""));` to T0674's contains-block at settings.rs:443-448. |
| T0668 `unknown_fields_survive_a_load_save_round_trip` | drop_assertion (verifier: CONFIRM_WITH_NOTE) | unknown_fields_survive_a_load_save_round_trip | r1:A13b-8 | 24 | 0 | 0 | Do not delete settings.rs:305-306 unless T0667's three pre-save assertions land in the same commit — without them `reread.extra == s.extra` passes vacuously (both empty) if `load` ever stops collecting `extra`. |
| T0669 `unknown_nested_launch_keys_survive_a_load_save_round_trip` | drop_assertion (verifier: CONFIRM_WITH_NOTE) | unknown_nested_launch_keys_survive_a_load_save_round_trip | r2:A13a-3; r2:A13b-4 | 34 | 0 | 0 | Delete settings.rs:341 only and leave the bare `A13a-3 / A13b-4` doc label at settings.rs:314-316 exactly as written — 3.6 forbids rewriting existing bare labels into the r2: form. |
| T0674 `round_trips_camel_case_through_the_file` | drop_assertion (verifier: CONFIRM_WITH_NOTE) | missing_file_loads_as_default |  | 36 | 0 | 2: replace match guard e.kind() == std::io::ErrorKind::NotFound; replace == with != in load | Carrier chain: T0665 is itself a drop_assertion loser elsewhere, so the applier must keep `assert_eq!(s, Settings::default())` at settings.rs:255 (only the three implied follow-ups at 256-258 may go) or this drop loses its carrier. |

### `sabrage-core:util/mod.rs` — 2 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0196 `host_manifest_is_template_minus_trailing_newline` | delete | render_host_manifest_matches_the_on_disk_template (`sabrage-parity:lib.rs`) |  | 10 | 0 | 0 | Safe as written, but the same commit must fix sabrage/crates/sabrage-parity/src/lib.rs:1073-1077, whose comment claims 'sabrage-core's own unit tests already cover the compiled-in-vs-itself case' — deleting T0196 with T0197 makes that sentence false at its source. |
| T0197 `host_manifest_json_escapes_the_dylib_path` | merge (verifier: CONFIRM_WITH_NOTE) | render_host_manifest_json_escapes_the_dylib_path (`sabrage-parity:lib.rs`) | r1:A1-8 | 27 | 1 | 0 | Add the missing `/// r1:A1-8 regression: …` label above sabrage/crates/sabrage-parity/src/lib.rs:1108 (the round prefix is mandatory — A1-8 also exists in round 2) and, in the same commit, correct the now-false claim at lib.rs:1073-1077 that sabrage-core still covers the compiled-in-vs-itself case. |

### `sabrage-core:util/winpath.rs` — 1 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0204 `table` | merge (verifier: CONFIRM_WITH_NOTE) | win_path_table (`sabrage-parity:lib.rs`) |  | 51 | 8 | 2: replace win_path -> String with "xyzzy".into(); delete ! in win_path | The merge is only safe if all three missing rows land in T0860 — spaces-and-parentheses on the C: branch, Some(Path::new("")) as an empty prefix, and the drive_cache prefix-match sibling — because after the cut sabrage/crates/sabrage-core/src/util/winpath.rs has no tests at all. |

### `sabrage-parity:lib.rs` — 8 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0840 `a_commented_out_chk_line_contributes_no_slug` | table (verifier: CONFIRM_WITH_NOTE) | NEW-V14-SLUG-COMMENTS | r1:A1-7 | 8 | 9 | 0 | Label the commented-out and indented rows `r1:A1-7 regression: …` (the round prefix is mandatory — r2:A1-7 is a different finding), and budget ~9 lines for this row, not 6: the reviewer's estimate omits the table scaffold, which rule 9 charges to the row that creates NEW-V14-SLUG-COMMENTS. |
| T0841 `a_trailing_comment_does_not_hide_the_call_before_it` | table | NEW-V14-SLUG-COMMENTS |  | 4 | 2 | 0 | Move it verbatim as one labelled row ("trailing comment") of NEW-V14-SLUG-COMMENTS; no assertion changes. |
| T0842 `a_hash_inside_a_parameter_expansion_or_a_string_is_not_a_comment` | table (verifier: CONFIRM_WITH_NOTE) | NEW-V14-SLUG-COMMENTS |  | 5 | 2 | 0 | Move the slug expectation as a plain row, but do NOT add the reviewer's per-row strip_comment column — instead strengthen this row's fixture by appending a second slug-bearing call after the quoted hash (e.g. `…"; chk ok build.woxr-dll "ok"` expecting two slugs), so a quote-blind lexer changes the o |
| T0843 `a_loop_header_whose_body_emits_nothing_covers_no_slug` | table (verifier: CONFIRM_WITH_NOTE) | NEW-V14-SLUG-LOOPS | r1:A1-7 | 13 | 12 | 0 | When T0845's fixtures join the same table, label them `r2:A1-6` (its true id per findings-index.tsv) and reserve `r1:A1-7` for this row's gutted-loop fixture, so the two loop findings do not collide on one label. |
| T0844 `a_loop_that_extracts_the_slug_into_a_variable_still_counts` | table (verifier: CONFIRM_WITH_NOTE) | NEW-V14-SLUG-LOOPS |  | 6 | 6 | 0 | This row must also create the NEW-V14-SLUG-LOOPS scaffold (count it once, here, not again on T0845/T0846) and its row must add the `header_only_loops == 0` column that the loser never asserted, and keep the `// Section 10's shape` explanation from lib.rs:607 as the row label. |
| T0845 `an_unrelated_emission_in_the_loop_body_credits_nothing` | table (verifier: CONFIRM_WITH_NOTE) | NEW-V14-SLUG-LOOPS | r2:A1-6 | 23 | 13 | 0 | label_to_keep is incomplete: the `gutted` row must carry BOTH the existing bare `A1-7` label and the new `r2:A1-6` label (round2.md:129's regression sketch is that exact fixture), and the `decoy` row keeps the `// Same shape, but with the extraction inline…` explanation from lib.rs:629-630 as its la |
| T0846 `only_a_call_carrying_the_slug_counts` | table (verifier: CONFIRM_WITH_NOTE) | NEW-V14-SLUG-LOOPS |  | 21 | 18 | 0 | Give the `near_miss` row (lib.rs:653-658) an explicit label naming whole-variable matching — it is the only pin anywhere on `assigned_variable`/`loop_body_emits` rejecting a prefix-collision variable name, and a table row without that label loses the reason it exists. |
| T0875 `native_run_only_die_text_is_verbatim_in_run_sh` | drop_assertion (verifier: DOWNGRADE) | preflight_die_and_warn_text_is_verbatim_in_run_sh | r1:A1-3 | 48 | 0 | 0 | Keep both functions and instead delete only the two now-redundant copied literals from T0876's fragment list — lib.rs:2022-2025 ("bridge not built — ./demo.sh build") and lib.rs:2039-2042 ("--wired needs adb …") — while leaving lib.rs:2043-2046 in place, since T0875 never evaluates that branch. |

### `sabrage/src-tauri:commands.rs` — 5 rows

| test | verdict | carrier | label kept | lines now | +carrier | mutants uniquely caught (checklist) | note |
|---|---|---|---|---:|---:|---|---|
| T0897 `no_doctor_row_group_matches_the_contracts_run_only_slugs` | delete (verifier: CONFIRM_WITH_NOTE) | doctor_slug_coverage_matches_the_contract (`sabrage-parity:lib.rs`) |  | 17 | 0 | 0 | The cut stands but its carrier was wrong twice over: parity T0868 (sabrage/crates/sabrage-parity/src/lib.rs:1375-1388) never reads a check's `group`, and the verifier's substitute carrier T0610 (checks/run_only.rs:568) is itself a confirmed delete; the real carrier is parity T0839 `doctor_slug_cover |
| T0900 `launch_opts_game_id_defaults_to_none_and_round_trips_through_the_struct` | delete (verifier: CONFIRM_WITH_NOTE) | last_session_to_record_needs_a_game_id_a_launched_event_and_a_settled_outcome |  | 8 | 0 | 0 | Land this under H3-4 with a §3.7 (derive/struct-storage) justification rather than the reviewer's 'T0907 is the carrier' wording, and in the same commit fix the Phase 4 block comment at commands.rs:2777-2787, whose 'construction, defaults' clause becomes false once T0900 and T0901 are gone. |
| T0901 `app_state_carries_the_new_default_bottle_and_bs_dir_fields` | delete (verifier: CONFIRM_WITH_NOTE) | settings_defaults_fill_only_what_env_and_gui_left_unset |  | 11 | 0 | 0 | Delete it as part of H3-4 (which already names commands.rs:2789-2810) and state in the commit that the field-complete-literal canary is preserved by `get_app_state`'s own field-complete literal at commands.rs:272-279, not by T0883. |
| T0903 `repo_root_source_variants_are_pairwise_distinct` | delete (verifier: CONFIRM_WITH_NOTE) | classify_repo_root_source_follows_resolve_repo_roots_own_precedence |  | 13 | 0 | 0 | Delete T0903 only — never fold it into T0902, whose body comment at commands.rs:2814-2817 carries the `resolve_repo_root` precedence finding rationale — and justify the cut as §3.7 derive-output plus §3.3 (the nested `for` loop at :2851-2853 is logic in a test), not as duplication of T0902. |
| T0895 `should_intercept_quit_only_when_unapproved_and_live` | merge | quit_is_intercepted_once_and_given_up_on_when_nobody_answers |  | 9 | 5 | 0 | Add the `(quit_approved = true, session_is_live = false) → PassThrough` row to T0890 (after commands.rs:2583) in the same commit that deletes commands.rs:2672-2681. |

### Deletions vetoed by the kill matrix

| test | mutants only this test catches |
|---|---|
| T0115 `process_scan_agrees_with_the_single_needle_convenience_functions` | replace CheckOptions::from_env::value -> Option<String> with Some(String::new()); replace SabrageError::tail -> &[String] with Vec::leak(vec!["xyzzy".into()]); replace terminate_and_wait -> bool with true |
| T0246 `write_never_clobbers_a_file_created_in_the_toctou_window` | delete - in walk |
| T0431 `the_guard_texts_are_run_shs_verbatim` | replace audio_switched_line -> String with "xyzzy".into() |
| T0576 `registry_binds_in_contract_order_and_covers_every_slug` | replace Registry::checks -> &[BoundCheck] with Vec::leak(Vec::new()); replace Contract::check_slugs -> Vec<&str> with vec!["xyzzy"] |
| T0686 `the_two_tokens_are_independent` | replace <impl std::fmt::Debug for LiveSessionHandle>::fmt -> std::fmt::Result with Ok(Default::default()) |

---

## 4. Rejected and downgraded claims

8 vertical cut proposals were rejected outright; 26 were downgraded. Each line names what killed it.

| test | proposed | killed by | verifier note |
|---|---|---|---|
| T0272 `the_enums_serialize_as_the_toml_spellings` (`sabrage-core:config/runtime_toml.rs`) | drop_assertion → T0272 | unsubsumed: the serde wire spellings "alvr"/"h265"/"inproc", which sabrage/ui/src/ipc.ts:844-850 mirrors by hand and no other test pins |  |
| T0273 `the_view_serializes_camel_case_for_the_ui` (`sabrage-core:config/runtime_toml.rs`) | delete → T0227 | unsubsumed: camelCase wire names for RuntimeConfigView; the named carrier T0227 (runtime_toml.rs:2196-2215) contains no JSON assertion at all |  |
| T0712 `stop_keeps_the_record_when_the_audio_could_not_be_restored` (`sabrage-core:session/reconcile.rs`) | delete → T0711 | different units: finish_stopped_session_inner (reconcile.rs:574-625) is not a delegate of reconcile_with; unsubsumed: stop must not clear a record whose audio guard is still pending |  |
| T0723 `stop_restores_and_clears_a_session_it_did_not_start` (`sabrage-core:session/reconcile.rs`) | delete → T0697 | different units: T0697 exercises reconcile_with (reconcile.rs:357), the loser exercises finish_stopped_session_with -> finish_stopped_session_inner (reconcile.rs:532/571), which is NOT a delegate — it adds bottle scoping, its own untouchable/alive branches and the tolerate_reconcile_failure policy; unsubsumed: stop's successful restore-and-clear | Keep T0723: it is the only test in the file where the stop tail actually restores and clears a record, and deleting it leaves stop's documented normal case (reconcile.rs:504-513) with no assertion. |
| T0725 `stop_leaves_a_session_this_process_supervises_to_its_own_teardown` (`sabrage-core:session/reconcile.rs`) | delete → T0708 | unsubsumed: stop's own live-run-id early return (reconcile.rs:596-598) is pinned by no other test; T0708 pins reconcile_with's distinct branch (reconcile.rs:391-408) | Keep T0725: it is the sole guard against stop racing its own supervise loop, and no other test passes a matching live run id to finish_stopped_session_with. |
| T0728 `stop_without_a_record_is_a_silent_noop` (`sabrage-core:session/reconcile.rs`) | delete → T0695 | different units: the loser pins finish_stopped_session_inner's own early return (reconcile.rs:584-587), T0695 pins reconcile_with's (reconcile.rs:366-369); the two are separate `let ... else` sites in sibling functions, not one shared branch | Weakest of the four stop-tail rows but still a reject: keep T0728 (11 lines) unless the owner signs off under §3.4's 'when (a) cannot be run' clause, because it is the only test of stop's no-record path. |
| T0862 `slugs_are_unique` (`sabrage-parity:lib.rs`) | merge → T0839 | unsubsumed: run-only slug uniqueness (T0839 filters `c.group != "run-only"` at lib.rs:493); golden: contract slug cardinality; overlap is partial, not full | Leave T0862 where it is in `contract_sanity`; T0839 cannot carry it because `contract_row_slugs()` at lib.rs:490-498 drops every run-only check before the comparison. |
| T0880 `demo_shs_root_is_the_logical_pwd` (`sabrage-parity:lib.rs`) | merge → T0881 | different units: T0880 pins a demo.sh shell byte, T0881 exercises sabrage_core::resolve_repo_root; §3.3 'When setup or assertions differ, they stay separate functions' | Keep T0880 as its own function; if V6-6's prologue compression lands, make its one-line replacement name both test functions so r1:A2-6 keeps a home. |

Downgraded:

| test | proposed → final | note |
|---|---|---|
| T0027 `with_step_shares_the_plan` (`sabrage-core:executor.rs`) | delete → keep | Keep T0027 as the only executor-unit pin of with_step's shared plan Arc — it is the exact seam 08-31 item H3-1 stresses ('with_step returns a new decorator wrapping inner.with_step(step) carrying both the fault and the shared recorder Arc', 2026-08-31-simplification.md:147). |
| T0062 `an_ordinary_append_is_never_mistaken_for_a_rewrite` (`sabrage-core:logs.rs`) | merge → keep | Keep logs.rs:1267-1287 as it stands — T0051 never grows past CONTINUITY_SIGNATURE_BYTES, so deleting T0062 leaves update_signature's drain branch (logs.rs:280-283) with no test that can fail. |
| T0106 `spawn_streamed_does_not_populate_a_tail_nobody_reads` (`sabrage-core:process.rs`) | delete → keep | Keep T0106 and, while there, give it the missing §3.6 label (`/// E-C1-io-waste regression: spawn_streamed passes capture_tail=false`) so the next scan does not propose it again. |
| T0260 `write_refuses_a_file_it_cannot_round_trip` (`sabrage-core:config/runtime_toml.rs`) | drop_assertion → keep |  |
| T0270 `edit_protocol_creates_the_file_from_the_template_when_it_is_absent` (`sabrage-core:config/runtime_toml.rs`) | drop_assertion → keep |  |
| T0395 `the_action_ids_match_the_contract_in_order` (`sabrage-core:stages/run/actions.rs`) | delete → drop_assertion | Delete only the contract-equality assertion at actions.rs:947 (T0869 owns it in CI) and keep lines 949-952, renaming the test to something like `the_guarded_actions_are_listed_and_launch_is_last`. |
| T0435 `a_dry_run_guard_drop_restores_nothing` (`sabrage-core:stages/run/guards.rs`) | delete → merge | Before deleting T0435, add one line to T0432 near guards.rs:899 — `assert!(guard.dry_run, "a dry run's guard never restores from Drop");` — or AudioGuard::inert's propagation of the executor's dry-run flag (guards.rs:232) loses its only assertion. |
| T0466 `guards_are_released_in_run_shs_trap_order` (`sabrage-core:stages/run/mod.rs`) | drop_assertion → drop_assertion | Drop only mod.rs:2221 (planned-empty, fully carried by guards.rs:904, guards.rs:1351 and mod.rs:1264) and keep mod.rs:2220, then rename the test since the fixture observes no ordering. |
| T0470 `a_dry_run_plans_the_launch_supervises_nothing_and_owns_no_session` (`sabrage-core:stages/run/mod.rs`) | drop_assertion → drop_assertion | Drop only the headline assertion at mod.rs:2348; keep the `   log: `/`beatsaber-` assertion at 2349-2351 (and the `let printed` binding it needs), since it is the sole pin on pick_log_path naming the real log and on launch_wine emitting the banner at all. |
| T0480 `effective_string_reads_the_key_the_way_the_runtime_does` (`sabrage-core:stages/run/preflight.rs`) | merge → merge | Merge is right, but the carrier must also gain the unquoted-value row and the indented/no-space-`=` row (four moved cases, not two): the reviewer's claim that T0242 covers unquoted values points at effective_accepted, a different function. |
| T0483 `encoder_mode_table` (`sabrage-core:stages/run/preflight.rs`) | delete → merge | Before deleting, add one assertion to T0487 (and/or T0504) that a clean `auto`/`native` run emits no line containing "unrecognized" — otherwise the auto/native-are-recognized fact, and the arm-swap mutant it kills, leaves the tree with the private matrix. |
| T0527 `missing_output_fails_with_the_build_remedy` (`sabrage-core:checks/build.rs`) | drop_assertion → keep | Keep build.rs:198 as written — the drop saves one line, leaves the test's own name ('..._fails_...') unbacked by an assertion, and the /nonexistent-root input it covers is not the input T0529 exercises. |
| T0528 `present_output_passes_with_relative_path` (`sabrage-core:checks/build.rs`) | drop_assertion → keep | Keep build.rs:211 as written, for the same reason as T0527 — the standard's redundancy rule is about tests, and the test's name ('..._passes_...') should keep an assertion behind it. |
| T0537 `missing_toml_fails_supported_and_skips_legacy` (`sabrage-core:checks/config.rs`) | table → keep | Keep missing_toml_fails_supported_and_skips_legacy as its own function — exactly as the reviewer already kept the analogous missing_session_json_is_skipped (config.rs:563-569) out of the session shape matrix — and build the protocol table from the four file-writing rows only. |
| T0665 `missing_file_loads_as_default` (`sabrage-core:store/settings.rs`) | drop_assertion → keep | Keep settings.rs:257-259 as they are — they are the tree's only assertions on the content of `impl Default for Settings`; the separate T0675 absorption the reviewer bolts onto this row is fine but adds about five lines to this test, not zero. |
| T0673 `an_ordinary_settings_file_carries_no_extra_keys` (`sabrage-core:store/settings.rs`) | delete → merge | Do the cut as a merge, not a delete: before removing T0673, add `assert!(text.contains("\"version\""));` and `assert!(text.contains("\"defaultBsDir\""));` to T0674's contains-block at settings.rs:443-448. |
| T0675 `a_minimal_file_loads_on_defaults` (`sabrage-core:store/settings.rs`) | merge → keep | Keep T0675 as its own function but rewrite it in place to go through the public loader — write `{}` to a scratch settings.json and assert `load(&path).unwrap() == Settings::default()`, then clean up — instead of folding it into T0665. |
| T0688 `now_unix_ms_is_a_plausible_wall_clock` (`sabrage-core:session/mod.rs`) | delete → keep | Keep mod.rs:1165-1169; if the platform half offends §3.7, rewrite the body as a unit assertion (e.g. now_unix_ms() / 1000 within a second of SystemTime::now()'s as_secs()) rather than deleting the crate's only pin on the millisecond unit. |
| T0777 `check_against_the_working_checkout_is_in_sync` (`sabrage-contract-gen:lib.rs`) | delete → merge |  |
| T0793 `verbose_with_no_detail_prints_nothing_extra` (`sabrage-cli:main.rs`) | merge → table | Move the assertion into the NEW-V13-doctor-outcome-rendering table as the verbose=true row for T0785's outcome (2 lines) instead of appending a second fixture to T0792, so the r2:A3b-3 regression test keeps exactly one behaviour and one fixture. |
| T0803 `merge_stage_options_empty_cli_values_clear_a_preset_env_base` (`sabrage-cli:main.rs`) | table → keep | Keep T0803 as a named function: the 08-31 wave-5 cautions explicitly exclude main.rs's merge_* family from table-driving, the doctor-side third arm of the same r1:A14-1 fix cannot join the table (different unit, merge_doctor_options), and once the scaffold and the 1748-1751 rationale are counted the |
| T0804 `merge_stage_options_empty_cli_values_stay_none_with_no_preset` (`sabrage-cli:main.rs`) | table → keep | Keep T0804 for the same reason as T0803 — it is the sibling arm of the same r1:A14-1 section and cannot form a table by itself. |
| T0805 `merge_doctor_options_empty_cli_values_clear_a_preset_env_base` (`sabrage-cli:main.rs`) | table → keep | Leave both merge_doctor_options tests as separately named functions - the already-programmed V7-2 item explicitly excludes main.rs's merge_* family from table-driving. |
| T0806 `merge_doctor_options_empty_cli_values_stay_none_with_no_preset` (`sabrage-cli:main.rs`) | table → keep | Keep this as its own named function alongside T0805; the V7-2 program already ruled main.rs's merge_* family not table-drivable. |
| T0824 `dry_run_plan_with_no_actions_says_so_rather_than_printing_nothing` (`sabrage-cli:main.rs`) | drop_assertion → keep | Leave the full-vector equality alone - swapping it for len()==2 plus lines[1] unpins the header in the empty branch and makes the test two lines longer, not shorter. |
| T0875 `native_run_only_die_text_is_verbatim_in_run_sh` (`sabrage-parity:lib.rs`) | merge → drop_assertion | Keep both functions and instead delete only the two now-redundant copied literals from T0876's fragment list — lib.rs:2022-2025 ("bridge not built — ./demo.sh build") and lib.rs:2039-2042 ("--wired needs adb …") — while leaving lib.rs:2043-2046 in place, since T0875 never evaluates that branch. |

Horizontal claims: 133 claims from X1–X5, verified {'CONFIRM': 28, 'CONFIRM_WITH_NOTE': 61, 'CONFLICT': 43, 'REJECT': 1}.

| claim | kind | tests | proposed | verifier | note |
|---|---|---|---|---|---|
| X1#5 | cross_layer_dup | T0395, T0869 | drop_assertion | CONFLICT T0395's three literal assertions — assert!(LAUNCH_ACTION_IDS.contains(&"audio-route")), assert!(... contains(&"dashboard")), assert_eq!(LAUNCH_ACTION_IDS[6], "launch-wine") at sabrage/crates/sabrage-core/src/stages/run/actions.rs:949-952 — which the vertical's delete would leave with no test home anywhere in the tree | The claim's own half (drop only :942-947, exactly 6 lines) is sound and is the conservative reading; the conflict is that the vertical would additionally delete :949-952. Do not resolve here. |
| X1#9 | cross_layer_dup | T0572, T0005 | delete | CONFLICT T0005: vertical delete vs claim keep | The two vertical rows name each other as carrier (T0572 merge->T0005, T0005 delete->T0572); applying both would leave the compiled-vs-live equality with no home at all. The claim's direction is the safe one and I found no assertion it loses |
| X1#11 | mirror | T0499, T0603 | delete | CONFLICT the run()-level assertion that a run.wired-adb Fail aborts the launch with the evaluator's message unprefixed (preflight.rs:1721-1726) | Chained hazard the claim does not mention: its survivor T0603 is itself marked `merge` into parity T0875 by its own vertical row. If both are applied the fact ends up only in sabrage-parity/src/lib.rs:1945-1954 — legal under §3.5 (T0875 ass |
| X1#16 | architect_map | T0401, T0603 | keep | CONFLICT T0603's `assert_eq!(out.message, "--wired needs adb ...")` as a core-side literal on run_wired_adb (checks/run_only.rs:410-413) | The T0401-vs-T0603 half of the claim is verified and must not be collapsed: they are two independent implementations of the same sentence (actions.rs:199-204 `st.fatal(...)` inside adb_forward_hygiene, run_only.rs:227-232 `CheckOutcome::fai |
| X1#17 | architect_map | T0557, T0609 | keep | CONFLICT T0609's third assertion `first_connected_serial("only-a-header\n") == None` (run_only.rs:564) — no run_wired_adb fixture feeds a single-line non-header output | T0557 is not in dispute: the vertical's 'table' verdict folds it into a headset-local table (headset.rs:136/142/152 differ only in literals), which keeps the fact and is compatible with the claim's keep. Also: 08-31 item V4-5 (sabrage/docs/ |
| X1#21 | architect_map | T0675, T0735 | keep | CONFLICT T0675's `serde_json::from_str::<Settings>("{}") == Settings::default()` (settings.rs:456-458) — T0665 as written never deserializes anything | T0735 is not in dispute (vertical keep, claim keep) and is clearly not platform-testing: it pins the omitted-field defaults of a *required-identity* schema plus `!s.has_pending_guards()` (state.rs:595-614). If the critic sides with the vert |
| X1#25 | cross_layer_dup | T0419, T0872 | keep | CONFLICT T0419's `assert_eq!(spec.display(), "/Applications/CrossOver.app/x/bin/wine --bottle Steam --no-update --cx-app C:\\Program Files (x86)\\Steam\\steamapps\\common\\BS\\Beat Saber.exe")` (sabrage/crates/sabrage-core/src/stages/run/actions.rs:1973-1978) — the rendered one-line command string, including the literal win-path bytes; T0873 asserts spec.program and spec.args separately and derives the Windows path via `util::win_path(..)` rather than a literal, and never calls display(). | The claim's own pair relation (T0419 vs T0872 = different units) is sound and needs no action; the conflict is with the vertical's separate T0419-vs-T0873 drop_assertion. If an applier does drop T0419's argv assertion, the display()-renderi |
| X1#26 | architect_map | T0478, T0579, T0578, T0897 | keep | CONFLICT T0897's group facts: `contract().check("run.wine-exec").group == NO_DOCTOR_ROW_GROUP` and the same for "run.bridge-built", plus `assert_ne!` for "sys.arch" (sabrage/src-tauri/src/commands.rs:2731-2744). T0868 (sabrage/crates/sabrage-parity/src/lib.rs:1374-1388) asserts only `preflight_slugs() == contract checks with native_gate != Gate::None, in contract order` — it never reads a check's `group`, so the run-only group mapping is not carried by it. | If an applier follows the vertical and deletes T0897, the two named slugs' membership in NO_DOCTOR_ROW_GROUP must survive somewhere: the nearest existing pins are derived, not literal — `checks/run_only.rs:567-576` (defs() == contract's run |
| X2#1 | cross_layer_dup | T0681, T0752 | drop_assertion | CONFLICT sabrage/crates/sabrage-core/src/session/mod.rs:934-942 — the three negatives (stale stamp, dead pid, no process_id) are the only assertions that prove ensure_idle_at *delegates* to watcher::runtime_status_live instead of refusing on freshness alone. T0752 (watcher.rs:755-777) tests the predicate in isolation and would still pass if session_block_at reverted to its own local freshness read, which is precisely the A10-8 defect. Round-2 A10-8's own regression spec demands them at the door: 'session_block_at over four status fixtures (fresh+live pid -> blocks, fresh+dead pid -> …, fresh+no pid -> …, stale -> no block)' (sabrage/docs/reviews/2026-08-30-codex-round2.md:914). | If a critic still wants the cut, the surviving positive at mod.rs:927-932 must be paired with at least one door-level negative (stale is the cheapest) or the A10-8 door assertion has no home. |
| X2#2 | cross_layer_dup | T0764, T0769 | drop_assertion | CONFLICT sabrage/crates/sabrage-core/src/session/watcher.rs:2073-2074 — the Stalled scenario is the only place `!s.runtime_fresh` is asserted *in the Stalled phase* with a stale 'streaming' heartbeat and no process_id in the file (:2048); T0764's !runtime_fresh assertion (:1379) is made on the Running branch, and its Stalled branch (:1389) asserts phase only. | If the cut is taken, carry `assert!(!s.runtime_fresh)` onto T0764's second snapshot (watcher.rs:1389) so the stale-file/runtime_fresh pairing keeps a home, and keep the 'the only state oxrsys heartbeats is streaming' note (watcher.rs:2037-2 |
| X2#3 | cross_layer_dup | T0246, T0019 | drop_assertion | CONFLICT T0246: vertical delete vs claim drop_assertion | The two dispositions are mutually exclusive: the claim keeps T0246 (minus :2766-2772) as the write-side A10-1 carrier; the vertical deletes the whole function, naming T0019 and T0250 as carriers. Whoever applies must pick one. If the claim  |
| X2#10 | finding_group | T0753, T0759, T0760, T0761, T0769 | keep | CONFLICT T0760's HEVC-positive adopted-session case (watcher.rs:1092-1116) is the disputed fact; T0753/T0759/T0761/T0769 are undisputed keeps | If the merge is taken, T0761's table must gain an HEVC positive row (a line at started+1000 with codec HEVC) so the accepted-preload half of r1/r2:A9-6 keeps a positive pin; if the claim is taken, nothing changes. |
| X2#11 | finding_group | T0242, T0481, T0505, T0541 | keep | CONFLICT the two-reader agreement assertions (preflight.rs:1021-1030, :1046-1051) — read_toml_facts vs config::runtime_toml::read on the same file — exist nowhere else | If the merge is taken, the surviving carrier must still compare read_toml_facts against crate::config::runtime_toml::read on one shadowed file; T0505 only exercises read_toml_facts through `run`. |
| X2#12 | finding_group | T0242, T0456, T0481, T0505 | keep | CONFLICT same as claim #11 — preflight.rs:1021-1030,:1046-1051 two-reader agreement; the r1-vs-r2 A7-1 split itself is correct | The bare `A7-1` doc tags are ambiguous: T0456 belongs to r1:A7-1 and T0242/T0481/T0505 to r2:A7-1. Under 3.6 any NEW label here must be written `r2:A7-1`; do not let a cut treat the four as one finding's four copies. |
| X2#13 | finding_group | T0393, T0726, T0809, T0833 | keep | CONFLICT the claim's own thesis is sound; what is disputed is whether T0809 survives — its fatal-with-remedy continuation-line bytes (main.rs:1866-1878) plus the seven-space-indent equality with fail_row (:1881-1884) | Whatever is decided, the `FATAL <msg>` / `       remedy: <r>` shape is doctor/lib.sh die-shaped output; if T0809 goes, the carrier must still pin the seven-space continuation indent shared with fail_row. |
| X2#15 | finding_group | T0246, T0247, T0248 | keep | CONFLICT the only fact T0246 could still own is that runtime_toml::write's absent-file branch goes through create_new (O_EXCL) rather than write_atomic — and the test as written does NOT exercise that branch; it drives ex.create_new directly (runtime_toml.rs:2766-2772) | If T0246 is deleted, the r1:A10-1 O_EXCL-publication fact loses its named home even though its assertions survive piecemeal; the deleter must name T0019 (executor.rs:1400-1412) and T0250 (runtime_toml.rs:2885-2910) per 3.4(b), and 3.6 wants |
| X2#24 | finding_group | T0631, T0682, T0763 | keep | CONFLICT sabrage/crates/sabrage-core/src/store/goldberg.rs:569-572 assert!(session::live_session_reason(&paths).is_none()) — the vertical would drop it as fixture restatement, but it calls production code and is the control that proves the later refusal comes from the argv probe, not from a file signal | Group relation verified: the three tests protect three different units. Applier must keep goldberg.rs:569-572 (live_session_reason baseline) even if goldberg.rs:565-568 (session_state_path().exists()) is dropped as a pure §3.7 setup restate |
| X2#25 | finding_group | T0694, T0703, T0765 | keep | CONFLICT sabrage/crates/sabrage-core/src/session/reconcile.rs:1583 assert_eq!(restore_mode(Classification::Unverifiable), None) and :1584-1587 assert!(!signalable(&unobserved)) — neither pure-function fact is asserted by T0704 (:2007-2034), T0692 (:1533-1542) or T0693 (:1544-1562); T0704 only observes the consequence via ctx.executor.planned().is_empty() at :2027-2030 | If the vertical merge is applied anyway, the merged carrier must re-assert restore_mode(Unverifiable)==None and !signalable(start_time=0 live pid); the classify(recycled)==IdentityMismatch tail at reconcile.rs:1589-1592 is genuinely a dupli |
| X2#27 | finding_group | T0764, T0766, T0769 | keep | CONFLICT sabrage/crates/sabrage-core/src/session/watcher.rs:2074 assert!(!s.runtime_fresh) in the Stalled state — T0764's Stalled half asserts only the phase (:1388) and its !runtime_fresh assertion (:1375) is on the Running branch, so 'assertions_lost: none' is wrong unless that assertion is moved into T0764 | If the Stalled block (watcher.rs:2037-2075) is cut, T0764 must gain assert!(!s.runtime_fresh) beside its Stalled assertion at :1388; the 20 000 ms staleness literal at :2054/:2066 is the only near-stall-grace input in the tree, T0764 uses 6 |
| X2#31 | finding_group | T0405, T0503 | keep | CONFLICT Nothing is lost between T0405 and T0503 — the claim's own pairwise relation is sound. The conflict is with the vertical: T0503's only assertion (preflight.rs:1829-1833) is character-identical to T0494's first assertion (:1599-1604), the two tests differing solely in the toml literal, which is the §3.3 table case. | If the vertical's table is applied, the A2/A7-2 regression comment at preflight.rs:1815-1818 must become the row label for the shadowed-toml row (§3.6: the assertion may move, never vanish), and the table must not swallow T0494's extra two  |
| X2#35 | finding_group | T0455, T0459 | keep | CONFLICT T0455: vertical merge vs claim keep | If the merge is ever applied, T0455's third case (prev_audio_output None with audio_restored false carries nothing) and the r1:A9-2 label must both move onto T0459, which today carries the r2:A9-2 label. |
| X2#37 | finding_group | T0809, T0884 | keep | CONFLICT T0809: vertical delete vs claim keep | The grouping half of this claim is correct regardless of T0809's fate: 'Finding #4' at sabrage/crates/sabrage-cli/src/main.rs:1861 and at sabrage/src-tauri/src/commands.rs:2357 are unrelated section numbers and must never be treated as one  |
| X3#2 | cross_layer_dup | T0204, T0860 | drop_assertion | CONFLICT T0204: vertical merge (carrier T0860, move the 3 edge rows into parity and drop the core test) vs claim drop_assertion (keep T0204 in core holding those 3 rows) | Row-by-row the subsumption is exact — if the claim is applied, delete core winpath.rs lines 61-68, 79-83, 84-89, 90-94 and 95-96 only, and keep the spaces row (:69-78), the empty-prefix row (:97-101) and the drive_cache-sibling row (:102-10 |
| X3#3 | cross_layer_dup | T0395, T0869 | drop_assertion | CONFLICT T0395: vertical delete (carrier T0869, 'the four core assertions are subsumed') vs claim drop_assertion (remove only the equality, keep three assertions) | If applied, remove exactly sabrage/crates/sabrage-core/src/stages/run/actions.rs:942-947 (the `from_contract` binding plus `assert_eq!(LAUNCH_ACTION_IDS.to_vec(), from_contract)`), keeping :949-952. `LAUNCH_ACTION_IDS[6]` stays safe because |
| X3#6 | finding_group | T0005, T0572 | merge | CONFLICT T0005: vertical delete (carrier T0572) vs claim keep-as-survivor; T0572: vertical merge (carrier T0005) vs claim delete — the two vertical rows point in opposite directions and cannot both be applied | If T0572 goes, remove its doc comment too (sabrage/crates/sabrage-core/src/checks/meta.rs:275-284) — that comment is the only place the phrase 'the A1-1 regression test' names the unchanged-behaviour half, and it would be false once the tes |
| X3#8 | fixture_ceremony | T0302, T0303 | delete | CONFLICT T0302: vertical merge (bucket F, carrier T0031) vs claim keep-as-survivor | Deleting T0303 is safe and loses nothing; but T0302 is NOT 'the real planned-copy behavior' — it also hand-rolls the branch (ctx.executor.dir_copy + ctx.step(...).ok at install.rs:1008-1010) and never enters install::run. If T0302 is merged |
| X3#11 | architect_map | T0513, T0520, T0552, T0562, T0588, T0610 … | delete | CONFLICT T0576: vertical delete (carrier T0867) vs claim keep-as-survivor | The eight-test reduction is safe under either survivor. If T0576 also goes, T0867 (sabrage-parity/src/lib.rs:1341-1358, CI-run) is the legal carrier and §3.5 favours it ('parity wins ties; a parity test never loses to a core carrier'). Owne |
| X3#15 | architect_map | T0537, T0538, T0539, T0540, T0541, T0542 | table | CONFLICT T0541: vertical keep vs claim table-row | Three things for the critic and the applier. (1) The accepted V4-3 program itself sides with the vertical: 2026-08-31-simplification.md:807 says 'leave shadowed_protocol_alvr_then_oxrsys_resolves_to_the_last_assignment … as standalone fns w |
| X3#18 | architect_map | T0754, T0755, T0757 | table | CONFLICT T0757: vertical keep vs claim table | If the fold goes ahead, resolve the two fixture lines OUTSIDE the table (let hevc = find("(HEVC, native helper)"); let h264 = find("(H.264, in-process)"); let bare = "OXRSys/ALVR: encoder ready 2064x2208 @72Hz 100Mbps (HEVC, native helper)" |
| X3#19 | architect_map | T0780, T0781, T0782, T0785, T0786, T0787 … | table | CONFLICT The claim's fourth table ('empty-chain merging at T0803') cannot be one table: T0803/T0804 (main.rs:1746-1777) call merge_stage_options(StageOptions, &StageArgs) and T0805/T0806 (:1779-1807) call merge_doctor_options(CheckOptions, &DoctorArgs). Different units with different argument types — one table would need an enum plus a branch per row (forbidden by 3.3) and would fuse two units 3.4 explicitly warns about. It must be split into two tables (stage merge, doctor merge), making the claim's 'four independent tables' actually five. | Legal shape of the surviving tables: (1) doctor parse errors T0780/T0781/T0782 → rows of (label, argv, expected_err) on parse_doctor_args; the comment at :1450-1451 ('a bare positional hits the same *) branch in demo.sh') must survive as th |
| X3#21 | harness_self_test | T0840, T0841, T0842, T0843, T0844, T0845 … | keep | CONFLICT T0852: vertical merge (carrier T0848) vs claim keep; T0853: vertical merge (carrier T0855) vs claim keep; T0840/T0841/T0842/T0843/T0844/T0845/T0846: vertical table (carriers NEW-V14-SLUG-COMMENTS, NEW-V14-SLUG-LOOPS) vs claim keep | The claim's blanket 'keep' and the verticals' 'table' for T0840-T0846 are not actually incompatible: a labelled table of (label, fixture, expected order, expected header_only_loops) keeps every scanner fixture alive and satisfies 3.7's requ |
| X4#1 | cross_layer_dup | T0572, T0005 | delete | CONFLICT T0005: vertical delete vs claim keep | Exactly one of T0572 and T0005 may be cut. If T0005 goes instead, `COMPILED_CONTRACT_SHA256.len() == 64` (contract.rs:376) loses its only assertion unless T0199 is verified to carry it. |
| X4#16 | fixture_ceremony | T0582, T0583 | table | CONFLICT T0583: vertical keep vs claim table (T0582 also vertical keep) | If tabled, the adb row must inject `c.paths.adb = None` (precedent: checks/network.rs:266-269) and the rows differ in two setup fields plus two skip reasons ('probes disabled' vs 'adb not found'), neither of which either test asserts today. |
| X4#18 | cross_layer_dup | T0395, T0869 | drop_assertion | CONFLICT T0395: vertical delete (carrier T0869) vs claim drop_assertion (keep T0395) | Dropping the equality at actions.rs:947 also strands `from_contract` (actions.rs:942-946), so the edit is 6 lines, not the claimed 1; and T0395 has no 'uniqueness' assertion — the three it keeps are `contains("audio-route")`, `contains("das |
| X4#19 | cross_layer_dup | T0007, T0856, T0857 | drop_assertion | REJECT assert!(HOST_MANIFEST_TEMPLATE.ends_with('\n')) — sabrage-core/src/contract.rs:415; no other test fails if the template loses its trailing newline | Half the claim is sound: the placeholder assertion at contract.rs:414 is genuinely subsumed (util/mod.rs:421-423 and parity lib.rs:1108-1130 both fail if `@OXR_DYLIB@` disappears). The newline assertion at contract.rs:415 is not, and it is  |
| X4#20 | fixture_ceremony | T0235 | drop_assertion | CONFLICT T0235: vertical merge (carrier T0236) vs claim drop_assertion (keep T0235) | The parse guard is runtime_toml.rs:2407-2410 (4 lines), not a single line at :2407; the fixture literal it guards is :2405-2406 and must stay. |
| X4#29 | harness_self_test | T0840, T0841, T0842, T0843, T0844, T0846 … | keep | CONFLICT T0853 (sabrage/crates/sabrage-parity/src/lib.rs:1036-1051) and T0854 (:1053-1058) do not invoke the scanner decision their names claim: neither calls group_verb_errors (defined at lib.rs:845-911) nor tag_gate_map. T0853 builds two local std Regex objects (lib.rs:1040-1041) and asserts the group body contains/lacks the word `die`; T0854's single assertion, `assert!(!Regex::new(r"\bdie\b").unwrap().is_match(&g.body))` at lib.rs:1057, only proves the string literal the test itself wrote two lines earlier has no `die` in it — a fact created by the test's own setup, forbidden outright by docs/code-standards.md 3.7, and nothing is 'rejected' anywhere in either test. So the claim's blanket 'assertions_lost: None' plus the 3.7 scanner carve-out does not cover T0853/T0854: the 3.7 carve-out protects self-tests of scanners that decide whether a gate fired, and these two exercise std Regex, not the deciding code. | T0847, T0850 and T0855 are safe to keep as-is (verticals agree). For the six slug_coverage rows the disagreement is form, not substance: the vertical's `table` verdict keeps every literal as a labelled row, so nothing is lost either way, EX |
| X4#30 | finding_group | T0845 | keep | CONFLICT T0845: vertical table (carrier NEW-V14-SLUG-LOOPS) vs claim keep | The substance is not in dispute — the vertical's `table` verdict also preserves T0845's fixtures and its label, so neither side loses the regression. If the table route is taken, the row label must carry the id (3.6: 'When a behaviour table |
| X5#1 | mirror | T0114, T0115 | merge | CONFLICT After the proposed rewrite nothing unconditionally exercises the public convenience functions themselves: find_processes_by_exe (process.rs:653-655) loses its only test (T0114's positive own-pid match plus /nonexistent negative, process.rs:1325-1336), and find_processes_by_cmdline (process.rs:681-683) is called in T0123 only inside `if !filter.is_empty()` (process.rs:1515-1521), which is false under a plain `cargo test` run. A mutant that made either delegate return Vec::new() would survive. | Both directions are defensible; a critic must pick one. If the claim's direction wins, the rewritten T0115 must call the convenience functions (not only ProcessScan) for the own-exe and nonexistent-exe rows, or T0114 must stay. |
| X5#6 | mirror | T0296, T0297 | delete | CONFLICT `assert_eq!(ctx.executor.planned().len(), 1)` at stages/build.rs:1309 is a unique mutant kill. run_ninja_build_ok's real-run path does NOT go through ctx.executor at all — it ends in `process::run_ok(&spec, &sink, &ctx.cancel)` at stages/build.rs:409, a real spawn. Delete or invert the `if ctx.executor.is_dry_run()` guard at stages/build.rs:381-383 and T0296 still passes (it calls run_child_ok, a different function) while a dry run really spawns /bin/false and planned() is 0; only T0297 fails. T0299 (stages/build.rs:1347+) is a real-run test and cannot catch it either. | If a later pass still wants the 9 lines, the safe form is a two-row table over (fn under test, expected plan length) that keeps a dry-run assertion for run_ninja_build_ok — not a deletion. |
| X5#9 | mirror | T0780, T0797 | merge | CONFLICT sabrage-cli/src/main.rs:1644-1651 — parse_stage_args(&args(&["--bottle"])).unwrap_err() == "error: --bottle needs a name" (and the --bs-dir twin) is the ONLY assertion anywhere that parse_stage_args propagates parse_common_flag's Err through the `?` at main.rs:656 instead of falling through to its own `other => return Err(format!("error: unknown argument '{other}'"))` arm at main.rs:669. T0780 calls parse_doctor_args exclusively (main.rs:1433, :1437, :1441) and would stay green under that mutation. | If the merge is applied anyway, the folded table must call BOTH parsers per row (a per-row fn pointer, not a branch) so the stage-side propagation assertion survives; the two verbatim strings are demo.sh:32 and demo.sh:34 bytes with no sabr |
| X5#10 | harness_self_test | T0840, T0841, T0844, T0845, T0846 | keep | CONFLICT T0840: vertical table vs claim keep; T0841: vertical table vs claim keep; T0844: vertical table vs claim keep; T0845: vertical table vs claim keep; T0846: vertical table vs claim keep | No assertion is at risk on either side — the disagreement is form (five standalone fns vs two labelled tables), not coverage. If the vertical's tables are built: rows need TWO expected columns (Scan.order and header_only_loops.len()) or the |
| X5#11 | finding_group | T0385, T0387 | keep | CONFLICT T0385: vertical table vs claim keep; T0387: vertical table vs claim keep | Neither side loses an assertion. If the two-row table is built, the A5-2 label must become the row label (3.6: the regression is a row, not a function) and must be written as the round-qualified 'r2:A5-2' since A5-2 exists in both rounds; t |
| X5#12 | finding_group | T0539, T0541 | keep | CONFLICT T0539: vertical table vs claim keep | The load-bearing half of the claim is not in dispute: T0541 must survive as an individually named fn — vertical says keep, and the accepted program says so explicitly (2026-08-31-simplification.md:171 lists shadowed_protocol_alvr_then_oxrsy |

Missed duplicates flagged by the keep verifiers (a `keep` the reviewer gave that a verifier believes is a duplicate):

| test | partner | proposed | outcome | note |
|---|---|---|---|---|
| T0041 `wine_log_candidate_stamped_takes_no_date_time_type_at_all` | T0040 | merge | merge (second-pass) | Apply this as the real fold the carrier_note describes, not as a bare delete: move T0041's two absolute literals into T0040 (turn `for attempt in [0, 1, 3]` at logs.rs:776 into a labelled `(attempt, expected_full_path)` table asserting stam |
| T0101 `splits_on_lf_cr_and_crlf` | T0102 | drop_assertion | drop_assertion (second-pass) | Drop only process.rs:954, 958 and 962 (and their inline comments at 957 and 961); keep at least the 956/960/963/964 rows so the terminator-blind push/finish delegations that logs.rs:331,499,511 actually call retain a mutant kill. |
| T0254 `backups_are_pruned_to_the_newest_ten` | T0257 | drop_assertion | drop_assertion (second-pass) | Drop only runtime_toml.rs:3017-3020 and trim the comment on runtime_toml.rs:3012 to "The three oldest went." — its "the new one is newest" clause is backed by no other assertion in the test once the ordering assert goes. |
| T0479 `every_gated_slug_is_accounted_for` | T0179 | drop_assertion | drop_assertion (second-pass) | When dropping preflight.rs:953-959 also drop the explanatory comment at preflight.rs:950-952 (it only explains that assertion) and collapse the now single-assert `Gate::Autofix => { … }` block; leave the `use crate::fixes::{… FixAction …}`  |
| T0518 `bottle_template_warns_with_an_empty_parenthetical_when_the_key_is_absent` | T0517 | table | keep (not re-verified) | Fold the absent-key case into a labelled `bottle_template` row-table with T0517's template assertions; the row must stay, it is the only kill for `unwrap_or("")`. |
| T0592 `stale_overlay_remedy_uses_the_name_placeholder_without_a_bottle` | T0591 | delete | drop_assertion (second-pass) | Apply only the one-line trim — delete `assert_eq!(o.status, CheckStatus::Fail);` at overlay.rs:206 (implied by the remedy assert, since only CheckOutcome::fail sets a remedy) and keep the rest of T0592 including the comment at overlay.rs:20 |
| T0821 `colors_from_is_independent_per_stream` | T0820 | table | table (second-pass) | Fold T0820 + T0821 into one three-row `(label, (no_color, stdout_tty, stderr_tty), Colors)` table under the verbatim `── A14-5: color gating is per-stream ──` header (main.rs:2233), carrying T0821's 'other half of the same bug' prose into i |

---

## 5. Contradictions found

Duplicate-looking tests can hide two units that deliberately disagree. The index computed every pair of tests that feed the same input literal to a function and expect different values; the reviewers reported the contradictions they saw while reading; the critic de-duplicated and adjudicated them. None of them is a bug in the Rust code. The only one that needs an owner decision is the one the standard already names: `checks::config::parse_protocol` mirrors doctor.sh's last-match `awk` and `stages::setup::parse_protocol_awk` mirrors setup.sh's first-match `awk`, so the shell disagrees with itself on a file with two `protocol` lines. Aligning the two recipes touches the shell and the fingerprint and is not part of this program.

| tests | kind | fact A | fact B | adjudication | owner decision |
|---|---|---|---|---|---|
| T0039, T0040 | same_function | wine_log_candidate(/repo/logs, .., 1) == /repo/logs/beatsaber-20260829-101112-2.log and (.., 3) == -4.log (T0039, crates/sabrage-core/src/logs.rs:752-765). | T0040 at crates/sabrage-core/src/logs.rs:767-783 pairs the same /repo/logs with the bare stamp 20260829-101112 and no absolute path. | Refuted. The index keyed only on the logs_dir argument; the differing argument is `attempt` (0/1/3) and, in T0040, the expected value is the stamped form rather than a literal. Same function, different inputs, all outputs consistent with logs.rs:71-78. |  |
| T0076, T0774 | two_implementations | shell_quote("it's") == 'it'\''s' (crates/sabrage-core/src/privilege.rs:288-301, /bin/sh argv). | zsh_scalar("it's") == "it's" while zsh_word("it's") == 'it'\''s' (crates/sabrage-contract-gen/src/lib.rs:259-286). | Confirmed as V14 adjudicated: three encoders for three grammars — /bin/sh argv, a zsh scalar assignment RHS, and a standalone zsh word. zsh_scalar keeps the historical double-quoted form because an apostrophe is literal inside "…"; zsh_word must survive word splitting, so it single-quotes. No semant |  |
| T0163, T0345, T0536 | unknown | For the text `protocol = "alvr"\n`, encoder_process_or_default returns "auto" (crates/sabrage-core/src/fixes/helper.rs:94-101). | For the same text the protocol parsers return "alvr" (crates/sabrage-core/src/stages/setup.rs:350; crates/sabrage-core/src/checks/config.rs:82). | Refuted — index artifact. encoder_process_or_default reads the `encoder_process` key, absent from that fixture, so `${ENCODER_PROC:-auto}` applies; the parsers read `protocol`. Different keys, different callees, no disagreement. |  |
| T0198, T0774 | two_implementations | json_escape_string escapes only backslash and double quote, deliberately leaving control characters alone (crates/sabrage-core/src/util/mod.rs:243-251). | zsh_scalar single-quotes anything containing a control character (crates/sabrage-contract-gen/src/lib.rs:259-267). | Refuted. Two different target grammars (install.sh's two-substitution JSON escape vs a zsh scalar RHS); the JSON one is intentionally partial for artifact-byte parity with the shell, documented in place. No shared contract to contradict. |  |
| T0241, T0242 | two_implementations | effective_string("protocol = \"alvr\"\nprotocol = \"banana\"\n", "protocol") == banana — raw last-wins (crates/sabrage-core/src/config/runtime_toml.rs:726-731). | effective_accepted on the same bytes == alvr — the last value Config.cpp's whitelist would accept (crates/sabrage-core/src/config/runtime_toml.rs:753-759). | Confirmed as V01 adjudicated. Two deliberately different readers with the divergence documented at runtime_toml.rs:734-744: effective_string answers 'what does the last line say' for unmodelled keys, effective_accepted answers 'what is the runtime holding' for the six modelled keys. Both stay. |  |
| T0241, T0345, T0536 | shell_recipes_differ | stages::setup::parse_protocol_awk returns "first" for two protocol assignments and returns empty for an unquoted `protocol = alvr` (crates/sabrage-core/src/stages/setup.rs:350-364). | checks::config::parse_protocol returns "second" for the same two lines (crates/sabrage-core/src/checks/config.rs:82-97), while config::runtime_toml::effective_string returns the last value and unquote | Confirmed; de-duplicates V01(T0241,T0536), V01(T0241,T0345), V08(T0345,T0536) and V11(T0536,T0345) into one three-way fact. The shell genuinely disagrees with itself: scripts/demo/setup.sh:45 is `{print $2; exit}` (first match) while scripts/demo/doctor.sh:182 and scripts/demo/run.sh:60 are `{v=$2}  | Optional and out of scope for a test scan: the owner may want setup.sh:45 unified onto the last-match recipe, but that is a pipeline behaviour change that must land on both front-ends in one commit. |
| T0235, T0236 | two_implementations | toml_edit parses `note = """\nprotocol = "oxrsys"\n"""` as valid TOML whose root protocol is alvr (asserted at crates/sabrage-core/src/config/runtime_toml.rs:2407-2410). | read_lines_like_the_runtime, mirroring Config.cpp's physical-line reader, returns Protocol::Oxrsys for the same bytes (runtime_toml.rs:2412-2416, 2430-2434). | Confirmed as V01 adjudicated, with the refinement that this is not a cross-test contradiction at all: both halves are asserted inside each single test, deliberately, as the ground truth for the A10-3/A10-4 refusal. T0235 keeps the view-level refusal, T0236 keeps the apply_patch/write/edit_protocol r |  |
| T0237 | two_implementations | Handed the BOM-stripped body, toml_edit sees an editable root `protocol` key. | read_lines_like_the_runtime sees `\u{feff}protocol` and returns None, so the runtime default applies (crates/sabrage-core/src/config/runtime_toml.rs:2482-2489). | Confirmed as V01 adjudicated. Mirror image of the multiline case and, again, both facts live inside the one test on purpose: the runtime answer is preserved and the rewrite is refused (runtime_toml.rs:2491-2510). Do not normalize the reader. |  |
| T0374, T0431, T0720 | two_implementations | 'MacBook Pro Speakers' appears as the expected value of stop.rs's audio_report (crates/sabrage-core/src/stages/stop.rs:684). | The same string is embedded in longer expected lines from guards.rs's audio_switched_line/audio_restored_line (crates/sabrage-core/src/stages/run/guards.rs:166,173) and reconcile.rs's audio_row/audio_ | Refuted — index artifact. Five distinct renderers in three modules take the device name as an input and each wraps it in its own verbatim message; the collision key was the device name, not the function's output contract. |  |
| T0412, T0414, T0624, T0625, T0626, T0627, T0628, T0629, T0630, T0631, T0633, T0634, T0635 | same_function | Reading steam_api64.dll / steam_api64.dll.orig-steam yields REAL-STEAM-BYTES in one test. | Reading the same filenames yields GOLDBERG-EMULATOR-BYTES, CUSTOM-GOLDBERG-BUILD, GOLDBERG-NOW, etc. in others (crates/sabrage-core/src/store/goldberg.rs:347, 382, 403, 448). | Refuted — index artifact. The 'input' the index keyed on is the filename; the fixture payload each test writes into that filename before reading it back is the actual input and differs per test by design. |  |
| T0444 | two_implementations | wine_exit_line(139, /l.log) == 'wine exited with status 139 (log: /l.log)' (crates/sabrage-core/src/stages/run/mod.rs:1027-1029). | detached_line(/l.log) == '-- detached: leaving the session running (log: /l.log)' (crates/sabrage-core/src/stages/run/mod.rs:1035-1040). | Confirmed benign as V03 adjudicated. Two distinct renderers for two different supervision outcomes that share only the log path argument; wine_exit_line is run.sh:269 verbatim, detached_line is Sabrage-only. Their texts must differ. |  |
| T0636, T0664 | two_implementations | library_path(/x/Sabrage) == /x/Sabrage/library.json (crates/sabrage-core/src/store/library.rs:181-183). | settings_path(/x/Sabrage) == /x/Sabrage/settings.json (crates/sabrage-core/src/store/settings.rs:161-163). | Confirmed benign as V10 adjudicated. One base directory feeding two distinct path constructors; each filename is that constructor's public contract. |  |
| T0781, T0794, T0798, T0811 | unknown | 'Steam' as an argv element yields "error: unknown argument 'Steam'" from the CLI arg parsers (crates/sabrage-cli/src/main.rs:274, 650). | 'Steam' as a bottle label yields doctor footers and stage closing lines that embed it (crates/sabrage-cli/src/main.rs:526, 548). | Refuted — index artifact. The same token plays two different roles (a stray positional argument vs the bottle_label parameter) across four unrelated functions. |  |
| T0794, T0811, T0812, T0815, T0816, T0817, T0818, T0819, T0822 | same_function | With the literal '<name>' as input, one test expects '\nbuild complete — next: ./demo.sh install --bottle <name>'. | With '<name>' the stage_event_lines tests expect '== wine-vr demo run ==', '-- Goldberg', ' 42%', 'FATAL boom', '  FAIL copy failed', and so on (crates/sabrage-cli/src/main.rs:712-800, tests at 1954-2 | Refuted — index artifact. '<name>' is the constant bottle_label argument in every one of these calls; the varying input is the StageEvent (or the Stage), which the collision key never captured. stage_event_lines is one function rendering many event kinds. |  |
| T0138 | two_implementations | remove_adb_forwards refuses with 'refusing to remove adb port forwards while a session is live' and never spawns adb (crates/sabrage-core/src/fixes/adb.rs:721-727). | remove_adb_forwards_at removes the forward in the same live-session fixture (crates/sabrage-core/src/fixes/adb.rs:730-734). | Confirmed as V09 adjudicated, with the refinement that both halves are asserted inside the single test T0138 on purpose. Deliberate entry-point split: the standalone Doctor button is gated; launch preflight must clear leftover --wired forwards before publishing its own session. |  |
| T0145 | shell_recipes_differ | rewrite_graphics_backend on the bare header '[EnvironmentVariables]' returns '[EnvironmentVariables]\n"CX_GRAPHICS_BACKEND" = "dxmt"' with no trailing newline (crates/sabrage-core/src/fixes/backend.rs | Measured BSD `sed -i '' '/^\[EnvironmentVariables\]$/a\…'` concatenates onto the header and adds a trailing newline the file never had (documented at backend.rs:76-90 and 421-435). | Confirmed as V09 adjudicated. This is the measured finding-#10 improvement over sed in exactly one cell; every other cell is sed-identical. It must remain a labelled row in the NEW-backend-rewrite-cases table, never normalized to sed's output. |  |
| T0143, T0155 | two_implementations | rewrite_graphics_backend returns Branch::Rewrote with an unquoted value left untouched and no target anchor (crates/sabrage-core/src/fixes/backend.rs:395-407). | set_graphics_backend rejects those same bytes with 'could not force graphics backend to dxmt in ' and writes nothing (crates/sabrage-core/src/fixes/backend.rs:620-645). | Confirmed as V09 adjudicated — two layers per docs/code-standards.md §3.5. The pure helper models sed's transformation faithfully; the public fix adds the postcondition that stops a false success. Both stay. |  |
| T0158, T0159 | two_implementations | set_graphics_backend refuses with 'live wineserver' and leaves cxbottle.conf untouched (crates/sabrage-core/src/fixes/backend.rs:705-730). | set_graphics_backend_for_launch edits successfully under the identical fixture (crates/sabrage-core/src/fixes/backend.rs:739-757). | Confirmed as V09 adjudicated, and documented in place at backend.rs:735-738: run.sh rewrites cxbottle.conf in preflight and kills that wineserver two blocks later, so launch must not be blocked, while a standalone edit must survive CrossOver's later writeback. |  |
| T0151, T0192 | two_implementations | wineservers_indicate_live is false when every observed WINEPREFIX belongs to a different bottle (crates/sabrage-core/src/fixes/backend.rs:512-519). | any_wineserver_alive blocks deleting ALVR session.json regardless of WINEPREFIX (crates/sabrage-core/src/fixes/session_json.rs:416-432). | Confirmed as V10/V09 adjudicated. Two predicates for two scopes: cxbottle.conf is bottle-scoped so a different WINEPREFIX can be ruled out, whereas session.json is machine-global so no wineserver can be ruled out. The comment at session_json.rs:422-427 states this explicitly. |  |
| T0640, T0667 | two_implementations | A library.json carrying futureField loads as Library::default() — unknown fields dropped (crates/sabrage-core/src/store/library.rs:705-715). | A settings.json carrying futureField/anotherOne loads them into Settings::extra for re-emission (crates/sabrage-core/src/store/settings.rs:273-285). | Confirmed as V10 adjudicated. Intentional store-policy difference: library relies on schema-version refusal for incompatible formats, settings supports lossless opaque-key passthrough for the downgrade case. Both stay; T0667's separately-approved merge into T0668 (which round-trips the same fields)  |  |
| T0544, T0550 | same_function | cfg_session_pins on `{not json` warns with the native serde_json parse error and must not mention python3 (crates/sabrage-core/src/checks/config.rs:573-596). | cfg_session_pins on `[1,2,3]` warns with the shell-style 'could not inspect … (broken python3?)' (crates/sabrage-core/src/checks/config.rs:721-733). | Refuted as a contradiction, confirming V11's disposition: same function, different inputs. Unparseable bytes fail in native Rust so the r2 A3b-3 assertion forbids blaming python3; parseable-but-wrong-shape mirrors the shell walk outside its try/except and keeps the shell's exact diagnosis. Both stat |  |
| T0557, T0607, T0609, T0397 | two_implementations | checks/headset.rs defines its own first_connected_serial fed by an unbounded Command::output probe (headset.rs:45). | checks/run_only.rs defines a second first_connected_serial behind a deadline-killed child (run_only.rs:199), and stages/run/actions.rs a third parser behind cancellable capture_with, with matching lit | Confirmed as V12 adjudicated, and the current program already honours it: T0557 becomes a table built only from headset.rs, T0609 is deleted into T0605 inside its own run_only.rs module, and T0397 and T0607 are untouched keeps. No cross-module carrier was used, so the post-r1:A7-4 probe-blocking div |  |
| T0586, T0587 | shell_recipes_differ | net_adb_forwards Warns with 'could not query adb port forwards' when the probe cannot spawn adb, explicitly not 'no stale adb port forwards' (crates/sabrage-core/src/checks/network.rs:264-278). | doctor.sh ignores the `adb forward --list` pipeline's exit status, sees empty output, and taps net.adb-forwards ok. | Confirmed as V12 adjudicated. This is the declared divergence at sabrage/PARITY.md:117 plus the r2:A4-4 regression; the native Warn is the carrier and is not duplicate coverage of the shell's clean-state row. T0587 additionally pins the helper's nonzero-exit Err text, a different assertion again. |  |
| T0064 | unknown | The scan's scope note attributes the Tailer rotation tests to r1:A8-4. | r1 A8-4 is 'The guarded audio action permanently overwrites BlackHole's volume' (sabrage/docs/reviews/2026-08-30-codex-round1.md:878); the rotation finding is r2 A8-4, 'Rotation reports previous-file  | Confirmed as V17 adjudicated: classify T0064 as r2:A8-4 by file, fix sketch and blame. Scan-report bookkeeping only — the bare 'A8-4' labels already in crates/sabrage-core/src/logs.rs:246,1332,1351 are NOT rewritten, because docs/code-standards.md §3.6 says existing bare labels stay and resolve thro |  |
| T0065 | unknown | The test comment names the read-window race A8-7 (crates/sabrage-core/src/logs.rs:1215). | The round-2 finding whose fix sketch requires this test-only hook is r2:A8-3, titled '[unfixed] A8-7 The continuity signature still has a truncate/read race' (sabrage/docs/reviews/2026-08-30-codex-rou | Confirmed as V17 adjudicated: the round-qualified id is r2:A8-3 and A8-7 is the inherited round-1 defect name carried inside the round-2 title. Again bookkeeping only; §3.6 keeps the existing bare A8-7 label in the code. |  |

As reported by the reviewers (before the critic's de-duplication):

| reviewer | tests | kind | A | B | adjudication |
|---|---|---|---|---|---|
| V01 | T0241, T0242 | two_implementations | For `protocol = "alvr"` followed by `protocol = "banana"`, effective_string returns raw-last `banana`. | For the same input, effective_accepted returns last-accepted `alvr`. | Both stay. T0241 is the intentionally unfiltered reader for unmodeled keys and diagnostic fallback; T0242 is the whitelist-aware reader for modeled runtime state. |
| V01 | T0235, T0236 | two_implementations | toml_edit treats the multiline-string fixture as valid TOML with the assignment-shaped line inside string content. | Config.cpp's physical-line reader treats that same embedded line as a live protocol assignment. | The disagreement is deliberate ground truth, not duplication. The view reports it as non-round-trippable and all mutation entry points must refuse it. |
| V01 | T0237 | two_implementations | After leading-BOM stripping, toml_edit sees a root `protocol` key. | Config.cpp's raw line reader sees `U+FEFFprotocol` and therefore no protocol occurrence. | Do not normalize the reader. Preserve the runtime answer and refuse the unsafe rewrite. |
| V01 | T0241, T0536 | shell_recipes_differ | The runtime-oriented effective_string accepts an unquoted `protocol = alvr` as `alvr`. | checks/config.rs's doctor.sh-compatible quote-field parser returns an empty value for the same line. | Both are load-bearing implementations of different consumers. T0241 mirrors Config.cpp; T0536 mirrors doctor.sh's `awk -F'"'` recipe. |
| V01 | T0241, T0345 | shell_recipes_differ | The runtime-oriented reader uses the last assignment across tables. | setup.sh's parser mirror returns the first matching assignment. | Keep both and preserve the divergence: setup.sh uses an exit-after-first awk recipe, whereas Config.cpp and the runtime reader scan the whole file. |
| V03 | T0444 | two_implementations | wine_exit_line(139, /l.log) renders "wine exited with status 139 (log: /l.log)". | detached_line(/l.log) renders "-- detached: leaving the session running (log: /l.log)". | The conflict-candidate is benign: the same log literal is supplied to renderers for two different supervision outcomes, so their text must differ; the functions are distinct at stages/run/mod.rs:1027 and :1035. |
| V08 | T0345, T0536 | two_implementations | stages::setup::parse_protocol_awk returns `first` for two matching protocol assignments. | checks::config::parse_protocol returns `second` for the same two-line input. | Keep both. T0345 mirrors setup.sh's first-match `awk ... { print $2; exit }`, while T0536 mirrors doctor.sh's last-match recipe; their six shared literals do not make the opposite seventh assertion redundant. |
| V09 | T0138 | two_implementations | remove_adb_forwards refuses and does not spawn adb during a live session. | remove_adb_forwards_at proceeds and removes the forward in the same live-session fixture. | Deliberate entry-point split: the standalone Doctor door is gated, while launch preflight must clear leftovers before publishing its own session. |
| V09 | T0145 | shell_recipes_differ | The Rust insertion preserves the missing final newline and inserts a real line break after the section header. | The measured BSD sed recipe concatenates the inserted key onto a final unterminated header and adds a newline. | This is the documented F#10 improvement and must remain a labelled table row, not be normalized to shell output. |
| V09 | T0143, T0155 | two_implementations | The pure sed-fidelity transformer returns Branch::Rewrote with malformed bytes unchanged and no target anchor. | The public fix rejects those transformed bytes and writes nothing. | Both are correct at their layers: the helper models sed's transformation, and the public fix adds the postcondition that prevents false success. |
| V09 | T0158, T0159 | two_implementations | The standalone backend fix refuses and preserves cxbottle.conf when the bottle's wineserver is live. | The launch backend variant edits successfully under the same live-wineserver state. | The policy difference is intentional: launch resets that wineserver immediately afterward, while a standalone edit must survive CrossOver's later writeback. |
| V09 | T0151, T0192 | two_implementations | Known wineservers for only different WINEPREFIX values do not block a bottle-scoped backend edit. | Any wineserver blocks deletion of machine-global ALVR session.json, regardless of WINEPREFIX. | These operate on differently scoped state; bottle-specific cxbottle.conf may narrow ownership, while session.json cannot. |
| V10 | T0636, T0664 | two_implementations | For /x/Sabrage, library_path returns /x/Sabrage/library.json. | For /x/Sabrage, settings_path returns /x/Sabrage/settings.json. | Intentional, not redundancy: the same base directory feeds two distinct store path constructors, and each filename is part of that constructor's public contract. |
| V10 | T0640, T0667 | two_implementations | A current-version library file ignores an unknown field and returns the closed Library value. | A settings file captures unknown fields into Settings::extra for later re-emission. | Intentional store-policy difference: library relies on schema-version refusal for incompatible future formats, while settings supports lossless opaque-key passthrough. T0667 may merge into T0668, but not into T0640. |
| V11 | T0536, T0345 | two_implementations | checks::config::parse_protocol returns "second" for two matching assignments because the last match wins. | stages::setup::parse_protocol_awk returns "first" for the same bytes because its setup.sh recipe stops on the first match. | Keep both. They are separate functions mirroring different shell recipes; consolidating the shared literals would hide their deliberately opposite multi-assignment semantics. |
| V11 | T0544, T0550 | same_function | Malformed JSON warns with an accurate native parse error and explicitly excludes "python3". | Successfully parsed but non-object JSON warns with the exact shell-style "broken python3?" diagnosis. | This is an intentional state distinction, not redundancy: parse/read failures occur in native Rust, while shape corruption mirrors the shell walk outside its Python try/except. Preserve the different expected prose when table-driving only the parsed-shape case |
| V12 | T0557, T0607, T0609, T0397 | two_implementations | checks/headset.rs feeds its first_connected_serial parser from an unbounded Command::output adb probe. | checks/run_only.rs now uses a deadline-killed child, while stages/run/actions.rs uses cancellable capture_with, even though their parser literals match. | Do not use T0609 or T0397 as a duplicate carrier for the headset tests. They belong to independent launch implementations whose probe blocking semantics deliberately differ after r1:A7-4; consolidate only T0557-T0559 within headset. |
| V12 | T0586, T0587 | shell_recipes_differ | The native network evaluator treats spawn failure and nonzero adb exit as unknown state and Warns. | doctor.sh ignores the forward --list pipeline status, receives empty output, and taps net.adb-forwards ok. | This is the declared PARITY.md failed-probe divergence and the r2:A4-4 regression. Keep the native Warn carrier; it is not a duplicate of shell clean-state coverage. |
| V14 | T0774 | two_implementations | zsh_scalar("it's") uses the historical double-quoted scalar spelling. | zsh_word("it's") uses embedded-apostrophe-safe single-quote splitting. | Intentional, not a semantic contradiction: scalar assignment RHS and standalone shell-word contexts have different minimal safe spellings, as defined at crates/sabrage-contract-gen/src/lib.rs:259 and :276. |
| V17 | T0064 | unknown | The scope note says the Tailer rotation tests pin r1:A8-4. | Round 1 A8-4 is the refuted BlackHole-volume finding; the queued-backlog rotation finding matching T0064 is round 2 A8-4. | Classify T0064 as r2:A8-4 based on the finding's file, fix sketch, regression-test request, and the test's r2 blame. |
| V17 | T0065 | unknown | The existing test comment calls the read-window race A8-7. | The round-2 finding whose fix sketch requires this test-only hook is r2:A8-3, titled '[unfixed] A8-7 The continuity signature still has a truncate/read race'. | Use r2:A8-3 as the round-qualified finding id; A8-7 is the inherited round-1 defect name embedded in the round-2 title. |

---

## 6. Policy price

The standard lets a finding-pinned (P) test merge if its label survives, and lets a core-side copy of a golden byte (G) go when parity pins identical bytes. Forcing either class to stay untouched costs:

| | G rows may move | G rows all kept |
|---|---|---|
| **P rows may move** | −133 tests / −1,888 lines (standard) | −106 / −1,547 |
| **P rows all kept** | −116 / −1,490 | −91 / −1,190 (strict) |

---

## 7. Method

**Index first, then models.** A brace-tracking scanner assigned every `#[test]`/`#[tokio::test]` fn a stable id (`T0001`…`T0907`), measured it, fingerprinted the string literals inside its assertions, and computed collision pairs, duplicate names and same-input/different-expected candidates. Every reviewer and verifier had to echo those ids, so nothing was matched by name.

**Codex classified, opus verified, mutants vetoed.** Seventeen vertical reviewers each read one module's production code and tests in full and returned one row per roster test plus a behaviour inventory; five horizontal reviewers read across modules through one lens each (cross-layer duplication, finding-pinned policy, top-down architecture, harness self-tests, entry-point mirrors and private seams). Every non-keep row went to a cut verifier that had to name, for each assertion of the loser, the carrier line that proves it; targeted keeps (shared rare literals, duplicate names, a systematic sample) went to keep verifiers hunting for missed duplicates; every horizontal claim was verified against the vertical row for the same test; one completeness critic per reviewer checked roster coverage, unprotected behaviours and truncation; missed duplicates flagged by keep verifiers went through a second cut-verifier pass before counting. Ten contradiction-critic agents adjudicated every vertical-versus-horizontal disagreement, the carrier chains, and the computed contradictions.

**The kill matrix is the empirical gate.** `cargo mutants` ran once over `sabrage-core` with the core and parity test packages, `-j 4`, inside a `sandbox-exec` profile with a scratch `HOME`, a minimal `PATH` and network denied (nothing under the real `~/Library/Application Support` changed). Because `cargo test` does not fail fast inside a binary, each caught mutant's log lists every failing test, which gives `catchers(m)` for every mutant; a deletion set `D` loses exactly the mutants whose catchers are all in `D`. Six proposed deletions lost a mutant and were reset to keep. Parity is necessary, not sufficient: the `parse_protocol` pair is distinguished by no generated mutant, which is why reading verification stays mandatory.

**What this scan did not do.** It ran no test to confirm a cut (verifiers read, the matrix is the only execution), it did not re-measure the 08-31 scaffolding program (that program is counted separately and its items are marked), and it did not touch `contract/`, `scripts/`, `demo.sh`, `docs/` or `ext/`. After the scan, `cargo test --workspace` (907 tests), `npm run check` and `scripts/dev/parity.sh --live=off` were run to prove the tree is untouched; `git status` was clean before and after. 27 of 118 verifier launches died on a session usage limit and were re-run (8 had already written their file); the final state has no unverified row. Two contradiction-critic parts disagreed about one test (`write_never_clobbers_a_file_created_in_the_toctou_window`); the kill matrix decided it (a unique kill on `walk`), so it stays.

| stage | count |
|---|---:|
| tests indexed (attribute-driven, brace-tracked) | 907 in 55 files |
| Codex reviewers | 17 vertical + 5 horizontal, all at `xhigh` (no capacity kills; every run landed first time) |
| reviewer rows (every roster id exactly once) | 907 |
| non-keep rows proposed → verified by a cut verifier | 262 → 262 |
| targeted keeps re-read by a keep verifier | 185 |
| horizontal claims → verified | 133 → 133 |
| completeness critics | 17 |
| contradiction critic resolutions applied | 18 |
| mutants (sabrage-core, `cargo-mutants` 27.1, `-j 4`, sandboxed) | 2,248: caught 1,599, missed 196, timeout 20, unviable 433 |
| verifier agents that returned nothing | 27 of 118 verifier launches (session limit), all re-run; 0 unverified rows in the final state |

Stages of the 262 proposed cuts: {"confirmed": 210, "critic": 13, "downgraded": 26, "second-pass": 6, "mutation-veto": 5, "rejected": 8}.

---

## 8. Appendix — artifacts

All under the session scratchpad `…/scratchpad/testred/` (local, not committed):

- `metrics/tests-index.tsv` — the per-test index (907 rows) with assertion fingerprints
- `metrics/assertion-collisions.tsv`, `pairs-summary.tsv`, `conflict-candidates.tsv`
- `out/merged/V01..V17.json`, `X1..X5.json` — reviewer outputs (strict schema)
- `verify/out/*.json` — every verifier verdict with its `subsumption[]` mapping
- `synth/program.json`, `synth/deletions.tsv` — the joined program; `synth/critic.json`
- `synth/mutants-join.json`, `mutants-lost.tsv`; `../mutants/out/full/kill_matrix.tsv` (mutant → catching tests)

