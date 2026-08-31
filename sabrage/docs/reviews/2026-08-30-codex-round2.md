# Sabrage adversarial review — Round 2 (post-fix re-review)

> **Provenance.** Second adversarial pass over the Sabrage implementation on branch `dingyifei/gui`, run 2026-08-31 00:40–03:10 PDT against commit `1e0540f` (the end of the round-1 fix phase). Each of the 16 areas got its round-1 findings + verdicts and the fix diff since `8e501ee`, and was asked to (a) confirm each round-1 fix actually closes its finding, (b) hunt regressions in the changed code, (c) report new material findings. Every finding was then adversarially verified by an opus refuter with runnable evidence.
>
> **Effort caveat.** 14 areas ran at `xhigh`. OpenAI returned "Selected model is at capacity" mid-session for A7 (2×), A9 (5×) and A12 (3×); A7 succeeded on its third `xhigh` attempt, A9 and A12 were run at **`high`** effort (attempts 6 and 4 respectively). Their findings were verified exactly like the others.
>
> **Fix loop.** One bounded fix→test→commit loop followed (no third full review): opus/sonnet fixers per area in three waves (core → IPC/CLI → UI), a verify tail after each wave, an opus adversarial review of the whole fix diff (8 findings, 5 fixed in the tail, 1 escalated and adjudicated by the lead, 2 landed as PARITY rows), then lead-owned shell/docs/PARITY hand-offs. Outcome per confirmed item is in the table at the end; the *residue* rows are the known, documented leftovers.


Codex `gpt-5.6-sol` @ xhigh, read-only, one session per area; every finding adversarially verified by an opus refuter (CONFIRMED / REFUTED / UNVERIFIABLE).

## Summary

| Area | Codex verdict | findings | confirmed | refuted | unverifiable | unverified | tokens |
|---|---|---|---|---|---|---|---|
| A1 contract-parity-spine | needs-attention | 8 | 8 | 0 | 0 | 0 | 306,048 |
| A2 core-primitives | needs-attention | 4 | 4 | 0 | 0 | 0 | 538,060 |
| A3a checks-doctor-core | needs-attention | 2 | 2 | 0 | 0 | 0 | 233,412 |
| A3b checks-doctor-config-net-game | needs-attention | 3 | 3 | 0 | 0 | 0 | 265,470 |
| A4 lock-and-fixes | needs-attention | 4 | 3 | 1 | 0 | 0 | 462,800 |
| A5 stages-setup-build-stop | needs-attention | 5 | 4 | 1 | 0 | 0 | 229,589 |
| A6 install-privilege | needs-attention | 3 | 3 | 0 | 0 | 0 | 222,299 |
| A7 run-preflight-actions | needs-attention | 5 | 5 | 0 | 0 | 0 | 245,310 |
| A8 run-supervise-guards-logs | needs-attention | 4 | 4 | 0 | 0 | 0 | 337,749 |
| A9 session-reconcile-telemetry | needs-attention | 9 | 9 | 0 | 0 | 0 | 348,936 |
| A10 config-runtime-toml | needs-attention | 9 | 8 | 1 | 0 | 0 | 265,033 |
| A11 ipc-boundary | needs-attention | 3 | 3 | 0 | 0 | 0 | 295,251 |
| A12 ui-shell-session | needs-attention | 5 | 4 | 1 | 0 | 0 | 244,076 |
| A13a store-rust | needs-attention | 3 | 3 | 0 | 0 | 0 | 213,214 |
| A13b ui-settings-library | needs-attention | 4 | 4 | 0 | 0 | 0 | 245,884 |
| A14 cli | needs-attention | 1 | 0 | 1 | 0 | 0 | 207,036 |

Totals — confirmed 67, refuted 5, unverifiable 0, unverified 0, areas not reviewed 0.

## Findings by area

## A1 — contract-parity-spine

**Codex verdict:** needs-attention — NO-SHIP: the X-binary/Y-checkout guard still does not protect direct setup, build, or install mutations. Six additional round-1 failure classes remain incompletely closed, and generated contract strings can still be evaluated by zsh rather than preserved literally.



### A1-1 [high, conf 0.99] [unfixed] A1-1 Direct mutating stages bypass the compiled-contract identity guard
`sabrage/crates/sabrage-core/src/stages/mod.rs:770-795` — **CONFIRMED** (re-rated high)

The new identity comparison protects the Doctor/launch-preflight path, but direct Setup, Build, and Install entry points still dispatch without it. A binary built from checkout X can therefore mutate checkout Y using X's embedded pins and templates, even when Y is internally self-consistent. This can install or download bytes that Y's contract does not describe.
Evidence: `sabrage/crates/sabrage-core/src/stages/mod.rs:770-795` enters `run_stage`, calls only `deny_stage_while_session_live(stage, ctx)?`, takes the operation locks, and executes `dispatch(stage, ctx).await`; no `COMPILED_CONTRACT_SHA256` comparison occurs before Setup/Build/Install. `sabrage/crates/sabrage-core/src/stages/setup.rs:171-205` subsequently uses `let deps = &contract().deps;` to construct the downloads.

*Recommendation:* Put a single compiled-versus-checkout contract guard in the shared mutating-stage boundary before Setup, Build, Install, or Run can perform their first mutation. Add X-binary/Y-checkout tests for each direct stage and the aggregate workflow; keep Stop available for recovery.

*Verifier:* The identity guard exists only as the second half of the `meta.contract-sync` evaluator (sabrage/crates/sabrage-core/src/checks/meta.rs:88-101), which runs in doctor and in the launch preflight (stages/run/preflight.rs:1292,1312,1346). sabrage/PARITY.md:111 declares exactly that scope ("the native launch preflight refuses to mutate") — it does not cover Setup/Build/Install, and those stages mutate global state (install writes the root-owned /usr/local/share/openxr manifest and the CrossOver-global DXMT/wineopenxr overlays; setup downloads pinned artifacts into Y from X's URL+sha pins).

*Fix sketch:* Add `pub fn binary_matches_checkout(repo_root: &Path) -> Result<(), (String,String)>` (or a `SabrageError`-returning `assert_contract_identity`) next to COMPILED_CONTRACT_SHA256 in sabrage/crates/sabrage-core/src/contract.rs, reusing util::contract_hash and the exact message/remedy strings already in checks/meta.rs so the row and the abort say the same thing. Call it in stages/mod.rs::run_stage (and run_stage_holding_lock) for Stage::Setup|Build|Install|Run before `dispatch`, immediately after deny_stage_while_session_live and before the lock is taken/any Executor call; leave Stage::Stop ungated so recovery still works. A missing/unreadable contract/ under repo_root should fail closed with the same fatal. Update sabrage/PARITY.md:111 to say "every mutating stage", not just the launch preflight.

*Regression test:* sabrage/crates/sabrage-core/src/stages/mod.rs `mod tests`: build the same self-consistent foreign checkout (mutate one pipeline.toml scalar, write a contract.gen.sh header recomputed from it), then for each of Setup, Build, Install assert `run_stage(stage, &StageCtx::for_fixture(...)).await` is `Err` whose message names the compiled/checkout skew and whose `ctx.executor.planned()` is empty (nothing planned before the refusal), and assert `Stage::Stop` still succeeds. Plus one test asserting run_stage against the real repo_root is not blocked.

*Cross-area files:* sabrage/crates/sabrage-core/src/stages/mod.rs, sabrage/crates/sabrage-core/src/checks/meta.rs, sabrage/PARITY.md

### A1-2 [medium, conf 1] [unfixed] A1-2 The --bs-dir setup fallback still embeds the depot pin
`scripts/demo/setup.sh:58-62` — **CONFIRMED** (re-rated low)

The generator now emits the previously omitted asset and install-leaf fields, but another real shell consumer still hard-codes contract scalars. After an app/depot/manifest update, `setup --bs-dir` will print a stale DepotDownloader command while regeneration, `--check`, and contract-sync all remain green.
Evidence: `scripts/demo/setup.sh:58-62` assigns `DEPOT_CMD="DepotDownloader -app 620980 -depot 620981 -manifest 6291266771922375922 ..."` instead of using the sourced `BS_APPID`, `BS_DEPOT`, and `BS_MANIFEST` variables.

*Recommendation:* Construct this fallback from `$BS_APPID`, `$BS_DEPOT`, and `$BS_MANIFEST`. Expand the hard-code/mutation test to cover every emitted scalar at every shell consumer branch, rather than only the three newly added variables.

*Verifier:* Real duplication of contract scalars in a shell consumer, and the parity hard-code test genuinely does not cover these three. Severity re-rated down from medium: the value is only interpolated into an advisory `info "  $DEPOT_CMD"` line printed when Beat Saber is absent (setup.sh:63-67) — nothing is fetched or written from it — and it can only diverge after a future game/depot/manifest pin bump. Native (contract.rs:352-358 `depot_command`) and shell agree today.

*Fix sketch:* scripts/demo/setup.sh:61 -> `DEPOT_CMD="DepotDownloader -app $BS_APPID -depot $BS_DEPOT -manifest $BS_MANIFEST -username <steam-user> -dir \"$BS_DIR\""` (byte-identical expansion to lib.sh:100). Then re-bless sabrage/parity/shell.fingerprint via `scripts/dev/parity.sh --bless`.

*Regression test:* Extend `the_shell_sources_the_emitted_asset_and_install_leaf_scalars` in sabrage/crates/sabrage-parity/src/lib.rs to iterate every scalar contract.gen.sh emits (appid, depot, manifest, deps url, both asset names, both sha256s, bs_dir_leaf, host_xr_json, port lists) over the executable lines of every scripts/demo/*.sh consumer, asserting the literal never appears; keep the positive `$VAR is referenced` assertions for the vars each file uses.

*Cross-area files:* scripts/demo/setup.sh, sabrage/parity/shell.fingerprint

### A1-3 [medium, conf 1] [unfixed] A1-3 CI still skips native unit tests that the parity suite says are required
`sabrage/crates/sabrage-parity/src/lib.rs:1667-1694` — **CONFIRMED** (re-rated low)

The local parity script was strengthened, but the CI Tier-1 workflow still runs only `sabrage-parity` and `sabrage-contract-gen`. The parity module explicitly admits that several native literals remain copied substring expectations whose native half is enforced only by sabrage-core unit tests. A native regression in those functions can therefore pass CI.
Evidence: `sabrage/crates/sabrage-parity/src/lib.rs:1667-1694` states that `wineserver_reset`, `goldberg_stage`, ADB cleanup, encoder notices, and game-version wording remain "substring-only" because the functions are not public and the fragments are "still copied from source below". `.github/workflows/parity.yml:27-28` runs `cargo test ... -p sabrage-parity` and `-p sabrage-contract-gen`, but not `-p sabrage-core`.

*Recommendation:* Run sabrage-core's relevant unit tests in Tier 1, or expose the remaining native renderers/constants so sabrage-parity compares live native output directly. Add a CI assertion that prevents the required package list from silently shrinking.

*Verifier:* The factual claim holds — a native-only edit to those non-`pub` renderers turns nothing red in CI. Re-rated down from medium because it is a coverage gap, not a defect: the divergence is deliberate and documented (parity.sh:72-74, lib.rs:1673-1676 — sabrage-core's suite probes lipo/SwitchAudioSource/CrossOver paths and cannot run on ubuntu), the same-direction shell edit IS gated, and the mandatory local gate (scripts/dev/hooks/pre-push -> parity.sh --live=off) does run `-p sabrage-core` on macOS before every push.

*Fix sketch:* Two parts. (1) Make the package list machine-checked: add a sabrage-parity test that parses `TIER1_PKGS=(...)` out of scripts/dev/parity.sh and asserts .github/workflows/parity.yml's run line contains the same package set (or an explicitly allow-listed subset with a stated reason), so the CI list cannot silently shrink. (2) Close the coverage itself either by adding a macos-latest tier-1 job that runs `-p sabrage-core`, or by making the remaining renderers `pub` (as A1-3's earlier pass already did for banner_events/wine_exit_line/guard constants) and pinning their output from sabrage-parity, which is hermetic and already in CI.

*Regression test:* sabrage/crates/sabrage-parity/src/lib.rs, new `mod ci_gate`: `tier1_packages_in_ci_match_parity_sh` reads both files from repo_root and asserts set equality of the `-p <pkg>` tokens; plus, per closed renderer, a `native_<fn>_text_is_verbatim_in_run_sh` test calling the now-`pub` function instead of comparing a copied fragment.

*Cross-area files:* .github/workflows/parity.yml, scripts/dev/parity.sh, sabrage/crates/sabrage-core/src/stages/run/actions.rs, sabrage/crates/sabrage-core/src/stages/run/preflight.rs, sabrage/crates/sabrage-core/src/fixes/adb.rs

### A1-4 [medium, conf 1] [unfixed] A1-4 contract-sync still authenticates the header rather than the sourced body
`sabrage/crates/sabrage-core/src/util/mod.rs:382-390` — **CONFIRMED** (re-rated low)

The runtime check still extracts a self-reported hash from `contract.gen.sh` and compares it only with the contract files. Editing executable generated-shell content while retaining the header is invisible, even though `lib.sh` sources that body. The UI can consequently report the contract as synchronized while the shell executes altered pins, ports, or paths.
Evidence: `sabrage/crates/sabrage-core/src/util/mod.rs:382-390` implements `contract_gen_recorded_hash` with `text.lines().find_map(|line| line.strip_prefix("# contract-sha256: "))`; it never validates the remaining generated bytes. `sabrage/crates/sabrage-core/src/checks/meta.rs:39-51` expressly notes that a body-only hand edit is invisible to this check.

*Recommendation:* Validate the generated body itself—preferably by comparing it with fresh generator output, or by recording and recomputing a body digest. Add a scratch-checkout test that keeps the header intact while changing an executable assignment and requires both native and shell Doctor checks to fail.

*Verifier:* Accurate: the runtime check authenticates the header, not the sourced body, and lib.sh sources that body. Re-rated down from medium because the drift is caught before it can ship — sabrage/crates/sabrage-parity/src/lib.rs:43-55 (`generate_matches_the_committed_contract_gen_sh`, byte-for-byte `include_str!`) and :139-148 (`check_reports_in_sync_against_the_live_checkout`) are in CI's tier-1 set — and reaching it requires hand-editing a file whose first line reads `# GENERATED from contract/ — DO NOT EDIT` (or a half-resolved merge/partial checkout). Impact is a green doctor row on a machine whose zsh front-end uses altered pins/ports; the native front-end is unaffected (it never reads the body).

*Fix sketch:* Make the header cover the body, keeping the check zero-Rust on the shell side: in sabrage/crates/sabrage-contract-gen/src/lib.rs::generate_from, emit a second header line `# body-sha256: <sha256 of every byte after that header line>` (computed over the rendered body, so `--check`/`--regen` stay byte-exact); add `contract_gen_body_hash(repo_root) -> Option<(recorded, recomputed)>` beside contract_gen_recorded_hash in sabrage/crates/sabrage-core/src/util/mod.rs; have checks/meta.rs::meta_contract_sync fail with the existing out-of-sync message+remedy when the two differ; mirror it in scripts/demo/doctor.sh section 0 with `sed '1,/^# body-sha256: /d' … | shasum -a 256`. Alternative (native-only, weaker parity): compare the on-disk file against `sabrage_contract_gen::generate()`, which requires sabrage-core to depend on sabrage-contract-gen and has no shell counterpart.

*Regression test:* sabrage/crates/sabrage-core/src/checks/meta.rs `mod tests`: `fails_when_only_the_body_was_edited` — real contract/ bytes + real contract.gen.sh with one executable assignment mutated and the contract-sha256 header left current; assert Fail with the out-of-sync message and the `scripts/dev/parity.sh --regen` remedy. Mirror it in sabrage/crates/sabrage-parity/src/lib.rs as a tier-2-style fixture asserting doctor.sh's section-0 shell recipe reports the same FAIL on that fixture, and update the header-only fixture at checks/meta.rs:190-196 to carry a valid body digest.

*Cross-area files:* sabrage/crates/sabrage-core/src/checks/meta.rs, scripts/demo/doctor.sh, scripts/demo/contract.gen.sh, sabrage/parity/shell.fingerprint

### A1-5 [medium, conf 0.99] [unfixed] A1-6 Mixed tag groups can still swap warn and block behavior undetected
`sabrage/crates/sabrage-parity/src/lib.rs:731-766` — **CONFIRMED** (re-rated low)

The new scanner distinguishes gate values in comments, but it verifies verbs only at tag-group granularity. A group containing both block and warn slugs passes whenever its combined body contains one `die` and one `warn`; it does not bind either verb to the corresponding slug or branch. The current protocol group has exactly this shape, so making legacy protocol fatal while merely warning on an unsupported protocol would pass the test.
Evidence: `sabrage/crates/sabrage-parity/src/lib.rs:731-766` computes only `has_die` and `has_warn` for the entire `group.body`; the warn-side prohibition on `die` is skipped whenever `group.gates.contains(&Gate::Block)`. `scripts/demo/run.sh:55-64` places `cfg.protocol.supported` and `cfg.protocol.legacy-oxrsys` in one mixed block.

*Recommendation:* Bind each slug to its actual executable branch and terminal verb, using explicit per-branch annotations or structured fixtures. Add a mutation test that swaps the two protocol branches' `warn` and `die` calls and must fail.

*Verifier:* Factually reproduced: verb checking is group-granular, so a mixed group is satisfied by any one `die` plus any one `warn` anywhere in the block, and the branch<->slug binding the doctring claims ('ties each tag group to the verb its block actually uses') is not established for the only mixed group in the file. Severity is low, not medium: today's run.sh is correct, the divergence requires a separate future mis-edit, that edit would also redden `shell_fingerprint` and demand an explicit `--bless`, and the blast radius is one launch-time warn/die swap on the legacy-protocol path (already a declared divergence in sabrage/PARITY.md:29). It is a hole in a guard, not live wrong behaviour.

*Fix sketch:* In sabrage/crates/sabrage-parity/src/lib.rs: keep `tag_groups` but stop treating the body as one bag of verbs. Either (a) anchor verb-to-message: for each slug in a mixed group require the run.sh line carrying that slug's pinned message (e.g. `protocol=oxrsys (legacy USB path)` for cfg.protocol.legacy-oxrsys, `is not valid for the demo` for cfg.protocol.supported) to start with the gate's verb — a per-slug `(needle, verb)` table checked line-by-line inside `each_preflight_tag_group_uses_the_verb_its_gate_claims`; or (b) split the body into shell `case`/`if` branches and require every emitting branch to be attributable to exactly one tagged slug, rejecting groups where a branch cannot be bound (which would force per-branch tags in run.sh).

*Regression test:* sabrage/crates/sabrage-parity/src/lib.rs, module `tests::run_sh_tags`: a mutation test that reads scripts/demo/run.sh, swaps the `oxrsys)` and `*)` arms' verbs in memory, and asserts the (refactored) group checker returns a failure for `cfg.protocol.legacy-oxrsys`; plus the existing unmutated run must still pass.

### A1-6 [medium, conf 0.98] [unfixed] A1-7 Loop coverage still credits unrelated emissions
`sabrage/crates/sabrage-parity/src/lib.rs:365-398` — **CONFIRMED** (re-rated low)

Loop validation independently asks whether the body contains a slug extraction and whether it contains any `chk` or `tap`. It never proves that the call's slug argument derives from the loop variable. Deleting the real per-item emission while leaving the extraction plus an unrelated static check still credits every slug named by the loop header. The scanner also records only first emission, so duplicate unconditional rows remain invisible.
Evidence: `sabrage/crates/sabrage-parity/src/lib.rs:365-398` sets `saw_extraction` when a line contains `${var%%:*}`, sets `saw_call` for any `chk`/`tap`, and returns `saw_extraction && saw_call` without connecting the two data flows.

*Recommendation:* Require the `chk`/`tap` slug argument to syntactically derive from the loop variable or its tracked slug temporary. Track emission counts and reject duplicate unconditional rows. Add fixtures with an orphaned extraction plus unrelated call and with duplicate emissions.

*Verifier:* The code does exactly what the finding says — coverage is presence-based, not data-flow-bound — and there is a plausible (not merely contrived) miss: adding a `continue`/conditional skip inside a loop body, or emitting for only some items, still credits every slug the header names, because no branch analysis exists. That justifies CONFIRMED. But two things pull the severity down to low: (1) the docstring at :368-374 deliberately scopes the check to 'deleting the body's chk lines', which IS caught for every loop except section 10's standalone-`_slug` shape; (2) the finding's second half — 'track emission counts and reject duplicate unconditional rows' — is unsound and I refute it: doctor.sh emits most slugs from BOTH arms of an if/else (e.g. `chk ok build.helper-arm64` at doctor.sh:132 vs `tap build.helper-arm64 skipped` at doctor.sh:136), so a static duplicate-emission rule would false-positive on nearly every check in the file. No live divergence exists today; tier-2's live tap diff still catches a real missing row.

*Fix sketch:* In sabrage/crates/sabrage-parity/src/lib.rs `loop_body_emits`: replace the two independent booleans with one join. Track the set of shell words that provably hold the loop's slug — `${var%%:*}` itself, plus any `_name="${var%%:*}"` assignment's LHS (as `$_name` / `"$_name"`) — and return true only if some line matched by `call_re` ALSO contains one of those words in argument position. Optionally record a `conditional_loops` list when the body contains `continue`/`break`/`return` so a skip inside the loop is surfaced rather than silently credited.

*Regression test:* sabrage/crates/sabrage-parity/src/lib.rs, in the module that owns `loop_body_emits` (private, so an in-crate `#[cfg(test)]` fixture test): synthetic doctor.sh strings asserting (a) header + `_slug="${_o%%:*}"` + an unrelated `tap other.slug ok` in the body => header_only_loops (not credited), (b) the real section-10 shape => credited, (c) a body with `continue` before the chk => surfaced as not-covered/conditional.

### A1-7 [medium, conf 0.99] [unfixed] A1-8 Control characters still produce an invalid privileged host manifest
`sabrage/crates/sabrage-core/src/util/mod.rs:223-250` — **CONFIRMED** (re-rated low)

Quote and backslash paths are fixed, but the escape helper deliberately leaves JSON control characters unchanged. macOS paths can contain newlines or tabs, so such a checkout produces invalid JSON that may replace the root-owned active OpenXR manifest and break runtime discovery. Artifact parity does not justify preserving invalid output; both front ends can reject or correctly encode it together.
Evidence: `sabrage/crates/sabrage-core/src/util/mod.rs:223-250` says control characters "stay unescaped" and implements only `s.replace('\\', "\\\\").replace('"', "\\\"")`; the associated test explicitly pins a literal newline as unchanged.

*Recommendation:* Use complete JSON-string escaping on both Rust and zsh paths, or fail closed before the privileged write when the path contains controls or cannot be represented losslessly. Test newline and tab paths for valid JSON round-trip or explicit rejection.

*Verifier:* The control-character path is real and unguarded on both sides, so CONFIRMED — but only low: it needs a checkout directory literally containing a newline or tab, which no realistic macOS checkout has; the failure is self-diagnosing (doctor.sh:170's python json.load fails and prints `chk fail host.manifest` with a remedy) and fully repaired by re-running install from a sane path, so it is neither data loss nor irreversible root state. It is not a parity break either — both implementations produce the same invalid bytes. The genuinely reachable sibling defect is the un-escaped `print --` backslash divergence noted above, which the finding asserts is already fixed.

*Fix sketch:* Two options, both must land on both sides in one commit per CLAUDE.md. (a) Encode fully: extend `json_escape_string` in sabrage/crates/sabrage-core/src/util/mod.rs to the JSON minimum (`\b \f \n \r \t` and `\u00XX` for other C0) and make install.sh do the same substitutions plus switch `print --` to `print -r --`. (b) Fail closed: have `render_host_manifest`/`host_manifest_file_bytes` return a Result (or add a `path_is_json_safe` guard called by stages/install.rs before the privileged write) that rejects any path containing chars < 0x20, with install.sh gaining the matching `case $OXR_DYLIB in *[$'\001'-$'\037']*) die ...` guard.

*Regression test:* sabrage/crates/sabrage-core/src/util/mod.rs unit tests (next to the existing escape test that currently pins a literal newline as unchanged): assert `serde_json::from_str::<Value>(&host_manifest_file_bytes(Path::new("/tmp/a\nb/lib.dylib")))` succeeds and round-trips library_path exactly (option a), or that the guard rejects it (option b); plus a golden-byte test in sabrage-parity mirroring whatever install.sh now emits for the same path.

*Cross-area files:* scripts/demo/install.sh, sabrage/crates/sabrage-core/src/stages/install.rs, sabrage/crates/sabrage-core/src/privilege.rs

### A1-8 [medium, conf 0.96] [new] Generated TOML strings are evaluated as zsh code
`sabrage/crates/sabrage-contract-gen/src/lib.rs:176-226` — **CONFIRMED** (re-rated low)

Contract strings are interpolated directly into double-quoted zsh assignments, while array values are joined as unquoted words. Valid TOML containing `$`, command substitution, quotes, whitespace, or glob characters is therefore expanded, split, or rendered as invalid shell when `contract.gen.sh` is sourced. Native Rust retains the literal TOML value, so regeneration can bless a cross-frontend divergence—and command substitution can execute during sourcing.
Evidence: `sabrage/crates/sabrage-contract-gen/src/lib.rs:176-226` builds `dxmt_files` with `c.dxmt.files.join(" ")`, emits assignments such as `DEPS_URL=\"{deps_url}\"` and `BS_DIR_LEAF=\"{bs_dir_leaf}\"`, and emits `DXMT_FILES=({dxmt_files})` without a zsh-literal encoder.

*Recommendation:* Encode every generated string as a literal zsh word—prefer single quoting with correct embedded-quote handling—and encode each array element separately. Add generation-and-source tests containing spaces, `$()`, backticks, quotes, backslashes, and glob characters.

*Verifier:* The escaping gap is real and I reproduced each consequence, so CONFIRMED. Severity low, not medium, and explicitly NOT a trust-boundary break: contract/pipeline.toml is a committed, reviewed repo file with exactly the same trust level as contract.gen.sh, lib.sh and demo.sh themselves — anyone who can edit it can already run arbitrary shell — so 'command substitution executes during sourcing' adds no privilege. The only realistic non-malicious trigger is the unquoted `DXMT_FILES` array: a future artifact set whose filename contains a space or a glob character would silently split/expand and desync from the native `Vec<String>`. Current values (contract.gen.sh:26) are all glob- and space-free, and the one scalar that does contain a space (BS_DIR_LEAF="Beat Saber 1294") is correctly double-quoted, so there is no live divergence today.

*Fix sketch:* In sabrage/crates/sabrage-contract-gen/src/lib.rs add `fn zsh_literal(s: &str) -> String` that single-quotes with `'` -> `'\''` handling, use it for every scalar assignment (`DEPS_URL=`, asset/sha, `BS_DIR_LEAF=`, `HOST_XR_JSON=`) and per-element for the array (`DXMT_FILES=({elems})` where elems is `files.iter().map(zsh_literal).join(" ")`); leave the numeric appid/depot/manifest/ports emitted as digits but assert they are digit-only. Regenerate the committed scripts/demo/contract.gen.sh (bytes change: quotes become single quotes) — `generate_matches_the_committed_contract_gen_sh` enforces this; meta.contract-sync is unaffected because it hashes contract/ inputs, not the output.

*Regression test:* sabrage/crates/sabrage-contract-gen/src/lib.rs `mod tests`: property-ish test calling `generate_from` with a pipeline.toml whose url/bs_dir_leaf/dxmt file entries contain `$(echo X)`, backticks, `'`, `"`, `\`, a space and `*`, then (a) assert the generated text contains no unescaped `$`/backtick, and (b) shell out to `zsh -c 'source <tmp>; print -rl -- $DEPS_URL $BS_DIR_LEAF; print -rl -- $DXMT_FILES'` and assert every value comes back byte-identical to the TOML value and the array length equals `files.len()`.

*Cross-area files:* scripts/demo/contract.gen.sh

*Codex next steps:* Add an X/Y checkout fixture and assert direct Setup, Build, Install, Run, and aggregate workflows reject the mismatch before their first Executor mutation. · Mutate appid, depot, and manifest, regenerate, and verify the `setup --bs-dir` fallback contains only the mutated values and no literal baseline pin. · Add adversarial scanner fixtures that swap mixed-group warn/block branches, orphan a loop extraction beside an unrelated check, and emit a duplicate row. · Keep a valid generated header while altering an executable body assignment; require native and shell contract-sync checks to fail. · Test newline/tab host paths and shell-metacharacter contract values through JSON parsing and isolated zsh sourcing, then run the full Tier-1 package set including sabrage-core.

## A2 — core-primitives

**Codex verdict:** needs-attention — No-ship: A2-3, A2-5, and A2-7 remain incompletely fixed, and the new no-clobber writer can permanently publish a truncated write-once configuration.

### A2-1 [medium, conf 0.96] [unfixed] A2-3 Reconciliation still uses an uncancellable probe path
`sabrage/crates/sabrage-core/src/process.rs:790-792` — **CONFIRMED** (re-rated low)

`capture` is bounded but manufactures a token unrelated to the running operation. The Stop reconciliation path calls it behind a five-second timeout, so Cancel cannot interrupt a wedged CoreAudio probe and the operation lock remains occupied until that timeout. Because the outer timeout fires before `capture`'s ten-second deadline, its explicit process-group kill branch is also bypassed; inference: a forked descendant can outlive the dropped direct child.
Evidence: `sabrage/crates/sabrage-core/src/process.rs:790-792` calls `capture_with(spec, &CancellationToken::new(), DEFAULT_PROBE_TIMEOUT)`, while `sabrage/crates/sabrage-core/src/session/reconcile.rs:634-638` uses `timeout(PROBE_TIMEOUT, process::capture(spec))` without `ctx.cancel`.

*Recommendation:* Pass the operation token into reconciliation and call `capture_with(spec, &ctx.cancel, PROBE_TIMEOUT)`. Preserve cancellation as `SabrageError::Cancelled` instead of collapsing it into `Option::None`, and test that a cancelled wedged probe releases the stage promptly with its entire process group gone.

*Verifier:* The token really is not wired through, and the 5s non-interruptible window under a wedged CoreAudio call is real - but it is bounded, both probes run concurrently (tokio::join!) so the worst case stays 5s, no process is leaked, and the trigger (CoreAudio wedge) is rare. User-visible cost is a Cancel that takes up to 5s instead of being immediate, not wrong behaviour.

*Fix sketch:* Give `probe_capture` the token: `async fn probe_capture(spec, cancel: &CancellationToken)` calling `process::capture_with(spec, cancel, PROBE_TIMEOUT)`; `current_output_device` passes `&ctx.cancel`. Map `Err(SabrageError::Cancelled)` out of the `Option` (return a small enum or propagate `Result`) so `restore_and_finish` aborts with Cancelled instead of silently treating cancellation as 'could not look'. Drop the now-redundant `tokio::time::timeout` wrapper and refresh the stale PROBE_TIMEOUT doc comment.

*Regression test:* sabrage/crates/sabrage-core/src/session/reconcile.rs tests module: inject a probe spec of `/bin/sh -c 'sleep 30'`, cancel the ctx token after ~100ms, assert the reconcile call returns within well under PROBE_TIMEOUT and yields SabrageError::Cancelled (today it blocks the full 5s and yields None).

*Cross-area files:* sabrage/crates/sabrage-core/src/session/reconcile.rs

### A2-2 [medium, conf 0.99] [unfixed] A2-5 Escalation still infers process-group death from pipe closure
`sabrage/crates/sabrage-core/src/process.rs:425-450` — **CONFIRMED** (re-rated medium)

After SIGTERM, the configured grace period still waits only for the leader. If the leader exits, SIGKILL is issued only when pipe draining later times out. A TERM-ignoring descendant that closes or redirects stdout/stderr makes both pumps finish and therefore survives cancellation while continuing its work. The inverse is also wrong: a descendant retaining the pipes gets only the fixed two-second pump grace, not the default five-second kill grace, before SIGKILL.
Evidence: `sabrage/crates/sabrage-core/src/process.rs:425-450` returns immediately from the grace wait on `Ok(Ok(st)) => st` and conditions group escalation on `if !drain_pumps(..., PUMP_DRAIN_GRACE).await`; `process.rs:473` fixes that separate pump grace at two seconds.

*Recommendation:* Track process-group liveness independently of pipe EOF through the configured `kill_grace`. At the deadline, SIGKILL any surviving group, reap the leader, and only then perform a bounded pipe drain. Add a regression child that ignores TERM and redirects both standard streams to `/dev/null`, then assert no group member remains.

*Verifier:* Cancellation is documented as 'signals the whole tool tree' (process.rs:292-295, 361), and the code infers group death from pipe EOF, which is not the same property. Latent: no tool in this pipeline is known to both ignore SIGTERM and detach its stdio while staying in the process group (daemons that do this usually setsid and escape killpg anyway), so it is an unusual-conditions gap rather than a wrong result on a normal path.

*Fix sketch:* In `spawn_streamed_inner`'s cancel arm (process.rs:420-433): after SIGTERM, keep a deadline of `spec.kill_grace` for the whole *group*, not just the leader - reap the leader with `child.wait()`, then poll `killpg(pgid, None)`/`kill(-pgid, 0)` until the deadline; at the deadline SIGKILL the group unconditionally. Only then run the bounded `drain_pumps`, and keep the existing abort-on-timeout fallback. Leave the uncancelled path untouched (wineserver must survive).

*Regression test:* sabrage/crates/sabrage-core/src/process.rs tests module, beside `cancellation_escalates_when_a_descendant_outlives_the_leader`: spawn `/bin/sh -c "( trap '' TERM; exec >/dev/null 2>&1; exec sleep 30 ) & echo $! > $PIDFILE; sleep 30"`, cancel, then assert `!process::is_alive(pid)` for the recorded descendant after spawn_streamed returns Cancelled.

### A2-3 [medium, conf 0.99] [unfixed] A2-7 Atomic writes still report success without durable publication
`sabrage/crates/sabrage-core/src/executor.rs:817-838` — **CONFIRMED** (re-rated low)

The replacement's contents are synced, but permissions are changed afterwards without another file sync, and both failure to open the parent directory and failure to sync it are discarded. Consequently `write_atomic` returns success even when the rename is not crash-durable. That violates the persist-before-mutate contract: the audio guard may switch devices after a `session-state.json` save whose directory entry can still be lost on power failure.
Evidence: `sabrage/crates/sabrage-core/src/executor.rs:817-838` performs `file.sync_all()`, then `set_permissions`, then `rename`, but finishes with `if let Ok(dir) = ... { let _ = dir.sync_all().await; }` and unconditional `Ok(())`.

*Recommendation:* Apply final metadata before the file sync, then rename and propagate parent-open and parent-sync failures. If some filesystems cannot provide this guarantee, expose a strict durable-write primitive for session state and refuse the subsequent machine mutation when durability cannot be established.

*Verifier:* The code fact is exactly as stated and the contradiction with the function's own contract is real, but realizing harm needs a directory-fsync failure AND a power loss in the same window; the permissions-after-sync nit costs at worst a 0600 mode surviving a crash, not data. Latent/minor.

*Fix sketch:* In `write_atomic_real`: move `set_permissions(&tmp, ...)` before `file.sync_all()` (or use `File::set_permissions` on the open handle and sync after), then rename, then propagate the parent-directory open and `sync_all` errors as `SabrageError::io(parent, e)` instead of `let _ =`. If some target filesystem genuinely cannot fsync a directory, downgrade only that specific errno (ENOTSUP/EINVAL/EBADF) to a warning rather than swallowing everything.

*Regression test:* sabrage/crates/sabrage-core/src/executor.rs tests module: `write_atomic` into a path whose parent is removed between the temp write and the publish (or whose parent open is forced to fail) must return Err, plus an assertion that the published file has mode 0644 and that a pre-existing destination's mode is preserved.

### A2-4 [medium, conf 0.99] The new create_new primitive can strand a partial write-once configuration
`sabrage/crates/sabrage-core/src/executor.rs:846-874` — **CONFIRMED** (re-rated low)

`create_new_real` creates the final pathname before writing its bytes. Its cleanup handles ordinary returned errors, but process termination or power loss after the exclusive open bypasses cleanup and leaves an empty or partial file. Every later call sees `AlreadyExists` and refuses to replace it. This is especially damaging for `oxrsys-runtime.toml`: the write-once path subsequently treats the corrupt file as user-owned existing content.
Evidence: `sabrage/crates/sabrage-core/src/executor.rs:846-874` opens `path` directly with `.create_new(true)`, writes afterwards, returns `Ok(false)` on `AlreadyExists`, and removes the file only inside `if written.is_err()`; `sabrage/crates/sabrage-core/src/config/runtime_toml.rs:1184-1195` uses this primitive for the initial template and reads the existing path when creation returns false.

*Recommendation:* Write, chmod, and sync a unique sibling temp first, then publish it atomically without replacement using a same-filesystem hard link or platform `rename-no-replace`; remove the temp and sync the parent directory. Treat `AlreadyExists` as the race loser only after the fully written temp was ready.

*Verifier:* The unprotected window is genuine but sub-millisecond and reachable only via SIGKILL/power loss on the narrow 'GUI edits config before the file has ever existed' path; recovery is deleting a zero-length file, and doctor already FAILs on a missing protocol. Latent.

*Fix sketch:* In `create_new_real`: write+chmod+`sync_all` a unique sibling temp (reuse `sibling_tmp`), then publish without replacement - `std::fs::hard_link(tmp, path)` on the same filesystem (or `renamex_np(..., RENAME_EXCL)` on macOS), unlink the temp, and fsync the parent. `AlreadyExists` from the link/rename is the race-loser answer `Ok(false)`; remove the temp in every failure path.

*Regression test:* sabrage/crates/sabrage-core/src/executor.rs tests module: assert `create_new` leaves no zero-length target when the write fails (already partly covered), plus a new case that a pre-existing zero-length file is reported `Ok(false)` and, once fixed, that no sibling temp survives a successful create; a fault-injection test that kills a child process between open and write (e.g. a helper binary) can live in crates/sabrage-core/tests/ if the temp-publish rewrite is not taken.

*Codex next steps:* Add a cancellation test whose TERM-ignoring descendant closes stdout/stderr and assert the process group is gone after the configured grace. · Fault-inject termination after `create_new` opens and partially writes the target; verify the target is either absent or contains the complete template. · Inject parent-directory open/sync failures and verify `write_atomic` returns an error before any audio-device mutation. · Exercise Stop cancellation against a wedged reconciliation probe and verify prompt exit, lock release, and complete process-group cleanup.

## A3a — checks-doctor-core

**Codex verdict:** needs-attention — No ship: the contract-identity fix detects skew but still permits setup/build/install mutations, while A1-4 remains a documented false-green. No new evidence defeats the four REFUTED A3a findings.

### A3a-1 [high, conf 0.99] [unfixed] A1-1 Contract skew still permits setup/build/install mutations
`sabrage/crates/sabrage-core/src/checks/meta.rs:89-101` — **CONFIRMED** (re-rated medium)

An X-built binary pointed at self-consistent checkout Y detects the mismatch only when this evaluator is run. The generic stage paths dispatch Setup, Build, and Install without evaluating it; `all` runs those three mutating stages before Run reaches the new gate. Whole-stage Doctor fixes also use the unguarded holding-lock dispatcher. Inference: Setup can download X's pins and create X's write-once runtime template, and Install can deploy X's artifacts before Launch finally refuses.
Evidence: `sabrage/crates/sabrage-core/src/checks/meta.rs:89-101` confines the compiled-checkout comparison to `meta_contract_sync`: `if &want != compiled { return CheckOutcome::fail(...) }`. Conversely, `sabrage/crates/sabrage-core/src/stages/mod.rs:770-795` performs only `deny_stage_while_session_live(stage, ctx)?` before `dispatch(stage, ctx).await`, and `:837-842` dispatches `Stage::Setup`, `Stage::Build`, and `Stage::Install` directly. `sabrage/crates/sabrage-core/src/stages/setup.rs:255-276` then writes `util::toml_template()` when the write-once config is absent.

*Recommendation:* Enforce one compiled-vs-checkout identity guard at the common mutation boundary before Setup, Build, Install, and Run dispatch, including `run_stage_holding_lock`/whole-stage fix paths. Add X-binary/Y-checkout tests proving every mutating entry point produces no planned or real action on mismatch.

*Verifier:* The round-1 fix added detection and one enforcement point (the launch preflight). Setup, Build, Install — the stages that download pinned artifacts, write the write-once ~/Library/Application Support/OXRSys/oxrsys-runtime.toml from the COMPILED template, and sudo-write the host XR manifest / DXMT overlay — dispatch with no identity gate, and neither does run_stage_holding_lock (doctor's whole-stage fixes). sabrage/PARITY.md:111 states the rationale as 'Every install/launch artifact written by such a binary would be X's bytes in Y's tree', which is broader than the guard actually implemented. Re-rated high->medium: the precondition (installed binary from checkout X repointed at a checkout Y whose contract/ differs) is unusual though realistic in this repo's multi-worktree workflow; doctor FAILs, Settings shows 'mismatched', launch is blocked, and every mutation is re-runnable (the write-once toml is the stickiest, and is deletable). Not high, because it is not the default single-checkout path; not low, because `all` and `install` really do mutate (with sudo) before anything refuses.

*Fix sketch:* Hoist the compiled-vs-checkout comparison out of checks::meta into a small reusable predicate (e.g. util::contract_identity_mismatch(root) -> Option<(checkout,binary)>), keep checks::meta calling it, and call it once at the common mutation boundary: in stages::run_stage next to deny_stage_while_session_live AND in stages::run_stage_holding_lock (both entry points, so fixes::apply/apply_holding_lock inherit it), returning ctx.fatal with meta.rs's existing message+remedy for Stage::Setup|Build|Install|Run. Leaving Stop ungated is deliberate (it is the way out). The preflight row stays as-is; because meta.contract-sync is contract-order slug 0 the launch path just fails a step earlier.

*Regression test:* sabrage/crates/sabrage-core/src/stages/mod.rs #[cfg(test)]: build a scratch root with self-consistent-but-foreign contract files (same recipe as checks/meta.rs:168-226) and assert run_stage(stage, ctx) and run_stage_holding_lock(stage, ctx) both return SabrageError::Fatal with the 'built from a different contract' message for each of Setup/Build/Install/Run, that ctx.executor.planned() is empty (zero planned actions), and that the same calls against repo_root() do NOT fail on this gate. Plus a sabrage-cli test asserting run_all aborts before Stage::Setup on mismatch.

*Cross-area files:* sabrage/crates/sabrage-core/src/stages/mod.rs, sabrage/crates/sabrage-core/src/fixes/mod.rs, sabrage/crates/sabrage-core/src/util/mod.rs, sabrage/crates/sabrage-cli/src/main.rs, sabrage/PARITY.md

### A3a-2 [low, conf 1.0] [unfixed] A1-4 Generated-body drift is acknowledged but still reported in sync
`sabrage/crates/sabrage-core/src/checks/meta.rs:104-108` — **CONFIRMED** (re-rated low)

The added documentation explicitly concedes that a hand-edited `contract.gen.sh` body is invisible at runtime, but the evaluator still returns Pass with the broad message that the files are “in sync.” A checkout that missed CI can therefore source divergent shell pins or ports while Doctor displays a clean row. Moving detection exclusively to CI does not close the confirmed runtime false-green.
Evidence: `sabrage/crates/sabrage-core/src/checks/meta.rs:41-51` says a modified body is `invisible to this check at runtime` and only becomes `a red CI run, not a red doctor row`; nevertheless `:104-108` returns `CheckOutcome::pass("meta.contract-sync", "contract/ in sync with scripts/demo/contract.gen.sh")`. The shell consumes that unchecked body via `source "$ROOT/scripts/demo/contract.gen.sh"` at `scripts/demo/lib.sh:9`.

*Recommendation:* Generate and record a digest of the complete generated body, validate it in both doctors, and add a fixture that changes a body scalar while preserving the existing contract header. At minimum, narrow the slug/message so it cannot claim full synchronization.

*Verifier:* Factually as described: the pass message asserts full synchronization while only the header is verified, so a checkout whose generated body was hand-edited (and pushed past PARITY_SKIP=1 / a cargo-less pre-push hook) gives a green doctor row on both sides while demo.sh sources divergent pins/ports. Kept at low, not medium: contract.gen.sh carries a DO-NOT-EDIT banner, CLAUDE.md forbids editing it, it is git-tracked, and the tier-1 parity test + CI workflow + pre-push hook all catch the drift — this is a defense-in-depth gap on an off-policy path, not a defect on any normal workflow. The native side is unaffected either way (it uses the compiled contract, never the shell body).

*Fix sketch:* In sabrage-contract-gen, emit a second header line `# contract-gen-body-sha256: <sha256 of everything after the header block>`; in checks::meta::meta_contract_sync recompute that digest from the on-disk contract.gen.sh body and fail with the existing 'contract.gen.sh was hand-edited' wording when it differs; mirror it in scripts/demo/doctor.sh section 0 with `sed '1,/^# contract-gen-body-sha256:/d' | shasum -a 256` so both doctors agree (zero Rust needed). Minimum viable alternative if the body digest is judged too much machinery: narrow the pass message on BOTH sides to say the header is fresh, not that the files are in sync.

*Regression test:* sabrage/crates/sabrage-core/src/checks/meta.rs #[cfg(test)]: the fixture used above — real contract/ copied to a scratch root, real contract.gen.sh with header preserved and one body scalar mutated — asserting Fail with the hand-edited message (today it asserts nothing because it Passes). Plus a sabrage-parity tier-1 case asserting doctor.sh and the native evaluator agree on that same fixture, and a sabrage-contract-gen test that the emitted body digest matches a manual recompute.

*Cross-area files:* sabrage/crates/sabrage-contract-gen/src/lib.rs, sabrage/crates/sabrage-parity/src/lib.rs, scripts/demo/doctor.sh, scripts/demo/contract.gen.sh, sabrage/PARITY.md

*Codex next steps:* Add mismatch tests for standalone Setup, Build, Install, Run, and each whole-stage Doctor fix; assert zero executor actions before the Fatal. · Add an `all`-chain regression proving contract mismatch aborts before Stage::Setup rather than at the final Run stage. · Add a preserved-header/mutated-body fixture and require both native and shell doctors to fail `meta.contract-sync`. · Run `cargo test -p sabrage-core -p sabrage-parity` and `scripts/dev/parity.sh --live=off` after implementing the guards.

## A3b — checks-doctor-config-net-game

**Codex verdict:** needs-attention — No ship: A3b-1 is only partially closed, the recovery documentation still prescribes a now-declared black-screen-causing deletion, and A3b-3's truthful error detail never reaches users.

### A3b-1 [medium, conf 0.99] [unfixed] A3b-1
`sabrage/crates/sabrage-core/src/config/runtime_toml.rs:670-674` — **CONFIRMED** (re-rated medium)

The accepted duplicate-assignment case now uses the last value, but launch still does not implement the runtime's last-valid-assignment rule. `effective_string` returns the last raw occurrence without whitelist validation, and preflight consumes it for both `protocol` and `encoder_process`. With the repository's existing fixture—valid `protocol = "alvr"` followed by invalid `protocol = "bogus"`—the runtime and Settings retain ALVR, while native preflight sees `bogus` and blocks launch. A trailing invalid encoder assignment can likewise invoke helper gates even when the runtime retains `inproc`. This directly contradicts the declared divergence that native launch uses the previous valid value.
Evidence: `sabrage/crates/sabrage-core/src/config/runtime_toml.rs:670-674` is `raw_assignments(...).filter(...).map(...).last()`, while `:621-625` states the actual runtime keeps the last value it would accept; `sabrage/crates/sabrage-core/src/stages/run/preflight.rs:125-142` calls this unfiltered helper for both facts.

*Recommendation:* Have preflight obtain modeled protocol and encoder values through a key-aware last-valid reader, preserving its caller-specific behavior for genuinely absent keys. Drive the existing `oxrsys-runtime.shadowed-invalid-last.toml` fixture through `read_toml_facts` and full preflight, asserting agreement with `read_lines_like_the_runtime`.

*Verifier:* Reproduced with the repo's own fixture. Not refutable as a declared divergence: PARITY.md declares the opposite behaviour, so the ledger is false as written. Mitigating (why medium, not high): the outcome matches scripts/demo/run.sh:57+70 (`awk -F'"' … END{print v}` is also last-raw), so shell parity is preserved and the failure mode is a fail-closed launch refusal with an actionable message, not a wrong backend. It also needs a hand-edited file with a duplicate assignment whose trailing occurrence is invalid.

*Fix sketch:* Add a key-aware last-valid reader next to `read_lines_like_the_runtime` (e.g. `effective_modeled_string(text, key) -> Option<String>`: fold `raw_assignments` through `Setting::read_raw`, keep the last Ok(Some(_)) rendering, fall back to None when no occurrence is accepted). Point `stages::run::preflight::read_toml_facts` at it for `protocol` and `encoder_process` while keeping `effective_string` for unmodeled keys, and preserve the "absent key -> empty string / auto" caller behaviour. Alternatively (smaller): leave the code and correct sabrage/PARITY.md:115 to say launch uses last-raw like the doctors — but then the fixture's documented runtime semantics and the launch gate stay out of sync.

*Regression test:* sabrage/crates/sabrage-core/src/stages/run/preflight.rs `mod tests` (next to the existing effective_string cases at :860-870): drive tests/fixtures/phase4/oxrsys-runtime.shadowed-invalid-last.toml through `read_toml_facts` and assert protocol == "alvr" and encoder_process == "native", i.e. agreement with `runtime_toml::read_lines_like_the_runtime`'s RuntimeConfigValues; plus a full-preflight case asserting `cfg.protocol.supported` does not die on that fixture.

*Cross-area files:* sabrage/crates/sabrage-core/src/config/runtime_toml.rs, sabrage/crates/sabrage-core/src/stages/run/preflight.rs, sabrage/PARITY.md

### A3b-2 [high, conf 0.99] Known-bad session deletion remains the official troubleshooting fix
`docs/troubleshooting.md:36-36` — **CONFIRMED** (re-rated medium)

The fix now warns users not to delete `session.json`, and the divergence ledger calls deletion a proven-broken remedy that produces an 800x900 black screen. However, the primary troubleshooting table still tells users encountering the exact stale-IP symptom to delete that file; `CLAUDE.md` repeats the same instruction. A user following repository documentation can therefore take the destructive action this branch specifically withheld and regress from a connection problem to the documented black-screen failure.
Evidence: `docs/troubleshooting.md:36` says `delete ~/Library/Application Support/OXRSys/alvr/session.json`, while `sabrage/crates/sabrage-core/src/checks/config.rs:334-336` says `do not delete the file: a recreated session.json streams a black 800x900 screen`; `CLAUDE.md:94` also still recommends deletion.

*Recommendation:* Update `docs/troubleshooting.md` and `CLAUDE.md` in the same change to prescribe editing or clearing the relevant `manual_ips` entry in place while preserving the file. Add a consistency check preventing user-facing documentation from recommending session.json deletion.

*Verifier:* Both doc lines exist verbatim and target the same symptom the code now refuses to auto-fix. Not refuted by any PARITY.md entry — PARITY.md:116 argues the other way. Downgraded from high to medium: it is documentation, not a code path; the resulting black-screen state is recoverable, and the repository's primary interactive surfaces (Sabrage doctor row, doctor.sh row, the withheld fix) all already give the correct advice, so a user has to follow the prose table rather than the tool.

*Fix sketch:* docs/troubleshooting.md:36 — replace the deletion clause with 'edit (or clear) the `manual_ips` entry for the client in `~/Library/Application Support/OXRSys/alvr/session.json` in place; do not delete the file (a recreated session.json streams a black 800x900 screen)'. CLAUDE.md:94 — same substitution in the Config/state bullet. Optionally add a grep guard (a test in sabrage-parity or a doctor `meta.*` row) asserting no tracked user-facing doc matches /delete .*session\.json/.

*Regression test:* sabrage/crates/sabrage-parity/tests — a hermetic test that reads docs/troubleshooting.md, CLAUDE.md and README.md and asserts none of them recommends deleting `alvr/session.json` (regex over the file text), mirroring the existing 800x900 consequence assertion at sabrage/crates/sabrage-core/src/fixes/session_json.rs:241.

*Cross-area files:* docs/troubleshooting.md, CLAUDE.md, sabrage/crates/sabrage-parity/tests

### A3b-3 [low, conf 0.98] A3b-3 hides the real error behind an impossible Python diagnosis
`sabrage/crates/sabrage-core/src/checks/config.rs:317-326` — **CONFIRMED** (re-rated low)

The false-green status is fixed, but both new failure branches display `(broken python3?)` even though the native check uses Rust filesystem and serde code. The actual read/parse error exists only in `detail`; the GUI renders only `message` and `remedy`, and the CLI formatter also ignores `detail`. Consequently, ordinary users cannot distinguish malformed JSON from a permission failure and receive a diagnosis that cannot be true on this code path.
Evidence: `sabrage/crates/sabrage-core/src/checks/config.rs:317-326` pairs the `broken python3?` message with truthful `read error`/`JSON parse error` details; `sabrage/ui/src/components/CheckRow.svelte:39-43` renders no `row.detail`, and `sabrage/crates/sabrage-cli/src/main.rs:440-449` formats only `o.message`/`o.remedy`.

*Recommendation:* Use an accurate native message such as `could not inspect <path>` and render `detail` as secondary GUI text. Expose it in an appropriate CLI mode without disturbing default shell-parity output, then add surface-level malformed and permission-denied tests.

*Verifier:* Verified by reading both render paths; no third surface consumes `detail`. Parity is not a defence: scripts/demo/doctor.sh:200-209 catches every json/open exception (`except Exception: sys.exit(0)`) and reports `chk ok` for malformed/unreadable files, so the native Warn already diverges from the shell on these two branches — the borrowed message text buys no tap-channel parity (tap carries slug+status only). Severity low: a warn row with misleading wording, no wrong action taken.

*Fix sketch:* In `cfg_session_pins` (checks/config.rs:317-331) give the Unreadable/Malformed arms an accurate message, e.g. `could not inspect {path} ({e})` or `could not read/parse {path}` with `detail` kept, and leave the Corrupt arm's shell-mirroring text alone (or reword it too). Then surface `detail`: add `{#if row.detail}<div class="text-muted detail">{row.detail}</div>{/if}` to CheckRow.svelte's body, and print it in the CLI only in a verbose mode so default output stays shell-parity byte-identical. Update sabrage/PARITY.md:18, whose "can never fire natively" claim is stale.

*Regression test:* sabrage/crates/sabrage-core/src/checks/config.rs `mod tests`: two cases writing a temp `alvr/session.json` — one with invalid JSON, one with mode 0o000 (skip if running as root) — asserting `CheckOutcome::status == Warn`, that `message` does NOT contain "python3", and that `detail` names the parse/read error. Plus a CheckRow.svelte assertion in the UI test suite (or a snapshot) that a row with `detail` renders it.

*Cross-area files:* sabrage/ui/src/components/CheckRow.svelte, sabrage/crates/sabrage-cli/src/main.rs, sabrage/PARITY.md

*Codex next steps:* Add a full-preflight regression using `oxrsys-runtime.shadowed-invalid-last.toml`. · Update both stale session.json recovery documents and add a consistency guard. · Verify malformed and unreadable session warnings in the GUI and CLI, including visible native error details. · Run `cargo test -p sabrage-core`, `cargo test -p sabrage-parity`, and the hermetic parity tier after the fixes.

## A4 — lock-and-fixes

**Codex verdict:** needs-attention — No-ship: A4-1, A4-2, A4-3, and A4-5 remain open end-to-end. A4-4, A4-7, A4-9, and A4-10 appear closed on their cited paths.

### A4-1 [high, conf 0.99] [unfixed] A4-1 Queued mutations are not rechecked after a session starts
`sabrage/crates/sabrage-core/src/stages/mod.rs:770-794` — **CONFIRMED** (re-rated medium)

Stage and fix entry points check liveness before potentially waiting on the operation lock, then dispatch without checking again. A concurrent run publishes its live handle and session record before releasing that lock. Inference: an operation admitted while idle can wait, acquire the lock after launch, and run setup/build/install or another forbidden fix against the active session.
Evidence: `sabrage/crates/sabrage-core/src/stages/mod.rs:774-794` executes `deny_stage_while_session_live(stage, ctx)?`, awaits `acquire_operation_lock_cancellable`, then calls `dispatch(stage, ctx).await` with no second denial; `sabrage/crates/sabrage-core/src/fixes/mod.rs:346-348` has the same `deny → await lock → apply` ordering. `sabrage/crates/sabrage-core/src/stages/run/mod.rs:444-477` calls `set_live_session(...)` before `drop(lock)`.

*Recommendation:* Keep the pre-lock fast refusal, but re-run the same liveness predicate immediately after acquiring the lock and before dispatch. Preserve the launch-only `apply_holding_lock` exemption. Add a test that queues setup/build/install and a forbidden fix, publishes a live session while they wait, releases the lock, and asserts no executor mutation occurs.

*Verifier:* The pre-lock refusal is a TOCTOU: liveness is sampled at admission, the wait can span the whole of `run`'s pre-launch phase, and nothing re-samples after the guard is handed over. Partial mitigation the reviewer missed (so severity is medium, not high): four of the five non-stage fixes re-assert liveness inside their own bodies after the lock — fixes/backend.rs:297 (bottle_wineserver_is_live), fixes/session_json.rs:77 (any_wineserver_alive), fixes/adb.rs:140 (live_session_block), config/runtime_toml.rs:1488 (edit-protocol). What is genuinely unguarded is Stage::Setup/Build/Install (no in-body liveness check anywhere in stages/{setup,build,install}.rs) and FixAction::RestageHelper (fixes/helper.rs has no liveness reference at all). Reachable in-process (GUI stage queued behind `run`'s pre-launch phase) and cross-process (GUI + `sabrage` CLI serialized only by the advisory file lock).

*Fix sketch:* In stages::run_stage, after `let Some(guard) = acquire_operation_lock_cancellable(...)` and before the `stage == Stage::Run` branch, call deny_stage_while_session_live(stage, ctx)? a second time (cheap: it is a fixture/proc read) and route the refusal through finish_stage so the already-emitted StageStarted gets its StageFinished. In fixes::apply, after `let _guard = crate::stages::acquire_operation_lock().await`, re-run deny_if_session_live(action, ctx)? before delegating to apply_holding_lock; leave apply_holding_lock itself unchecked (it is the launch preflight's door, documented at fixes/mod.rs:40-42).

*Regression test:* sabrage/crates/sabrage-core/src/stages/mod.rs `mod tests`: a #[tokio::test] next to run_stage_refuses_setup_build_and_install_while_a_session_is_live that holds acquire_operation_lock(), spawns run_stage(Stage::Build) on an idle ctx_at() fixture, writes a fresh runtime_status.json while it waits, drops the guard, and asserts the result is Err containing 'refusing to run build while a session is live' and that the DryRunExecutor recorded no planned action; a sibling test in fixes/mod.rs `mod tests` doing the same for FixAction::RestageHelper via fixes::apply, plus one asserting apply_holding_lock still runs under a live session.

### A4-2 [high, conf 1.0] [unfixed] A4-2 The frontend bypasses the deferred-fix registry
`sabrage/ui/src/ipc.ts:305-313` — **CONFIRMED** (re-rated high)

Rust withholds `fix.delete-session-json`, but the TypeScript mirror still models it and resolves any `FIX_META` key as offerable. `CheckRow` therefore renders a Fix button for the contract's `cfg.session-pins` row, and Doctor dispatches the known-black-screen deletion after confirmation. The consequence dialog reduces surprise but does not implement the branch's declared no-button safeguard.
Evidence: `sabrage/ui/src/ipc.ts:264-273` retains the `"delete-session-json"` metadata, and `sabrage/ui/src/ipc.ts:311-313` returns `bare in FIX_META ? (bare as FixAction) : null`; `sabrage/ui/src/components/CheckRow.svelte:25-50` renders and invokes the button whenever that resolver is non-null. This contradicts `sabrage/crates/sabrage-core/src/fixes/mod.rs:149-158`, where deferred IDs return `None` specifically so the GUI offers no button.

*Recommendation:* Make `contractFixIdToAction("fix.delete-session-json")` return null and add a component test proving `cfg.session-pins` renders no Fix button. Prefer supplying the offered-fix set from Rust or the contract so the TypeScript mirror cannot bypass future deferrals.

*Verifier:* The deferral is enforced in exactly one place (a Rust parser the GUI does not use) while the offer surface is the TypeScript mirror. A user therefore gets a one-click button for the known-broken remedy (PARITY.md/CLAUDE.md: deleting session.json leaves the client at an 800x900 black screen) that the branch declares must have no button. Not critical only because session_json.rs backs the file up to Application Support/Sabrage/backups first and a consequence dialog is shown.

*Fix sketch:* Add the deferred-id set to sabrage/ui/src/ipc.ts (e.g. `const DEFERRED_FIX_IDS = new Set(["fix.create-z-drive", "fix.delete-session-json"])`) and return null from contractFixIdToAction for them, keeping the FIX_META entry only for titles/consequence rendering — or, better, drop the mirror and have Doctor read an offered-fix set exposed by a Tauri command over fixes::FixAction::from_contract_id. Add backend defence-in-depth in sabrage/src-tauri/src/commands.rs::fix: reject an action whose to_contract_id() is in sabrage_core::fixes::DEFERRED_CONTRACT_FIX_IDS.

*Regression test:* No JS test harness exists in sabrage/ui (no vitest/test script in package.json), so pin it in Rust: a test in sabrage/src-tauri/src/commands.rs `mod tests` that reads ../ui/src/ipc.ts and asserts every entry of DEFERRED_CONTRACT_FIX_IDS appears in the file's deferred set literal (and that FixAction::from_contract_id returns None for each), plus a unit test that commands::fix returns Err for FixAction::DeleteSessionJson even with confirmed: true while it stays deferred.

*Cross-area files:* sabrage/ui/src/ipc.ts, sabrage/src-tauri/src/commands.rs, sabrage/ui/src/components/CheckRow.svelte, sabrage/ui/src/screens/Doctor.svelte

### A4-3 [medium, conf 0.99] [unfixed] A4-3 Cross-process serialization silently fails open
`sabrage/crates/sabrage-core/src/stages/mod.rs:478-529` — **REFUTED** (re-rated low)

The advisory lock is optional: failure to create/open the lock file or any non-contention `flock` error produces an `OperationGuard` containing only the process-local mutex. Two GUI/CLI processes then mutate concurrently with no warning. This finding does not challenge the declared `demo.sh` exclusion; it breaks the promised GUI/CLI serialization under a realistic damaged or unwritable support directory.
Evidence: `sabrage/crates/sabrage-core/src/stages/mod.rs:478-483` defines `_file: Option<File>` and states that cross-process exclusion degrades; `sabrage/crates/sabrage-core/src/stages/mod.rs:526-533` converts `FileLock::Unavailable` to `None` and still returns a guard, while `sabrage/crates/sabrage-core/src/stages/mod.rs:575-603` maps open and lock errors to `Unavailable`.

*Recommendation:* Distinguish cancellation from lock-establishment failure and fail closed before dispatching a mutation, with the lock path and OS error in the fatal event. Add injected open/lock-failure tests asserting the stage body is never reached.

*Verifier:* This is a documented, deliberate design decision, not a missed guard, and the reviewer's own recommendation (distinguish cancellation) is already implemented. The precondition — ~/Library/Application Support/Sabrage not creatable, or an flock-less filesystem — is a machine state in which the settings store, the game library, the runtime-toml backups and session-state.json (paths.rs:87, store/settings.rs, store/library.rs) are all equally broken, so it is not a realistic 'GUI and CLI silently race' scenario; on it the correct product answer is arguably still to run rather than refuse. The in-process mutex, which covers the GUI's own concurrency, is never bypassed.

### A4-4 [high, conf 0.99] [unfixed] A4-5 The GUI's mandatory recheck turns an ADB query failure green
`sabrage/crates/sabrage-core/src/checks/network.rs:82-121` — **CONFIRMED** (re-rated medium)

The fix now emits a warning, but Doctor captures only Fatal events, discards the returned report, and always reruns its checks. That recheck still folds an ADB spawn failure into an empty list, ignores non-zero exit status, and reports `no stale adb port forwards`. A persistent ADB failure therefore ends in the exact clean UI state the round-1 fix was meant to eliminate while stale forwards may still break WiFi discovery.
Evidence: `sabrage/crates/sabrage-core/src/checks/network.rs:82-91` returns `Vec::new()` on spawn failure and never checks `out.status`; `sabrage/crates/sabrage-core/src/checks/network.rs:110-121` converts that empty vector into `CheckOutcome::pass(..., "no stale adb port forwards")`. Meanwhile `sabrage/ui/src/screens/Doctor.svelte:113-131` records only `ev.kind === "fatal"` and unconditionally calls `runChecks()` in `finally`, so the warning emitted at `sabrage/crates/sabrage-core/src/fixes/adb.rs:185-191` is not retained.

*Recommendation:* Give the doctor probe a fallible result and render query failures as Warn or Skipped/unknown, never Pass. Surface the fix's warning/report in Doctor until a successful re-probe supersedes it. Test spawn failure and non-zero `forward --list` through the complete apply-fix-and-rerun flow.

*Verifier:* The doctor row going green on an adb query failure is intentional shell parity and cannot be 'fixed' in Rust alone; but the round-1 warning that was supposed to keep the user informed is dropped on the floor by the GUI, so a persistent adb failure ends in exactly the clean UI state the round-1 fix targeted. Realistic path (adb present but its server/daemon failing) and user-visible, but the damage is a missing warning rather than a wrong mutation, hence medium.

*Fix sketch:* In sabrage/ui/src/screens/Doctor.svelte::runFix, capture warn events (and the resolved FixReport's `description` when `changed === false`) alongside the fatal, and render them as a persistent per-slug notice above/next to the row that survives the `runChecks()` repaint until the next successful application of that fix. Optionally give checks::network::adb_forward_local_specs a Result and, in the SAME commit, teach doctor.sh 16b to distinguish a failed `adb forward --list` (new `chk warn` slug text) plus regenerate contract/scripts/demo/contract.gen.sh via scripts/dev/parity.sh --regen — parity forbids doing that on one side only.

*Regression test:* sabrage/crates/sabrage-core/src/fixes/adb.rs `mod tests`: assert remove_adb_forwards_at with an adb path that cannot be spawned emits exactly one Severity::Warn StageEvent containing 'could not query adb forwards' and returns FixReport::unchanged (pins the payload the UI must render). For the UI half there is no JS harness today; add the smallest one (vitest + a Doctor.svelte component test asserting the warn text is still in the DOM after runChecks() resolves) or, if that is out of scope, a Tauri-side test asserting commands::fix forwards non-fatal events to the channel.

*Cross-area files:* sabrage/ui/src/screens/Doctor.svelte, sabrage/crates/sabrage-core/src/checks/network.rs, scripts/demo/doctor.sh, contract/pipeline.toml

*Codex next steps:* Add lock-handoff tests where liveness changes while a stage or fix is queued. · Add a frontend test asserting `fix.delete-session-json` resolves to null and renders no button. · Inject advisory-lock open/lock errors and prove every mutation fails before dispatch. · Exercise persistent ADB spawn and non-zero failures through the full Doctor fix/recheck path.

## A5 — stages-setup-build-stop

**Codex verdict:** needs-attention — No-ship. A5-1 and A5-7 are only partially closed, and setup still has a critical write-once race. A5-2, A5-3, A5-4, and A5-5 appear closed in the current code; A5-6 is not re-reported.

### A5-1 [medium, conf 0.99] [unfixed] A5-1 Setup dry-run still reports unperformed writes as completed
`sabrage/crates/sabrage-core/src/stages/setup.rs:227-280` — **CONFIRMED** (re-rated medium)

When all DXMT payload files exist but the marker is absent or stale, `dxmt_ok` starts the dry-run extraction plan, `dxmt_files_ok` remains true, and the stage emits a green row claiming the marker was written although DryRunExecutor wrote nothing. A missing runtime TOML has the same defect: it reports `wrote` after merely planning the write. The new ready-checkout test explicitly creates no marker and blesses the false Ok row.
Evidence: `sabrage/crates/sabrage-core/src/stages/setup.rs:227-240` computes `let extracted_ok = util::dxmt_files_ok(...)` and then emits `st.ok("extracted ext/dxmt-artifacts (provenance marker written)")`; lines 260-280 call `write_atomic` and unconditionally emit `st.ok(format!("wrote {} ..."))`. Lines 755-781 construct a marker-absent fixture and require that extraction Ok.

*Recommendation:* Under dry-run, base severity and tense on the entire claimed postcondition. Any planned marker or TOML write must emit an Info `would ...` row; reserve Ok for a marker already containing the pinned bytes or a config already present.

*Verifier:* Both sub-claims reproduce verbatim under `sabrage setup --dry-run`, an advertised CLI flag (sabrage-cli/src/main.rs:116). Two Ok rows assert completed on-disk state that DryRunExecutor never established, which is precisely the invariant the file's own dry-run-honesty tests and module doc exist to enforce (SUBMODULES_WOULD_INIT_INFO / PATCHSET_WOULD_CHECKOUT_INFO / DXMT_WOULD_EXTRACT_INFO). Text-only, no machine state changes, so medium rather than high.

*Fix sketch:* setup.rs `setup_pinned`: capture the marker's truth (`util::dxmt_ok(&ctx.paths)` before the plan, or `extracted_ok && !exec.is_dry_run()`) and emit the Ok row only when files AND a current marker are on disk; otherwise emit DXMT_WOULD_EXTRACT_INFO (or a marker-specific `would write the provenance marker` Info). setup.rs `setup_config`: wrap the write branch in `if exec.is_dry_run() { st.info(format!("would write {} (protocol=alvr, 42 Mbps, encoder_process=auto)", ...)) } else { st.ok(...) }`, keeping the real-run text byte-identical to setup.sh:53.

*Regression test:* sabrage/crates/sabrage-core/src/stages/setup.rs tests mod: flip `a_dry_run_over_an_already_set_up_checkout_still_reports_the_ok_rows` (marker-absent fixture) to require the Info would-row and forbid the Ok `extracted ext/dxmt-artifacts (provenance marker written)`, and add a `setup_config` dry-run test asserting no `Severity::Ok` row starting with `wrote ` plus `!paths.toml_path.exists()`; keep the existing real-run `config_writes_the_shared_template_when_absent` Ok assertion.

### A5-2 [medium, conf 0.96] [unfixed] A5-7 A local helper match suppresses cross-checkout detection
`sabrage/crates/sabrage-core/src/stages/stop.rs:219-229` — **CONFIRMED** (re-rated low)

The foreign-helper scan runs only when no helper from the current checkout matched. With one stale helper from checkout A and another from the current checkout B, Stop kills B, skips the foreign scan, and never reports A. This preserves the original cross-checkout blind spot on a realistic multi-worktree leftover state.
Evidence: `sabrage/crates/sabrage-core/src/stages/stop.rs:219-229` assigns `helper_matched = reap(...)` and gates the only `report_foreign_helpers(...)` call behind `if !helper_matched`. Inference: when both local and foreign helpers are in the process snapshot, the true local result makes the foreign process unreachable to reporting.

*Recommendation:* Run the report-only foreign-helper scan after every local reap, preferably from a fresh snapshot, and add a regression with simultaneous local and foreign helper processes.

*Verifier:* Real, but the residual gap is a missing additive warning, not a false statement: with a local match the stage prints `encoder helper killed …` and the `no leftover encoder helper` Ok row (the false-green the fix targeted) is only ever printed from report_foreign_helpers itself. The shell reference reports nothing at all here (`pkill -f "$OXR_HELPER_BIN"`, scripts/demo/stop.sh:20-22), so native remains strictly ahead of parity, and the scenario needs two checkouts with live/leftover helpers simultaneously.

*Fix sketch:* stop.rs::run — call `report_foreign_helpers(ctx, &ctx.paths.root, scan.by_cmdline(HELPER_BASENAME))` unconditionally after the helper reap, and make it emit NO_LEFTOVER_HELPER only when `!helper_matched && foreign.is_empty()` (pass `helper_matched` in) so the killed-row and the not-found row never both print. Re-scanning (`ProcessScan::scan()`) after the reap is optional; the existing snapshot is enough for a report-only row.

*Regression test:* sabrage/crates/sabrage-core/src/stages/stop.rs tests mod, alongside the existing foreign-helper tests: feed a synthetic Vec<ProcInfo> containing one exe under `paths.root` and one under a foreign root into the (made pub(crate)-testable) reporting path and assert the foreign `leftover encoder helper from another checkout: <pid> <exe>` Warn is emitted while NO_LEFTOVER_HELPER is not.

### A5-3 [critical, conf 0.97] Setup's write-once config creation can overwrite a concurrent writer
`sabrage/crates/sabrage-core/src/stages/setup.rs:260-276` — **CONFIRMED** (re-rated low)

The promised write-once invariant is implemented as a check followed by an atomic replacement, not an exclusive create. An editor, runtime, or concurrent `demo.sh` process can create a hand-maintained TOML after the probe; Sabrage then renames its template over that file without a backup, irreversibly losing the intervening contents. The Sabrage flock does not cover `demo.sh` or editors.
Evidence: `sabrage/crates/sabrage-core/src/stages/setup.rs:260-276` first checks `if ctx.paths.toml_path.is_file()` and later calls `exec.write_atomic(&ctx.paths.toml_path, ...)`. The two operations have no compare-and-swap or exclusive-create guard. Inference: another creation between them enters the absent branch and is replaced by the atomic write.

*Recommendation:* Use the existing Executor exclusive-create primitive for initial TOML creation. If it reports that another writer won, re-read and report the resulting config instead of replacing it; add a barrier-controlled concurrent-creator regression.

*Verifier:* The TOCTOU is factually there, but the 'critical / irreversible data loss' framing does not hold. (a) The shell reference does the same check and worse — scripts/demo/setup.sh:44-54 `if [ -f "$TOML" ] … else cat contract/oxrsys-runtime.toml.template > "$TOML"`, a truncating non-atomic write — so this is not a native-only regression. (b) The only realistic concurrent creator is the other front-end or the oxrsys runtime, and both create the same contract-shared template bytes, so nothing distinguishable is lost. (c) A human/editor authoring a hand-maintained TOML inside a sub-millisecond window is not a realistic path. Worth fixing cheaply (the primitive exists), hence low rather than refuted.

*Fix sketch:* setup.rs `setup_config`: replace the else-branch `write_atomic` with `exec.create_new(&ctx.paths.toml_path, util::toml_template().as_bytes())`; on `Ok(true)` keep the byte-identical `wrote {} (protocol=alvr, 42 Mbps, encoder_process=auto)` Ok row, on `Ok(false)` (someone else won) re-read the file, run it through `parse_protocol_awk`, and emit the existing `config present: …`/`config present with protocol='…'` rows instead of replacing anything. Keep the dry-run tense fix from A5-1 in the same branch.

*Regression test:* sabrage/crates/sabrage-core/src/stages/setup.rs tests mod: a test that pre-creates `paths.toml_path` with distinctive bytes and calls `setup_config` with a stub executor whose `create_new` reports `false`, asserting the file is byte-unchanged and a `config present: …` row is emitted (plus the existing absent-file test still asserting the template and the `wrote …` Ok row).

*Cross-area files:* sabrage/PARITY.md

### A5-4 [medium, conf 1] Ninja builds bypass the Executor mutation boundary
`sabrage/crates/sabrage-core/src/stages/build.rs:381-410` — **REFUTED** (re-rated low)

The two Ninja build mutations take a separate real-run path that calls `process::run_ok` directly. Dry-run goes through Executor, so real and preview no longer share the mandated mutation path. Any executor policy, instrumentation, denial, or future safety behavior is silently bypassed for the oxrsys and helper builds.
Evidence: `sabrage/crates/sabrage-core/src/stages/build.rs:381-410` delegates only the dry-run arm to `run_child_ok`/`ctx.executor`, then executes `process::run_ok(&spec, ...)` directly on real runs. That helper is used for the mutating CMake builds at lines 525-536 and 567-578.

*Recommendation:* Keep every build spawn behind Executor. Add an executor progress/output hook or derive Ninja progress from the executor's emitted events instead of creating a direct real-run process path.

*Verifier:* There is no executor policy, instrumentation or denial to bypass on the real path — `run_child` and `run_ok` bottom out in the identical function with the identical sink and cancellation token, and the dry-run/plan boundary the finding is really about is explicitly preserved by the `is_dry_run()` early return at build.rs:382-384. The only observable delta is that a failing ninja build carries a captured tail in `SabrageError::ChildFailed` instead of an empty one, which is strictly more information and is not what the finding alleges.

### A5-5 [medium, conf 0.9] Hung stop probes ignore cancellation and can block teardown indefinitely
`sabrage/crates/sabrage-core/src/stages/stop.rs:359-367` — **CONFIRMED** (re-rated medium)

The cancellation checkpoints cannot help while either reporting subprocess is awaiting completion. A present `lsof` blocked in the OS or a `SwitchAudioSource` blocked by degraded CoreAudio can hold Stop and the operation lock indefinitely; in the `lsof` case, helper reaping and persisted guard restoration are never reached. This is distinct from the refuted A5-6 missing-binary/status case: the executable exists but never returns.
Evidence: `sabrage/crates/sabrage-core/src/stages/stop.rs:359-367` awaits `Command::new("lsof").output()` without a timeout or cancellation select, and lines 608-613 do the same for `SwitchAudioSource`. The corresponding checkpoints occur only after these awaits at lines 214-215 and 246-247.

*Recommendation:* Run both probes with a bounded, cancellation-aware capture path and `kill_on_drop(true)`. Return an explicit unknown/error result on timeout and render Warn rather than blocking or claiming a clean state.

*Verifier:* The reviewer's read of the code is exact and the guard they might have missed does not exist: `checkpoint` only fires between steps, never during an await, and nothing wraps the stage in a deadline. Realistic triggers exist — `lsof` is invoked with `-nP` but not `-b`, so it can block in the kernel on a hung/stale network mount, and `SwitchAudioSource -c` can block on a degraded coreaudiod (the very reason guards.rs bounds its own copies). Impact when it fires is severe (Stop hangs forever, Cancel is inert, the operation lock is never released so no further Sabrage stage can start, and the previous session's audio/dashboard/adb guards are never restored), but the trigger is an unusual machine state, so medium rather than high. Distinct from the refuted missing-binary case: spawn succeeds, the child just never exits.

*Fix sketch:* Route both probes through `crate::process::capture_with(&spec, &ctx.cancel, process::DEFAULT_PROBE_TIMEOUT)`, mirroring `run/guards.rs::list_output_devices`. (1) `stale_listeners()` gains a `&StageCtx` parameter and builds `ctx.child("lsof", step::STOP_PORTS).args(LSOF_ARGS).env_path(process::default_child_path())`; return a tri-state (`Ok(String)` / `Err(unknown)`) rather than `String`, so `report_ports` renders `warn("could not read the streaming ports: <reason>")` on timeout instead of the false `ok("streaming ports free")` that today's `Err(_) => String::new()` arm produces. (2) `report_audio` builds the same spec for `SwitchAudioSource -c -t output` and, on `Err`, warns "could not read the current audio output device" instead of falling through `unwrap_or_default()` → `audio_report("")` → `ok("audio output: ")` (stop.rs:583-589). (3) Propagate `SabrageError::Cancelled` from both (or let the following `checkpoint` catch it) so Cancel during a probe still exits 130. Note the ports probe duplicates `checks/network.rs:39`, which uses a blocking `std::process::Command::output()` with the same hole — worth the same treatment, but it is outside this area.

*Regression test:* Two `#[tokio::test]`s in `sabrage/crates/sabrage-core/src/stages/stop.rs`'s existing `mod tests`, shaped like `run/guards.rs::list_output_devices_honors_an_already_cancelled_token`: (a) with `ctx.cancel` already cancelled and a scratch `lsof`/`SwitchAudioSource` stub that `sleep 30`s, assert `report_ports`/`report_audio` return in well under the probe timeout and that `run(&ctx)` yields `Err(SabrageError::Cancelled)`; (b) with a stub whose runtime exceeds a short injected deadline, assert the emitted row is a `Warn` naming the unreadable probe and never `"streaming ports free"` / `"audio output: "` with an empty device.

*Codex next steps:* Add dry-run fixtures for files-complete/marker-missing DXMT and absent runtime TOML; assert no completed-state Ok rows. · Replace setup's TOML check/write with exclusive creation and race it against a controlled concurrent writer. · Exercise Stop with simultaneous local and foreign helper processes and require the foreign warning. · Inject non-returning `lsof` and `SwitchAudioSource` stand-ins; verify timeout, cancellation, and continued teardown. · Route Ninja builds through Executor, then run the focused stage tests and `scripts/dev/parity.sh`.

## A6 — install-privilege

**Codex verdict:** needs-attention — Do not ship. A6-1, A6-2, and A6-5 appear closed, but A6-3 and A6-4 are not. The registry fix also introduces a cancellation path that can report success or enter privileged installation after Cancel.

### A6-1 [high, conf 0.99] Cancellation during the registry poll is swallowed and can enter the privileged layer
`sabrage/crates/sabrage-core/src/stages/install.rs:461-470` — **CONFIRMED** (re-rated high)

The new poll represents both timeout and cancellation as `false`. Its caller interprets that as a lazy-flush warning, emits `OK ActiveRuntime registered`, and continues into layer 4. Inference: cancelling during this two-second window produces a successful stage when the host manifest is current; when stale, the privilege path can spawn sudo/osascript before its child helper observes the already-cancelled token, potentially showing an authorization prompt or completing the root write after Cancel.
Evidence: `sabrage/crates/sabrage-core/src/stages/install.rs:467-468` says `if ctx.cancel.is_cancelled() ... { return false; }`, while `:288-295` converts `false` into WARN + OK and continues. `sabrage/crates/sabrage-core/src/privilege.rs:444-448` stages and begins elevation without a cancellation guard, and `:596-605` spawns before selecting on cancellation.

*Recommendation:* Return `Result<bool>` from the poll and propagate `SabrageError::Cancelled`; also guard cancellation before layer 4 and before each raw elevation spawn. Add a test cancelling during the poll that asserts no OK, NeedsAdmin, staging write, or child spawn follows.

*Verifier:* The cancellation-swallow is real and its blast radius is the privileged layer. Two user-visible consequences on a realistic path (Stop pressed during install — the poll is a 2s window, and cancellation anywhere between the `reg add` child returning and the elevation is equally unguarded): (a) the run log says Warn+OK and StageFinished ok=true/exit 0 after the user cancelled; (b) with a non-current /usr/local/share/openxr/1/active_runtime.x86_64.json, install emits NeedsAdmin, writes the staging file and spawns osascript/sudo after Cancel — an authorization dialog for a root write the user just aborted (it is killed after CANCEL_REAP_GRACE=500ms, privilege.rs:158, but is shown, and a fast enough authorization completes the root write). Not covered by any PARITY.md divergence: line 117 declares only the re-probe, line 118 declares the *inside*-elevation cancel semantics, neither declares 'a cancelled install reports success'. Severity high rather than critical: the post-cancel work is confined to layer 4 (the DXMT/bottle layers do stop, because RealExecutor::guard, executor.rs:404, aborts every primitive), and the only state it can reach is the host manifest write the stage was going to do anyway.

*Fix sketch:* 1) `wait_for_registry_flush` (install.rs:461) returns `Result<bool>`: `Err(SabrageError::Cancelled)` when `ctx.cancel.is_cancelled()`, `Ok(false)` only on real timeout; the caller at install.rs:288 uses `?` so a cancelled poll ends the stage before the Warn/OK rows are emitted. 2) Add an explicit cancellation check at the top of layer 4 in `install::run` (before `ctx.section("host OpenXR registration")`), so a Stop between `reg add` and the elevation also stops there. 3) In `privilege::write_host_manifest_privileged` (privilege.rs:~430, right after the `is_dry_run` branch) return `Err(SabrageError::Cancelled)` before `ctx.emit(NeedsAdmin)`/`StagedTemp::create`, and add the same pre-spawn check at the head of `run_capturing`/`run_inheriting` so an already-cancelled token never spawns osascript/sudo at all.

*Regression test:* In `sabrage/crates/sabrage-core/src/stages/install.rs` tests mod, next to `a_late_system_reg_flush_is_waited_for_instead_of_warned_about`: a `real_seeming_fixture()` whose TestExecutor cancels `ctx.cancel` inside `run_child` (the `reg add`); assert `run(&ctx)` returns `Err(SabrageError::Cancelled)`, and that no event after the cancel is a Warn/`ok("ActiveRuntime registered")`, no `StageEvent::NeedsAdmin`, and no `SECTION: host OpenXR registration` row. Plus a privilege.rs unit test: `write_host_manifest_privileged` with a pre-cancelled token and a non-current dest returns `Cancelled`, emits no NeedsAdmin, and leaves `sabrage_temp_dir()` without a new staging file.

### A6-2 [high, conf 0.99] [unfixed] A6-4 Registry polling waits for a token that does not satisfy the launch gate
`sabrage/crates/sabrage-core/src/stages/install.rs:429-465` — **CONFIRMED** (re-rated low)

The launch-equivalent predicate requires `ActiveRuntime`, `openxr`, and `wineopenxr64.json` in order on one line, but the new poll succeeds on any occurrence of `ActiveRuntime`. A bottle containing an old or malformed ActiveRuntime value therefore ends the poll immediately before the new value flushes; Install emits success and an immediate Launch still blocks. The regression test explicitly treats `"ActiveRuntime"=` with no value as a sufficient flush, codifying the contradiction rather than closing it.
Evidence: `sabrage/crates/sabrage-core/src/stages/install.rs:435-443` defines the three-literal `registry_current` predicate, but `:464-465` returns success from `system_reg_contains(system_reg, "ActiveRuntime")`. The test at `:1486-1495` writes only `"ActiveRuntime"=` and expects no warning.

*Recommendation:* Poll `registry_current(system_reg)`, the same predicate used by the blocking gate. Replace the half-entry test with a complete registry value and add a case where a stale bare ActiveRuntime remains present until the correct value flushes.

*Verifier:* Mechanically true and reachable, but much smaller than stated. (a) The `ok("ActiveRuntime registered")` row is emitted on BOTH poll outcomes (install.rs:293-295) and in the shell too (install.sh:41-42), so 'Install emits success' is not caused by the predicate. (b) The bare grep is deliberate byte-parity with install.sh:41 (`grep -q 'ActiveRuntime'`) and documented as such at install.rs:446-449, so this is not a divergence bug — the shell behaves identically. (c) The pre-state required is a bottle whose ActiveRuntime is present but does not match the full launch predicate, i.e. a bottle previously pointed at some other OpenXR runtime; after any successful wine-vr install `registry_current` is true and the `reg add` branch is never taken. What is genuinely lost is only the benefit PARITY.md:117 claims for the divergence ('a launch immediately after a successful install does not trip bottle.registry') in exactly that corner, plus a missed lazy-flush warning. Latent, unusual pre-state, no wrong bytes written → low. The reviewer's third leg (the 1480-1495 test 'codifies the contradiction') is a misread of intent: that fixture writes a half line to force the `reg add` branch deterministically, and its comment says so.

*Fix sketch:* Poll on `registry_current(system_reg)` inside `wait_for_registry_flush` (install.rs:464) so the wait ends only when the launch gate would pass, and keep the shell's bare `system_reg_contains` as the predicate for whether to print the warn — i.e. wait for the strict form up to REGISTRY_FLUSH_TIMEOUT, then `warn` iff the loose form is still false (byte-identical warn behaviour to install.sh, strictly more waiting, which is what PARITY.md:117 already declares).

*Regression test:* In install.rs tests mod: a `real_seeming_fixture()` whose system.reg is pre-seeded with a non-matching `"ActiveRuntime"="C:\\other\\x.json"` line and a writer thread that appends the correct line after ~150ms; assert `run()` does not return before that write lands (no Warn row, and `registry_current` true at the end). Keep the existing never-flushes test asserting the single Warn.

*Cross-area files:* sabrage/PARITY.md

### A6-3 [medium, conf 0.99] [unfixed] A6-3 Non-empty partial backups remain trusted, and the recovery advice can capture the fork
`sabrage/crates/sabrage-core/src/stages/install.rs:146-193` — **CONFIRMED** (re-rated medium)

The fix detects only an entirely empty backup. A `cp -R` interrupted after its first entry leaves a non-empty truncated tree that is still reported as complete. Moreover, cancellation prevents the attempted cleanup because `RealExecutor::remove_dir_all` checks the cancelled token, and cleanup errors are discarded. The next run accepts that partial backup and overlays the live DXMT tree. Even the empty case continues applying the overlay after telling the user to remove the backup and rerun; following that advice after this run captures the fork as the alleged stock backup.
Evidence: `sabrage/crates/sabrage-core/src/stages/install.rs:146-160` treats every non-empty directory as `stock DXMT backup already exists`; `:169-178` ignores cleanup failure with `let _ = ...remove_dir_all`; and `:190-193` proceeds to overwrite the live overlay regardless of the incomplete-backup warning.

*Recommendation:* Copy into a uniquely owned sibling partial directory, verify it against the source or add a completion marker, then atomically rename it. Refuse to apply the overlay when an existing backup cannot be proven complete, and make cleanup both cancellation-safe and error-visible.

*Verifier:* Reachable and irreversible-ish: Stop pressed during layer 1's `cp -R` (the first and heaviest write into CrossOver.app), a SIGKILL, or a power loss leaves a truncated `dxmt.stock-backup`; the next install trusts it and overwrites the live stock dlls, so the stock copies of whichever contract files the partial tree is missing are gone — recoverable only by reinstalling/updating CrossOver, which is the documented rollback this backup exists to avoid needing. The empty-backup branch has the same shape one level up: it warns 'remove it and re-run install to recapture stock' and then applies the overlay anyway (install.rs:190-193), so a user who follows that remedy after the run captures the fork as the alleged stock backup. Not critical (no user data lost; CrossOver is reinstallable and the fork overlay still works), not high (needs an interrupted copy), so medium as filed. Note the non-cancellation failure path is fine — the cleanup runs when the token is live; it is specifically cancellation that defeats it.

*Fix sketch:* In `install::run` layer 1: copy into a uniquely named sibling `dxmt.stock-backup.partial-<uuid>` and `fs::rename` it onto `dxmt.stock-backup` only after `cp -R` succeeds (rename is atomic within the same directory, so 'exists' regains its meaning). Treat any leftover `dxmt.stock-backup.partial-*` as an incomplete capture: sweep it before copying. Make the failure cleanup cancellation-exempt (a `remove_dir_all` that does not go through `guard()`, or direct `tokio::fs::remove_dir_all` for this one path) and surface its error as a `warn` instead of `let _ =`. Change the empty/unprovable-backup branch from `warn`-and-continue to a fatal with the remedy, so the overlay is never applied over a backup that cannot be proven complete (matching `dxmt_files_ok`'s existing 'never half-applies the overlay' stance).

*Regression test:* In install.rs tests mod, beside `dxmt_backup_present_is_reported_not_replanned`: (1) seed `lib/dxmt.stock-backup.partial-<x>` plus no committed backup and assert `run()` refuses (fatal, no overlay copy planned); (2) a TestExecutor whose `dir_copy` fails with `Cancelled` — assert no `dxmt.stock-backup` exists afterwards (only/none of the partial), that a warn names the failed cleanup if any, and that the next `run()` re-copies rather than reporting 'backup already exists'; (3) an empty `dxmt.stock-backup` makes `run()` fail before any `install_if_changed` into `lib/dxmt`.

*Cross-area files:* sabrage/crates/sabrage-core/src/executor.rs, sabrage/PARITY.md

*Codex next steps:* Add cancellation coverage for the registry-wait window, including a stale host manifest. · Test the registry poll with a pre-existing wrong ActiveRuntime value and require the full launch-gate predicate. · Fault-inject `cp -R` failure or cancellation after one copied entry and verify no partial backup is accepted. · Verify backup construction under concurrent Sabrage/demo.sh activity using a unique partial path and atomic commit. · After fixes, run the targeted core tests and `scripts/dev/parity.sh`; tests were not run in this read-only review.

## A7 — run-preflight-actions

**Codex verdict:** needs-attention — Do not ship. A7-1 and A7-6 appear closed, but A7-2, A7-4, and A7-5 remain incomplete. The branch also retains an uncancellable wired preflight and a destructive Goldberg backup edge case.

### A7-1 [medium, conf 0.99] [unfixed] A7-2 Launch still does not implement last-valid runtime semantics
`sabrage/crates/sabrage-core/src/stages/run/preflight.rs:125-142` — **CONFIRMED** (re-rated medium)

The new reader selects the last raw assignment, whereas the runtime preserves the last accepted assignment. For example, `protocol = "alvr"` followed by `protocol = "banana"` is rejected here although the runtime continues using ALVR; `encoder_process = "inproc"` followed by junk can trigger an unnecessary helper mutation or refusal although the runtime remains in-process. The same mismatch produces a dishonest event: unquoted `protocol = alvr` is accepted by `facts`, but the doctor-derived outcome is emitted as a blocking Fail before launch continues.
Evidence: `sabrage/crates/sabrage-core/src/stages/run/preflight.rs:125-142` calls `runtime_toml::effective_string(...).unwrap_or_default()` for both facts; `sabrage/crates/sabrage-core/src/config/runtime_toml.rs:657-674` explicitly describes that helper as having “no accepted-set filtering” and ends with `.last()`; `preflight.rs:431,444-446` emits the unchanged evaluator outcome and then returns `Ok(())` for `facts.protocol == "alvr"`.
Inference: a hand-edited duplicate containing a trailing invalid value either blocks a configuration the runtime accepts or shows a red blocking check while proceeding, violating both the declared divergence and the no-dishonest-UI invariant.

*Recommendation:* Derive protocol and encoder facts from `read_lines_like_the_runtime`, applying runtime defaults when no valid assignment exists. Build the launch Check outcomes from those facts rather than emitting the doctor parser’s outcome. Add trailing-invalid and unquoted-value tests for both the returned verdict and emitted Check status.

*Verifier:* The parser mismatch is real and reachable: the launch reader is last-RAW, the runtime and the GUI Settings view (runtime_toml::read) are last-ACCEPTED, and PARITY.md documents the launch side as last-valid. Consequence on a hand-edited toml with a junk trailing duplicate: Sabrage refuses a launch the runtime would accept (matching run.sh's awk, so no shell-parity break), or requires/restages the arm64 helper for a config that would stay in-process, while the Settings pane shows the accepted value — an internal contradiction inside one app. Not high: it needs a duplicated key whose LAST occurrence is invalid, there is no data loss, and the refusal direction is the safe one (the shell dies on the same file). The 'dishonest Fail event' half is a declared divergence and does not stand.

*Fix sketch:* In stages/run/preflight.rs, derive TomlFacts from `runtime_toml::read_lines_like_the_runtime` (or add a `runtime_toml::effective_accepted(text, key)` that folds the whitelist the way Config.cpp does) instead of the raw-last `effective_string`, applying the code defaults ('' for protocol, 'auto' for encoder_process) when no valid assignment exists; keep `effective_string` for keys Sabrage does not model. Update the module doc at preflight.rs:107-124 so it stops claiming runtime semantics it does not implement. Alternative, cheaper: leave the code and correct PARITY.md:115 + the doc comment to say last-RAW — but that keeps the Settings-vs-preflight contradiction.

*Regression test:* sabrage/crates/sabrage-core/src/stages/run/preflight.rs tests: extend `the_preflight_facts_agree_with_the_settings_view_on_a_shadowed_key` (preflight.rs:900) with a trailing INVALID duplicate (`protocol="alvr"` then `protocol="banana"`; `encoder_process="inproc"` then `"garbage"`) and assert facts.protocol=="alvr" / facts.encoder_process=="inproc", plus a stage-level test that `run()` succeeds (no die, no unrecognized-encoder warn, helper slugs skipped) on that file.

*Cross-area files:* sabrage/PARITY.md, sabrage/crates/sabrage-core/src/config/runtime_toml.rs

### A7-2 [medium, conf 0.99] [unfixed] A7-4 Failed rollback erases the recovery record it still needs
`sabrage/crates/sabrage-core/src/stages/run/actions.rs:127-139` — **CONFIRMED** (re-rated low)

Rollback discards every `adb --remove` result and then clears all forward records. If a transient adb failure leaves either port installed, the following successful state save records that nothing remains, permanently removing the retry path that crash recovery exists to provide. This directly contradicts reconciliation, which deliberately retains a record after an indeterminate removal.
Evidence: `sabrage/crates/sabrage-core/src/stages/run/actions.rs:127-139` performs `let _ = rb.executor.run_child(&undo).await;` for each port and then unconditionally executes `sess.wired_forwards.clear()`; `sabrage/crates/sabrage-core/src/session/reconcile.rs:802-808` states that failed removals must keep the record because the forward may still be installed.
Inference: after a second-forward failure plus a rollback failure, a stale first forward can silently break the next WiFi session while both memory and disk claim cleanup succeeded.

*Recommendation:* Track each removal result, delete only successfully removed ports from `sess.wired_forwards`, and persist any failed or indeterminate entries. Add a rollback test where one `--remove` fails and assert that port remains in session-state.json.

*Verifier:* The asymmetry with reconcile is unambiguous and the on-disk record ends up asserting a cleanup that may not have happened. Downgraded from medium to low because it needs TWO faults (a failed forward AND a failed removal) and the record is not the only recovery path: any later non-wired run runs the list-based hygiene `fixes::adb::remove_adb_forwards_at` (actions.rs:167-171; fixes/adb.rs:1-37 walks `adb forward --list` and removes exactly ports.stream per serial), and doctor WARNs on leftovers — so the stale forward is not permanent, only unrecorded until the next normal run.

*Fix sketch:* In `rollback_forwards` (actions.rs:118-140) capture each `run_child` result; retain in `sess.wired_forwards` exactly the WiredForward entries whose `--remove` did not succeed (mirroring reconcile::restore_forwards' `still_installed`), clear the rest, then save. Guard against the fresh teardown executor's dry-run mode the same way reconcile does.

*Regression test:* sabrage/crates/sabrage-core/src/stages/run/actions.rs tests, next to the existing `slow_second_port_adb` cases (~actions.rs:1173): a fake adb whose `forward tcp:9944` fails and whose `forward --remove tcp:9943` also exits non-zero; assert the returned error is run.sh's die text AND that session-state.json still lists {serial, 9943} while 9944 is dropped (or kept, per the chosen rule) and `guards.forwards_cleared` is false.

### A7-3 [medium, conf 0.98] [unfixed] A7-5 Unpinned Goldberg payloads can still be reported as restored
`sabrage/crates/sabrage-core/src/stages/run/actions.rs:417-432` — **CONFIRMED** (re-rated low)

The launch action detects Goldberg by comparing against the actual configured DLL, but the revert implementation recognizes a poisoned backup only by the official contract hash. Because launch explicitly tolerates an existing Goldberg DLL with a different hash, an already-installed custom build is copied into `.orig-steam`, warned about transiently, and later passes the pin-only revert guard. Revert then returns `restored: true` after copying Goldberg back over Goldberg.
Evidence: `sabrage/crates/sabrage-core/src/stages/run/actions.rs:417-432` computes `already_goldberg = util::cmp_files(&ctx.paths.gbe_dll, &api)`, creates the backup anyway, and emits only a warning; `sabrage/crates/sabrage-core/src/store/goldberg.rs:137-150` refuses only when `file_sha256_matches(&backup, gbe_dll_sha256)` and otherwise copies the backup; `preflight.rs:335-348` allows any existing Goldberg DLL despite a hash mismatch.
Inference: the warning carries no durable provenance, so the later revert API cannot distinguish an accepted unpinned Goldberg backup from Steam bytes.

*Recommendation:* Persist poisoned-backup provenance when `already_goldberg` is detected, or pass the repository Paths into revert and refuse when the backup byte-matches the configured Goldberg payload. Cover an unpinned Goldberg stage-to-revert sequence end to end.

*Verifier:* The gap is real: an unpinned (user-built) Goldberg dll that already sits in the game folder is copied into `.orig-steam`, and revert later reports 'restored steam_api64.dll from the .orig-steam backup' for a Goldberg-over-Goldberg copy. Downgraded to low: it needs BOTH an unpinned gbe_dll and a game install that was already Goldberg'd with those exact bytes before Sabrage/demo.sh ever ran (any earlier run.sh/Sabrage launch would have minted the backup from the real Steam dll), and no additional data is lost — the real dll was already gone before Sabrage touched anything. The damage is a false 'restored: true' report, i.e. a dishonest-UI violation, not destruction.

*Fix sketch:* Persist the `already_goldberg` fact when the backup is minted (a `goldberg_backup_is_goldberg` flag in session/store state next to the backup, or a sibling marker file), and have `store::goldberg::revert_with_pin` refuse (`restored: false`, current pinned-backup wording) on that marker. Equivalent alternative: pass the repo `Paths` into revert and additionally refuse when `util::cmp_files(&paths.gbe_dll, &backup)` — refusing on the configured payload, not only on the contract pin.

*Regression test:* sabrage/crates/sabrage-core/src/store/goldberg.rs tests (they already use the `revert_with_pin` pin seam): stage a bs_dir whose steam_api64.dll equals a NON-pinned fake gbe_dll, run `actions::goldberg_stage`, then `revert_with_pin` with a different pin, and assert `restored == false` plus the 'itself the Goldberg dll' wording.

*Cross-area files:* sabrage/crates/sabrage-core/src/store/goldberg.rs, sabrage/crates/sabrage-core/src/session/state.rs

### A7-4 [high, conf 0.99] Wired preflight can hang forever before reaching the cancellable probe
`sabrage/crates/sabrage-core/src/checks/run_only.rs:129-180` — **CONFIRMED** (re-rated medium)

The action-layer probe gained timeout and cancellation, but `run.wired-adb` first invokes `adb devices` synchronously with no deadline. A wedged adb process blocks inside the registry evaluator; the cancellation checkpoint surrounding evaluation cannot interrupt it, so Run retains the operation lock and Cancel cannot complete.
Evidence: `sabrage/crates/sabrage-core/src/checks/run_only.rs:132-136` calls `Command::new(adb).arg("devices").output()` with no timeout or cancellation token, and `run_only.rs:180` invokes it on every wired evaluation. By contrast, the later action uses `capture_with` and `DEFAULT_PROBE_TIMEOUT` at `sabrage/crates/sabrage-core/src/stages/run/actions.rs:87-94`.
Inference: the existing slow-adb cancellation test exercises only the later action and therefore cannot detect the earlier permanent hang.

*Recommendation:* Remove the duplicate synchronous device probe or move it into an async, timeout- and cancellation-aware preflight path using `capture_with`. Add a fake `adb devices` that never exits and verify Cancel releases the stage and operation lock within the probe budget.

*Verifier:* Unambiguous and reachable on the `--wired` path with `allow_adb_probes` on: a wedged adb server or an unresponsive USB device makes `adb devices` block indefinitely and Run cannot be cancelled or unlocked. Downgraded from high to medium: it requires an externally wedged adb (not a normal slow start, which merely delays), affects only `--wired`, and the same blocking-probe pattern exists across every check evaluator (checks/headset.rs:35,86; checks/network.rs:39,83; checks/audio.rs:35,89), i.e. it is a check-layer-wide latent property rather than a regression unique to this file.

*Fix sketch:* Give the run preflight its own per-slug override, exactly like the existing `dep.goldberg` / `cfg.protocol.*` overrides in preflight.rs:329-357: for `run.wired-adb`, build the CheckOutcome from the async, cancellable `actions::probe_device_serial` (reusing DEFAULT_PROBE_TIMEOUT) instead of calling the registry evaluator, and pass the resulting serial down so the action does not re-probe. Keep run_only.rs's sync evaluator for doctor, but bound it with a kill-on-deadline `spawn` + `try_wait` loop so doctor cannot wedge either.

*Regression test:* sabrage/crates/sabrage-core/src/stages/run/preflight.rs tests: with `opts.wired = true` and `paths.adb` pointed at a `#!/bin/sh\nsleep 30` fake (same shape as actions.rs:1313 `slow_devices_adb`), assert `preflight::run` returns `SabrageError::Cancelled` within a couple of seconds after `ctx.cancel.cancel()`, and a second test asserting it returns the '--wired: no Quest over adb' fatal within the probe budget when the token is never cancelled.

### A7-5 [high, conf 0.98] A non-file `.orig-steam` path causes the real DLL to be overwritten without a backup
`sabrage/crates/sabrage-core/src/stages/run/actions.rs:406-440` — **CONFIRMED** (re-rated medium)

Backup existence is tested with `exists()` rather than requiring a regular file. If `steam_api64.dll.orig-steam` is a directory or other unusable path, backup creation is skipped and the live Steam DLL is subsequently replaced by Goldberg. The only local copy of the original is then lost.
Evidence: `sabrage/crates/sabrage-core/src/stages/run/actions.rs:409-410` uses `if !backup.exists()` to decide whether to copy the live DLL, while `actions.rs:437-440` proceeds to overwrite the live DLL whenever its bytes differ from Goldberg.
Inference: an unexpected filesystem object at the reserved backup name turns an otherwise recoverable launch failure into destructive state change.

*Recommendation:* Treat an existing non-file backup path as fatal before modifying the live DLL. Use `is_file()`/`symlink_metadata` to distinguish a usable backup from a conflicting directory or special file, and add a fixture asserting the live DLL remains byte-identical when that conflict exists.

*Verifier:* The path is unambiguous and reachable, and I reproduced the destructive outcome end-to-end through the public API with the real executor. Severity re-rated from high to medium: the consequence is genuine local data loss and a divergence from shell (which preserves the dll inside the directory), but the trigger — a directory, fifo, socket or device node at exactly `steam_api64.dll.orig-steam` — is unusual filesystem state, not a realistic everyday path, and the dll is recoverable by re-verifying the game install. Not critical: nothing irreversible at the machine level and no trust-boundary break.

*Fix sketch:* In `goldberg_stage` (sabrage/crates/sabrage-core/src/stages/run/actions.rs:409): replace `if !backup.exists()` with a three-way decision on `std::fs::symlink_metadata(&backup)`: (a) Err(NotFound) -> mint the backup as today; (b) Ok(md) where `md.file_type().is_file()` or a symlink resolving to a regular file -> skip as today; (c) anything else (dir/fifo/socket/device, or a symlink to one) -> `return Err(st.fatal(...))` BEFORE the `copy_if_changed(&ctx.paths.gbe_dll, &api)` at line 437, with a remedy naming the path ("move or delete <backup>; it is the reserved name for the original Steam dll"). Cheapest equivalent: change `exists()` to `is_file()` — the subsequent `copy_if_changed(&api, &backup)` then fails (copy_atomic cannot rename over a directory) and the existing `die_with_cause(..., "backup of original steam_api64.dll failed")` at 415-422 fires before the live dll is touched — but the explicit fatal gives a better remedy and also covers the symlink-to-nonfile case. Either way the existing `already_goldberg` warn path is unaffected.

*Regression test:* New `#[tokio::test]` beside the existing goldberg tests in sabrage/crates/sabrage-core/src/stages/run/actions.rs (mod tests, near `goldberg_writes_the_appid_digits_with_no_trailing_newline` at ~line 1512, using the RealExecutor swap that test already does). Fixture: live dll b"REAL-STEAM", `steam_api64.dll.orig-steam` created as a directory, gbe dll b"GOLDBERG". Asserts: `goldberg_stage(&ctx).await` is Err; the error text names the backup path; and `fs::read(api_dir.join("steam_api64.dll")) == b"REAL-STEAM"` (live dll byte-identical, i.e. no destructive write). A sibling case with a plain-file backup must still pass unchanged, pinning that the strictening does not alter the normal path.

*Cross-area files:* sabrage/PARITY.md

*Codex next steps:* Add last-valid protocol and encoder fixtures, including invalid trailing assignments and unquoted values, and assert both launch decisions and emitted Check statuses. · Fault-inject rollback removal failures and verify session-state.json retains exactly the ports whose cleanup was indeterminate. · Exercise an unpinned Goldberg DLL through launch staging and the revert API; assert revert refuses and reports `restored: false`. · Run the wired preflight with a permanently sleeping fake adb, cancel it, and verify bounded completion plus operation-lock release. · Test `.orig-steam` as a directory and special file, asserting launch fails before changing `steam_api64.dll`.

## A8 — run-supervise-guards-logs

**Codex verdict:** needs-attention — No ship. A8-5 remains unsafe, A8-3 and A8-7 were only partially closed, and rotation can mislabel old log data as new. The A8-1, A8-2, and A8-6 fixes appear structurally sound in the current code.

### A8-1 [high, conf 0.99] [unfixed] A8-5 External-session detection is display-only
`sabrage/crates/sabrage-core/src/stages/run/mod.rs:143-185` — **CONFIRMED** (re-rated high)

The launch path still understands only session-state records. A demo.sh session produces `Reconciled::NoSession`, while a live foreign Sabrage or protected newer record can produce `Busy`; run rejects only `Live` and proceeds through permanent preparation, including wineserver reset. The monitor may display `External`, but that phase is also omitted from the UI's `busy` predicate. Inference: Launch or Cmd-R during a detected external session can reset the wineserver underneath the running game and potentially overwrite recoverable state. Evidence: `sabrage/crates/sabrage-core/src/stages/run/mod.rs:143-159` contains `if let Reconciled::Live ...` followed by `_ => None`, then `sabrage/crates/sabrage-core/src/stages/run/mod.rs:183-185` executes `adb_forward_hygiene` and `wineserver_reset`; `sabrage/ui/src/screens/Session.svelte:107-112` does not treat `external` as busy.

*Recommendation:* Before preflight or any mutation, reject `Reconciled::Busy` and independently reject a fresh runtime-status record whose named PID is live. Treat `external` as busy in every launch surface, but retain the backend check as authoritative. Add an end-to-end test with a live PID and fresh runtime status but no session-state file, asserting that run plans no mutations.

*Verifier:* Realistic path: `./demo.sh run` streaming in a terminal (no session-state.json), Sabrage open on the Session screen. The monitor renders `External`, the Launch button is enabled, run's only gate returns NoSession, and the stage runs wineserver_reset on the bottle — killing the running game — after already doing adb-forward hygiene. The `Busy` half (a live foreign Sabrage owner, or a newer-schema record) reaches the same `wineserver_reset` from the CLI, where no UI predicate covers for it. Not critical: the loss is a killed game session, not irreversible machine state, and demo.sh's run.sh:129-141 resets wineserver unconditionally too — but Sabrage's stages/mod.rs:37 and PARITY.md:112 both claim run is covered by its own reconciliation, and it is strictly weaker than the shared predicate.

*Fix sketch:* In `stages::run::run` (mod.rs:120-146), before `phase.publish(SessionPhase::Preflight)` — so the run's own publication cannot self-block via session_block_at's run_phase arm — call `session::live_session_block(&ctx.paths)` and, when Some, return the `already_running`-shaped fatal (mod.rs:899) carrying the block's reason and `./demo.sh stop --bottle <name>`. Then widen the reconcile arm at mod.rs:144 to `if let Reconciled::Live { state } = &reconciled { … } if let Reconciled::Busy { reason, .. } = &reconciled { return Err(refusal(reason)) }`, so the foreign-owner and newer-schema records refuse instead of falling into `_ => None`. In sabrage/ui/src/screens/Session.svelte replace the hand-rolled `busy` phase list with `isLivePhase(status.phase)` (what Library.svelte already uses) so `external`/`stalled`/`stopping`/`detached` disable Launch, Dry-run and the option controls.

*Regression test:* sabrage/crates/sabrage-core/src/stages/run/mod.rs `mod tests`: build a StageCtx over a scratch Paths (oxr_appsup/sabrage_appsup redirected) holding a fresh runtime_status.json naming a live pid and NO session-state.json, run the stage with a recording DryRunExecutor, and assert it returns Err naming the running session and that `ctx.executor.planned()` is empty (no adb forward, no wineserver kill, no goldberg swap). A second case writes a session-state.json whose owner_pid is a live foreign process (reconcile.rs's `ForeignProcess::spawn()` helper) and asserts the same. UI: assert Session.svelte's disabled state for phase `external` in the existing screen tests, or pin `isLivePhase` as the single predicate.

*Cross-area files:* sabrage/ui/src/screens/Session.svelte, sabrage/crates/sabrage-core/src/session/mod.rs

### A8-2 [medium, conf 0.98] [unfixed] A8-3 Cancellation restoration bypasses the guard state machine
`sabrage/crates/sabrage-core/src/stages/run/guards.rs:323-343` — **CONFIRMED** (re-rated medium)

Arming the local guard protects the device, but a cancellation returned by the switch still unwinds `acquire` before the guard is installed in `held`. `Drop` then performs an unbounded synchronous restore and can emit `audio: restored`, but it cannot set `audio_restored` or persist that success. Teardown consequently keeps the record and reports a pending guard, producing contradictory state; a wedged SwitchAudioSource can also block cancellation indefinitely in `.status()`. Evidence: `sabrage/crates/sabrage-core/src/stages/run/guards.rs:335-343` arms the guard and then applies `?` to `run_child`; `sabrage/crates/sabrage-core/src/stages/run/mod.rs:411` assigns to `held.audio` only after `acquire` succeeds; `sabrage/crates/sabrage-core/src/stages/run/guards.rs:488-500` restores through raw `.status()` and only emits an event.

*Recommendation:* Split acquisition into prepare-and-switch phases so the armed guard is installed in `held` before awaiting the mutation. Route cancellation through the normal bounded async release that updates SessionState; reserve Drop for crash-only best effort. Test an executor that applies the switch and then returns Cancelled, asserting one truthful restore row and no pending record.

*Verifier:* Reachable whenever Stop/Cmd-Q fires during the SwitchAudioSource `-s BlackHole 2ch` child — a real but narrow window. The user sees `audio: restored output -> <dev>` and, moments later, `previous session record kept for a later restore`; session-state.json survives with a pending audio guard that is not pending. Downgraded from a data hazard because reconcile only restores when the current device really is BlackHole (reconcile.rs:729), so the next pass is a no-op rather than a wrong switch, and the next launch's carry-forward (mod.rs:153-159) still names the right device. The unbounded blocking `.status()` inside Drop, on a tokio worker during unwind, is the other half: a wedged SwitchAudioSource stalls the cancellation the user just asked for.

*Fix sketch:* Split `AudioGuard::acquire` (guards.rs:256-372) at line 335 into `arm(ctx, facts, state) -> Result<AudioGuard>` (eligibility, both probes, carry-forward decision, the pre-mutation `state::save`, and setting `previous_output`) and `apply_switch(&mut self, ctx, state) -> Result<()>` (the `run_child` switch, the volume `osascript`, and the failure branch at :360-370). In `guarded` (mod.rs:411) install the armed guard first — `held.audio = Some(AudioGuard::arm(...).await?); held.audio.as_mut().unwrap().apply_switch(ctx, sess).await?;` — so a Cancelled from the switch unwinds into `teardown`, whose `held.release` is the bounded, executor-routed path that sets `guards.audio_restored` and saves. Leave `Drop` as the crash-only fallback it documents itself to be.

*Regression test:* sabrage/crates/sabrage-core/src/stages/run/guards.rs `mod tests`: an Executor whose `run_child` records the switch and then returns `Err(SabrageError::Cancelled)`. Assert the run ends in `SabrageError::Cancelled`, exactly one `audio: restored output -> <dev>` row is emitted (not two, and not one from Drop), `sess.guards.audio_restored == true` on the saved state, and `finish_record` cleared session-state.json with no `RECORD_KEPT_LINE` row.

### A8-3 [medium, conf 0.94] [unfixed] A8-7 The continuity signature still has a truncate/read race
`sabrage/crates/sabrage-core/src/logs.rs:383-460` — **CONFIRMED** (re-rated low)

The signature is validated before the later seek and read. If ALVR truncates and regrows the same inode after that validation but before the read, the tailer reads the new file starting at the stale offset, replaces its signature with bytes from the new suffix, and will consider the next poll continuous. Inference from this ordering: the entire new prefix before the old cursor is silently lost. The added test rewrites wholly between polls and cannot exercise this window. Evidence: `sabrage/crates/sabrage-core/src/logs.rs:395-398` performs the only continuity comparison, while `sabrage/crates/sabrage-core/src/logs.rs:445-460` subsequently seeks to the old offset, reads, and updates the signature without revalidation.

*Recommendation:* Validate continuity around the read and discard/reopen the batch if the witness or file metadata changes, or use another stable-snapshot protocol. Add a barrier-controlled test that truncates and regrows specifically between the precheck and the data read.

*Verifier:* The ordering is unambiguous and the loss is silent, so the finding is real — but the window is a handful of consecutive syscalls inside a synchronous `poll()` with no await points, and it needs an external writer to truncate AND rewrite past the old cursor inside it. Down-rated from medium to low: the blast radius is missing lines in the Logs pane, never on-disk data, and the recovery path (a shrunken file at the next poll, :385 `len < self.offset`) covers every version of this race except the exact regrow-past-cursor one.

*Fix sketch:* In `Tailer::poll` (logs.rs:358-488), capture the pre-read witness and file identity, and after the read loop re-`fstat` the open handle (and optionally re-read the signature at the pre-read offset). If `file_identity`/`len` changed such that the read cannot have come from the same incarnation, discard `buf`/the splitter output, leave `self.offset`/`self.signature` untouched, reset `open`/`splitter`/`unterminated` and return `LogBatch { lines: <drained pending only>, rotated: true, .. }` so the next poll reopens from 0. Factor the precheck at :387-398 into `fn continuous(&mut self) -> Result<bool>` so both call sites share it.

*Regression test:* sabrage/crates/sabrage-core/src/logs.rs `mod tests`: extract the read step behind a test-only hook (`#[cfg(test)] fn between_precheck_and_read: Option<Box<dyn Fn()>>`) and, in the hook, truncate the file and rewrite it longer than the old offset with entirely different content. Assert the resulting batch is `rotated: true` and contains none of the new file's suffix, and that the following poll delivers the new file from byte 0.

### A8-4 [medium, conf 0.99] Rotation reports previous-file backlog as the beginning of the new file
`sabrage/crates/sabrage-core/src/logs.rs:405-429` — **CONFIRMED** (re-rated medium)

When rotation occurs with queued lines, poll explicitly returns lines from the previous incarnation with `rotated: true`, resets the cursor to the new file, and delays reading that new file until a later batch whose `rotated` flag is false. The UI therefore clears its buffer, labels the old lines as the new file's beginning, and later appends the actual new file without another clear. This can materially misattribute startup failures to the wrong session. Evidence: `sabrage/crates/sabrage-core/src/logs.rs:405-426` says the backlog is from the `previous incarnation` and returns it with `rotated: true`; `sabrage/ui/src/screens/Logs.svelte:92-102` clears on that flag and immediately appends those lines under the notice `log rotated — showing the new file from the start`.

*Recommendation:* Keep a pending-rotation epoch marker: drain old backlog as old-file data, then emit `rotated: true` only with the first batch from the reopened file. Update the existing backlog/rotation test to assert consumer-visible epoch ordering.

*Verifier:* Reachable without any race: open the Logs pane on a large `oxrsys-runtime.log` (the tailer drains it across polls at MAX_LINES_PER_POLL = 2000, logs.rs:176, and the 64 KiB chunk at :450-460 routinely overshoots that cap, so `pending` is non-empty for many consecutive polls), then start a session, which truncates/recreates that path. The pane clears, prints "showing the new file from the start", and displays the *old* file's tail under that banner, then silently appends the real new file after it with no second clear. Lines from two different sessions appear as one, in a pane whose whole purpose is diagnosing which session failed. Medium, not high: no data is lost or written, and the mislabel corrects itself once the old backlog scrolls off.

*Fix sketch:* Add `pending_rotation: bool` to `Tailer` (logs.rs:213-239). In the rotation branch (:401-431), when `backlog` is true set `self.pending_rotation = true` and return the drained backlog with `rotated: false` (it is old-file data and the consumer must keep it appended to the old buffer). At :479-487 compute `let rotated = rotated || std::mem::take(&mut self.pending_rotation);` so the first batch actually read out of the reopened file carries the flag — and make the same `take` happen on the `NotFound` (:358-373) and empty-read paths so the marker cannot be swallowed. `LogBatch::rotated`'s doc comment should then state it means "the lines in THIS batch begin a new incarnation".

*Regression test:* Update sabrage/crates/sabrage-core/src/logs.rs:1163 `a_backlog_queued_before_a_vanish_survives_reappearance_uncut` to assert consumer-visible epoch ordering: `b3.rotated == false` while still delivering `l4000..l4499` uncut (the F14 no-drop promise), and `b4.rotated == true` with `["new1"]`. Add a companion test that replays the batch sequence through the Logs.svelte `onBatch` contract (clear-on-rotated, then append) and asserts no `l*` line survives after the rotation marker.

*Codex next steps:* Exercise run against a live external PID with no session-state file and verify that no Executor mutation is planned. · Inject cancellation after the audio child applies its mutation and verify bounded teardown, persisted `audio_restored`, and non-contradictory events. · Add a synchronized truncate/regrow race test between signature validation and file reading. · Drive a pending-backlog rotation through the Logs consumer and assert that no old line appears after the new-file rotation marker.

## A9 — session-reconcile-telemetry

**Codex verdict:** needs-attention — NO-SHIP: launch treats protected recovery records as advisory and can reset another live session. A9-2, A9-3, A9-6, A9-7, A9-8, and A9-9 remain incomplete, with additional retry and UI-state regressions.

### A9-1 [high, conf 0.99] Busy recovery records do not block launch
`sabrage/crates/sabrage-core/src/stages/run/mod.rs:143-185` — **CONFIRMED** (re-rated high)

`reconcile` correctly returns `Busy` for a live foreign owner, but `run` rejects only `Live`. A same-bottle launch therefore continues through auto-fixing preflight, adb cleanup, and `wineserver_reset`, which can kill the foreign session the new classification was meant to protect. The cross-process operation lock does not help because the original run releases it after spawning.
Evidence: `reconcile.rs:420-430` returns `Reconciled::Busy` for foreign-owned or newer records; `stages/run/mod.rs:143-159` rejects only `Reconciled::Live` and maps `Busy` to `None`; `stages/run/mod.rs:183-185` then reaches adb hygiene and the bottle-scoped wineserver reset.

*Recommendation:* Make every non-silent `Reconciled::Busy` a fatal launch refusal before preflight. Add a same-bottle integration test with a live foreign owner and assert no preflight fix, adb mutation, or wineserver command is executed.

*Verifier:* Reachable with two Sabrage front-ends on one Mac (GUI + `sabrage` CLI, the case owner_pid was added for). Same-bottle relaunch then runs the preflight auto-fixes (cxbottle.conf rewrite, helper restage) against a running game, `remove_adb_forwards_at` pulls the live --wired session's tcp:9943/9944 (fixes/adb.rs:163-223, list-driven so it does not need the record), and `wineserver_reset` kills the foreign session's game. That is exactly the mutation PARITY.md's 'a launch refused for a live session changed nothing' promise covers (run/mod.rs:53-59, :131-140). Not critical: no persistent data is destroyed, the machine state is recoverable by rerunning stop/install.

*Fix sketch:* Give `Reconciled::Busy` its `silent` flag (or a `BusyKind { InFlightSelf, ForeignOwner, NewerSchema }`) from `untouchable`, and in `stages::run::run` refuse every non-silent Busy before `checkpoint`/`preflight::run` with the same shape as `already_running` (name the owner pid and the `./demo.sh stop --bottle` remedy). `run`'s own reconcile can never see the silent in-flight variant (it has written no record for ctx.run_id yet), so refusing all Busy is also acceptable.

*Regression test:* sabrage/crates/sabrage-core/src/stages/run/mod.rs tests (next to `the_already_running_refusal_names_the_pid_the_time_and_both_stop_routes`): write a record for the ctx's bottle whose wine identity is live and whose owner_pid is a spawned /bin/sleep, run the stage with a DryRunExecutor, assert Err(Fatal) naming the owner pid AND `ctx.executor.planned().is_empty()` (no cxbottle fix, no adb removal, no `wineserver -k`).

*Cross-area files:* sabrage/crates/sabrage-core/src/stages/run/mod.rs

### A9-2 [medium, conf 0.96] [unfixed] A9-2 Retained forward recovery is still overwritten
`sabrage/crates/sabrage-core/src/stages/run/mod.rs:147-183` — **CONFIRMED** (re-rated low)

The carry-forward fix preserves only an unfinished audio device. A record retained because an adb removal failed has `pending=true` and outstanding `wired_forwards`, but launch ignores both, creates a fresh state containing only the carried audio field, and eventually overwrites the recovery record. If launch-time adb hygiene also cannot remove the forward, the exact serial/port retry metadata is lost.
Evidence: `stages/run/mod.rs:147-177` extracts only `unfinished_audio_restore(state)` and initializes a fresh `SessionState` with `sess.prev_audio_output = carried`; the outstanding `wired_forwards` and `pending` outcome are not transferred or used to refuse launch.

*Recommendation:* Either block launch whenever `pending` remains, or transfer every outstanding guard—including `wired_forwards`—into the new journal and prove that a repeated adb failure cannot erase it.

*Verifier:* The mechanic is real, but the consequence the finding claims ('the exact serial/port retry metadata is lost') is largely redundant: the launch-time hygiene the finding worries about is `fixes::adb::remove_adb_forwards_at`, which enumerates `adb forward --list` (fixes/adb.rs:175-223) and removes every stale tcp:9943/9944 on ANY serial — it never consults the record. The same list-driven fix backs `doctor`. A really-installed forward is therefore still recoverable after the record is overwritten; the record only loses its bookkeeping. Loss with consequence needs adb absent (or `--list` failing) at both the reconcile and the next launch, or a --wired relaunch onto a different serial while the old device stays connected — unusual, and the residue is a doctor WARN with a documented one-line remedy.

*Fix sketch:* In `stages::run::run`, carry the whole outstanding guard set forward, not just audio: extend the `Dead`/`IdentityMismatch` arm to also move `state.wired_forwards` (and `guards.forwards_cleared=false`) into the fresh `SessionState` before `adb_forward_hygiene` pushes this launch's own entries (dedupe on (serial, port)).

*Regression test:* sabrage/crates/sabrage-core/src/stages/run/mod.rs tests, beside the audio carry-forward test: a stale record with `wired_forwards=[(serial-A,9943)]`, `forwards_cleared=false`, adb missing so reconcile cannot remove it; run the stage dry and assert the record written by the launch still contains (serial-A, 9943).

*Cross-area files:* sabrage/crates/sabrage-core/src/stages/run/mod.rs

### A9-3 [medium, conf 0.98] [unfixed] A9-3 Record locking still fails open without CAS
`sabrage/crates/sabrage-core/src/session/state.rs:322-416` — **CONFIRMED** (re-rated medium)

The new record lock is explicitly abandoned after two seconds or any locking error, after which `save` and `clear` continue with an ordinary load followed by write/remove. That is not compare-and-swap: another process can replace the record between those operations, and `clear` carries no expected run ID at all. The original last-writer-wins/delete-newer-record failure therefore remains on the degraded or timed-out path.
Evidence: `state.rs:322-345` says `None — no lock, carry on` and returns `None` on timeout/error; `state.rs:379-396` then performs `load(path)` followed by `write_atomic`, while `state.rs:405-416` performs `load(path)` followed by unconditional `remove_file(path)`.

*Recommendation:* Fail closed when the record lock cannot be acquired, or implement a real expected-run/version CAS. Require `clear` to name the run it intends to remove and reject any different on-disk run regardless of owner status.

*Verifier:* The lock-fails-open half needs a degraded filesystem or a >2s holder and is genuinely rare (the window is two file ops on APFS), so on its own it would be low. What makes this medium is the missing expected-run on `clear`, which is reachable with the lock HELD and inside one process: run A drops the operation lock at its spawn (run/mod.rs:474), so the user can start run B while A's teardown is still restoring audio; B's save legitimately overwrites A's record (same owner_pid), then A's teardown `finish_record` -> `clear_state` deletes B's record while B's audio/dashboard guards are armed. If B then crashes, nothing on disk names the device to switch back to — the exact failure session-state.json exists to prevent.

*Fix sketch:* Add `state::clear_run(executor, path, expected: RunId)` (keep `clear` for the deliberate 'delete whatever is there' callers, or drop it) that reloads under the lock and returns Ok(()) without removing when `existing.run_id != expected`; call it from run/mod.rs:821 `clear_state` with `sess.run_id` and from reconcile.rs:915 with the reconciled record's run_id. Separately, make `lock_record` failure non-silent: either fail closed for `clear` (the destructive op) or emit a warn row so a degraded lock is visible.

*Regression test:* sabrage/crates/sabrage-core/src/session/state.rs `mod tests`: write run B's record, call `clear_run(.., run_a)` and assert the file survives and still parses as run B; then `clear_run(.., run_b)` and assert it is gone. Plus a run-stage test that a teardown for an already-superseded run leaves the newer record intact.

*Cross-area files:* sabrage/crates/sabrage-core/src/stages/run/mod.rs

### A9-4 [medium, conf 0.95] Forward-removal progress is not crash-resumable
`sabrage/crates/sabrage-core/src/session/reconcile.rs:824-847` — **CONFIRMED** (re-rated low)

The A9-4 fix removes successful forwards from the in-memory vector only after the entire loop. A crash after removing 9943 but before the final save leaves both ports recorded. On retry, the already-absent 9943 removal can return non-zero and is then retained indefinitely as “still installed,” preventing completion even though that forward is gone.
Evidence: `reconcile.rs:824-843` executes every removal while leaving `state.wired_forwards` unchanged; only after the loop do `reconcile.rs:845-847` replace the vector and persist it once.

*Recommendation:* Persist the remaining-forward list immediately after each successful removal, or distinguish an authoritative “listener absent” result from an indeterminate adb failure. Add a crash-after-first-removal resume test.

*Verifier:* Real but narrow: the crash window is the few ms between two adb child processes, and the consequence is bookkeeping only — a kept record, a repeated `RECORD_KEPT` info row, and a harmless retry each reconcile. No wrong mutation follows (the removal is idempotent and per-serial), and 'retained indefinitely' overstates it: the next launch overwrites the record wholesale (A9-2's mechanic), and a non-wired launch's list-driven `remove_adb_forwards_at` clears any forward that really is installed. Latent + unusual conditions, no user-visible wrong behaviour.

*Fix sketch:* In `restore_forwards`, drop the entry from `state.wired_forwards` and `state::save` immediately after each successful removal (accepting N saves for N<=2 forwards), so progress is crash-durable; optionally distinguish adb's 'listener not found' stderr from an indeterminate failure and treat the former as removed.

*Regression test:* sabrage/crates/sabrage-core/src/session/reconcile.rs `mod tests`: with a scripted executor whose second removal panics/errors after the first succeeded, assert the on-disk record already lists only the second forward; a follow-up reconcile with both removals failing must not re-add the first.

### A9-5 [medium, conf 0.98] Unverifiable live sessions are rendered as Exited
`sabrage/crates/sabrage-core/src/session/watcher.rs:319-348` — **CONFIRMED** (re-rated medium)

Reconciliation now conservatively treats an alive PID with `start_time == 0` as `Unverifiable`, but the monitor still uses only `is_same_process()`. The same fallback identity therefore appears as `Exited` even while the process is alive, exposing Launch/recovery UI for a session that the launch path later refuses as live.
Evidence: `watcher.rs:319-348` maps both a live handle and persisted record to `Exited` whenever `is_same_process()` is false; `reconcile.rs:249-257` explicitly classifies the alive zero-start-time case as `Unverifiable` rather than dead.

*Recommendation:* Derive monitor liveness through `reconcile::classify` and map both `Live` and `Unverifiable` to a non-exited phase. Add status tests for zero-start-time live handles and persisted records.

*Verifier:* Same identity, two answers. The Session screen renders `Exited` (Launch offered, Stop hidden) for a session every launch door refuses as live, so the user sees 'Exited' and Launch fails with `already_running`, with no way to Stop from the UI. Reachability is gated on `observe_with_retry` failing for ~200 ms after the spawn (executor.rs:700-714), which is unusual but is exactly the case the Unverifiable class was added for; hence medium, not high.

*Fix sketch:* In `SessionMonitor::snapshot` derive liveness from `reconcile::classify` instead of raw `is_same_process()`: for the persisted branch build the phase from `classify(&state)` (`Live | Unverifiable` -> Detached/Running per `state.detached`, `Dead | IdentityMismatch` -> Exited); for the live-handle branch at watcher.rs:325-331 use the same predicate on `handle.identity` (`is_same_process() || (identity.start_time == 0 && process::is_alive(identity.pid))`) so an unverifiable-but-alive child reads Running.

*Regression test:* watcher.rs's inline `#[cfg(test)] mod tests`: a test writing a `session-state.json` whose `wine` is `{pid: std::process::id(), startTime: 0}` and asserting `snapshot().phase == SessionPhase::Running` (not Exited), plus a companion asserting the same for a `LIVE_SESSION` handle with `start_time: 0` — both cross-checked against `reconcile::classify(..) == Unverifiable`.

### A9-6 [medium, conf 0.99] [unfixed] A9-6 Adopted sessions still inherit historical encoder lines
`sabrage/crates/sabrage-core/src/session/watcher.rs:394-534` — **CONFIRMED** (re-rated medium)

For a session that predates monitor construction, every encoder line in the 200-line preload is considered believable solely because the session started before the monitor. No log timestamp or per-session cursor proves the selected line belongs to that session. Reopening Sabrage before the current run negotiates can therefore still publish the previous run's HEVC chip—the original A9-6 trigger.
Evidence: `watcher.rs:394-403` selects the last encoder line from the preload, and `watcher.rs:521-534` attributes it to the current run whenever `started_at_unix_ms <= created_at_unix_ms`, without comparing the line time to the session start.

*Recommendation:* Establish a per-session log cursor or parse and compare the log timestamp against `started_at_unix_ms`. Do not publish preloaded encoder data unless its ownership is provable.

*Verifier:* `created_at_unix_ms`'s own doc claims the guard proves ownership; it only proves the session predates the monitor, which is a weaker statement. The realistic trigger is the one the reviewer names: Sabrage reopened onto a launch that has started but not yet negotiated, with the previous run's `encoder ready` still inside the 200-line preload window. The stale chip is corrected once the current run logs its own line (watcher.rs:522, `history` false thereafter), so the worst case is a wrong codec chip for the negotiation window — or for the whole session when the current run never negotiates, which is exactly when the `(H.264, in-process)` downgrade signal matters. Medium, not high, because it needs the previous line to survive inside 200 lines.

*Fix sketch:* Give `parse_encoder_ready` (or a sibling `parse_log_timestamp`) the spdlog `[%Y-%m-%d %H:%M:%S.%e]` prefix, and in the preload branch admit the parsed line only when its wall time >= `status.started_at_unix_ms` (local-time parse via the existing `chrono` dep). A line with no parseable timestamp stays unbelievable during preload, so ownership is never assumed.

*Regression test:* watcher.rs's inline tests: (a) a preloaded encoder line timestamped before an adopted session's `startedAtUnixMs` must yield `snapshot().encoder == None`; (b) one timestamped after it must still be published (keeps the adopt case working); (c) an unparseable/undated line during preload must be dropped.

### A9-7 [medium, conf 0.98] [unfixed] A9-7 Freshness history leaks across sessions
`sabrage/crates/sabrage-core/src/session/watcher.rs:211-430` — **CONFIRMED** (re-rated high)

The timestamp checks are bounded now, but `ever_fresh`, `last_fresh_unix_ms`, and cached runtime state remain monitor-global. After session A streamed, session B can inherit A's `ever_fresh=true` and cached `streaming` state. Once B passes startup grace, A's stale timestamp can classify B as Stalled even though B has never produced fresh telemetry.
Evidence: `watcher.rs:211-218` stores freshness history without a run identity; `watcher.rs:354-386` updates it only on fresh observations and never resets it on run changes; `watcher.rs:416-430` uses that inherited history to emit `Stalled`.

*Recommendation:* Bind runtime-status history to the current run identity and reset cached state, `ever_fresh`, and `last_fresh_unix_ms` whenever the reported run changes. Test two consecutive runs in one monitor.

*Verifier:* Realistic path, not a corner: play a session, Stop it (`wineserver -k` kills the runtime, so runtime_status.json is frozen at `state: "streaming"`), launch again, and put the headset on more than 30 s later — which is precisely the window `SESSION_STARTUP_GRACE` exists for (watcher.rs:64-76 says the runtime writes nothing until the client connects and can take ~30 s). B inherits A's `ever_fresh` and A's `last_fresh_unix_ms`, so B's 10 s `STALL_GRACE_AFTER_FRESH` is already spent and the second launch flips to `Stalled` the moment it crosses 30 s. The UI shows the red `phase-alert` dot and the banner 'Stream stalled — wake the headset.' (ui/src/screens/Session.svelte:290,301,479-480; ui/src/components/Sidebar.svelte:30,47-48) over a perfectly healthy startup — the same regression that was live-verified and fixed for the *first* launch on 2026-08-29.

*Fix sketch:* Bind the freshness history to the reported run: store `fresh_run_id: Option<RunId>` next to `ever_fresh`/`last_fresh_unix_ms`, and at the top of the stall section (or beside the existing `encoder_run_id` reset at watcher.rs:509-514) clear `ever_fresh`, `last_fresh_unix_ms` and `runtime_status` whenever `status.run_id` differs from the run they were recorded for. Record `fresh_run_id = status.run_id` at watcher.rs:367-368.

*Regression test:* watcher.rs's inline tests: a two-run fixture on ONE monitor — poll with run A fresh+`streaming`, then poll with a record for run B (started past the 30 s grace, status file stale) — asserting `phase == Running`, and a second assertion that within a single run a genuinely stale stream still reaches `Stalled` (so the reset does not disable stall detection).

### A9-8 [medium, conf 0.99] [unfixed] A9-8 Newer schemas are protected only inside reconcile
`sabrage/crates/sabrage-core/src/session/state.rs:385-396` — **CONFIRMED** (re-rated low)

A newer record produces `Busy`, but launch ignores that outcome and `state::save` performs no schema-version check. A dead v2 record can therefore survive reconciliation, then be replaced by the new v1 `SessionState`, dropping its unknown guard exactly as A9-8 described.
Evidence: `reconcile.rs:426-430` returns `Busy` for an unsupported version; `stages/run/mod.rs:156-159` explicitly treats `Busy` as carrying nothing and continues; `state.rs:385-396` serializes over the existing record without checking `existing.is_supported_version()`.

*Recommendation:* Enforce the version refusal in `state::save` and `state::clear`, not only in reconcile, and make launch abort on the corresponding `Busy`. Preserve the original bytes in a v2-with-unknown-guard launch test.

*Verifier:* The read of the code is correct and the overwrite chain is unambiguous, but nothing on any branch writes `version >= 2`: `SESSION_STATE_VERSION` is 1 (state.rs:79) and `is_supported_version` is `version <= 1`, so a v2 record can today only come from a hand-edited file or from downgrading to an older build after a *future* schema bump. It is a real forward-compatibility guarantee that is not enforced where the guarantee is stated, with no reachable user impact until that bump lands — hence low rather than the reported medium. (The launch half — Busy not blocking — is the same code path as A9-1 and is scored there.)

*Fix sketch:* In `state::save`, after the existing `load(path)` under `lock_record`, add `if !existing.is_supported_version() { return Err(newer_schema(path, existing.version)) }` (a sibling of `owned_elsewhere`), and the same check in `state::clear`; keep `reconcile::untouchable`'s row as the friendly report and make `stages/run/mod.rs`'s launch treat the corresponding `Reconciled::Busy` as fatal so the error surfaces before any mutation rather than mid-launch.

*Regression test:* state.rs's inline `mod tests`: write a record with `version: 2` carrying an unknown key, call `save` with a fresh v1 `SessionState` and assert (a) it returns `Err`, (b) the on-disk bytes are unchanged; same for `clear`. Plus a stages/run test asserting a launch over a dead v2 record refuses before `preflight::run`.

*Cross-area files:* sabrage/crates/sabrage-core/src/stages/run/mod.rs

### A9-9 [medium, conf 0.96] [unfixed] A9-9 Detach's timeout safety write can race a winning Stop
`sabrage/crates/sabrage-core/src/session/reconcile.rs:1028-1061` — **CONFIRMED** (re-rated medium)

Detach checks cancellation only before firing its token, then waits at most five seconds and writes `detached=true` even if the live slot never cleared or Stop fired during the wait. If Stop wins but teardown keeps the record because audio restoration failed, this safety write can relabel a stopped session as detached; it can also race the supervisor's guard-flag saves after the timeout. The Tauri Stop path then interprets that flag as proof the game is still running.
Evidence: `reconcile.rs:1036-1047` checks `cancel` once and exits the wait on deadline without requiring slot clearance; `reconcile.rs:1049-1061` subsequently writes `detached=true` with no cancellation or ownership recheck.

*Recommendation:* Only perform the safety write after positively observing that this run's live slot cleared through detach. Recheck the terminal Stop token after the wait, and return a typed timeout instead of writing concurrently with supervision.

*Verifier:* The pre-check is monotonic only against a Stop that already fired; nothing re-checks it across the up-to-5 s wait, and the safety write does not require that the slot cleared through a detach. Machine state is not corrupted (reconcile's classify() keys off wine liveness, not `detached`, so a Dead record still gets its guards restored), so this is a wrong user-visible Stop result plus a misleading on-disk record, and it needs two concurrent user actions (Detach/app-quit + Stop) plus a guard-release failure to become durable - medium, not high.

*Fix sketch:* In session/reconcile.rs::detach: make the wait report how it ended (`let cleared = loop { if !live_session_is(..) { break true }; if now >= deadline { break false }; sleep }`), then before the safety write `if !cleared || handle.cancel.is_cancelled() { return Ok(()); }` - Stop is terminal and owns the record, and a timed-out wait means the supervisor may still be writing. Close the resurrect window by doing the load/save under the record lock or adding a `state::save_existing` (session/state.rs) that refuses to create a missing file, and use it here. Keep the signature `Result<()>` so no caller changes.

*Regression test:* sabrage/crates/sabrage-core/src/session/reconcile.rs `mod tests`, next to `detach_does_nothing_once_stop_has_already_fired`: (a) `detach_does_not_relabel_a_session_stopped_during_the_wait` - write a kept record (prev_audio_output Some, guards.audio_restored false), set_live_session, spawn detach, then fire handle.cancel + clear_live_session mid-wait, and assert the on-disk record still has `detached == false`; (b) `detach_that_times_out_leaves_the_record_alone` - never clear the slot, assert the record is byte-identical after DETACH_WAIT (use tokio::time::pause/advance to keep it fast).

*Codex next steps:* Barrier-test a live foreign same-bottle record through full launch and assert zero mutations. · Test unresolved audio and adb records followed by launch; verify every pending guard survives or launch refuses. · Fault-inject record-lock timeout/failure and concurrent different-run save/clear operations. · Run two-session monitor fixtures covering adopted encoder preload, zero-start-time identities, and freshness-history reset. · Race Stop and Detach with teardown held beyond five seconds and a pending audio guard.

## A10 — config-runtime-toml

**Codex verdict:** needs-attention — NO-SHIP: A10-1, A10-2, A10-3, and A10-5 remain incomplete. A10-4 now blocks ordinary live sessions, but its status-only path disagrees with the UI/shared predicate. New backup-transaction, parser-parity, BOM, and invalid-value UI regressions remain.

### A10-1 [critical, conf 0.98] [unfixed] A10-1 The final “CAS” is still a check-then-overwrite
`sabrage/crates/sabrage-core/src/config/runtime_toml.rs:1243-1247` — **CONFIRMED** (re-rated low)

A non-locking editor can save after the final comparison but before Sabrage’s rename. `write_atomic` performs several awaited temp-file operations and then unconditionally renames over the destination. Inference: bytes saved during this interval are lost, while the reserved backup contains the earlier `base`, not the displaced edit.
Evidence: `sabrage/crates/sabrage-core/src/config/runtime_toml.rs:1243-1247` executes `still_safe_to_replace(...)?` and only afterward awaits `write_atomic(...)`; `sabrage/crates/sabrage-core/src/executor.rs:807-824` writes, syncs, chmods, then calls `tokio::fs::rename(&tmp, path)`.

*Recommendation:* Add an Executor transaction primitive that atomically preserves the displaced destination as the backup and verifies it equals `base` before committing. Abort and restore when it differs. Add a barrier test that edits the destination after the final check but before rename.

*Verifier:* The window is real and the code path is unambiguous, so the mechanism is CONFIRMED, but the reviewer's `critical` rating and its remedy are both wrong. POSIX offers no compare-and-rename and no way to atomically capture the displaced inode, so the proposed 'Executor transaction primitive that atomically preserves the displaced destination' cannot be built; every implementation keeps some window. With the flock covering all Sabrage writers and setup.sh guarded, the only way to hit this is a hand editor whose own rename lands in a few-ms window, which is 'latent, unusual' bordering on 'never'. Re-rated critical -> low.

*Fix sketch:* Do not chase atomicity; make the displaced bytes recoverable. In config/runtime_toml.rs::write, immediately before the final write_atomic, hard-link the destination into the backups dir as `<backup>.displaced` via a new Executor::hard_link (executor.rs) — link(2) captures the inode that is live at that instant. After the rename returns, read the link: if its bytes == `base`, unlink it (the common case, zero cost); if they differ, keep it and return/emit a warning naming it ('an outside edit landed while Sabrage was saving; the displaced file is at <path>'). blocking_session/still_safe_to_replace stay as they are. Optionally also document in runtime_toml.rs's header that the CAS is best-effort against non-flock-participating writers.

*Regression test:* sabrage/crates/sabrage-core/src/config/runtime_toml.rs `#[cfg(test)] mod tests`: a test that pre-creates the destination, calls write with a patch, and — using a test Executor whose write_atomic mutates the destination just before delegating to the real rename — asserts that after the call the backups dir contains a `.displaced` file whose bytes are the concurrent editor's, and that the returned report/warning names it. Plus a happy-path test asserting no `.displaced` file survives a normal save.

*Cross-area files:* sabrage/crates/sabrage-core/src/executor.rs

### A10-2 [high, conf 0.99] Backup rotation mutates recovery history before the config commit
`sabrage/crates/sabrage-core/src/config/runtime_toml.rs:1232-1247` — **CONFIRMED** (re-rated medium)

The writer creates a new backup and irreversibly removes old backups before its final race check and config replacement. A detected concurrent edit, cancellation, or `write_atomic` failure therefore returns an error with the config unchanged but recovery history already pruned; the CAS error’s “nothing was written” claim is false.
Evidence: `sabrage/crates/sabrage-core/src/config/runtime_toml.rs:1232-1247` calls `reserve_backup_path`, loops through `executor.remove_file(...)`, and only then calls `still_safe_to_replace` and `write_atomic`.

*Recommendation:* Reserve the new backup before replacement, but perform pruning only after the config commit succeeds. Treat post-commit cleanup separately and preserve all prior backups on any abort or write failure.

*Verifier:* Ordinary failure modes (EACCES/ENOSPC on the OXRSys dir, a detected concurrent edit, cancellation between the two steps) all leave the config untouched while permanently pruning recovery history. BACKUP_KEEP is 10 (:101), so ten consecutive failed saves evict every genuine backup and replace them with identical copies of the current file — the exact history the ring exists to preserve. Not high: it needs a write that fails, and the immediate config is never damaged. Confirmed at medium.

*Fix sketch:* In config::runtime_toml::write, split the backup step: keep `reserve_backup_path` where it is (the backup must describe `base` and must exist before the destination moves), but move the `for old in stale { executor.remove_file(...) }` loop to AFTER the successful `write_atomic`, recomputing nothing (the `stale` list is already snapshotted before the reservation, which is what keeps dry-run and real-run plans identical — keep that snapshot point). On any error path between reservation and commit, leave `stale` untouched; optionally also unlink the just-reserved backup so an aborted save is a complete no-op, and only then is the CAS error's 'nothing was written' wording true.

*Regression test:* sabrage/crates/sabrage-core/src/config/runtime_toml.rs `#[cfg(test)] mod tests`, next to the existing prune test at :2622-2632: `a_failed_write_prunes_no_backups` — seed BACKUP_KEEP+2 backups, make the commit fail (chmod 0555 on the toml's parent dir, restored in the test), assert write returns Err, assert list_backups is byte-for-byte the seeded list (same count, same paths, no new entry), and assert the config bytes are unchanged. Add a sibling case that forces the second still_safe_to_replace to fail (mutate the file through a test Executor hook) and asserts the same invariant plus the 'nothing was written' wording.

### A10-3 [high, conf 0.99] [unfixed] A10-2 The new preflight helper reintroduces last-raw-wins
`sabrage/crates/sabrage-core/src/config/runtime_toml.rs:670-674` — **REFUTED** (re-rated low)

`effective_string` returns the final raw assignment without applying the key’s accepted set. Run preflight consumes it for `protocol` and `encoder_process`. Thus `protocol = "alvr"` followed by `protocol = "bogus"` makes preflight block on `bogus`, although Config.cpp ignores it and remains on ALVR—the exact invalid-last divergence A10-2 was meant to close.
Evidence: `sabrage/crates/sabrage-core/src/config/runtime_toml.rs:670-674` ends the unfiltered assignment iterator with `.last()`; `sabrage/crates/sabrage-core/src/stages/run/preflight.rs:125-147` feeds that result directly into launch facts, while `ext/oxrsys/runtime/src/Config.cpp:435-440,541-544` assigns only accepted protocol values and retains the last valid/default value.

*Recommendation:* Use `read_lines_like_the_runtime` for modeled launch keys, or make the helper key-aware and skip rejected assignments. Add invalid-final protocol and encoder cases to preflight tests.

*Verifier:* No reintroduction: the fixed A10-2 was the view's effective values, and the view is correct. effective_string is a deliberately separate, documented shell-parity helper, and the native preflight matching run.sh is required by CLAUDE.md's rule that a run-preflight change must land in run.sh + contract/pipeline.toml + the native preflight in the same commit — 'fixing' only the Rust side would make Sabrage launch a config demo.sh refuses, which is the parity break, not the current state. The residual is a cosmetic internal inconsistency (Settings renders protocol=alvr with an invalid-value chip while Launch refuses naming 'bogus'), on a config that only a bad hand-edit produces, and the refusal is conservative and actionable rather than a silent wrong launch.

### A10-4 [high, conf 0.99] [unfixed] A10-3 The multiline refusal exists only in the view layer
`sabrage/crates/sabrage-core/src/config/runtime_toml.rs:826-844` — **CONFIRMED** (re-rated medium)

`round_trip_error` detects physical assignments hidden inside TOML multiline strings, but neither `apply_patch` nor core `write` invokes it. `edit_protocol` calls `write` directly, so it can rewrite an outer protocol line and report success while the runtime continues obeying a later physical line inside the multiline string.
Evidence: `sabrage/crates/sabrage-core/src/config/runtime_toml.rs:826-844` validates and parses directly with `strip_bom(text).parse()` without calling `round_trip_error`; `:1199` calls this function from `write`. The existing multiline test only asserts `view.parse_error` rather than exercising a write.

*Recommendation:* Enforce `round_trip_error` inside `apply_patch` or immediately after `write` reads `base`, so every caller receives the refusal. Test `apply_patch`, `write`, and `edit_protocol` against the multiline fixture.

*Verifier:* Real and reachable, but not on the GUI Save path the reviewer implies. The realistic route is doctor: checks/config.rs's parse_protocol sees the physical line inside the string, FAILs cfg.protocol.legacy-oxrsys, offers fix.edit-protocol, and the fix then reports a false success (or, when the outer line already says alvr, 'already has protocol = "alvr"') while the runtime is unchanged — doctor stays red forever with a fix that claims it worked. It needs a multiline TOML string containing an assignment-shaped physical line, which no template produces and only an odd hand-edit creates. Also worth fixing: the stale claim at commands.rs:1696-1698 that 'write would refuse on its own re-parse too (via apply_patch)' is true only for the unparseable case, not this one. Re-rated high -> medium.

*Fix sketch:* Call round_trip_error from apply_patch (config/runtime_toml.rs), right where it currently does `strip_bom(text).parse::<DocumentMut>()` at :838-843: replace that with `if let Some(e) = round_trip_error(text) { return Err(SabrageError::InvalidInput(format!("oxrsys-runtime.toml cannot be safely rewritten: {e}"))) }` followed by the existing parse (round_trip_error already returns the toml_edit parse error for case 1, so the two messages unify). Every caller — write, edit_protocol, the Tauri command, the golden tests — then inherits the refusal, and edit_protocol's existing `?` at :1518 surfaces it as a fix error instead of a false success. Update the now-inaccurate doc comment at src-tauri/src/commands.rs:1696-1698.

*Regression test:* sabrage/crates/sabrage-core/src/config/runtime_toml.rs `#[cfg(test)] mod tests`: extend the existing multiline case (today it only asserts view.parse_error) with (a) apply_patch on the multiline fixture returns Err whose message contains 'multiline string', (b) write on that file returns Err and leaves the bytes byte-identical and creates no backup, (c) edit_protocol over a fix_ctx seeded with that file returns Err rather than a changed/unchanged FixReport. Add the fixture as sabrage/crates/sabrage-core/tests/fixtures/phase4/oxrsys-runtime.multiline-shadow.toml (new file, existing fixtures untouched).

*Cross-area files:* sabrage/src-tauri/src/commands.rs

### A10-5 [high, conf 0.98] A BOM on a root assignment makes Save unable to change the runtime value
`sabrage/crates/sabrage-core/src/config/runtime_toml.rs:353-374` — **CONFIRMED** (re-rated medium)

The TOML document is parsed after removing the BOM, but the runtime-style scanner sees the original BOM-prefixed key. The mismatch check only rejects when physical occurrences exceed parsed occurrences, so a root `\uFEFFprotocol = "alvr"` has zero runtime occurrences and one editable TOML occurrence yet is declared writable. A Save edits—or treats as already equal—the stripped key and restores the BOM; Config.cpp still ignores the key and uses its `oxrsys` default.
Evidence: `sabrage/crates/sabrage-core/src/config/runtime_toml.rs:353-374` parses `strip_bom(text)`, scans `raw_assignments(text)`, and checks only `physical > occurrences`; `:839-840,946-947` likewise strips before editing and reinserts the BOM afterward.

*Recommendation:* Treat any parsed editable occurrence without a corresponding runtime-visible physical occurrence as non-round-trippable, or insert a later bare runtime-visible assignment instead of editing the BOM-prefixed line. Add a root-key BOM regression test.

*Verifier:* Real and reachable, but it needs a BOM immediately followed by a root-level assignment of one of the six editable keys. The deployed file (contract/oxrsys-runtime.toml.template) starts with comments and puts all six keys under [streaming], and a BOM before a comment or before '[streaming]' is harmless (both are filtered by raw_assignments), so this only bites a hand-written/Windows-editor-saved file whose very first line is e.g. `protocol = ...`. Fail mode is a silent no-op Save or a 'changed_keys: [protocol]' success report with no effect on the runtime — latent, unusual conditions, no data loss (a backup is still taken). Downgraded high -> medium.

*Fix sketch:* In round_trip_error (runtime_toml.rs:353-374) compare the two counts with `!=` instead of `>` (or, equivalently, refuse when a parsed editable occurrence has no runtime-visible physical counterpart), so a BOM-prefixed root key yields the same 'edit this file by hand' parse_error the multiline case gets. Do NOT teach raw_assignments to strip the BOM: that would make the reader disagree with Config.cpp, which sees the byte too. Pair with the A10-3 fix so apply_patch/write consult round_trip_error, not only the view.

*Regression test:* sabrage/crates/sabrage-core/src/config/runtime_toml.rs #[cfg(test)] module, beside the existing multiline round-trip tests: a fixture "\u{feff}protocol = \"alvr\"\n[streaming]\n..." asserts (a) read().parse_error is Some, (b) apply_patch/write return InvalidInput instead of reporting changed_keys, and (c) read_lines_like_the_runtime still reports protocol=None so the view stays honest.

### A10-6 [medium, conf 1] [unfixed] A10-5 Mixed line endings are still normalized outside the edited value
`sabrage/crates/sabrage-core/src/config/runtime_toml.rs:908-935` — **CONFIRMED** (re-rated low)

The fix preserves only files whose line endings are uniformly CRLF. Any mixed file sets `crlf = false`, after which the complete `toml_edit` rendering remains LF. Even a CRLF-majority file with one LF therefore has every unrelated CRLF rewritten, violating the six-values-only byte contract.
Evidence: `sabrage/crates/sabrage-core/src/config/runtime_toml.rs:908-910` explicitly concedes mixed endings are not preserved, and `:919-935` sets `crlf` only when every break is CRLF and otherwise returns the LF rendering unchanged.

*Recommendation:* Patch the raw value span instead of rendering the whole document, preserve the per-line terminator sequence, or refuse mixed-ending files. Add both LF-majority and CRLF-majority mixed fixtures.

*Verifier:* Behaviour reproduced and it does violate the module header's 'six values and nothing else' byte contract. But it is an explicitly reasoned tradeoff in-code, it is not a declared PARITY.md divergence and not covered by any shell counterpart, and the damage is a whole-file CRLF->LF normalisation on a file that is already mixed — no data loss, a backup is taken, and mixed endings are hard to produce on this macOS-only path (setup.sh and Sabrage both write LF). Downgraded medium -> low.

*Fix sketch:* ByteShape::of/restore (runtime_toml.rs:908-956): either (a) record the per-line terminator sequence of the input and re-apply it line-by-line in restore (falling back to LF for lines the render added), or (b) patch only the raw value span instead of re-rendering the document, or (c) the cheap honest option — treat a mixed-ending file like the multiline case and refuse it in round_trip_error with an 'edit this file by hand' message rather than silently normalising it.

*Regression test:* runtime_toml.rs tests beside the existing byte-shape goldens: two fixtures (CRLF-majority-with-one-LF, LF-majority-with-one-CRLF) asserting that after a single bitrate edit the output differs from the input only in the bitrate value bytes (or, if option (c) is chosen, that apply_patch returns InvalidInput and the file is untouched).

### A10-7 [medium, conf 0.96] The std::stof emulation uses different precision from the runtime
`sabrage/crates/sabrage-core/src/config/runtime_toml.rs:550-586` — **CONFIRMED** (re-rated low)

The runtime parses `resolution_scale` into a C++ `float`, but Rust parses an `f64` and applies the bounds before any f32 rounding. Inference: on the targeted IEEE-754 platforms, valid TOML `1.00000001` rounds to `1.0f` and is accepted by Config.cpp, while this reader retains a value above 1.0 and rejects it, causing Settings to substitute 0.75 although the runtime uses 1.0.
Evidence: `sabrage/crates/sabrage-core/src/config/runtime_toml.rs:550-586` implements `stof` as `Option<f64>`; `ext/oxrsys/runtime/src/Config.cpp:367-372` uses `float val = std::stof(value)` before the range check.

*Recommendation:* Match `std::stof` by parsing/converting to f32 before validation, including its prefix syntax, then widen for the Rust view. Differential-test values around both bounds and hexadecimal float spellings.

*Verifier:* Divergence is real and demonstrated, but it needs a hand-written resolution_scale that differs from a bound by less than one f32 ULP (~6e-8) — i.e. a value nobody types and Sabrage itself never writes (it writes its own canonical spelling). The consequence is a wrong 'invalid value' chip and a wrong displayed value, not a wrong write (a Save writes a valid value and both sides then agree). Downgraded medium -> low.

*Fix sketch:* Narrow before validating, the way std::stof does: have stof parse its prefix into f32 (`s[..i].parse::<f32>().ok()`), keep in_scale_range's comparison in f32 against 0.25f32/1.0f32, and widen to f64 only when building RuntimeConfigValues.resolution_scale (f64::from). Setting::to_value must keep emitting Sabrage's canonical decimal spelling so no golden bytes move.

*Regression test:* runtime_toml.rs tests, next to the existing stof/`0.9 (was 0.75)` prefix tests: a table asserting read_lines_like_the_runtime accepts `1.00000001` as 1.0 and `0.2499999999` as 0.25 (empty `invalid`), still rejects `1.01` and `0.24`, and a comment naming Config.cpp:367-372 as the reference.

### A10-8 [medium, conf 0.99] The live-session guard blocks status files the UI deliberately treats as idle
`sabrage/crates/sabrage-core/src/config/runtime_toml.rs:2816-2828` — **CONFIRMED** (re-rated low)

The owned regression test asserts that a fresh `runtime_status.json` without `process_id` blocks writing. The shared SessionMonitor and declared divergence require both freshness and a live named PID, so the UI can show Idle and enable Save while the backend rejects the same pidless, dead-PID, or fresh-idle status. This is a cross-file dishonest state introduced by the A10-4 guard wiring.
Evidence: `sabrage/crates/sabrage-core/src/config/runtime_toml.rs:2816-2828` writes only `state` and `updated_at_unix_ms` and expects `write` to fail; `sabrage/crates/sabrage-core/src/session/watcher.rs:370-383` requires `process_id` to be alive before deriving External.

*Recommendation:* Make status-only blocking require the same fresh-and-live-PID predicate as SessionMonitor. Update this test to include a live PID and add negative cases for missing, dead, and idle status records.

*Verifier:* The predicate mismatch is real, but the realistic window is small and fails closed: the shipped runtime always writes process_id (ext/oxrsys/runtime/src/RuntimeStatus.cpp:241) and RUNTIME_STATUS_MAX_AGE is 3 s (watcher.rs:594), so the disagreement is confined to the <=3 s after the oxrsys process dies (or to a hand-made/older status file). The user sees one confusing refusal that resolves itself on the next poll; nothing is written, nothing is lost. The owned test at runtime_toml.rs:2806-2831 uses a pidless fixture the real runtime never produces, so it encodes deliberate conservatism rather than the shipped behaviour — worth aligning, not a high. Downgraded high -> low.

*Fix sketch:* Extract one predicate — e.g. `watcher::runtime_status_live(rs, now) -> bool` = is_fresh(stamp, now) && rs.process_id.is_none_or(is_alive) (or the stricter '&& pid alive', chosen once) — and call it from both session_block_at (session/mod.rs:455-467) and SessionMonitor's External derivation (watcher.rs:370-383), so the door and the phase always agree. Whichever way the pidless case is decided, decide it in that one function.

*Regression test:* sabrage/crates/sabrage-core/src/session/mod.rs tests: session_block_at over four status fixtures (fresh+live pid -> blocks, fresh+dead pid -> matches whatever the monitor reports for the same file, fresh+no pid -> explicit documented choice, stale -> no block); and update the owned runtime_toml.rs live-guard test (:2806-2831) to write a live process_id so it asserts the shared predicate rather than freshness alone.

*Cross-area files:* sabrage/crates/sabrage-core/src/session/mod.rs, sabrage/crates/sabrage-core/src/session/watcher.rs

### A10-9 [high, conf 0.95] Multiple invalid occurrences collide in the Settings keyed list
`sabrage/ui/src/screens/Settings.svelte:443-449` — **CONFIRMED** (re-rated medium)

The new fold can return multiple distinct invalid values for one key because it deduplicates the entire `InvalidValue`, including `raw`. Settings keys the list only by `iv.key`. Inference: two lines such as `protocol = "bad1"` and `protocol = "bad2"` violate Svelte’s keyed-each uniqueness requirement and can throw or misreconcile precisely when the screen is supposed to explain malformed hand-maintained input.
Evidence: `sabrage/crates/sabrage-core/src/config/runtime_toml.rs:643-646` pushes each non-identical invalid value, while `sabrage/ui/src/screens/Settings.svelte:443-449` renders them with `{#each configStore.view.invalid as iv (iv.key)}`.

*Recommendation:* Expose a stable occurrence identity or line index and key the UI by it; at minimum use a composite including index and raw value. Add a UI test with two different invalid assignments for the same key.

*Verifier:* Reproduced the Rust half exactly; the Svelte half is unambiguous from the vendored runtime source (duplicate keyed-each throws in dev and prod, no error boundary). Downgraded high -> medium: it needs a hand-edited config where one key is assigned twice, in two different tables, with two distinct runtime-invalid spellings; the more common same-key-twice-in-[streaming] case is intercepted by the parse-error branch.

*Fix sketch:* Give each rejected occurrence a stable identity: add `pub occurrence: usize` (serde camelCase) to `InvalidValue` in sabrage/crates/sabrage-core/src/config/runtime_toml.rs:222-240, set it from the per-key `count` in `read_lines_like_the_runtime` (runtime_toml.rs:635-647), and key the UI by `${iv.key}#${iv.occurrence}`. Cheaper alternative that touches no Rust: key the each block by `(iv.key + iv.raw)` in sabrage/ui/src/screens/Settings.svelte:445 — unique because the fold already drops entries identical in key+raw+reason. Either way mirror the type in sabrage/ui/src/ipc.ts.

*Regression test:* In runtime_toml.rs's `mod tests`, add `two_invalid_occurrences_of_one_key_stay_distinguishable`: read "[streaming]\nprotocol = 'alvr'\n\n[legacy]\nprotocol = \"oxrsys-usb\"\n", assert parse_error.is_none(), invalid.len()==2, both key=="protocol", and that the chosen UI identity is unique across the list (distinct `occurrence`, or distinct `(key, raw)` pairs). There is no JS test harness in sabrage/ui (no vitest in package.json), so the uniqueness invariant has to be asserted on the Rust side.

*Cross-area files:* sabrage/ui/src/screens/Settings.svelte, sabrage/ui/src/ipc.ts

*Codex next steps:* Add a barrier-controlled writer test that changes the destination after the final check and a failure-injection test proving aborted writes never prune backups. · Exercise multiline-string and root-BOM files through read, apply_patch, write, and edit_protocol—not only the view helper. · Run the invalid-final fixture through launch preflight and render Settings with two distinct invalid occurrences of one key. · Differential-test Rust parsing against Config.cpp for f32 boundary rounding and other std::stof prefix forms. · Align config-write session tests with SessionMonitor using live, dead, missing, and idle process IDs.

## A11 — ipc-boundary

**Codex verdict:** needs-attention — No-ship: A11-1 remains broken on Stop-and-quit, A11-3's fix cancellation is only cosmetic, and queued mutations can bypass the new live-session guard. A11-2, A11-4, and A11-5 otherwise appear closed.

### A11-1 [high, conf 0.99] [unfixed] A11-1 Stop-and-quit still approves exit without stopping or detaching
`sabrage/src-tauri/src/commands.rs:1080-1098` — **CONFIRMED** (re-rated low)

The direct Stop command now reports a timeout honestly, but Stop-and-quit does not. The stop wait first fires the session's cancel token. On timeout, `resolve_quit` calls detach, discards its result, sets `QuitApproved`, and exits. Core detachment explicitly returns success without doing anything once that cancel token is set, while `QuitApproved` makes the final exit hook skip its fallback. The app can therefore claim it detached and quit while the game or guards remain in an unconfirmed state.
Evidence: `sabrage/src-tauri/src/commands.rs:784` executes `handle.cancel.cancel()`; `commands.rs:1094-1098` then executes `let _ = detach_live_session().await;`, stores approval, and exits. The called implementation returns immediately at `sabrage/crates/sabrage-core/src/session/reconcile.rs:1036-1037`: `if handle.cancel.is_cancelled() { return Ok(()); }`. Finally, `commands.rs:839-843` skips termination detachment when approval is true.

*Recommendation:* Do not approve or exit on `TimedOut`; keep the dialog open and return the actionable error. If quitting must remain possible, add an explicit core stop-to-detach handoff that confirms the guards are safely disarmed and the record is marked detached before setting `QuitApproved`.

*Verifier:* The cited code path is unambiguous and reachable (any Stop-and-quit whose teardown exceeds LIVE_SESSION_STOP_TIMEOUT = 30 s, commands.rs:721), and the reproduction confirms the detach is a no-op. Re-rated high -> low: the resulting on-disk/machine state is the one the next reconcile fully recovers, the message already tells the user the game may still be running and gives the `./demo.sh stop` remedy, so the only real user-visible defect is the inaccurate word "detached" plus a misleading no-op call.

*Fix sketch:* In `resolve_quit`'s `Stop` arm (sabrage/src-tauri/src/commands.rs:1080-1096): (a) reword the `TeardownWait::TimedOut` message to state what actually happened — Sabrage stopped supervising and quit while teardown was still running, the game may still be running, the next launch will reconcile — and drop the claim of a detach; (b) make the `if refusal.is_some() { detach_live_session() }` call honest: either skip it on `TimedOut` (where `reconcile::detach` provably no-ops because `cancel` is already fired) or route the TimedOut arm through a distinct helper that only records the reason. Optionally add a `debug_assert`/comment at reconcile.rs:1036 naming `resolve_quit`'s TimedOut arm as the caller this early return silently absorbs.

*Regression test:* sabrage/src-tauri/src/commands.rs unit tests (next to `wait_for_slot_clear_reports_the_deadline_it_hit`, commands.rs:2370): a pure test over the refusal-string builder factored out of the `Stop` arm asserting the TimedOut text does not claim a detach; plus a sabrage-core test beside session/reconcile.rs's `mod tests` asserting `reconcile::detach` with an already-cancelled `cancel` leaves `detached: false` on disk and never fires `handle.detach` (i.e. pins the no-op as intended, so no future caller relies on it detaching).

*Cross-area files:* sabrage/crates/sabrage-core/src/session/reconcile.rs

### A11-2 [medium, conf 0.98] [unfixed] A11-3 The newly registered fix canceller has no reachable run ID
`sabrage/src-tauri/src/commands.rs:1140-1157` — **CONFIRMED** (re-rated medium)

The branch registers every fix in `RunRegistry`, but does not expose that random run ID before the fix acquires the operation lock. `applyFix` returns only `FixReport`; ordinary fixes emit no `StageStarted`/queued event, and whole-stage fixes invoked through this door emit `StageStarted` only after `fixes::apply` has acquired the lock. Worse, that acquisition is the uncancellable variant. A Doctor fix queued behind a long build therefore cannot be named or cancelled and can later mutate after its initiating UI has disappeared. The registration only creates the appearance of cancellation support.
Evidence: `sabrage/src-tauri/src/commands.rs:1152-1155` mints and registers `run_id` and immediately awaits `fixes::apply`, without emitting it. `sabrage/ui/src/ipc.ts:381-398` exposes only a `Promise<FixReport>` plus the event channel. Core waits at `sabrage/crates/sabrage-core/src/fixes/mod.rs:345-348` using `acquire_operation_lock().await`, which does not observe `ctx.cancel`.

*Recommendation:* Emit a per-fix queued/started event carrying the run ID before waiting, and acquire the operation lock through `acquire_operation_lock_cancellable(&ctx.cancel)`. Add a held-lock test proving a queued ordinary fix can be identified, cancelled, and settles without executor mutations.

*Verifier:* Reachable whenever the operation lock is contended — a Doctor "Fix" pressed while a GUI stage or a `sabrage` CLI build holds the in-process mutex or the advisory file lock (stages/mod.rs:459-466, PARITY.md:113 notes CLI builds can hold it for minutes). The fix then waits invisibly, cannot be cancelled (uncancellable acquire, and no observable run id even if it were), and mutates when its turn comes — including after a webview reload has discarded the initiating UI. Severity medium, not high: it needs contention, and the mutation that eventually lands is the one the user asked for; the defect is loss of cancellation and of the queued-wait feedback the stage door already guarantees.

*Fix sketch:* Two halves. Core: change `fixes::apply` (sabrage/crates/sabrage-core/src/fixes/mod.rs:345-348) to mirror `run_stage` — after `deny_if_session_live`, emit a queued/started marker carrying `ctx.run_id` (a `StageEvent::Section`/`Line` for ordinary fixes, or a new `FixStarted`-shaped event), emit the `waiting for another Sabrage operation to finish` info row when `operation_in_progress_anywhere()`, and acquire via `acquire_operation_lock_cancellable(&ctx.cancel)`, returning `SabrageError::Cancelled` when it yields `None`. Tauri: `commands::fix` (commands.rs:1140-1157) keeps its `RunRegistry` registration — it becomes reachable once the event carries the id — and must still `registry.forget(&run_id)` on the cancelled path. UI: `Doctor.svelte`'s `runFix` and `GateModal.svelte`'s `doApplyFix` capture `ev.runId` from the first event and enable a Cancel button wired to `cancelStage(runId)`.

*Regression test:* sabrage/crates/sabrage-core/src/fixes/mod.rs `mod tests`: hold `stages::OPERATION_LOCK` (via `acquire_operation_lock()`) in the test task, spawn `fixes::apply` for an ordinary (non-stage) fix with a recording sink, assert (a) an event carrying `ctx.run_id` arrives while the lock is still held, (b) firing `ctx.cancel` makes the call settle as `SabrageError::Cancelled`, and (c) the `RecordingExecutor` saw zero planned actions. Plus a UI test/assertion in sabrage/ui that `applyFix`'s event callback surfaces a runId to the Cancel affordance.

*Cross-area files:* sabrage/crates/sabrage-core/src/fixes/mod.rs, sabrage/ui/src/screens/Doctor.svelte, sabrage/ui/src/components/GateModal.svelte, sabrage/PARITY.md

### A11-3 [high, conf 0.97] Queued forbidden operations are not rechecked after a session becomes live
`sabrage/crates/sabrage-core/src/stages/mod.rs:770-795` — **CONFIRMED** (re-rated high)

The new live-session policy is checked only before waiting for the operation lock. A queued Run can acquire first, publish its live handle, and deliberately release the lock at the launch boundary; a Setup/Build/Install or forbidden fix that checked while the machine was idle then acquires the lock and dispatches without another liveness check. Inference: this is directly reachable across the first-class GUI/CLI frontends—for example, a GUI install waiting behind a CLI operation can lose the file-lock race to a CLI run, then proceed under the newly launched session. That can replace loaded artifacts or remove active wired forwards, violating the declared single live-session mutation guard.
Evidence: `sabrage/crates/sabrage-core/src/stages/mod.rs:774` calls `deny_stage_while_session_live` before the wait at line 782, then dispatches at line 794 with no second check. A Run publishes the live handle at `stages/run/mod.rs:444-454` and drops the operation lock at lines 473-477. Fixes repeat the same ordering at `sabrage/crates/sabrage-core/src/fixes/mod.rs:345-348`.

*Recommendation:* Keep the pre-lock fast refusal, but repeat the same live-session predicate immediately after acquiring the complete in-process/file lock and before dispatching every forbidden stage or fix. Test by holding the lock, queueing a forbidden operation, establishing a live session, releasing the lock, and asserting zero mutations.

*Verifier:* Unambiguous and reachable on a realistic two-front-end workflow (Sabrage GUI + `./demo.sh run`, the documented supported combination), with no timing race required in the demo.sh variant. The consequence is mutation of artifacts a running game has mapped — the exact class of breakage the unified live-session predicate exists to prevent.

*Fix sketch:* Add a post-acquire recheck at both doors. stages/mod.rs `run_stage`: after `acquire_operation_lock_cancellable` returns `Some(guard)` (line 782-784) and before the `Stage::Run` branch/`dispatch` (785-794), call `deny_stage_while_session_live(stage, ctx)?` again — it is already a pure `Result`-returning helper, and `finish_stage` must still emit `StageFinished` for the refusal (wrap as `return finish_stage(stage, ctx, Err(..))` so the UI does not see a started-never-finished stage). fixes/mod.rs `apply`: after line 347's acquire, call `deny_if_session_live(action, ctx)?` a second time before `apply_holding_lock`. Keep the existing pre-lock check as the fast refusal. Optionally factor the pair into a `guard_then_lock` helper so a third door cannot forget one half.

*Regression test:* sabrage/crates/sabrage-core/src/stages/mod.rs `mod tests` (beside `live_session_block_sees_a_running_session_recorded_on_disk`, line 1226): take `acquire_operation_lock()` in the test, spawn `run_stage(Stage::Install, &ctx)` against scratch `Paths` on an idle fixture so the pre-lock check passes, then write a live `session-state.json` (the existing `write_live_session_state` helper) / a fresh `runtime_status.json`, drop the guard, and assert the stage resolves to the `refusing to run install while a session is live` fatal and the ctx's RecordingExecutor planned zero actions. Mirror it in fixes/mod.rs's tests for one `forbidden_while_session_live` fix.

*Cross-area files:* sabrage/crates/sabrage-core/src/stages/mod.rs, sabrage/crates/sabrage-core/src/fixes/mod.rs

*Codex next steps:* Add a Stop-and-quit timeout test with an already-cancelled handle and assert neither approval nor exit occurs without confirmed teardown/detachment. · Hold the operation lock, queue an ordinary fix through the Tauri boundary, and verify its run ID is observable and cancellation ends the wait without mutations. · Queue Setup/Build/Install and every session-forbidden fix, establish a live session before releasing the lock, and verify the post-lock check refuses them. · Exercise a webview reload while a stage and fix are queued to verify no orphaned operation later mutates the machine.

## A12 — ui-shell-session

**Codex verdict:** needs-attention — Do not ship. A12-1 is only papered over, A12-4 can replay launches without a new command, and the gate/session controls expose three additional dishonest or inverted operation states. A12-2, A12-3, A12-5, A12-6, and A12-7 close their original failure paths.

### A12-1 [medium, conf 1.0] [unfixed] A12-1 — the GUI ignores the deferred destructive-fix policy
`sabrage/ui/src/ipc.ts:305-313` — **CONFIRMED** (re-rated medium)

Core now deliberately withholds `fix.delete-session-json`, but the independently mirrored frontend converts every key present in `FIX_META` into an actionable fix. `CheckRow` consequently still renders a Fix button for `cfg.session-pins`. The expanded confirmation discloses the black-screen outcome and backup, but the declared no-button fix did not reach the client, leaving a known-bad destructive recovery exposed.
Evidence: `sabrage/ui/src/ipc.ts:311-313` says `return bare in FIX_META ? (bare as FixAction) : null`, while `sabrage/crates/sabrage-core/src/fixes/mod.rs:65-76` explicitly puts `fix.delete-session-json` in `DEFERRED_CONTRACT_FIX_IDS`; `sabrage/ui/src/components/CheckRow.svelte:25` renders the button whenever this frontend conversion is non-null.
Inference: when the contract-backed Doctor row carries `fix.delete-session-json`, this mapper returns the action despite the core policy.

*Recommendation:* Make deferred status authoritative at the IPC boundary: project Doctor fixes through `FixAction::from_contract_id` server-side, or mirror and test the exact deferred-ID set in TypeScript. Assert that `contractFixIdToAction("fix.delete-session-json")` is null and that the row has no button.

*Verifier:* The TS mirror is authoritative for whether a button renders, and it models delete-session-json as actionable while core deliberately withholds it. ipc.ts:305-310's own comment is stale ("fix.create-z-drive is the one remaining deliberately-deferred id"), so nothing in the frontend encodes the second deferred id. Mitigations exist (in-app confirm showing FIX_META.consequence, backend `confirmed` gate, backup copy in fixes::session_json) so this is medium, not high: it takes an explicit confirm, but the outcome is the documented known-bad 800x900-black-screen remedy the row's own message tells the user not to do.

*Fix sketch:* Make the deferred set authoritative on the Rust side of the wire: in `run_doctor` (sabrage/src-tauri/src/commands.rs:215) project through `FixAction::from_contract_id` before filling `DoctorEvent.fix` (e.g. `spec.and_then(|s| s.fix.as_deref()).filter(|id| FixAction::from_contract_id(id).is_some()).map(str::to_owned)`), so a deferred id never reaches the client. Mirror it in the frontend for defence in depth: add `const DEFERRED_FIX_IDS = new Set(["fix.create-z-drive", "fix.delete-session-json"])` in sabrage/ui/src/ipc.ts and return null from `contractFixIdToAction` for members, updating that function's stale doc comment.

*Regression test:* Rust: a #[test] in sabrage/src-tauri/src/commands.rs asserting that for every id in `sabrage_core::fixes::DEFERRED_CONTRACT_FIX_IDS` the doctor-event projection yields `None` (and that a non-deferred id such as `fix.set-graphics-backend` survives). TS: once a vitest harness exists under sabrage/ui, assert `contractFixIdToAction("fix.delete-session-json") === null` and that CheckRow renders no `.fix-btn` for a row carrying that id.

*Cross-area files:* sabrage/ui/src/ipc.ts, sabrage/src-tauri/src/commands.rs

### A12-2 [high, conf 0.99] [unfixed] A12-4 — an old menu request launches again after Session remounts
`sabrage/ui/src/screens/Session.svelte:164-181` — **REFUTED** (re-rated low)

The request counter lives in persistent `App`, but its consumed counter lives inside the conditionally mounted `Session`. After any Pipeline → Launch, navigating away destroys `Session`; returning normally creates it with `lastHandledLaunchRequest = 0`, sees the old nonzero request, and calls `doLaunch(false)` without another menu action. If the prior session has exited, this silently launches Beat Saber again.
Evidence: `sabrage/ui/src/screens/Session.svelte:164-181` initializes `lastHandledLaunchRequest` to zero and launches whenever it differs from the prop; `sabrage/ui/src/App.svelte:31,63-65,78-87` retains the incremented counter while conditionally mounting and unmounting `Session`.
Inference: Svelte destroys the inactive `{#if}` branch, so the local acknowledgment is lost while the parent token survives.

*Recommendation:* Keep consumption state beside `launchRequest` in `App`, or pass an acknowledgment callback that clears/records the exact token after handling. Add a regression that invokes ⌘R once, exits the session, navigates away, then returns through the sidebar and asserts no second launch.

*Verifier:* The stale-token mechanism the reviewer describes is real (lastHandledLaunchRequest is component-local, launchRequest lives in App.svelte:31), but its claimed consequence — a second Beat Saber launch with no menu action — cannot occur: the effect always runs before this screen's own bottle prefill lands, so a replayed token dead-ends in the `No bottle selected` branch and is then consumed for good. NOTE for the parent: the same ordering is a real defect in the other direction (out of scope for this finding, and the reviewer did not report it): after wave 4, a Pipeline > Launch / ⌘R issued from any other screen — the only path that mounts Session — always consumes its token before the prefill and shows a false `No bottle selected — choose one below, then Launch.` notice instead of launching. ⌘R only works when Session is already mounted and prefilled.

### A12-3 [high, conf 0.98] Hiding a gate lets Cancel target the previous stage while displaying the next
`sabrage/ui/src/components/GateModal.svelte:117-130` — **CONFIRMED** (re-rated high)

A running modal can be hidden while `running` and `runId` remain live. The still-open Stages panel can then replace `request` with another stage. Because the effect refuses to start the new request until `running` becomes false, the modal immediately displays the new stage name over the previous stage's rows and Cancel still uses the previous `runId`. Pressing Cancel therefore cancels the operation the UI no longer names, then the newly displayed stage starts when the old promise settles.
Evidence: `sabrage/ui/src/components/GateModal.svelte:117-125` defers a replacement request while `running`; `:233-235` cancels the retained `runId`; `:454` titles the modal from the replacement `request.stage`; and `:567-569` offers Hide and Cancel during the old run. `sabrage/ui/src/components/StagesPanel.svelte:93-103` replaces the singleton gate request without closing the panel.
Inference: the `running` transition to false re-evaluates the effect and starts the deferred replacement after the misdirected cancellation.

*Recommendation:* Separate immutable `activeRequest`/`activeRunId` from any queued request. Keep the modal bound to the active operation until it settles, and either reject a second request or render it explicitly as queued with its own cancellation state.

*Verifier:* Every step is plain, unconditional code on a path a user reaches with three clicks (Run Setup -> Hide -> Run Build), and long-running stages are exactly what Hide exists for. The result is a modal that lies about which operation it is showing, a Cancel that aborts an operation the UI no longer names, and an unrequested-looking auto-start of the second stage afterwards. Cancelling install mid-flight (global DXMT/wineopenxr overlays) makes this more than cosmetic; hence high rather than medium.

*Fix sketch:* In GateModal.svelte, split the displayed operation from the pending one: keep `activeRequest`/`activeRunId` set when `start()` begins and cleared when it settles, render the title, rows and Cancel from `activeRequest`/`activeRunId`, and either (a) have `stageStore.openGate` refuse (or queue explicitly) while `stageStore.busy` is true — adding a `busy` flag the modal sets — or (b) render the second request as a distinct 'queued: <stage>' line with its own cancel. Simplest complete variant: add `running` to the stage store, disable StagesPanel's Run/Dry-run and Doctor's whole-stage Fix while it is true, and keep GateModal bound to `activeRequest` until settlement.

*Regression test:* A component test (needs a vitest + @testing-library/svelte harness under sabrage/ui, which does not exist yet) that opens a gate for `setup` with a stalled `runStage` mock, clicks Hide, calls `stageStore.openGate({stage:"build"})`, and asserts the dialog title still reads Setup and that clicking Cancel calls `cancelStage` with setup's runId and does not start build until setup's promise settles. Absent a JS harness, assert the store-level invariant instead: `openGate` while a run is in flight is rejected/queued (unit test on stage.svelte.ts once `busy` lives there).

*Cross-area files:* sabrage/ui/package.json

### A12-4 [high, conf 1.0] Session enables Launch during four phases the shared contract defines as live
`sabrage/ui/src/screens/Session.svelte:107-113` — **CONFIRMED** (re-rated high)

The Launch card's `busy` predicate handles only `preflight`, `launching`, and `running`. It omits `stalled`, `stopping`, `detached`, and the newly added `external` phase, even though `ipc.ts:isLivePhase` defines all four as live and Library already uses that shared predicate. Launch and ⌘R therefore attempt a second launch during realistic live-session states; the backend should refuse safely, but the enabled control and modal report an action that cannot succeed.
Evidence: `sabrage/ui/src/screens/Session.svelte:107-113` enumerates only `running`, `launching`, and `preflight`, while `sabrage/ui/src/ipc.ts:424-447` defines every phase except `idle` and `exited` as live and mutation-blocking.
Inference: because both the Launch buttons and menu effect use this `busy` value, omitted phases reach `doLaunch(false)` rather than the existing in-progress notice.

*Recommendation:* Replace the handwritten phase list with `isLivePhase(status.phase)` and use the same predicate for buttons and menu requests. Test all `SessionPhase` values, especially `external`, `detached`, and `stalled`.

*Verifier:* Realistic and user-visible: `stalled` is the documented standby freeze, and `external` is any demo.sh run started outside the GUI (both front-ends are supposed to stay alive per CLAUDE.md). In the stalled/detached/stopping cases the enabled Launch button and the gate modal report an action that only ends in a Fatal; in the external case it is worse than a dishonest control — it takes the running game down. Not critical: nothing irreversible beyond the killed session, and it mirrors what running demo.sh twice does.

*Fix sketch:* In sabrage/ui/src/screens/Session.svelte replace the hand-written phase list with the shared predicate: `const busy = $derived(sessionStore.launching || isLivePhase(status.phase) || stageStore.gate !== null)` (import `isLivePhase` from ../ipc, as Library.svelte:87 does), leaving the ⌘R effect (line 172) reading the same value. Optionally give the menu-request notice a phase-specific message for `external` ('a session started outside Sabrage is running — Stop it first'). Belt-and-braces on the core side, since `external` is the case the UI cannot be the only guard for: have `stages::run::run` also consult `session::live_session_reason`/runtime_status before `wineserver_reset` and refuse recordless-but-live sessions the way `already_running` does.

*Regression test:* Table test over every `SessionPhase` asserting Launch/Dry-run are disabled for all of idle/exited's complement — belongs beside the Session screen once a vitest harness exists under sabrage/ui; today the cheapest equivalent is a unit test on the extracted predicate (assert `busy` uses `isLivePhase`, i.e. true for external/detached/stalled/stopping, false for idle/exited). If the core-side refusal is added, a sabrage-core test in stages/run: fresh runtime_status.json naming a live pid + no session-state record => `run` returns the already-running Fatal and never emits the RUN_WINESERVER step.

*Cross-area files:* sabrage/ui/package.json, sabrage/crates/sabrage-core/src/stages/run/mod.rs

### A12-5 [medium, conf 1.0] Run Doctor remains a navigation command and can show cached results
`sabrage/ui/src/App.svelte:59-66` — **CONFIRMED** (re-rated medium)

The menu item named Run Doctor still only navigates. If Doctor is already selected, nothing happens. If another screen is selected but the previous run settled within 60 seconds, remounting Doctor explicitly suppresses its automatic run. A user can therefore request fresh diagnostics after changing machine state and receive stale rows with no indication that the command was ignored.
Evidence: `sabrage/ui/src/App.svelte:61-62` handles `doctor` solely with `navigate("doctor")`; `sabrage/ui/src/screens/Doctor.svelte:38-42` skips `runChecks()` whenever the cached run is fresh.
Inference: assigning the already-current screen does not remount it, and a recent remount takes the documented cache path.

*Recommendation:* Give Run Doctor its own request token and acknowledgment, analogous to the corrected launch mechanism, and make an explicit menu request bypass `AUTORUN_STALE_MS`. Test both already-on-Doctor and recent-cache navigation paths.

*Verifier:* Realistic path: apply a remedy (e.g. re-run install after a CrossOver update), press Cmd-D expecting fresh diagnostics, get either literally nothing (already on Doctor) or the cached rows (returned within 60s) with no indication the command was ignored. Not rated high because the harm is bounded — stale advisory rows with the explicit "Run checks" button one click away, no mutation and no data loss; not rated low because a menu command that silently no-ops while presenting stale machine diagnostics can misdirect the fix/re-check loop.

*Fix sketch:* Mirror the launch fix. App.svelte: add `let doctorRequest = $state(0)`, change the menu arm to `if (id === "doctor") { navigate("doctor"); doctorRequest++; }`, and pass `{doctorRequest}` to `<Doctor />`. Doctor.svelte: accept `doctorRequest = 0` via `$props()`, keep the `AUTORUN_STALE_MS` path in `onMount` for plain re-navigation, and add an `$effect` (guarded by a `lastHandledRequest` local, and by `doctorStore.bottlesLoaded` so the bottle pick has resolved, exactly as Session.svelte:166-181 guards on `bottlesLoaded`) that on a bumped token calls `runChecks()` unconditionally — bypassing the freshness cache. When `doctorStore.running`, set a `doctorRequestNotice` ("Checks are already running.") rendered next to the summary instead of queueing a second pass, so the command is always acknowledged.

*Regression test:* sabrage/ui has no test runner today (package.json has only dev/build/check/tauri; no vitest, no @testing-library/svelte), so this needs a frontend harness first. With vitest + @testing-library/svelte + jsdom: sabrage/ui/src/screens/Doctor.test.ts mounts Doctor with a mocked ../stores/doctor.svelte whose `lastRunAtMs = Date.now()` and `hasRun = true`, asserts `run` was NOT called on mount (cache path preserved), then rerenders with `doctorRequest: 1` and asserts `run` WAS called exactly once; a second case asserts the notice appears instead of a second `run` when `running === true`. Plus sabrage/ui/src/App.test.ts: fire the `menu://doctor` listener registered through a mocked ./ipc `onMenu` while `screen === "doctor"` and assert the `doctorRequest` prop handed to Doctor incremented.

*Cross-area files:* sabrage/ui/package.json, sabrage/ui/package-lock.json, sabrage/ui/vite.config.ts

*Codex next steps:* Add a frontend contract test proving every core-deferred fix ID maps to no Doctor button. · Exercise one ⌘R launch followed by Session unmount/remount after exit; assert the invocation count remains one. · Start Setup, Hide, request Build, then press Cancel; verify the modal continues naming Setup and Build never starts unless explicitly retained as a visible queued request. · Table-test Session launch enablement across every `SessionPhase`, including external, detached, stalled, and stopping. · Invoke Run Doctor both while already on Doctor and within the 60-second cache window; verify a new check pass starts each time.

## A13a — store-rust

**Codex verdict:** needs-attention — Do not ship: A13a-1, A13a-2, and A13a-6 remain partially fixed. A13a-3, A13a-4, and A13a-5 are closed on the current production paths.

### A13a-1 [high, conf 0.99] [unfixed] A13a-1 — Non-current Goldberg builds are still presented and restored as the Steam original
`sabrage/crates/sabrage-core/src/store/library.rs:515-521` — **CONFIRMED** (re-rated medium)

The fix recognizes only the Goldberg DLL pinned by the currently running binary. Any older or third-party Goldberg build with no backup is still classified as `Original`; the next run backs those bytes up, and Revert later accepts them because they do not match the current pin. The backend success message is now neutral, but the UI still promises to restore the original DLL, leaving the user on Goldberg after an explicit Steam-original action.
Evidence: `sabrage/crates/sabrage-core/src/store/library.rs:515-521` computes `dll_is_goldberg` against one current pin and maps every other DLL without a backup through `(_, false, false) => GoldbergState::Original`; `sabrage/crates/sabrage-core/src/store/goldberg.rs:137-150` refuses only that same pinned hash and otherwise copies the backup; `sabrage/ui/src/screens/EditGame.svelte:366-376` still says “Restore the original” and “Revert original”. Inference: pin rotation or a different Goldberg distribution takes the false branch throughout.

*Recommendation:* Require positive Steam provenance before using `Original`: persist a provenance marker/hash when a trusted Steam DLL is captured, or classify every unrecognized no-backup DLL as `Unverified`. Revert and its UI must offer “restore backup” rather than “original” unless that provenance verifies.

*Verifier:* Reproduced from the code path and the existing test fixture; no guard makes the (_, false, false) arm unreachable. Severity lowered from high to medium: the backend RevertReport message is already provenance-neutral ('restored steam_api64.dll from the .orig-steam backup', goldberg.rs:152-159), and in the triggering scenario no real Steam dll exists on the machine, so nothing is destroyed — the defect is a false 'original' label plus a Revert verb that promises provenance the code cannot establish, and it needs the unusual missing-backup precondition.

*Fix sketch:* Add a `GoldbergState::Unverified` (or reuse `Modified`'s wording) for the `(present, not-pin, no-backup)` arm in `library::validate_pinned` and stop calling it `Original` unless positive Steam provenance exists; record provenance when it can be known — `stages::run::actions::goldberg_stage` already computes `already_goldberg` at backup time, so have it write a sibling marker (e.g. `steam_api64.dll.orig-steam.provenance`) recording whether the snapshotted bytes were Goldberg-like, and have `goldberg::revert_with_pin` consult that marker in addition to the pin comparison. Rename the UI verb to 'Restore the .orig-steam backup'.

*Regression test:* sabrage/crates/sabrage-core/src/store/library.rs `mod tests::goldberg_state_covers_all_five_variants` — add a fixture whose bytes are neither the pin nor real Steam and assert it is NOT `GoldbergState::Original`; sabrage/crates/sabrage-core/src/store/goldberg.rs `mod tests` — a case where `.orig-steam` holds a non-pin Goldberg build plus its provenance marker and `restored == false` with a message that never claims 'original'.

*Cross-area files:* sabrage/ui/src/screens/EditGame.svelte, sabrage/ui/src/screens/Library.svelte, sabrage/ui/src/ipc.ts, sabrage/crates/sabrage-core/src/stages/run/actions.rs

### A13a-2 [high, conf 0.99] [unfixed] A13a-2 — Revert still races a demo.sh run before runtime telemetry exists
`sabrage/crates/sabrage-core/src/store/goldberg.rs:102-114` — **CONFIRMED** (re-rated medium)

The operation guard closes native GUI/CLI races, but `demo.sh` deliberately does not participate. Its run can finish installing Goldberg and still be several steps away from spawning Wine; during that interval it has no Sabrage session record, live handle, or fresh runtime status. Revert therefore sees idle, copies the backup over the DLL, and the shell subsequently launches with those bytes despite having reported Goldberg installed.
Evidence: `sabrage/crates/sabrage-core/src/store/goldberg.rs:106-114` takes the native operation lock and relies exclusively on `live_session_reason` for the shell case. `sabrage/crates/sabrage-core/src/stages/mod.rs:463-465` states that `demo.sh` does not take that file lock. `scripts/demo/run.sh:150-152` installs Goldberg, while Wine is not spawned until line 267; `sabrage/crates/sabrage-core/src/session/watcher.rs:69-72` states that `runtime_status.json` is not written until streaming begins. That ordering leaves a concrete unguarded window.

*Recommendation:* Serialize the DLL itself across both surfaces, for example with a narrow per-install advisory lock or in-progress marker held by `run.sh` from before backup/install through Wine spawn and by Revert through validation/copy. Add a barrier test that pauses the shell after Goldberg installation and proves Revert cannot proceed.

*Verifier:* Path is unambiguous and reachable; not covered by the declared divergence. Severity lowered from high to medium: `RealExecutor::copy_if_changed` goes through `copy_atomic` (executor.rs:461, temp+rename), so a game that already mapped the dll keeps its inode and is not corrupted, and the next `run` reinstalls Goldberg unconditionally — the harm is a launch that starts with the Steam dll (Steam/Meta gate) and is fully recoverable, and it requires the user to trigger Revert in the GUI within the seconds-to-tens-of-seconds window of a shell launch they started themselves.

*Fix sketch:* Two layers: (a) inside Sabrage, add a machine-visible liveness signal to `session::session_block_at` that does not depend on Sabrage having written anything — e.g. a bottle-scoped process probe (`Beat Saber.exe` / that bottle's `wineserver`) — which closes everything after the wine spawn; (b) for the pre-spawn window, add a narrow in-progress marker under the game dir (or a per-install advisory lock) taken by both `scripts/demo/run.sh` and `stages::run::actions::goldberg_stage` from before the backup/install until the wine spawn, and required by `goldberg::revert_with_pin` before its copy. (b) is a shared-pipeline behaviour change: contract slug + both implementations in the same commit per CLAUDE.md.

*Regression test:* sabrage/crates/sabrage-core/src/store/goldberg.rs `mod tests` — a test that creates the in-progress marker (no Sabrage session record, no runtime_status.json) and asserts `revert_with_pin` returns the fatal 'a launch is in progress' error and leaves the dll bytes unchanged; plus a parity test in sabrage/crates/sabrage-parity that asserts run.sh emits/removes the same marker path.

*Cross-area files:* scripts/demo/run.sh, contract/pipeline.toml, sabrage/crates/sabrage-core/src/session/mod.rs, sabrage/crates/sabrage-core/src/stages/run/actions.rs, sabrage/PARITY.md

### A13a-3 [medium, conf 0.99] [unfixed] A13a-6 — Settings versioning is inert and nested future fields are still destroyed
`sabrage/crates/sabrage-core/src/store/settings.rs:148-160` — **CONFIRMED** (re-rated low)

The flattened map preserves only unknown top-level keys. `LaunchDefaults` remains a closed nested struct, and `load` never rejects `version > SETTINGS_VERSION`. A future file such as version 2 with `launch.futureFlag` loads successfully, drops that field, retains `version: 2`, and loses the newer data on the next autosave—the original downgrade failure in a narrower location.
Evidence: `sabrage/crates/sabrage-core/src/store/settings.rs:40-47` defines the closed four-field `LaunchDefaults`; lines 102-103 place the only `#[serde(flatten)]` map on the outer `Settings`; lines 148-160 deserialize and return every parseable version without comparing it to `SETTINGS_VERSION`.

*Recommendation:* Reject settings files whose version exceeds `SETTINGS_VERSION`, mirroring `library::load`, unless every extensible nested object also preserves unknown fields. Add a version-2 fixture containing an unknown nested launch key and assert either byte-unchanged refusal or lossless round-trip.

*Verifier:* Behaviour reproduced exactly as described, and it contradicts the module's own forward-compatibility claim (settings.rs:18-23). Severity lowered from medium to low: `SETTINGS_VERSION` is still 1 and no version-2 writer exists, so nothing can be lost today; the loss requires a future build to add a nested field and a user to then downgrade. It is a latent inconsistency with `library::load`, not a live defect.

*Fix sketch:* In `settings::load`, after deserializing, return a fatal `SabrageError` when `s.version > SETTINGS_VERSION`, worded like `library::load`'s ('update Sabrage (or move settings.json aside)'). Optionally also give `LaunchDefaults` its own `#[serde(flatten)] extra: Map<String, Value>` so nested additions round-trip; if that is done, the rejection can stay version-gated only for changes `extra` cannot absorb.

*Regression test:* sabrage/crates/sabrage-core/src/store/settings.rs `mod tests` — a `version: SETTINGS_VERSION + 1` fixture with an unknown nested `launch` key asserting `load` returns `Err` and the file's bytes are unchanged on disk; keep the existing top-level-`extra` round-trip test alongside it.

*Codex next steps:* Add an old/different Goldberg fixture and verify it is never classified or presented as the Steam original. · Pause `demo.sh run` after its Goldberg copy, invoke Revert concurrently, and verify the DLL cannot change before Wine spawns. · Add a future-version settings fixture with an unknown nested `launch` field and verify safe refusal or preservation. · After fixes, run the store tests, Tauri command tests, Svelte checks, and the full parity suite.

## A13b — ui-settings-library

**Codex verdict:** needs-attention — Do not ship: A13b-3, A13b-4, A13b-7, and A13b-8 remain partially unfixed. A13b-1/2/5/6/10 appear closed, and no artifact-byte parity regression was found.

### A13b-1 [high, conf 0.99] [unfixed] A13b-3: Library bypasses the failed-load quarantine
`sabrage/ui/src/screens/Library.svelte:87-99` — **CONFIRMED** (re-rated medium)

Library enables Run without requiring a successful settings load. If `get_settings` is slow or fails, the launch silently substitutes `false` for every non-overridden global flag. A corrupt or transiently unreadable settings file can therefore launch with the wrong audio, dashboard, wired, and verbose behavior instead of stopping at the advertised hard-error boundary.
Evidence: `sabrage/ui/src/screens/Library.svelte:26-29` starts `settingsStore.load()` without awaiting it; `:87-99` defines `busy` without `settingsStore.loadOk` and uses `global?.noAudio ?? false`, `global?.wired ?? false`, and equivalent fallbacks. No settings error is rendered on this screen.

*Recommendation:* Disable Run until `settingsStore.loadOk` is true, render the load error with a retry action, and reserve fallback defaults for the backend's successful missing-file result rather than load failures.

*Verifier:* Real quarantine bypass, but it is not the reviewer's 'high': it needs a corrupt/unreadable settings.json AND global launch defaults that differ from all-false, and the consequence is a launch with the wrong four run.sh flags (audio routed / dashboard opened / no --wired / no --verbose) plus a silently missing error banner — user-visible and wrong, no data loss or irreversible state. Per-entry `launchOverrides` still apply, and the fallback equals the fresh-install default, so nothing unsafe is invented.

*Fix sketch:* Library.svelte: add `|| !settingsStore.loadOk` to the `busy` $derived (:87-89) so both Run buttons disable until a load succeeded; render `settingsStore.error` in the screen body with a retry that calls `settingsStore.load()`; keep the `?? false` fallbacks in `effectiveLaunchOpts` only for the backend's successful missing-file default (they are then unreachable for the failure case). EditGame.svelte:94 has the same unawaited load but never launches, so it needs no gate.

*Regression test:* sabrage/ui has no test runner at all (package.json scripts = dev/build/check/tauri, devDeps have no vitest), so a component test needs the runner added first; the assertion: mount Library with a `getSettings` that rejects, assert every Run button is `disabled`, assert `launch` was never invoked, and assert the load error string is in the DOM.

*Cross-area files:* sabrage/ui/package.json, sabrage/ui/vite.config.ts

### A13b-2 [medium, conf 0.92] [unfixed] A13b-4: An older failed autosave can leave controls behind a newer successful save
`sabrage/ui/src/stores/settings.svelte.ts:74-104` — **CONFIRMED** (re-rated low)

With two rapid changes A and B, if A fails but B succeeds, B can be persisted while its visible control is reset to the old value. A's failure reloads the old disk state and its caller reseeds every local control; B then runs from the queue and succeeds, but success only flashes “Saved” and never reseeds the controls. The next Library launch reads B from the store while Settings displays not-B.
Evidence: `sabrage/ui/src/stores/settings.svelte.ts:80-83` performs `await load(); throw e` on failure while later updates remain queued at `:98-104`; `sabrage/ui/src/screens/Settings.svelte:98-105` calls `seedFromSettings()` only on rejection and merely `flashSaved()` on success. Inference: B is chained through the internal settlement promise, so A's awaiting caller handles the rejection before B's queued callback runs.

*Recommendation:* Return and apply the authoritative saved state for the latest update, or attach generation IDs so an older failure cannot reseed controls after a newer edit exists. Add a deferred-IPC component test where A rejects and queued B succeeds.

*Verifier:* The path is real and the ordering is deterministic, but it needs a *failed* saveSettings (a local atomic write to ~/Library/Application Support) followed by a *successful* one, with two edits overlapping one round-trip (e.g. bsDir blur + a checkbox click). That is an unusual condition; the damage is a stale Settings screen plus one unintended flag on disk, with the save error still banner-visible, and it clears on the next load/seed. Latent, not a realistic everyday path.

*Fix sketch:* settings.svelte.ts: have `performSave`/`update` resolve with the authoritative saved `Settings` (or expose a monotonically increasing generation id on the store) and have Settings.svelte's `persistSettings` reseed from the store on success as well as on rejection — or make `seedFromSettings()` on rejection a no-op when a newer write is still queued (compare a generation captured before `update` with the store's current one). Also build `persistLaunch`'s patch from the store's launch at execution time (inside the enqueued step) rather than from a click-time snapshot, so a failed A cannot ride along on B's write.

*Regression test:* Same missing-runner caveat as A13b-1. Test: deferred `saveSettings` mocks, reject update A, resolve queued update B, then assert disk bytes, `settingsStore.settings` and every visible control agree, and that A's value was not persisted by B.

*Cross-area files:* sabrage/ui/package.json, sabrage/ui/vite.config.ts

### A13b-3 [low, conf 0.99] [unfixed] A13b-7: Revert still authenticates an unverified backup as original
`sabrage/ui/src/screens/EditGame.svelte:364-376` — **CONFIRMED** (re-rated low)

The backend now cautiously describes restoring the `.orig-steam` backup, but the UI still tells the user it is restoring “the original.” Only the pinned Goldberg hash is rejected; another modified or third-party DLL can still be copied over the live DLL under this confirmation.
Evidence: `sabrage/ui/src/screens/EditGame.svelte:366` asks `Restore the original steam_api64.dll?`, and `:375-376` labels the action `Revert original steam_api64.dll`; the same file still labels `GoldbergState.original` as `original steam_api64.dll still present` at `:43-48`.

*Recommendation:* Rename the action and confirmation to “Restore the .orig-steam backup,” display `RevertReport.dllPath`, and classify non-Goldberg bytes without trusted verification as unverified rather than original.

*Verifier:* The pre-action strings do contradict a rule the core module states twice and enforces in its own message text, so the one screen where the user consents calls unverified bytes 'the original'. But nothing unsafe follows — the copy source is by construction the file that was in place before Goldberg, the destination is rewritten by the very next launch, and the post-action text is already honest. Wording/trust polish, not behaviour.

*Fix sketch:* EditGame.svelte: change :366 to 'Restore steam_api64.dll from the .orig-steam backup?' and :375-376 to 'Restore .orig-steam backup', and render `revertReport.dllPath` next to `revertReport.message`. Leave GOLDBERG_LABEL.original alone.

*Regression test:* Same missing-runner caveat; alternatively a grep-style assertion in the existing UI text checks (none exist today): no pre-action revert string may contain the word 'original' outside the GoldbergState.original label.

### A13b-4 [low, conf 0.97] [unfixed] A13b-8: Nested future launch settings are still stripped on downgrade
`sabrage/ui/src/stores/settings.svelte.ts:88-104` — **CONFIRMED** (re-rated low)

The new flattened map preserves only unknown top-level keys. A future key added inside `launch` is discarded while deserializing `LaunchDefaults`, and the next autosave serializes the known four-field object back, silently deleting that newer preference. This is the same downgrade-loss mechanism at one nesting level deeper.
Evidence: `sabrage/ui/src/stores/settings.svelte.ts:88-103` explicitly shallow-replaces the whole `LaunchDefaults`; `sabrage/crates/sabrage-core/src/store/settings.rs:40-47` defines only four launch fields with no flattened extras, while the preservation map at `:91-103` is explicitly top-level. This conclusion relies on Serde's default treatment of unknown struct fields.

*Recommendation:* Add flattened opaque extras to `LaunchDefaults` and mirror them in TypeScript, or reject writes from a binary older than `settings.version` when lossless preservation cannot be guaranteed.

*Verifier:* The mechanism is real and is the exact downgrade-loss the module's own header (settings.rs:19-24) says it set out to prevent — but it can only bite a Sabrage version that does not exist yet (no nested `launch` key has ever been written), and the loss is one preference, silently reset to a default, on a downgrade+toggle. Latent forward-compat gap.

*Fix sketch:* store/settings.rs: add `#[serde(flatten, skip_serializing_if = "Map::is_empty")] pub extra: Map<String, Value>` to `LaunchDefaults` (drops `Copy`/`Eq`, same trade `Settings` already made) and mirror it with an index signature on ipc.ts's `LaunchDefaults`; or make `load`/`save_settings` refuse to write a file whose `version` exceeds `SETTINGS_VERSION`, which is what the doc comment already promises the version field is for.

*Regression test:* sabrage/crates/sabrage-core/src/store/settings.rs `mod tests`, next to `unknown_fields_survive_a_load_save_round_trip`: write `{"launch":{"wired":true,"futureFlag":7}}`, load, flip `allow_adb_probes`, save, and assert the re-read file still contains `futureFlag` (or that the save was refused with a version error).

*Cross-area files:* sabrage/crates/sabrage-core/src/store/settings.rs, sabrage/ui/src/ipc.ts

*Codex next steps:* Add a Library component test proving Run stays disabled and `launch` is never invoked while settings are loading or failed. · Use deferred `saveSettings` promises to reject update A and resolve queued update B; assert disk, store, and every visible control agree afterward. · Round-trip a fixture containing `launch.futureFlag` through get/update/save and require preservation or an explicit version refusal. · Test modified and Goldberg backup fixtures and assert no pre-action UI string calls unverified bytes “original.”

## A14 — cli

**Codex verdict:** needs-attention — No-ship: A14-1, A14-4, and A14-5 are closed, but A14-3 is only fixed for CR output sent to a TTY. Redirected CR output and every EOF chunk are still rewritten with newlines.

### A14-1 [medium, conf 0.99] [unfixed] A14-3 Output terminators are still rewritten off-TTY and at EOF
`sabrage/crates/sabrage-cli/src/main.rs:728-795` — **REFUTED** (re-rated low)

Piped or redirected output—the normal path for `tee`, CI, and build logs—still converts every CR progress update into a permanent LF-terminated line. Independently, an unterminated EOF chunk gains an LF even on a TTY, changing how it composes with the next stage row. The regression test now explicitly accepts both conversions instead of asserting byte preservation.
Evidence: `sabrage/crates/sabrage-cli/src/main.rs:728-737` makes repaint conditional on `ChunkEnd::Cr` plus `*_tty` and maps every other case to plain `RenderedLine::Stdout/Stderr`; `main.rs:786-787` renders those variants with `println!("{s}")` and `eprintln!("{s}")`. Tests pin off-TTY CR to the plain variant at `main.rs:2087-2090` and EOF to it at `main.rs:2120-2123`.
Inference: because the plain variants discard `end`, the renderer cannot distinguish LF from EOF or an off-TTY CR, so all three become LF.

*Recommendation:* Carry `ChunkEnd` through the rendered value and write the exact delimiter (`\n`, `\r`, or nothing) on every destination; terminal status may control flushing or presentation, but not the emitted bytes. Replace the current assertions with exact-byte capture tests.

*Verifier:* Design disagreement, not a reachable defect. No artifact bytes, exit code, tap line, or machine state changes; the only observable delta is console/log cosmetics under redirection (e.g. `sabrage setup 2>&1 | tee log` shows curl's --progress-bar as N scrolled lines instead of one repainting line), and the chosen trade-off (readable log file vs. CR-soup single line) is documented at the decision site. The one real gap is documentation, not behaviour: this divergence has no row in sabrage/PARITY.md even though CLAUDE.md says intentional divergences belong there - a docs nit far below the reported medium severity.

*Codex next steps:* Capture stdout and stderr for `a\rb\nc` through both a PTY and a pipe; require identical terminators and no newline after `c`. · Repeat the capture with `--quiet` and verify that all child-output bytes are suppressed. · After correcting the renderer, run the CLI tests and parity harness, including direct-shell comparison for redirected curl progress.



## Fix outcome per confirmed item

Tally — high fixed: 9 · medium fixed: 28 · low deferred: 5 · low fixed: 25.

| id | sev | title | outcome | note |
|---|---|---|---|---|
| A1-1 | high | [unfixed] A1-1 Direct mutating stages bypass the compiled-contract identity guard | fixed | stages/mod.rs: `deny_on_contract_skew` (pub(crate)) delegates to `checks::meta::assert_binary_matches_checkout` — the shared predicate area A1 landed while I was working, so message+remedy come from checks::meta and cann |
| A11-3 | high | Queued forbidden operations are not rechecked after a session becomes live | fixed | Same change as A4-1 — one `deny_before_dispatch` / `deny_before_apply` helper per door so a third door cannot implement half the policy. Both doors keep the pre-lock fast refusal. |
| A12-3 | high | Hiding a gate lets Cancel target the previous stage while displaying the next | fixed | GateModal.svelte: split the displayed operation from the queued replacement. Renamed the identity-only `started` local into a reactive `activeRequest` ($state) and derived `displayRequest = activeRequest ?? request`; tit |
| A12-4 | high | Session enables Launch during four phases the shared contract defines as live | fixed | Core belt-and-braces half only (the file I own). `stages::run::run` now refuses a recordless-but-live session before any mutation via `launch_block` — see A8-1. The UI half (replace Session.svelte's hand-written `busy` p |
| A4-2 | high | [unfixed] A4-2 The frontend bypasses the deferred-fix registry | fixed | Made the deferred set authoritative on the Rust side of the wire AND mirrored it: commands.rs run_doctor now fills DoctorEvent.fix through new pure fn offered_fix_id() = FixAction::from_contract_id(...).map(to_contract_i |
| A6-1 | high | Cancellation during the registry poll is swallowed and can enter the privileged layer | fixed | wait_for_registry_flush now returns Result<bool> and yields Err(Cancelled) (via a new ensure_not_cancelled helper) instead of folding cancellation into the timeout's `false`; the caller uses `?`, so a Stop during the wai |
| A8-1 | high | [unfixed] A8-5 External-session detection is display-only | fixed | Added `launch_block(ctx)` in run/mod.rs, called BEFORE `phase.publish(SessionPhase::Preflight)` and before reconcile: it calls `session::session_block_at(Path::new(""), &ctx.paths.oxr_appsup.join("runtime_status.json"))` |
| A9-1 | high | Busy recovery records do not block launch | fixed | `run` now refuses every non-silent `Reconciled::Busy` before `checkpoint`/`preflight::run`: `if let Reconciled::Busy { state, reason, silent: false } = &reconciled { return Err(refuse_launch(ctx, "refusing to launch over |
| A9-7 | high | [unfixed] A9-7 Freshness history leaks across sessions | fixed | watcher.rs: added `fresh_run_id: Option<RunId>` next to `ever_fresh`/`last_fresh_unix_ms`; snapshot() clears `ever_fresh`, `last_fresh_unix_ms` and the cached `runtime_status` whenever the reported run changes (placed be |
| A10-2 | medium | Backup rotation mutates recovery history before the config commit | fixed | write() now executes the prune only AFTER the commit, and unlinks the just-reserved backup when the commit never happens, so an aborted save is a complete no-op and the CAS's 'nothing was written' is true. The stale list |
| A10-4 | medium | [unfixed] A10-3 The multiline refusal exists only in the view layer | fixed | Split round_trip_error into a parse step plus `line_document_mismatch(&doc, text)`, and called the latter from apply_patch right after the toml_edit parse — so write, edit_protocol, the CLI and the golden tests all inher |
| A10-5 | medium | A BOM on a root assignment makes Save unable to change the runtime value | fixed | line_document_mismatch now refuses BOTH directions of the physical/parsed disagreement (was `physical > parsed` only). A BOM sitting on a root key's own line gives parsed=1, physical=0 — toml_edit is handed the stripped  |
| A10-9 | medium | Multiple invalid occurrences collide in the Settings keyed list | fixed | runtime_toml dedupes on the whole InvalidValue and pins `(key, raw)` unique; Settings.svelte already keys the list on key+raw+reason. |
| A11-2 | medium | [unfixed] A11-3 The newly registered fix canceller has no reachable run ID | fixed | Core half done in fixes/mod.rs: `apply` now emits `StageEvent::info(ctx.run_id, Some(action.step_id()), "applying fix '<id>'")` BEFORE any wait (that row is the front-end's only source of the run id), emits the `waiting  |
| A12-1 | medium | [unfixed] A12-1 — the GUI ignores the deferred destructive-fix policy | fixed | Same change as A4-2 — this packet's server-side projection (run_doctor -> offered_fix_id) plus the ipc.ts DEFERRED_FIX_IDS mirror and the updated contractFixIdToAction doc comment; covered by the same regression test. |
| A12-5 | medium | Run Doctor remains a navigation command and can show cached results | fixed | Mirrored the launch-token mechanism. App.svelte: added `doctorRequest` counter, bumped (alongside `navigate("doctor")`) on the `menu://doctor` handler, passed to `<Doctor {doctorRequest} />`. Doctor.svelte: accepts `doct |
| A13a-1 | medium | [unfixed] A13a-1 — Non-current Goldberg builds are still presented and restored as the Steam original | fixed | "Is Goldberg" is now pin OR configured-payload bytes, per the lead directive. library.rs: validate/validate_with_bottle/validate_pinned thread `paths.gbe_dll` (validate already took `paths`, previously `let _ = paths`);  |
| A13a-2 | medium | [unfixed] A13a-2 — Revert still races a demo.sh run before runtime telemetry exists | fixed | The lead's directive (revert consults session::live_session_block via live_session_reason) was already in the tree from round 1; the remaining gap was the window that predicate structurally cannot see. Added a direct arg |
| A13b-1 | medium | [unfixed] A13b-3: Library bypasses the failed-load quarantine | fixed | Library.svelte: `busy` now also requires `settingsStore.loadOk`, disabling both Run buttons until a settings load has actually succeeded. Added a banner (`settingsStore.loaded && !settingsStore.loadOk`) above the table r |
| A2-2 | medium | [unfixed] A2-5 Escalation still infers process-group death from pipe closure | fixed | process.rs cancelled arm now measures the TREE, not the leader or the pipes. After SIGTERM it reaps the leader with timeout_at(deadline=now+kill_grace), then polls new `group_alive(pgid)` (`killpg(pgid, None)`, i.e. kill |
| A3a-1 | medium | [unfixed] A1-1 Contract skew still permits setup/build/install mutations | fixed | This item's actual enforcement point (stages::run_stage / run_stage_holding_lock in stages/mod.rs, owned by A4) is outside my files, so per lead_notes I did the A3a-owned half: extracted the compiled-vs-checkout comparis |
| A3b-1 | medium | [unfixed] A3b-1 | fixed | My side landed: new `pub fn effective_accepted(text, key) -> Option<String>` in config/runtime_toml.rs (re-exported from config/mod.rs), a key-aware last-VALID reader that folds raw_assignments through the same Setting:: |
| A3b-2 | medium | Known-bad session deletion remains the official troubleshooting fix | fixed (lead) | docs/troubleshooting.md and CLAUDE.md no longer advise deleting `session.json`; the doctor remedy strings already said "edit in place". |
| A4-1 | medium | [unfixed] A4-1 Queued mutations are not rechecked after a session starts | fixed | stages/mod.rs: new `deny_before_dispatch` (live-session + contract skew) is now run a SECOND time in `run_stage` after `acquire_operation_lock_cancellable` returns the guard and before the `Stage::Run` branch/`dispatch`; |
| A4-4 | medium | [unfixed] A4-5 The GUI's mandatory recheck turns an ADB query failure green | fixed | core pins one warn row + an unchanged FixReport for an unspawnable adb (fixes/adb.rs); Doctor.svelte keeps that warning visible across the automatic recheck instead of repainting the row green. |
| A5-1 | medium | [unfixed] A5-1 Setup dry-run still reports unperformed writes as completed | fixed | Two half-fixes, one rule (the row must claim the WHOLE postcondition). (a) setup_pinned: the extraction Ok row also claims the provenance marker, and `util::dxmt_ok` (files AND current marker) already short-circuits abov |
| A5-5 | medium | Hung stop probes ignore cancellation and can block teardown indefinitely | fixed | Both probes now go through `process::capture_with(&spec, &ctx.cancel, deadline)` (own process group, SIGKILL on cancel/deadline) instead of a bare `tokio::process::Command::output().await`. `stale_listeners` gained `(ctx |
| A6-3 | medium | [unfixed] A6-3 Non-empty partial backups remain trusted, and the recovery advice can capture the fork | fixed | Layer 1 no longer copies into the trusted name at all: `cp -R` goes to a sibling `dxmt.stock-backup.partial-<uuid>` (new PARTIAL_BACKUP_PREFIX) and is committed with `/bin/mv` (rename(2), atomic within one directory) onl |
| A7-1 | medium | [unfixed] A7-2 Launch still does not implement last-valid runtime semantics | fixed | Same core helper as A3b-1 — `effective_accepted` — which is the whole of my side of both packets. Nothing in this packet needs a second runtime_toml change; the remaining work (deriving TomlFacts from it, and building th |
| A7-4 | medium | Wired preflight can hang forever before reaching the cancellable probe | fixed | Two layers. (1) checks/run_only.rs: adb_devices_output is no longer an unbounded Command::output() — it spawns with piped stdout, polls try_wait against a deadline (new ADB_PROBE_TIMEOUT = process::DEFAULT_PROBE_TIMEOUT, |
| A7-5 | medium | A non-file `.orig-steam` path causes the real DLL to be overwritten without a backup | fixed | goldberg_stage's backup decision is now three-way on the reserved name: `backup.is_file()` (incl. a symlink resolving to a regular file) -> skip as before; nothing there -> mint as before; anything else (directory, fifo, |
| A8-2 | medium | [unfixed] A8-3 Cancellation restoration bypasses the guard state machine | fixed | Split `AudioGuard::acquire` into `arm()` (eligibility, both read-only probes, carry-forward decision, pre-mutation `state::save`, sets `previous_output`/new `carried` field) and `apply_switch(&mut self, ctx, state)` (the |
| A8-4 | medium | Rotation reports previous-file backlog as the beginning of the new file | fixed | Rotation is now announced on the batch that actually carries the new file's bytes. Added `Tailer::carry` (count of leading `pending` entries from the previous incarnation) and `Tailer::pending_rotation`. The rotation-wit |
| A9-3 | medium | [unfixed] A9-3 Record locking still fails open without CAS | fixed | My side only, and implemented locally because `state::clear_run` does not exist in the current tree (A9's side had not landed). `clear_state(ctx, path, expected: RunId)` now reloads the record and returns Ok(()) without  |
| A9-5 | medium | Unverifiable live sessions are rendered as Exited | fixed | reconcile.rs: `classify` split into `classify_identity(Option<&ProcInfo>)` + `Classification::is_live()` (Live/Unverifiable). watcher.rs snapshot now derives both the live-handle branch (`classify_identity(Some(&handle.i |
| A9-6 | medium | [unfixed] A9-6 Adopted sessions still inherit historical encoder lines | fixed | watcher.rs: added `parse_log_timestamp` (oxrsys' spdlog `[%Y-%m-%d %H:%M:%S.%e]` prefix, parsed as LOCAL time via the existing chrono dep); the tail now carries `(EncoderInfo, Option<u64>)` and a preloaded line is believ |
| A9-9 | medium | [unfixed] A9-9 Detach's timeout safety write can race a winning Stop | fixed | reconcile.rs `detach`: the wait now reports how it ended (`cleared`), and the safety write happens only when the slot provably cleared AND `handle.cancel` is still unfired after the wait; the write goes through the new ` |
| A1-2 | low | [unfixed] A1-2 The --bs-dir setup fallback still embeds the depot pin | fixed (lead) | scripts/demo/setup.sh now expands `$BS_APPID/$BS_DEPOT/$BS_MANIFEST` from contract.gen.sh (byte-identical output); fingerprint re-blessed. |
| A1-3 | low | [unfixed] A1-3 CI still skips native unit tests that the parity suite says are required | deferred | CI (ubuntu) keeps the two hermetic crates; `-p sabrage-core` runs in the local tier-1 only (decided in round 1: the core suite spawns macOS-only fakes). |
| A1-4 | low | [unfixed] A1-4 contract-sync still authenticates the header rather than the sourced body | deferred (residue) | Authenticating contract.gen.sh's BODY needs a second generated header line (`# body-sha256:`) → changes the generated file's bytes on both sides; the tier-1 `generate()==include_str!` test already catches body drift in CI and in `parity.sh`. Left for a dedicated change. |
| A1-5 | low | [unfixed] A1-6 Mixed tag groups can still swap warn and block behavior undetected | fixed | run_sh_tags: the verb check no longer treats a mixed-gate tag group as one bag of verbs. Added VERB_ANCHORS (slug -> the pinned run.sh message its branch emits) and per-slug gates on TagGroup (slug_gates — `gates` is per |
| A1-6 | low | [unfixed] A1-7 Loop coverage still credits unrelated emissions | fixed | slug_coverage::loop_body_emits no longer ANDs two independent booleans. It now tracks the shell text that provably carries the loop item's slug — `${_x%%:*}` plus any variable it was assigned to (new assigned_variable()  |
| A1-7 | low | [unfixed] A1-8 Control characters still produce an invalid privileged host manifest | fixed (adjudicated) | A6 landed the fail-closed refusal (`privilege::reject_unrepresentable_manifest_path`, before the currency test and any prompt). A1's widening of `util::json_escape_string` to escape C0 was REVERTED by the lead: the escaper stays exactly install.sh's two substitutions (pinned by test) so every accepted path renders byte-identically on both sides; declared in PARITY.md. install.sh also gained `print -r --` (a real byte divergence for a backslash in the checkout path). |
| A1-8 | low | [new] Generated TOML strings are evaluated as zsh code | fixed | sabrage-contract-gen now encodes every contract value as a zsh literal: new zsh_scalar() (assignment RHS) and zsh_word() (array element / bare BS_MANIFEST), both minimal — a value that is already literal in the historica |
| A10-1 | low | [unfixed] A10-1 The final “CAS” is still a check-then-overwrite | deferred (residue) | A2 landed `Executor::hard_link` (link(2), never replaces; DryRun records `PlannedKind::Link`); switching runtime_toml's final publish from check-then-rename to link-then-rename is left for a follow-up (write() already holds the cross-process `lock_toml`, so the window needs a non-Sabrage writer). |
| A10-6 | low | [unfixed] A10-5 Mixed line endings are still normalized outside the edited value | deferred (residue) | Preserving mixed line endings outside the edited value needs per-line terminator capture in `ByteShape`; the file is normalized to one convention on save today (documented). |
| A10-7 | low | The std::stof emulation uses different precision from the runtime | fixed | in_scale_range now narrows to f32 before comparing, matching ext/oxrsys/runtime/src/Config.cpp:367-372 (`float val = std::stof(value); if (val >= 0.25f && val <= 1.0f)`), so `1.00000001`/`0.2499999999` are accepted as th |
| A10-8 | low | The live-session guard blocks status files the UI deliberately treats as idle | fixed | Extracted `watcher::runtime_status_live(rs, now) = is_fresh(..) && process_id.is_some_and(process::is_alive)` and call it from BOTH `session_block_at`'s status signal and SessionMonitor's External derivation — the pidles |
| A11-1 | low | [unfixed] A11-1 Stop-and-quit still approves exit without stopping or detaching | fixed | My side only: `reconcile::detach`'s early `if handle.cancel.is_cancelled()` return now names `commands::resolve_quit`'s stop-then-timeout arm as the caller it silently absorbs, and the function doc spells out the three h |
| A13a-3 | low | [unfixed] A13a-6 — Settings versioning is inert and nested future fields are still destroyed | fixed | settings::load now returns a Fatal when `version > SETTINGS_VERSION`, worded like library::load ("is version N — this Sabrage understands version M and would silently drop everything the newer one wrote", remedy "update  |
| A13b-2 | low | [unfixed] A13b-4: An older failed autosave can leave controls behind a newer successful save | fixed | settings.svelte.ts: added a `writeSeq` counter bumped synchronously inside `enqueue` (before any await), exposed as a getter; widened `update()` to accept `Partial<Settings> / ((current: Settings) => Partial<Settings>)`, |
| A13b-3 | low | [unfixed] A13b-7: Revert still authenticates an unverified backup as original | fixed | EditGame.svelte: confirm text changed to 'Restore steam_api64.dll from the .orig-steam backup?', button label changed from 'Revert original steam_api64.dll' to 'Restore .orig-steam backup', and the revert report now also |
| A13b-4 | low | [unfixed] A13b-8: Nested future launch settings are still stripped on downgrade | fixed | Packet (A13b-owned finding), Rust side: LaunchDefaults gained `#[serde(flatten, skip_serializing_if = "Map::is_empty")] pub extra: Map<String, Value>` and lost `Copy`/`Eq` (same trade Settings already made). Only two str |
| A2-1 | low | [unfixed] A2-3 Reconciliation still uses an uncancellable probe path | fixed | reconcile.rs: `probe_capture(spec, cancel)` now calls `process::capture_with(spec, cancel, PROBE_TIMEOUT)` (the outer `tokio::time::timeout` is gone, so the process-group kill on expiry actually runs), and `current_outpu |
| A2-3 | low | [unfixed] A2-7 Atomic writes still report success without durable publication | fixed | executor.rs `write_atomic_real`: chmod moved BEFORE `file.sync_all()` (fsync flushes inode metadata, so the published mode is now durable too), and the parent-directory fsync is no longer best-effort — extracted as `sync |
| A2-4 | low | The new create_new primitive can strand a partial write-once configuration | fixed | executor.rs `create_new_real` rewritten as stage-then-publish: bytes are written, chmodded 0644 and fsynced on a unique `sibling_tmp`, then the final name is claimed with `tokio::fs::hard_link(tmp, path)` (link(2) refuse |
| A3a-2 | low | [unfixed] A1-4 Generated-body drift is acknowledged but still reported in sync | deferred (residue) | Same mechanism as A1-4; the doctor Pass message keeps saying "in sync" (both sides) until the body hash lands. |
| A3b-3 | low | A3b-3 hides the real error behind an impossible Python diagnosis | fixed | Fixed the accuracy bug within owned scope: SessionPinState::Unreadable and ::Malformed in checks/config.rs no longer claim '(broken python3?)' — that phrasing is now reserved for the ::Corrupt arm, which genuinely mirror |
| A5-2 | low | [unfixed] A5-7 A local helper match suppresses cross-checkout detection | fixed | `report_foreign_helpers` is now called unconditionally after the helper reap and takes `local_matched: bool`, which gates only the shell's NO_LEFTOVER_HELPER row (printed only when neither the local reap nor the foreign  |
| A5-3 | low | Setup's write-once config creation can overwrite a concurrent writer | fixed | `setup_config`'s creation branch now uses `exec.create_new` (O_EXCL, already documented in executor.rs as the primitive for the write-once documents, `oxrsys-runtime.toml` named explicitly) instead of `write_atomic`. On  |
| A6-2 | low | [unfixed] A6-4 Registry polling waits for a token that does not satisfy the launch gate | fixed | Small and local, so done despite `low`. wait_for_registry_flush now WAITS on registry_current (the three-literal launch-gate predicate `bottle.registry` blocks on) and only WARNS on install.sh's looser `grep -q ActiveRun |
| A7-2 | low | [unfixed] A7-4 Failed rollback erases the recovery record it still needs | fixed | rollback_forwards no longer discards the removal results: it zips contract().ports.stream with the specs, records which `forward --remove` calls actually succeeded, and retains in sess.wired_forwards exactly the entries  |
| A7-3 | low | [unfixed] A7-5 Unpinned Goldberg payloads can still be reported as restored | fixed | Packet: revert now refuses on a backup that byte-matches the configured payload (`util::cmp_files(&paths.gbe_dll, &backup)`), not only the contract pin — the sketch's "equivalent alternative", chosen over the provenance  |
| A8-3 | low | [unfixed] A8-7 The continuity signature still has a truncate/read race | fixed | Fixed rather than deferred (it stayed local: ~45 lines incl. the test seam, no public signature change). `poll` now snapshots `offset_before`/`signature_before`/`pending_before` before the read loop and, when `consumed > |
| A9-2 | low | [unfixed] A9-2 Retained forward recovery is still overwritten | fixed | Replaced the `Option<String>` carry with a `Carried { audio, forwards }` struct and `carry_forward(state)`: a Dead/IdentityMismatch record now also hands over `wired_forwards` (unless `guards.forwards_cleared`), which `r |
| A9-4 | low | Forward-removal progress is not crash-resumable | fixed | reconcile.rs `restore_forwards`: each successful `adb forward --remove` now drops its entry from `state.wired_forwards` and `state::save`s immediately, so a crash between two removals leaves a record naming only what is  |
| A9-8 | low | [unfixed] A9-8 Newer schemas are protected only inside reconcile | fixed | My side: launch now aborts on the corresponding `Reconciled::Busy` (newer-schema records take the same refusal path as foreign-owned ones — see A9-1), so the error surfaces before any mutation instead of mid-launch. The  |

### Residue (documented, not fixed in this loop)

- **Generated-body authentication** (A1-4 / A3a-2): `meta.contract-sync` still authenticates only `contract.gen.sh`'s header; body drift is caught by tier-1 (`generate() == include_str!`) locally and in CI, not by doctor. Needs a `# body-sha256:` header emitted by the generator (byte change on both sides).
- **CAS on the final config publish** (A10-1): `Executor::hard_link` exists; `runtime_toml::write` still check-then-renames under the cross-process lock.
- **Mixed line endings** (A10-6): a saved `oxrsys-runtime.toml` is normalized to one terminator convention.
- **CI package list** (A1-3): ubuntu tier-1 runs `sabrage-parity` + `sabrage-contract-gen` only; `sabrage-core` runs in the local tier-1.
- **Bare-CR chunk delay** (round-1 diff review, skipped with cause): a bare `\r` progress repaint is emitted only when the next byte arrives or at EOF.
- **Off-TTY CR/EOF rendering** (A14-1, refuted as a defect): the CLI renders piped/redirected child output as LF-terminated lines by design; declared console-presentation behaviour, not artifact bytes.
- **No JS test harness**: every UI fix is verified by `svelte-check`/build and by reading; the verifiers' suggested component tests need a vitest harness that this branch deliberately does not add.
