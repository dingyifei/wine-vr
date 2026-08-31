# Sabrage adversarial review — round 1

Codex `gpt-5.6-sol` @ xhigh, read-only, one session per area; every finding adversarially verified by an opus refuter (CONFIRMED / REFUTED / UNVERIFIABLE).

## Summary

| Area | Codex verdict | findings | confirmed | refuted | unverifiable | unverified | tokens |
|---|---|---|---|---|---|---|---|
| A1 contract-parity-spine | needs-attention | 9 | 8 | 1 | 0 | 0 | 208,194 |
| A2 core-primitives | needs-attention | 9 | 6 | 3 | 0 | 0 | 243,761 |
| A3a checks-doctor-core | needs-attention | 4 | 0 | 4 | 0 | 0 | 244,654 |
| A3b checks-doctor-config-net-game | needs-attention | 3 | 2 | 1 | 0 | 0 | 185,613 |
| A4 lock-and-fixes | needs-attention | 10 | 8 | 2 | 0 | 0 | 385,826 |
| A5 stages-setup-build-stop | needs-attention | 7 | 6 | 1 | 0 | 0 | 310,502 |
| A6 install-privilege | needs-attention | 5 | 5 | 0 | 0 | 0 | 153,884 |
| A7 run-preflight-actions | needs-attention | 6 | 5 | 1 | 0 | 0 | 267,068 |
| A8 run-supervise-guards-logs | needs-attention | 7 | 6 | 1 | 0 | 0 | 243,195 |
| A9 session-reconcile-telemetry | needs-attention | 9 | 9 | 0 | 0 | 0 | 267,365 |
| A10 config-runtime-toml | needs-attention | 5 | 5 | 0 | 0 | 0 | 165,370 |
| A11 ipc-boundary | needs-attention | 5 | 5 | 0 | 0 | 0 | 424,288 |
| A12 ui-shell-session | needs-attention | 7 | 7 | 0 | 0 | 0 | 181,725 |
| A13a store-rust | needs-attention | 6 | 6 | 0 | 0 | 0 | 181,138 |
| A13b ui-settings-library | needs-attention | 10 | 9 | 1 | 0 | 0 | 391,502 |
| A14 cli | needs-attention | 6 | 4 | 2 | 0 | 0 | 191,981 |

Totals — confirmed 91, refuted 17, unverifiable 0, unverified 0, areas not reviewed 0.

## Findings by area

## A1 — contract-parity-spine

**Codex verdict:** needs-attention — No-ship: production contract identity can split across checkouts, the generator silently drops source-of-truth fields, and tier-1 has several false-green paths around native behavior and byte artifacts.

### A1-1 [high, conf 0.99] A binary can validate checkout Y while executing checkout X's contract
`sabrage/crates/sabrage-core/src/contract.rs:38-52` — **CONFIRMED** (re-rated high)

An installed binary built from checkout X can be pointed at supported checkout Y, pass `meta.contract-sync`, and still use X's registry, pins, ports, and templates. Tier 2 rebuilds the CLI from Y, so it never exercises this installed-binary skew.
Evidence: `sabrage/crates/sabrage-core/src/contract.rs:38-52` compiles inputs with `include_str!("../../../../contract/...")`, while `sabrage/crates/sabrage-core/src/util/mod.rs:318-329` opens contract files under runtime `repo_root`. `sabrage/crates/sabrage-core/src/checks/meta.rs:34-40` compares only those runtime files to Y's recorded header. Persisted GUI `repo_root` is explicitly supported at `sabrage/crates/sabrage-core/src/paths.rs:98-100`, while `scripts/dev/parity.sh:237-249` always rebuilds and runs Y's debug CLI. Inference: neither the meta check nor tier 2 compares Y's contract hash to the binary's compiled contract.

*Recommendation:* Add a distinct binary-contract identity check comparing an embedded compiled hash with the runtime checkout hash, and block mutating stages on mismatch; alternatively parse and use the runtime contract consistently. Add an X-binary/Y-checkout fixture.

*Verifier:* A supported configuration (installed Sabrage binary + user-pointed repo_root) yields a green 'contract in sync' row while setup/install/run execute the binary's baked-in pins, URLs, ports, host_xr_json, templates and check registry. The check's own message asserts synchronization that has not been verified, and the mutating stages write machine state (host XR manifest, oxrsys-runtime.toml, downloaded pins) from the unverified side. Not critical: the writes stay inside the normal pipeline's artifact set and are repairable by re-running install from a matching checkout.

*Fix sketch:* Expose a compile-time identity in sabrage-core/src/contract.rs (e.g. `pub static COMPILED_CONTRACT_SHA256: LazyLock<String>` computed by the same recipe as util::contract_hash over PIPELINE_TOML/RUNTIME_TOML_TEMPLATE/HOST_MANIFEST_TEMPLATE, reusing a shared `contract_sha256_from` helper factored out of sabrage-contract-gen::contract_sha256_from). Add a new contract slug (e.g. `meta.binary-contract`) to contract/pipeline.toml with gate=block on the native side / none on the shell side (shell has no compiled contract — declare that asymmetry in sabrage/PARITY.md), bind an evaluator in checks/meta.rs comparing COMPILED_CONTRACT_SHA256 against util::contract_hash(root), and make the native run/install/setup preflight treat the mismatch as SabrageError::Fatal. Surface it in commands.rs get_repo_info as a third RepoInfo flag so the GUI's repo-root panel shows it. Alternative (larger): stop using the compiled contract at runtime and parse repo_root's pipeline.toml, keeping include_str! only as the build-time tripwire.

*Regression test:* sabrage/crates/sabrage-parity/src/lib.rs (or a sabrage-core integration test): build a scratch checkout from the real contract/ with one mutated scalar plus a self-consistent regenerated contract.gen.sh, assert `meta.contract-sync` still Passes there but the new `meta.binary-contract` Fails, and assert both Pass against the live repo_root; plus a preflight test asserting the mismatch aborts a mutating stage with Fatal.

*Cross-area files:* contract/pipeline.toml, scripts/demo/contract.gen.sh, scripts/demo/doctor.sh, sabrage/PARITY.md, sabrage/src-tauri/src/commands.rs, sabrage/crates/sabrage-core/src/stages/run/preflight.rs

### A1-2 [high, conf 1.0] The generator omits contract fields that the shell hard-codes
`sabrage/crates/sabrage-contract-gen/src/lib.rs:62-101` — **CONFIRMED** (re-rated medium)

Routine asset or install-leaf updates can make native setup and the shell setup use different URLs or default directories even after `--regen` and `--check` pass.
Evidence: `sabrage/crates/sabrage-contract-gen/src/lib.rs:72-76` omits `gbe_dll_asset` and `dxmt_tgz_asset`, and `:83-84` parses `bs_dir_leaf` only as dead code; none is emitted by `:179-219`. The shell hard-codes the leaf at `scripts/demo/lib.sh:99` and asset filenames at `scripts/demo/setup.sh:26,32`, while native setup consumes contract assets at `sabrage/crates/sabrage-core/src/stages/setup.rs:155-176`. Inference: changing one of these TOML fields and regenerating changes the header but leaves the shell literals unchanged, after which full-file `--check` reports synchronization.

*Recommendation:* Emit `GBE_DLL_ASSET`, `DXMT_TGZ_ASSET`, and `BS_DIR_LEAF`, consume them from the shell, and add mutation tests proving every shell-consumed contract field changes generated output.

*Verifier:* Real but latent: it needs someone to edit gbe_dll_asset / dxmt_tgz_asset / bs_dir_leaf. The asset cases then fail loudly on the shell side (404 or sha256 mismatch, since the sha pins ARE emitted and would change together with a rename); the bs_dir_leaf case diverges silently — native and shell would default to different Beat Saber directories with every parity gate green. Not high, because no current value is wrong and the loud half self-reports.

*Fix sketch:* In sabrage-contract-gen/src/lib.rs: add gbe_dll_asset/dxmt_tgz_asset to the Deps struct, drop the #[allow(dead_code)] on Game::bs_dir_leaf, and emit `GBE_DLL_ASSET="…"`, `DXMT_TGZ_ASSET="…"`, `BS_DIR_LEAF="…"` in the generated file; then consume them in scripts/demo/setup.sh (both fetch_pinned URLs and the tarball path) and scripts/demo/lib.sh:99 (`.../common/$BS_DIR_LEAF`), regenerate via scripts/dev/parity.sh --regen and re-bless sabrage/parity/shell.fingerprint.

*Regression test:* sabrage/crates/sabrage-contract-gen/src/lib.rs tests: a mutation test that, for every contract field the shell consumes, rewrites that scalar in a copy of contract/pipeline.toml and asserts generate_from() output differs in a body line, not only in the `# contract-sha256:` header. Plus a sabrage-parity test asserting scripts/demo/lib.sh and setup.sh contain no literal copies of the three values (grep for 'Beat Saber 1294', 'gbe-steam_api64', 'dxmt-artifacts-monofunc').

*Cross-area files:* scripts/demo/lib.sh, scripts/demo/setup.sh, scripts/demo/contract.gen.sh, sabrage/parity/shell.fingerprint

### A1-3 [high, conf 1.0] Tier 1 relies on sabrage-core unit tests that it never runs
`sabrage/crates/sabrage-parity/src/lib.rs:1001-1010` — **CONFIRMED** (re-rated medium)

Native-only changes to launch text and other byte invariants can remain green because the parity test checks only the shell copy and delegates the native half to unselected dependency tests.
Evidence: `sabrage/crates/sabrage-parity/src/lib.rs:1001-1010` says sabrage-core's frozen-text tests pin the native side and explicitly states this module "never calls native code at all." However `scripts/dev/parity.sh:73-76` and `.github/workflows/parity.yml:27-28` run only `sabrage-parity` and `sabrage-contract-gen`; Cargo does not run unit tests of the `sabrage-core` dev-dependency. Thus a native banner change can pass the advertised tier-1 gate while violating the six-line banner invariant.

*Recommendation:* Either add `sabrage-core` to every tier-1 invocation or make the parity crate call exported native renderers directly. Do not rely on dependency unit tests unless the package is explicitly selected.

*Verifier:* The advertised tier-1/pre-push/CI gate does not run the tests it delegates the native half of run-text parity to, so a native-only edit to a banner/die/warn literal passes every automated gate while diverging from run.sh. Impact is divergent user-facing text between the two front-ends rather than broken behaviour, and a plain `cargo test` in sabrage/ would catch it, so medium rather than high.

*Fix sketch:* Add `-p sabrage-core` to the tier-1 invocations in scripts/dev/parity.sh (run_tier1, both branches) and to .github/workflows/parity.yml's step, keeping the say/step names in sync; if sabrage-core's suite is too slow or machine-touching for CI, instead promote the frozen-text renderers to `pub` (or a `pub fn banner_lines()`-style accessor) and assert them directly from sabrage-parity, and delete the doc claim at sabrage-parity/src/lib.rs:1001-1010 that dependency unit tests pin the native side.

*Regression test:* sabrage/crates/sabrage-parity/src/lib.rs: a test that reads scripts/dev/parity.sh and .github/workflows/parity.yml from the checkout and asserts every tier-1 `cargo test` invocation selects sabrage-core (so dropping the package again turns tier-1 red) — mirroring the existing shell-fingerprint tripwire style.

*Cross-area files:* scripts/dev/parity.sh, .github/workflows/parity.yml

### A1-4 [medium, conf 1.0] meta.contract-sync trusts the generated file's self-reported header
`sabrage/crates/sabrage-core/src/util/mod.rs:332-339` — **CONFIRMED** (re-rated low)

Editing `contract.gen.sh` body scalars while leaving its header intact produces a false Pass, even though the sourced shell contract is now divergent. Wrong `WIRED_PORTS` could consequently alter forward cleanup while the GUI claims synchronization.
Evidence: `sabrage/crates/sabrage-core/src/util/mod.rs:335-339` merely extracts the first `# contract-sha256:` value; `sabrage/crates/sabrage-core/src/checks/meta.rs:34-40` compares that value to the contract-input hash. The body is executed through `source "$ROOT/scripts/demo/contract.gen.sh"` at `scripts/demo/lib.sh:9`. No generated-body bytes participate in the runtime verdict.

*Recommendation:* Verify the complete generated body against output derived from the on-disk contract, or record and validate a separate generated-body digest. Rename the check if it intentionally verifies only header freshness.

*Verifier:* The behaviour is real and the check name over-promises, but the residual exposure is narrow: it needs a hand-edited or half-merged 'GENERATED — DO NOT EDIT' file in a checkout that is never run through tier-1, and the tier-1 golden already covers the developer path. Latent and minor, not medium.

*Fix sketch:* Either (a) rename/redocument: change the check's own doc comment (checks/meta.rs:19-31) and the contract slug description to say it verifies header freshness only, or (b) strengthen on both sides in one commit — record a second `# generated-body-sha256:` line in the generator (sabrage-contract-gen/src/lib.rs:179-219, hashed over the body below the header) and have util::contract_gen_recorded_hash return both, with checks/meta.rs comparing the body digest too and scripts/demo/doctor.sh section 0 gaining the identical two-hash comparison plus a contract regen. Option (b) is the only one that closes the hole and it is a both-sides change.

*Regression test:* sabrage/crates/sabrage-core/src/checks/meta.rs tests: a scratch-root case that copies the real contract/ and a contract.gen.sh whose header is intact but whose body has one mutated scalar, asserting Fail with the existing out-of-sync message/remedy; paired with a sabrage-contract-gen test that the generator's recorded body digest matches a fresh recompute.

*Cross-area files:* scripts/demo/doctor.sh, scripts/demo/contract.gen.sh, contract/pipeline.toml, sabrage/parity/shell.fingerprint

### A1-5 [medium, conf 1.0] The native --tap path truncates despite the parity API requiring append
`sabrage/crates/sabrage-core/src/tap.rs:59-74` — **REFUTED** (re-rated low)

An existing tap file loses all previous rows under `sabrage doctor --tap`, while the zsh channel appends. The tier-2 harness hides the discrepancy by pre-truncating both scratch files.
Evidence: `sabrage/crates/sabrage-core/src/tap.rs:59-73` documents and implements zsh-compatible append via `OpenOptions::append(true)`, matching `scripts/demo/lib.sh:69-71` and its `>>`. Nevertheless `sabrage/crates/sabrage-cli/src/main.rs:292-296` deliberately calls `std::fs::write`, which truncates, and `scripts/dev/parity.sh:227-231` empties both files before exercising them. No corresponding divergence is declared in `sabrage/PARITY.md`.

*Recommendation:* Route the CLI through `append_tap`. If per-run snapshot semantics are intended instead, declare the divergence and use a separate option/API so a parity-channel path is not destructively overwritten.

*Verifier:* The reviewer reads tap.rs's append_tap doc as an API contract the CLI must honour; it is a renderer primitive, and the only consumer of the tap channel (the tier-2 differ) reads a whole file per run, where per-run snapshot semantics are correct and are what --help promises. No zsh text or artifact bytes are involved, so there is no parity break to declare. Residual nit only: append_tap is currently dead code and PARITY.md has no CLI/GUI row for the `--tap` flag alongside its `--dry-run`/`--quiet` row.

### A1-6 [medium, conf 0.99] Run tag parity cannot distinguish warn from block
`sabrage/crates/sabrage-parity/src/lib.rs:374-418` — **CONFIRMED** (re-rated medium)

The test can green-light a shell preflight that aborts where the contract says warn, or warns where it says block. For example, changing `game.version` from `warn` to `die` leaves its tag and checked message substring unchanged.
Evidence: `sabrage/crates/sabrage-parity/src/lib.rs:379-400` merges every `# preflight:` and autofix slug into one `shell_gate != none` set; `:402-418` distinguishes only autofix. The contract declares `game.version` as `warn` at `contract/pipeline.toml:201-204`, while the executable shell behavior is `warn` at `scripts/demo/run.sh:13-15`. Tags are comments and are never bound to that command.

*Recommendation:* Encode and compare the full slug-to-gate map, including explicit warn and block tags, and verify each tag is structurally associated with the corresponding executable branch.

*Verifier:* The harness's stated job is that a run-preflight change lands on both sides; a shell gate silently strengthening (warn->die) or weakening (die->warn) relative to the contract passes tier-1 unchanged, which is precisely the drift class the tag scan exists to catch.

*Fix sketch:* In sabrage-parity's run_sh_tags: extract per-gate tags instead of one bag — recognise `# preflight:` (block), a new `# preflight-warn:` tag, and `# preflight-autofix:` — and compare the resulting slug->Gate map against the contract's shell_gate map (BTreeMap equality), keeping REQUIRE_BOTTLE_BLOCKS as an explicit block entry. Optionally bind tag to command by asserting the first non-comment logical line after a `# preflight:` tag contains `die`/`|| die` and after `# preflight-warn:` contains `warn` (and no `die`), which is the structural association the reviewer asks for and is cheap for run.sh's shape.

*Regression test:* sabrage/crates/sabrage-parity/src/lib.rs, mod run_sh_tags: replace preflight_and_autofix_tags_match_the_contracts_shell_gates with a map-equality test, plus a unit test over an inline fixture string asserting that a slug tagged `# preflight:` whose branch calls `warn` (or tagged `# preflight-warn:` whose branch calls `die`) fails the scan.

*Cross-area files:* scripts/demo/run.sh, sabrage/parity/shell.fingerprint, contract/pipeline.toml

### A1-7 [medium, conf 0.99] Doctor slug coverage can pass after real emissions are removed
`sabrage/crates/sabrage-parity/src/lib.rs:237-287` — **CONFIRMED** (re-rated medium)

The scanner invents coverage from loop headers, comments, or strings and discards order and duplicate emissions. Deleting a loop's actual `chk` body while retaining its `slug:value` header therefore remains tier-1 green; live tier 2 is optional and absent in CI.
Evidence: `sabrage/crates/sabrage-parity/src/lib.rs:257-285` regex-scans every logical line without excluding comments/strings and records every slug-shaped word from any `for _x in ...` header without verifying its body. `scripts/demo/doctor.sh:76-78` demonstrates the separation between the tool-slug header and actual `chk` calls. The final comparison at `sabrage/crates/sabrage-parity/src/lib.rs:315-321` reduces results to sets despite contract order being declared load-bearing.

*Recommendation:* Use a shell-aware parser or explicit emission tags tied to calls; validate loop bodies, reject comment/string matches, detect duplicate runtime rows, and compare the declared order rather than only sets.

*Verifier:* Three independent blind spots reproduced with the test's own regexes. (a) is the mildest — the loop header is the emitted slug list, so a realistic deletion removes the header too — but (b) a commented-out check keeping tier-1 green and (c) an unchecked load-bearing order are real gaps in the only automated guard the contract has for doctor.sh. Latent: it needs a subsequent bad edit to bite, and shell_fingerprint forces a human bless on any doctor.sh change.

*Fix sketch:* In slug_coverage: (1) strip comments in logical_lines — drop from an unquoted `#` that starts a word — before regex scanning, so a commented-out chk stops counting; (2) for loop headers, only credit the header's slugs when the loop body (lines to the matching `done`) contains a chk/tap call using the loop variable (`${_t%%:*}`), else report the slug as uncovered; (3) build the found slugs as an ordered Vec of first emission and assert_eq! against the contract's non-run-only slug order (verified equal today), keeping the existing per-section-disjointness assertion.

*Regression test:* sabrage/crates/sabrage-parity/src/lib.rs, mod slug_coverage: unit tests over inline doctor.sh-shaped fixtures asserting (i) a `#`-commented chk line contributes no slug, (ii) a loop header whose body has no chk/tap contributes no slug, (iii) an order-swapped fixture fails the ordered comparison; plus the existing on-disk test now asserting order.

### A1-8 [medium, conf 0.99] Host-manifest rendering does not JSON-escape the dylib path
`sabrage/crates/sabrage-core/src/util/mod.rs:215-218` — **CONFIRMED** (re-rated medium)

A valid checkout path containing `"` or `\` can replace the root-owned host manifest with invalid or misdirected JSON, breaking OpenXR until another privileged install repairs it. The golden reproduces the same bug instead of detecting it.
Evidence: `sabrage/crates/sabrage-core/src/util/mod.rs:215-218` performs raw `.replace(..., &dylib_path.to_string_lossy())`. The golden at `sabrage/crates/sabrage-parity/src/lib.rs:460-464` constructs `expected` with the identical raw replacement. The shell likewise substitutes raw path text at `scripts/demo/install.sh:51` before privileged writing at `:55-57`.

*Recommendation:* JSON-escape the placeholder value on both implementations while preserving existing bytes for ordinary paths. Add quote, backslash, and UTF-8 cases that parse the rendered result and verify the decoded path.

*Verifier:* Reachable: the dylib path is derived from the checkout location (Paths::new(repo_root)), and `"`/`\` are legal in APFS filenames; the result is an invalid or misdirected root-owned manifest written under sudo, i.e. no VR until a privileged rewrite from a sane path. Unusual condition, hence medium rather than high — and the write itself is repairable by re-running install from a normal path.

*Fix sketch:* Add a `json_escape_string` helper in sabrage-core::util and use it in render_host_manifest for the @OXR_DYLIB@ substitution (escape `\` and `"` at minimum; ordinary paths render byte-identically today, so no existing artifact changes). Mirror it in scripts/demo/install.sh by escaping `$OXR_DYLIB` before the `//@OXR_DYLIB@/` substitution (zsh `${OXR_DYLIB//\\/\\\\}` then `${.../\"/\\\"}`) so the two sides still render the same bytes; keep host_manifest_is_current comparing the same escaped form. Consider rejecting non-UTF8 paths outright rather than lossily rendering them.

*Regression test:* sabrage/crates/sabrage-parity/src/lib.rs, mod artifact_goldens: extend render_host_manifest_matches_the_on_disk_template with quote/backslash cases whose expected value is built by parsing the rendered JSON with serde_json and asserting the decoded `runtime.library_path` equals the input path (not by repeating the raw replace), and keep the ordinary-path byte golden unchanged.

*Cross-area files:* scripts/demo/install.sh, sabrage/parity/shell.fingerprint

### A1-9 [medium, conf 0.99] The steam_appid golden verifies byte count, not bytes
`sabrage/crates/sabrage-parity/src/lib.rs:813-855` — **CONFIRMED** (re-rated low)

The claimed end-to-end golden accepts any six-byte payload, including a wrong appid or a newline-substituted value. That can break Goldberg identity while tier 1 remains green, especially because the exact sabrage-core byte test is not selected by tier 1.
Evidence: `sabrage/crates/sabrage-parity/src/lib.rs:840-855` locates the planned write but asserts only `reason == format!("{} bytes", appid.len())`; it never observes the content. The separate assertion at `:836-838` proves only the contract string. The shell's required bytes are directly written with `printf '%s' "$BS_APPID"` at `scripts/demo/run.sh:150`.

*Recommendation:* Expose recorded write bytes or their digest from the dry-run executor and assert equality with `contract().game.appid.to_string().as_bytes()`, or perform the write against a scratch tree and read the exact file bytes.

*Verifier:* Real but narrow. The doc comment at lib.rs:804-812 claims the golden would catch a call-site regression; the length-only assertion does catch the realistic drift (appending a newline, `print`/`println` instead of `printf '%s'` -> "7 bytes"), and lib.rs:836-838 pins the contract value to "620980", but a same-length wrong payload (a hardcoded other 6-digit appid, or "62098\n") passes tier-1 and CI. Exact-byte coverage does exist in the workspace (actions.rs:1109-1114) — it is just outside the tier-1/CI selection — and PARITY.md:119-121 lists these bytes as a must-not-change invariant, so the gap is a weakened guard on a byte-parity invariant rather than any current wrong behaviour. Not medium: nothing is broken today and redundant coverage exists.

*Fix sketch:* Confine the change to the golden in sabrage/crates/sabrage-parity/src/lib.rs (`launch_goldens::steam_appid_txt_is_written_as_exactly_the_appid_digits_with_no_trailing_newline`): build the StageCtx with `dry_run: false` so StageCtx::new (stages/mod.rs:188-191) picks RealExecutor, let goldberg_stage write into the existing scratch fixture tree, and assert `std::fs::read(bs_dir.join("steam_appid.txt")) == contract().game.appid.to_string().into_bytes()` (keep the existing dst-location assertion and the `!appid.ends_with('\n')` shape check). Verified expressible from the public API only: the throwaway test above ran exactly this shape and printed `BYTES OK: "620980"`. Alternative if the dry-run path must stay: give PlannedAction a content digest, but that ripples into every reason-string assertion, so prefer the RealExecutor variant.

*Regression test:* Same test, in sabrage/crates/sabrage-parity/src/lib.rs (tests::launch_goldens), so it runs under tier-1 and .github/workflows/parity.yml: after a real goldberg_stage against a temp fixture tree, assert the file's bytes equal `contract().game.appid.to_string().into_bytes()` exactly (no trailing newline, no other byte) and that it lands beside the discovered steam_api64.dll. It must go red if the call site writes any different payload, same length or not.

*Codex next steps:* Build a binary from checkout X, point it at a fixture checkout Y with a different regenerated contract, and confirm whether `meta.contract-sync` passes while native preflight/setup still uses X values. · In a disposable checkout, change only `bs_dir_leaf`, `gbe_dll_asset`, and `dxmt_tgz_asset`; run regen/check and compare generated shell values and native dry-run URLs/default paths. · Introduce isolated mutations to native banner text, the steam-appid payload, `game.version` warn/block behavior, and a doctor loop body; run the exact CI tier-1 command and then `cargo test -p sabrage-core` to expose false greens. · Keep a valid contract header while changing `contract.gen.sh` body values, then run doctor; separately seed a tap file and compare zsh append behavior with native `--tap`. · Render/install the host manifest from a scratch repo path containing a quote and backslash, then parse the resulting bytes as JSON and verify the decoded `library_path`.

## A2 — core-primitives

**Codex verdict:** needs-attention — Do not ship: core primitives can select the wrong Rust toolchain, corrupt installed overlays on copy failure, mutate or hang during dry-run/cancellation, and lose crash-recovery guarantees under documented edge conditions.

### A2-1 [high, conf 0.99] Child PATH selects the known-incompatible Homebrew cargo
`sabrage/crates/sabrage-core/src/process.rs:158-173` — **REFUTED** (re-rated low)

`default_child_path` puts Homebrew ahead of both rustup and the caller's PATH. The build then resolves `cargo` from that reordered list and passes it into CMake, so a machine with both installations selects Homebrew cargo even when the user configured rustup. This can fail the required x86_64 cross-target build while the separate rustup gate still passes.
Evidence: `sabrage/crates/sabrage-core/src/process.rs:160-169` builds PATH as `"/opt/homebrew/bin", "/usr/local/bin", home.join(".cargo/bin")` and only then `parts.extend(inherited...)`; `sabrage/crates/sabrage-core/src/stages/build.rs:507-513` resolves the first `cargo` from that path; `CLAUDE.md:90` states that Homebrew cargo lacks the required cross-target std. The shell reference invokes bare `cargo` at `scripts/demo/build.sh:45`, preserving the caller's PATH precedence.

*Recommendation:* Preserve inherited PATH precedence for terminal launches. For Finder launches, append fallbacks with `~/.cargo/bin` before Homebrew, or explicitly resolve cargo through rustup. Add a test covering simultaneous Homebrew and rustup installations.

*Verifier:* The claimed consequence — 'can fail the required x86_64 cross-target build' — is impossible: that build never uses the PATH-resolved cargo. The only cargo Sabrage resolves builds alvr_dashboard for the native arch, which needs no cross std; ext/ALVR/Cargo.toml:8 pins rust-version = 1.82, satisfied by a current Homebrew rust. Residual (low): on a machine where the user's login PATH puts ~/.cargo/bin first, Sabrage still prefers Homebrew cargo for the dashboard where the shell would use rustup's — a version-skew nuisance, not the described failure.

### A2-2 [high, conf 0.96] copy_if_changed can destroy the last good installed overlay
`sabrage/crates/sabrage-core/src/executor.rs:404-417` — **CONFIRMED** (re-rated low)

A differing destination is overwritten directly. If the source read or destination write fails after truncation—for example ENOSPC—the previous working DLL/dylib is already damaged. Install uses this primitive for CrossOver's global DXMT and wineopenxr files, so one failed update can break every bottle even though the stage reports a cleanly handled error.
Evidence: `sabrage/crates/sabrage-core/src/executor.rs:410-417` calls `tokio::fs::copy(src, dst).await` directly after the comparison; `sabrage/crates/sabrage-core/src/stages/install.rs:130-154` sends the global `lib/dxmt` and `lib/wine` destinations through this primitive, and lines 282-288 explicitly include ENOSPC among expected failures. `scripts/demo/lib.sh:108-114` also uses direct `cp`, but project policy requires byte parity, not reproduction of its rollback weakness.

*Recommendation:* Copy into a unique file in the destination directory, preserve the required executable/permission metadata, sync it, and rename only after the complete copy succeeds. Leave the old destination untouched on every error.

*Verifier:* The mechanism is real, but the blast radius the reviewer implies is not: (a) scripts/demo/lib.sh:108-114 `install_if_changed` does exactly the same non-atomic `cp`, so this is reference-implementation behavior, not a native regression; (b) nothing is lost — the sources are the pinned local ext/dxmt-artifacts and the freshly built wineopenxr outputs, the stock DXMT tree is separately preserved at $CX/lib/dxmt.stock-backup, and CLAUDE.md's standing remedy for any broken overlay is 're-run install', which rewrites the byte-identical file; (c) it requires an IO failure or a kill inside the copy window, i.e. unusual conditions. Latent robustness gap, not user-visible wrong behavior on a realistic path.

*Fix sketch:* In RealExecutor::copy_if_changed (executor.rs:404-417) copy into a sibling temp in the destination directory using the existing tmp_path() helper (executor.rs, same module as download's `<dest>.tmp`), set the mode from the source's metadata, then tokio::fs::rename(tmp, dst); on every error path remove the temp and return the original SabrageError::io(dst, e) so install_if_changed's TCC/`copy failed:` classification (install.rs:292+) is unchanged. Keep the cmp_files short-circuit and the Copied::Unchanged/Copied contract identical so no printed row changes.

*Regression test:* sabrage/crates/sabrage-core/src/executor.rs #[cfg(test)] module (where the existing copy_if_changed tests live): (1) a copy into an existing dst whose parent directory is made read-only mid-flight (or whose src is removed) leaves dst's original bytes intact and leaves no stray temp file behind; (2) a successful copy still returns Copied::Copied, is byte-equal, and preserves the execute bit (the DXMT dylibs/dll case); (3) an unchanged copy still returns Copied::Unchanged and does not touch dst's mtime.

### A2-3 [high, conf 0.98] The probe bypass can hang operations and mutate a dry-run
`sabrage/crates/sabrage-core/src/process.rs:599-637` — **CONFIRMED** (re-rated medium)

`capture` has no timeout, cancellation token, process group, or kill-on-drop policy. A wedged `adb` or `SwitchAudioSource` therefore blocks Run/Stop and the process-wide operation lock indefinitely, and the Cancel button cannot interrupt it. It also classifies `adb devices` and `adb forward --list` as effect-free even though, when no adb server exists, those commands start a persistent server; this is an inferred but standard adb behavior, so a wired dry-run can mutate machine process state outside the Executor and omit it from the plan.
Evidence: `sabrage/crates/sabrage-core/src/process.rs:601-614` calls these adb commands effect-nil probes and says they bypass the Executor; lines 616-637 explicitly admit there is no cancellation hook and await `cmd.output()` without a bound. `sabrage/crates/sabrage-core/src/stages/run/actions.rs:79-86` executes `adb devices` through this path under dry-run.

*Recommendation:* Give capture a cancellation token and explicit per-probe deadline, run it in a process group, and TERM/KILL/reap on cancellation or timeout. Treat adb probes as side-effectful: route them through an executor-aware probe abstraction or skip them in dry-run and report the plan as conditional.

*Verifier:* Confirmed for the hang half: a wedged `adb`/`SwitchAudioSource` stalls the stage inside a future that Cancel provably cannot interrupt (no token is even in scope), including on the teardown path. This is a genuine native-only divergence — run.sh runs these probes as foreground children in the script's process group, so a Ctrl-C reaches adb itself. Needs an actually-stuck probe, so medium, not high. The second half of the finding is REFUTED: routing read-only probes outside the Executor so they also run under --dry-run is a deliberate, documented design choice (process.rs:601-614 and the 'read-only probe, so it bypasses the executor and runs under --dry-run too' comment at actions.rs:79-80, guards.rs:253-255) that keeps the printed plan accurate, and it matches run.sh, which runs `"$ADB" devices` unconditionally (run.sh:99-105); an adb server auto-started by a probe is the same process state the shell pipeline creates, and the checks layer already shells the same commands (checks/headset.rs:35, checks/network.rs:83, checks/run_only.rs:133).

*Fix sketch:* Give process::capture a bounded deadline and a cancel hook: change the signature to `capture(spec: &ChildSpec, cancel: &CancellationToken)` (or add `capture_with(spec, cancel, deadline)` and keep `capture` as a thin wrapper with a default probe deadline of a few seconds), spawn with .process_group(0) + .kill_on_drop(true), and `tokio::select!` the output future against cancel.cancelled() and tokio::time::sleep(deadline), killpg(SIGTERM→SIGKILL) on either, returning SabrageError::Cancelled for cancel and an empty/failed Captured for the timeout so existing `.ok()?`/`_ => Vec::new()` callers degrade exactly as they do for a missing binary today. Leave the dry-run probe routing as-is.

*Regression test:* sabrage/crates/sabrage-core/src/process.rs #[cfg(test)] module, next to the existing capture tests (process.rs:939-947): (1) capture of `/bin/sleep 30` with a pre-cancelled token returns SabrageError::Cancelled within well under the sleep and the child pid is gone afterwards; (2) capture with a short deadline against `/bin/sleep 30` returns the timeout outcome and reaps the child; (3) capture of a fast child still returns its full stdout unchanged (byte-for-byte, existing assertions).

*Cross-area files:* sabrage/crates/sabrage-core/src/stages/run/actions.rs, sabrage/crates/sabrage-core/src/stages/run/guards.rs, sabrage/crates/sabrage-core/src/session/reconcile.rs

### A2-4 [high, conf 0.99] Several mutating primitives still start after cancellation
`sabrage/crates/sabrage-core/src/executor.rs:459-558` — **REFUTED** (re-rated low)

The executor claims to reject mutations once cancelled, but `dir_copy`, `download`, `tar_xzf`, and `run_child` do not call `guard()`. `spawn_streamed` spawns before selecting the already-ready cancellation token, so a pre-cancelled operation can still start `cp`, `tar`, `reg add`, adb, or another mutating child and make partial changes before SIGTERM arrives. Cancellation during curl also returns through `?`, bypassing both `.tmp` cleanup branches.
Evidence: `sabrage/crates/sabrage-core/src/executor.rs:373-379` promises every filesystem primitive is guarded, while lines 459-500, 528-558 omit that guard; `sabrage/crates/sabrage-core/src/process.rs:319-323` performs `cmd.spawn()` before cancellation is selected at lines 357-365; `executor.rs:500-510` cleans the temp only after an `Ok(status)`, not a `Cancelled` error.

*Recommendation:* Guard every RealExecutor entry point before any directory creation or child spawn, add a pre-spawn cancellation check inside `spawn_streamed`, and use a scope guard so download temp cleanup runs for cancellation and all other early returns. Teardown already uses a fresh uncancelled executor, so it need not weaken this rule.

*Verifier:* The reviewer missed that spawn_streamed's select! cannot lose this race in practice: a just-spawned child's `child.wait()` is never ready on the first poll (it needs SIGCHLD), so the already-ready cancel branch wins and killpg(SIGTERM) reaches the child's own process group (process_group(0), set pre-exec) before /bin/cp, tar or the child's exec can do anything — 0 mutations in 300 attempts, and the call still returns Err(Cancelled) (process.rs:389-391), so the stage aborts identically to a guarded primitive. The remaining sub-claim is real but inert: a Cancel during curl propagates through `?` at executor.rs:495 and skips both `remove_file(&tmp)` branches, leaving `<dest>.tmp`; nothing ever reads a `.tmp` (download re-checks sha256 of `dest` at executor.rs:474 and curl -o truncates the temp on the next attempt), so PARITY.md's cleanup divergence still holds for its stated cases (curl failure, hash mismatch). Adding guard() to the four entry points is a cheap tidiness win, but there is no reachable partial-mutation defect to fix.

### A2-5 [medium, conf 0.95] Cancellation escalation watches only the process-group leader
`sabrage/crates/sabrage-core/src/process.rs:357-383` — **CONFIRMED** (re-rated medium)

After SIGTERM, the grace timeout waits only for the direct child. If that leader exits while a descendant ignores SIGTERM or retains stdout/stderr, the SIGKILL branch is skipped and the code then waits forever for pump EOF. The result is a hung cancellation with surviving build processes—the exact tree shape the process-group logic is intended to handle.
Evidence: `sabrage/crates/sabrage-core/src/process.rs:240-243` notes that cmake, cargo, and similar tools spawn children; lines 365-375 skip SIGKILL as soon as `child.wait()` succeeds; lines 379-383 subsequently await every pipe pump without a timeout.

*Recommendation:* Keep the grace deadline over both leader reaping and pipe/group shutdown. If the group or pumps remain after the deadline, SIGKILL the process group, reap the leader, and bound final pipe draining.

*Verifier:* Code path is unambiguous, reachable from the GUI Cancel button, and reproduced with a 3-line shell tree.

*Fix sketch:* In sabrage-core/src/process.rs::spawn_streamed_inner, compute a single deadline at cancellation time. Cancel arm: SIGTERM the group, then `tokio::time::timeout(spec.kill_grace, child.wait())`; regardless of whether the leader was reaped inside the grace, if the pumps have not finished by the deadline, `killpg(pgid, SIGKILL)` and reap. Replace the unconditional `for p in pumps { let _ = p.await }` with a bounded drain: `tokio::time::timeout(DRAIN_GRACE, join_all(pumps))` and, on expiry, abort the JoinHandles and take whatever tail was already collected. Apply the bounded drain on BOTH arms (the non-cancelled arm has the same hazard), so a daemonised descendant holding the pipe cannot wedge a stage.

*Regression test:* sabrage/crates/sabrage-core/src/process.rs `mod tests`, next to `cancellation_kills_the_process_group`: a test spawning `/bin/sh -c "(trap '' TERM; sleep 20) & sleep 20"` with kill_grace=300ms, cancelling after ~200ms, asserting `spawn_streamed` returns `SabrageError::Cancelled` well inside a 3s `tokio::time::timeout` (today it takes 20s). A second test with no cancellation at all -- `/bin/sh -c "(sleep 20) & exit 0"` -- asserting the call returns promptly with the leader's status instead of blocking on the orphan's pipe.

### A2-6 [medium, conf 0.97] Canonicalizing only the native repo root causes privileged-write thrash
`sabrage/crates/sabrage-core/src/paths.rs:104-149` — **CONFIRMED** (re-rated medium)

Sabrage canonicalizes explicit and environment roots before embedding the dylib path, but the shell derives a logical root spelling. With a symlinked checkout, both paths address the same dylib but produce different manifest bytes, so alternating between Sabrage and demo.sh treats the other frontend's file as stale and repeatedly prompts for sudo—the failure the comment claims to prevent.
Evidence: `sabrage/crates/sabrage-core/src/paths.rs:104-122` canonicalizes roots specifically because manifest bytes are compared literally; `demo.sh:25-26` derives `ROOT` with logical `pwd`; `scripts/demo/install.sh:51-57` embeds `$OXR_DYLIB`, compares literal bytes, and performs the sudo rewrite on mismatch.

*Recommendation:* Adopt one shared root-spelling contract, preferably canonicalizing the shell root with a physical-path operation as well. Add parity coverage that alternates both installers from symlinked and `..`-spelled roots and asserts identical manifest bytes with no second privileged write.

*Verifier:* Unambiguous, reachable whenever the checkout is reached through a symlink; consequence is a repeated privileged write and a false doctor FAIL, not data loss.

*Fix sketch:* Pick one spelling and state it in the contract: make the shell physical too -- demo.sh:25 `ROOT="$(cd -P "$(dirname "$0")" && pwd -P)"` -- keeping paths.rs::canonicalize_lossy as the native mirror, and add a one-line note in sabrage/PARITY.md that repo-root spelling is physical on both sides. (Rejected alternative: dropping canonicalisation in Sabrage -- there is no logical PWD to inherit in a Finder-launched .app, so it cannot reproduce the shell's spelling.) Since demo.sh changes, re-run scripts/dev/parity.sh and re-bless sabrage/parity/shell.fingerprint.

*Regression test:* sabrage/crates/sabrage-parity/src/lib.rs, alongside the existing host-manifest golden: a test that creates tmp/real/{demo.sh,scripts/demo/lib.sh}, symlinks tmp/link -> tmp/real, then asserts (a) `resolve_repo_root(Some("tmp/link"))` equals the canonicalised tmp/real, and (b) `util::host_manifest_file_bytes` rendered from that root is byte-identical to the bytes demo.sh's own ROOT derivation yields when invoked through the symlink (shell-side value captured by running the extracted `cd -P ... && pwd -P` line).

*Cross-area files:* demo.sh, sabrage/parity/shell.fingerprint, sabrage/PARITY.md, sabrage/crates/sabrage-parity/src/lib.rs

### A2-7 [medium, conf 0.97] Atomic writes are neither metadata-preserving nor crash-durable
`sabrage/crates/sabrage-core/src/executor.rs:674-686` — **CONFIRMED** (re-rated low)

The replacement file is created from scratch, so an existing destination's mode, ownership, ACLs, and extended attributes are not preserved. Neither the file nor containing directory is synced before success is returned. This matters for `cxbottle.conf` edits and, more critically, for persist-before-mutate session state: after `save` returns, a power loss can still lose the recovery record while the following audio mutation survives.
Evidence: `sabrage/crates/sabrage-core/src/executor.rs:674-686` uses `tokio::fs::write` followed immediately by `rename`, with no metadata cloning or `sync_all`; `sabrage/crates/sabrage-core/src/stages/run/guards.rs:283-294` saves the state and then switches the output device; `paths.rs:336-342` says this record exists specifically to recover from power loss.

*Recommendation:* Create the temp with explicit per-artifact permissions, preserve required metadata when replacing an existing file, write and `sync_all` the temp, rename it, then sync the containing directory before returning.

*Verifier:* The missing fsync is genuine and cheap to fix, but the metadata half has no affected call site and the durability window is a power-loss-only race; latent robustness, not user-visible behaviour.

*Fix sketch:* In sabrage-core/src/executor.rs::write_atomic_real, keep the same temp-then-rename shape but make it durable: open the temp with `OpenOptions::new().write(true).create_new(true).mode(0o600)`, write, `sync_all()` the temp file, set the final mode explicitly (0o644, or clone the destination's mode via `std::fs::metadata(path)` when it already exists), rename, then open the parent directory and `sync_all()` it before returning Ok. Keep the existing error path that removes the temp on rename failure. No signature change, so every call site is unaffected.

*Regression test:* sabrage/crates/sabrage-core/src/executor.rs `mod tests`, next to `write_atomic_replaces_and_leaves_no_temp_files`: assert that after replacing an existing file whose mode was set to 0o600, the destination's mode is still 0o600 (metadata preserved) and that a fresh file lands at 0o644 regardless of a hostile umask (set umask 0o077 for the duration); the fsync half is not observable from a test, so pin it by construction instead -- the test only guards the mode/no-stray-temp invariants.

### A2-8 [medium, conf 0.98] Missing or empty HOME redirects writers outside the user store
`sabrage/crates/sabrage-core/src/paths.rs:25-50` — **CONFIRMED** (re-rated low)

An unset HOME becomes `/`, so setup and GUI state target `/Library/Application Support/...`; an empty HOME is accepted and produces relative `Library/Application Support/...` paths under the working directory. Ordinary users get confusing permission failures, while an elevated invocation can mutate system-wide locations and violate the one-privileged-write boundary.
Evidence: `sabrage/crates/sabrage-core/src/paths.rs:25-31` falls back to `/` and does not reject an empty value; lines 35-49 derive the bottle and Sabrage stores from it; `sabrage/crates/sabrage-core/src/stages/setup.rs:220-240` creates the derived OXRSys directory and writes the runtime TOML through the Executor.

*Recommendation:* Resolve the home directory through a fallible platform API and require a non-empty absolute user home before constructing any mutating Paths. Permit degraded read-only probes separately, but fail closed before setup, install, run, or store writes.

*Verifier:* The unset/empty-HOME fallbacks exist exactly as claimed and the empty case writes under CWD; but it needs an already-broken environment (Finder, Terminal, and launchd all set HOME) and the worst outcome is a confusing error or a stray Library/ directory, not privilege escalation.

*Fix sketch:* Split the read-only and mutating paths in sabrage-core/src/paths.rs: add `fn home_dir_checked() -> Result<PathBuf, SabrageError>` that rejects a missing, empty, or non-absolute HOME with a Fatal carrying the remedy 'run Sabrage with HOME set to your user home'; keep `home_dir()` (the `/` fallback) for read-only doctor probes only, and call `home_dir_checked()` from `Paths::new` (making it fallible) so setup/install/run/stop and every store write fail closed before touching the filesystem. Callers that construct Paths -- the CLI and the Tauri command layer -- propagate the new Result.

*Regression test:* sabrage/crates/sabrage-core/src/paths.rs `mod tests` (which already serialises HOME mutation via the HOME_MUTEX pattern used in fixes/session_json.rs:176): assert `home_dir_checked()` is Err for HOME unset, HOME="", and HOME="relative/dir", Ok for an absolute temp dir, and that `Paths::new` propagates that Err instead of yielding a relative `oxr_appsup`/`sabrage_appsup`.

*Cross-area files:* sabrage/crates/sabrage-cli/src/main.rs, sabrage/src-tauri/src/commands.rs, sabrage/crates/sabrage-core/src/stages/mod.rs

### A2-9 [medium, conf 0.9] Second-resolution process identity can signal a recycled PID
`sabrage/crates/sabrage-core/src/process.rs:442-495` — **REFUTED** (re-rated low)

The persisted identity compares only PID and a start timestamp explicitly documented as seconds; executable identity is intentionally ignored. If the original process exits and the PID is reused within the same second, `is_same_process` returns true. This is an unusual but concrete race, and dashboard release/reconcile can then SIGTERM an unrelated process—the exact unrecoverable mistake this guard claims to prevent.
Evidence: `sabrage/crates/sabrage-core/src/process.rs:442-446` stores start time in seconds; lines 473-495 compare only that value for the live PID and explicitly omit `exe`; `sabrage/crates/sabrage-core/src/stages/run/guards.rs:615-621` uses the result to execute `/bin/kill -TERM <pid>`.

*Recommendation:* Persist a kernel-provided process-unique identifier or subsecond birth time that survives exec. If a strong identity cannot be obtained, mark the process unverifiable and never signal it rather than relying on second-granularity time.

*Verifier:* The guard is not 'PID + a coarse timestamp that collides easily'. Because macOS pids are handed out sequentially and wrap only at PID_MAX (~100k allocations), a recycled pid can only appear tens of seconds after the original's birth — always in a LATER epoch second than the stored one, so `current.start_time == self.start_time` is false and nothing is signalled. Making the collision real would require ~100k process creations inside one wall-clock second, which the measured fork rate (~470/s) and kern.maxproc=16000 rule out. The reviewer also missed the surrounding guards: `signalable` (reconcile.rs:688) rejects pid 0, `is_same_process` short-circuits on `is_alive`, and executor.rs:661's `start_time: 0` fallback can never equal a real start time, so an unobservable spawn is already treated as unverifiable-and-never-signalled — exactly the mitigation the recommendation asks for. The omission of `exe` is documented and load-bearing (process.rs:481-486): CrossOver's wine launcher execs into the real loader, so an exe equality test would misclassify every live session. Finally the shell reference this must stay in parity with is strictly weaker — scripts/demo/run.sh:165-169 `stop_dashboard` does `kill -0` on a bare pid with no identity check at all — so the native path is already the stronger of the two implementations. The only residual scenario is a deliberate backward system-clock step of the same magnitude as a pid wrap combined with a fork storm; that is not a realistic condition.

*Codex next steps:* On a machine containing both Homebrew cargo and rustup, inspect the effective PATH/cargo selected by Sabrage and run the x86_64 build gate; verify the rustup shim is used while inherited terminal precedence is preserved. · Add filesystem fault-injection tests for ENOSPC/short copy and atomic replacement; assert the old overlay remains intact, destination metadata is preserved, and file-plus-directory sync completes before a stubbed audio mutation runs. · Exercise cancellation with pre-cancelled `dir_copy`, `download`, `tar_xzf`, and mutating children, plus a leader that exits on TERM while a descendant ignores TERM and holds the pipes; assert no mutation starts, `.tmp` is removed, and cancellation finishes within budget. · Use a fake hanging adb/SwitchAudioSource and a stopped real adb server to test capture timeout, cancellation, reaping, and wired dry-run; assert no persistent adb server or unplanned machine state appears. · Add edge fixtures for symlinked/`..` repo roots, unset and empty HOME, and injected same-second PID reuse; assert manifest byte parity/no second admin write, no root-or-CWD state writes, and no signal to the replacement process.

## A3a — checks-doctor-core

**Codex verdict:** needs-attention — Do not ship: the default doctor path knowingly starts adb, concurrent operations can produce unqualified transient failures, bottle state is sampled before its load-bearing section, and valid-but-invalid-shaped host JSON receives a parity-breaking diagnosis.

### A3a-1 [high, conf 0.99] Default doctor checks start the adb daemon outside Executor
`sabrage/crates/sabrage-core/src/checks/mod.rs:231-260` — **REFUTED** (re-rated low)

The supposedly read-only doctor enables adb probes by default. Both `adb devices` and `adb forward --list` can start the adb server, creating machine state outside `Executor`, dry-run handling, and `OPERATION_LOCK`. This can bind port 5037 or replace an incompatible running server merely by opening Doctor or running the CLI.
Evidence: `sabrage/crates/sabrage-core/src/checks/mod.rs:231-240` explicitly says adb probing “starts its daemon” and sets `allow_adb_probes: true`; `sabrage/crates/sabrage-core/src/checks/headset.rs:34-36` executes `Command::new(adb).arg("devices").output()`; `sabrage/crates/sabrage-core/src/checks/network.rs:82-84` directly executes `adb forward --list`.

*Recommendation:* Replace these calls with a no-start probe that only connects to an already-running adb server and reports unavailable otherwise. Do not invoke daemon-starting adb commands from evaluators; any intentional adb mutation must go through Executor.

*Verifier:* Not a defect: the default is byte-faithful to doctor.sh, which itself invokes exactly these two adb commands; adb has no supported connect-only/no-start mode, so the recommended 'no-start probe' would be a new, undeclared parity divergence rather than a fix. The GUI already exposes the off switch (default true only to keep doctor parity). Residual risk is limited to adb's own daemon start, identical to what `./demo.sh doctor` does today.

### A3a-2 [medium, conf 0.97] Concurrent doctor results are emitted without the promised operation annotation
`sabrage/crates/sabrage-core/src/checks/mod.rs:585-602` — **REFUTED** (re-rated low)

Doctor is deliberately allowed to run while a mutating stage owns the operation lock, but every outcome is streamed unchanged. During build/install, checks can observe partially replaced overlays or build outputs and present transient FAIL rows and Fix actions as if they described stable machine state. Inference: because the loop reads live artifacts sequentially and neither it nor the Tauri forwarding path adds lock state, users cannot distinguish a real failure from an in-progress operation.
Evidence: `sabrage/crates/sabrage-core/src/checks/mod.rs:593-602` evaluates and immediately streams the raw outcome. Meanwhile `sabrage/crates/sabrage-core/src/stages/mod.rs:11-14` says concurrent doctor rows may be annotated when a build is halfway through, and `stages/mod.rs:444-449` provides `operation_in_progress()` specifically for that purpose.

*Recommendation:* Sample `operation_in_progress()` and attach a structured transient-state annotation to affected outcomes, ensuring both GUI and CLI render it; alternatively defer artifact-sensitive checks until the lock is released.

*Verifier:* The cited comment uses 'may', no code or UI promises an annotation, doctor is a live read-only snapshot exactly as the shell's is, and the dangerous half of the scenario (acting on a transient row) is already blocked by OPERATION_LOCK inside fixes::apply. At most this is an unimplemented UX nicety, not wrong behaviour, and implementing it would need a declared divergence since the shell has no such state.

### A3a-3 [medium, conf 0.93] Bottle existence is sampled before the load-bearing section-3 boundary
`sabrage/crates/sabrage-core/src/checks/mod.rs:267-298` — **REFUTED** (re-rated low)

`CheckCtx::new` tests `cxbottle.conf` before the doctor walk begins, despite comments claiming contract order mirrors section 3. If a bottle is created or removed while the earlier system/CrossOver checks run, `bottle.exists` and every downstream skip decision use the stale cached value. For example, a newly created bottle can still FAIL and skip its checks, while the shell would observe it at section 3. This can change tap statuses and FAIL counts.
Evidence: `sabrage/crates/sabrage-core/src/checks/mod.rs:267-269` calls section 3 load-bearing, but `mod.rs:290-298` immediately executes `named.filter(Bottle::exists)` during context construction. The shell does not test `cxbottle.conf` until `scripts/demo/doctor.sh:43-54`, after sections 0–2.

*Recommendation:* Resolve bottle existence at the `bottle.exists` position in a stateful doctor walk, then freeze that result for downstream evaluators. Add a regression test that creates or removes `cxbottle.conf` between the early checks and section 3.

*Verifier:* The divergence is a TOCTOU window widened by the duration of five read-only checks (three file stats plus `sw_vers`/`defaults read`), against an event — a human creating or deleting a CrossOver bottle — that takes seconds and is never triggered by the app itself. The shell is equally stale for every section after 3, so this is not a parity break; the tap channel can only differ if a bottle appears/disappears inside a few tens of milliseconds.

### A3a-4 [medium, conf 0.99] Valid host JSON with a non-string library_path gets the wrong diagnosis and remedy
`sabrage/crates/sabrage-core/src/checks/host.rs:101-108` — **REFUTED** (re-rated low)

The native parser treats any non-string `library_path` as a JSON parse failure. The shell successfully parses and stringifies such a value, then takes its “missing dylib” branch. A syntactically valid manifest such as `{"runtime":{"library_path":null}}` therefore produces different text and directs the native user toward Python/Xcode inspection instead of reinstalling the host registration.
Evidence: `sabrage/crates/sabrage-core/src/checks/host.rs:89-94` acknowledges Python would stringify number/bool/null/array/object values, while `host.rs:101-108` rejects them through `.as_str()?`; that `None` selects the “cannot parse” branch at `host.rs:48-56`. In contrast, `scripts/demo/doctor.sh:170-176` checks Python's exit code and, after successful `print(None)`, reports “host registration points at a missing dylib” with the install remedy.

*Recommendation:* Either reproduce Python’s stringification before applying the file checks, or classify non-string values as an invalid/missing dylib using the shell branch’s message and install remedy. Add parity fixtures for null, numeric, and container values.

*Verifier:* Not parity-breaking: tier-2 diffs the tap channel (slug + status), which is identical FAIL on both sides for every non-string value; the fail count and doctor exit code are unchanged. The only difference is human message/remedy text on a hand-corrupted manifest that install never writes, and that trade-off is documented in the function's own doc comment. The reviewer's stated impact ('parity-breaking diagnosis') misreads what the parity contract compares. Worth at most a PARITY.md row for tidiness, per CLAUDE.md's 'intentional divergences go in sabrage/PARITY.md' (PARITY.md:18 already covers host.manifest's native serde parse in the other direction).

*Codex next steps:* With the adb server stopped, run native doctor and verify whether it creates a daemon/listener; repeat after implementing a no-start probe. · Hold `OPERATION_LOCK` during a deliberately partial overlay/build fixture and verify doctor rows are deferred or visibly marked transient. · Add a temporary-HOME test that changes `cxbottle.conf` after context construction but before section 3, then compare native statuses with doctor.sh. · Compare shell and native output for host manifests whose `library_path` is null, numeric, and an object. · After fixes, run the core test suite plus `scripts/dev/parity.sh --live=off`, followed by the live doctor differ with a real bottle.

## A3b — checks-doctor-config-net-game

**Codex verdict:** needs-attention — No ship: launch validation can approve an unsupported runtime mode, default doctor probes violate the read-only contract and can hang, and corrupt session state is falsely reported clean.

### A3b-1 [high, conf 0.99] First-match protocol validation can approve a different runtime backend
`sabrage/crates/sabrage-core/src/checks/config.rs:55-67` — **CONFIRMED** (re-rated medium)

Duplicate `protocol` assignments are evaluated from the first occurrence, while the runtime applies the last valid occurrence. A file containing `protocol = "alvr"` followed by `protocol = "oxrsys"` therefore passes doctor and native preflight but launches the legacy backend that Sabrage explicitly refuses to support, producing streaming failure after a green gate.
Evidence: `sabrage/crates/sabrage-core/src/checks/config.rs:55-67` returns immediately from the first matching line (`return fields.next().unwrap_or("").to_string()`), matching `scripts/demo/doctor.sh:181` and its `awk ... exit`. Conversely, `ext/oxrsys/runtime/src/Config.cpp:309-440` walks every line and repeatedly assigns `values.streamingProtocol = value`; `sabrage/crates/sabrage-core/src/stages/run/preflight.rs:107-142` independently repeats the unsafe first-match interpretation. The divergent verdict for the duplicate-assignment example is an inference directly from those control flows.

*Recommendation:* Use one parser matching Config.cpp's table-blind, last-valid-assignment-wins semantics for both doctor and native preflight. Align the shell gate or declare the corrected divergence, and add duplicate-assignment fixtures covering both assignment orders.

*Verifier:* The reviewer's control-flow reading is right and the divergence is reachable, but the framing of the culprit is off in two ways. (1) It is faithful, deliberate doctor.sh parity: doctor.sh:181 `awk ... exit` is the spec, and checks/config.rs:47-53 plus the test at config.rs:342-345 ('First match wins even if a later line also matches') lock it in on purpose. (2) The stronger argument the reviewer missed is that the *same crate* already implements the runtime's rules twice and even surfaces the hazard: config/runtime_toml.rs:335-336 ('every table counts, the last assignment wins'), the public `read_lines_like_the_runtime` at :465-507, and the fix path at :1205-1210 which prints "'protocol' is assigned N times in this file; the last one wins". So Sabrage's config editor would show `oxrsys` while its own doctor row says `alvr`. Re-rated high -> medium: the trigger is a hand-edited config with the key assigned in two tables (setup writes exactly one occurrence and never rewrites the file), i.e. an unusual condition, though the consequence when hit is a green gate followed by a stream that never comes up.

*Fix sketch:* Replace the first-match readers with the runtime's semantics in one place: have `checks::config::read_protocol_state` and `stages::run::preflight::read_toml_facts` call a shared helper backed by `config::runtime_toml::read_lines_like_the_runtime` (last valid assignment wins, table-blind, quote-aware `#` stripping), keeping the FAIL/BLOCK message and remedy strings byte-identical. Keep `parse_protocol`'s awk emulation only if the parity differ needs it, and surface the shadowed-key list in the row `detail` (the field exists: CheckOutcome::with_detail). Because this changes what the native doctor reports for a file the shell reads differently, it needs either the matching change in scripts/demo/doctor.sh (awk without `exit`, taking the last valid value) plus `scripts/dev/parity.sh --bless`, or a new row in sabrage/PARITY.md declaring the corrected divergence.

*Regression test:* In `sabrage/crates/sabrage-core/src/checks/config.rs` tests: add `shadowed_protocol_resolves_like_the_runtime` asserting that `protocol = "alvr"` + `[streaming] protocol = "oxrsys"` yields Fail on `cfg.protocol.legacy-oxrsys` (with the exact doctor message/remedy) and the reverse order yields Pass; replace/extend `parse_protocol_matches_the_awk_recipe`'s first-match assertion. In `sabrage/crates/sabrage-core/src/stages/run/preflight.rs` tests: assert the protocol gate blocks for the shadowed alvr-then-oxrsys file. Cross-check against `config::runtime_toml::read_lines_like_the_runtime` on the same bytes so the two readers can never drift again.

*Cross-area files:* sabrage/crates/sabrage-core/src/stages/run/preflight.rs, sabrage/PARITY.md, scripts/demo/doctor.sh

### A3b-2 [high, conf 0.99] Default adb checks can mutate machine state and block doctor indefinitely
`sabrage/crates/sabrage-core/src/checks/headset.rs:33-38` — **REFUTED** (re-rated low)

The nominally read-only doctor directly invokes multiple adb commands using synchronous, unbounded `Command::output()`. With no running server, adb can start and leave its daemon behind; with a wedged server or USB transport, these calls can wait indefinitely and prevent all later rows and the final report. This is a realistic default path because adb probing defaults on.
Evidence: `sabrage/crates/sabrage-core/src/checks/headset.rs:15-19` explicitly acknowledges that probing may "wake the adb daemon", while `headset.rs:34-38` executes `adb devices` and `headset.rs:85-90` executes the device-shell probe without a deadline. `sabrage/crates/sabrage-core/src/checks/network.rs:82-86` does the same for `adb forward --list`. The shell reference also invokes adb at `scripts/demo/doctor.sh:218-225` and `:241-249`, but repository policy states that bug-for-bug parity is not required and checks remain read-only.

*Recommendation:* Query only an already-running adb server through a non-autostarting transport, and put every external probe behind a bounded, cancellable timeout. Return an explicit Skipped/Warn outcome when no server exists or a probe times out; apply the same helper to `network.rs`.

*Verifier:* Not a defect in this area: the 'mutation' is starting adb's own daemon, exactly what the parity spec (doctor.sh) does, explicitly documented, and already carrying the opt-out the shell lacks — the guard the reviewer missed. The 'unbounded' half is not adb-specific: it is the crate-wide convention for every external probe and matches the shell's, so singling out headset.rs/network.rs misstates the scope; a genuine wedged-adb-server hang would need live machine state that this verification is forbidden to create. If the maintainers want deadlines on external probes, that is a crate-wide design change (a new `checks::probe` helper), not a bug in these two modules.

### A3b-3 [medium, conf 0.98] Unreadable or malformed session.json is falsely reported as clean
`sabrage/crates/sabrage-core/src/checks/config.rs:276-280` — **CONFIRMED** (re-rated low)

Read and JSON-parse failures are collapsed into the same green result as a successfully inspected session with no pins. A partial ALVR write, malformed JSON, or permission problem therefore produces the affirmative message that no stale pins exist, hiding the exact degraded state doctor is supposed to expose.
Evidence: `sabrage/crates/sabrage-core/src/checks/config.rs:212-220` maps both filesystem errors and `serde_json` errors to `UnreadableOrMalformed`, then `config.rs:276-280` maps that state to `CheckOutcome::pass(..., "ALVR session state has no stale manual-IP pins")`. The test at `config.rs:455-464` locks in the false-clean behavior. `scripts/demo/doctor.sh:199-211` inherits the same issue by exiting zero after `json.load` fails, but artifact parity does not require preserving this diagnostic bug.

*Recommendation:* Distinguish successful clean inspection from read/parse failure and return Warn with the native error in `detail`. Align doctor.sh in the same change or record the intentional diagnostic divergence, and replace the malformed-is-clean test with malformed/unreadable warning cases.

*Verifier:* The behavior is real and reachable (a truncated session.json from a crashed ALVR write), but it is intentional, documented shell parity rather than an oversight — the reviewer is arguing for a divergence, not reporting a missed guard. Re-rated medium -> low: the lost signal is one advisory WARN about stale manual-IP pins (the file is auto-recreated by alvr_server_core, and no gate depends on this slug), and the misleading OK row is the only user-visible effect. Worth fixing, but only as a both-sides change with a declared divergence.

*Fix sketch:* Split `SessionPinState::UnreadableOrMalformed` into `Unreadable(io::Error)` and `Malformed(serde_json::Error)` in `inspect_session_pins` (checks/config.rs:210-222) and have `cfg_session_pins` (:265-285) return `CheckOutcome::warn` with doctor's existing 'could not inspect <path>' wording plus `.with_detail(<native error>)`, leaving `Clean` as the only Pass. Because the shell's python swallows the same failures, either mirror it in scripts/demo/doctor.sh (drop the bare `except: sys.exit(0)` in favour of a non-zero exit, which already lands in doctor's WARN branch) and re-run `scripts/dev/parity.sh --bless`, or add a sabrage/PARITY.md row declaring the diagnostic divergence.

*Regression test:* In `sabrage/crates/sabrage-core/src/checks/config.rs` tests: replace `malformed_json_is_silently_clean` with `malformed_json_warns` (assert Warn + the 'could not inspect' message on `{not json`) and add `unreadable_session_json_warns` (a directory at <oxr_appsup>/alvr/session.json, or chmod 000 where the test can, asserting Warn not Pass), keeping `no_client_connections_key_is_clean` as the only Pass case.

*Cross-area files:* sabrage/PARITY.md, scripts/demo/doctor.sh

*Codex next steps:* Add a duplicate-protocol fixture with first `alvr`, last `oxrsys`; assert the Rust check, native preflight, and Config.cpp runtime all resolve `oxrsys` and block launch. · Stop the adb server, run doctor while monitoring the adb process/port, and verify no daemon is created; repeat with a fake hanging adb and assert a bounded timeout plus a completed report. · Exercise malformed, truncated, and permission-denied `session.json` fixtures and verify they produce Warn rather than Pass in CLI, tap output, and GUI events.

## A4 — lock-and-fixes

**Codex verdict:** needs-attention — No-ship: live-session and cross-process serialization guarantees are ineffective, several fixes can falsely report success, and the GUI exposes a destructive remedy the code itself calls known-broken.

### A4-1 [high, conf 0.99] Live-session prohibition is metadata only
`sabrage/crates/sabrage-core/src/fixes/mod.rs:297-337` — **CONFIRMED** (re-rated high)

Every fix is marked forbidden during a live session, but the core apply path never reads that flag; it only acquires the operation mutex and dispatches. The Tauri command likewise checks only `destructive`. Once `run` releases the mutex at launch, a Doctor action can therefore remove the active wired forwards or start build/install against a running session.
Evidence: `sabrage/crates/sabrage-core/src/fixes/mod.rs:181-183` declares `forbidden_while_session_live`; `fixes/mod.rs:297-300` is only `let _guard = ...; apply_holding_lock(...)`; `sabrage/src-tauri/src/commands.rs:892-902` validates `destructive` and then calls `fixes::apply` without a live-session check.

*Recommendation:* Enforce `forbidden_while_session_live` centrally before dispatch using reconciled persistent session/process identity, not only in-memory `live_session()`. Route whole-stage Doctor actions through the same policy and add a live wired-session test proving `RemoveAdbForwards`, build, and install are rejected.

*Verifier:* The registry flag is metadata only. design-core.md:194 states the intended behaviour ('Fixes are serialized behind the same operation lock as stages and refuse to run while SessionMonitor reports a live session'); the implementation delivers only the first half. Realistic path: a --wired session is streaming, the user opens Doctor and clicks Fix on net.adb-forwards -> `adb forward --remove tcp:9943/9944` on the live forwards; or clicks Fix on a build/install row -> install rewrites $CX/lib/wine, $CX/lib/dxmt and the bottle's system32 while wine is running. Note the three actions that DO refuse do so via their own probes, not via the flag, so the flag can never be trusted by a caller.

*Fix sketch:* Add one policy function in fixes/mod.rs, e.g. `fn deny_if_session_live(action, ctx) -> Result<()>`, that resolves liveness from persistent identity (session::reconcile over session_state.json + runtime_status.json + a wineserver scan) rather than only the in-process `live_session()`, and call it at the top of `fixes::apply` (the GUI/CLI door). Do NOT put it in `apply_holding_lock`: that is the launch-preflight door, which runs before run/mod.rs:389 publishes the handle and which relies on backend.rs's `for_launch` edit-while-wineserver-alive behaviour. Give the three whole-stage actions the same gate by adding the check to `stages::run_stage` for Setup/Build/Install (leaving Stop/Run alone). Return the existing `ctx.fatal(..., Some("./demo.sh stop --bottle <name>"))` shape so the GUI renders the remedy it already renders for backend.rs/session_json.rs.

*Regression test:* sabrage/crates/sabrage-core/src/fixes/mod.rs `mod tests`: a `#[tokio::test]` that publishes a LiveSessionHandle via session::set_live_session and asserts `fixes::apply(a, &ctx, &sink)` is `Err` for every `FixAction::EVERY` whose def has `forbidden_while_session_live` (iterate the registry, so a new fix cannot slip through), plus the negative case that `apply_holding_lock` still succeeds on the same state (preflight door unchanged), and a wired-session case asserting RemoveAdbForwards never spawns adb. Mirror one case in sabrage/src-tauri (commands::fix rejects while live).

*Cross-area files:* sabrage/src-tauri/src/commands.rs, sabrage/crates/sabrage-core/src/session/reconcile.rs, sabrage/crates/sabrage-core/src/session/mod.rs, sabrage/ui/src/components/CheckRow.svelte, sabrage/ui/src/screens/Doctor.svelte

### A4-2 [high, conf 0.99] A known-broken destructive remedy remains user-runnable
`sabrage/crates/sabrage-core/src/fixes/mod.rs:69-76` — **CONFIRMED** (re-rated high)

The registry exposes deletion as a Fix action even though the implementation records that it causes an 800x900 black screen and that editing pins in place is the working recovery. The GUI confirmation only says “This cannot be undone”; the specific failure and restore advice are emitted after deletion, when it is too late for informed consent.
Evidence: `sabrage/crates/sabrage-core/src/fixes/mod.rs:69-75` says `Known-bad remedy` and describes the black screen; `sabrage/ui/src/screens/Doctor.svelte:207-216` presents only `This cannot be undone. Continue?`; `sabrage/crates/sabrage-core/src/fixes/session_json.rs:107-111` discloses the black-screen recovery only in the post-delete report.

*Recommendation:* Remove/defer `DeleteSessionJson` from the actionable registry until the in-place pin editor exists. If it must remain, require a remedy-specific confirmation that states the known black-screen outcome and backup restoration procedure before mutation.

*Verifier:* A remedy the repository itself records as broken is one generic-confirm click away, and the dialog's single sentence is both under-informative and factually wrong (session_json.rs:85-92 does take a timestamped backup under Sabrage's `backups/`, so it CAN be undone — the user is told the opposite of the useful truth). Mitigations that lower this below critical: the timestamped backup, and the `any_wineserver_alive` refusal at session_json.rs:69-81. But the outcome — a next session at an 800x900 black screen, with the recovery only discoverable in the row text after the fact — is user-visible wrong behaviour on a one-click path.

*Fix sketch:* Either (a) move `fix.delete-session-json` into `DEFERRED_CONTRACT_FIX_IDS` in fixes/mod.rs so `from_contract_id` returns None and the frontend renders no button (the same mechanism `fix.create-z-drive` already uses) until the in-place pin editor lands; or (b) keep it and carry the warning in the registry: add a `confirm_detail: Option<&'static str>` to `FixDef` naming the black-screen outcome, the backup directory, and 'edit the pinned IP in place instead', surface it through the fix-metadata IPC, and have the confirm dialog render it in place of the hard-coded 'This cannot be undone.' (which should also stop claiming irreversibility given the backup). Independently, gate the Fix button on a non-pass status.

*Regression test:* sabrage/crates/sabrage-core/src/fixes/mod.rs `mod tests`: for option (a), extend the existing 'unmodelled contract ids are exactly X' test so `fix.delete-session-json` is pinned as deferred and `FixAction::from_contract_id("fix.delete-session-json")` is None; for option (b), assert `FixAction::DeleteSessionJson.def().confirm_detail` is Some and contains both '800x900' and 'backup', and add a Doctor.svelte/Vitest case asserting the dialog body renders that string rather than the generic sentence.

*Cross-area files:* sabrage/ui/src/screens/Doctor.svelte, sabrage/ui/src/ipc.ts, sabrage/ui/src/components/CheckRow.svelte, sabrage/src-tauri/src/commands.rs, contract/pipeline.toml

### A4-3 [high, conf 0.98] The operation lock does not serialize GUI, CLI, and demo.sh
`sabrage/crates/sabrage-core/src/stages/mod.rs:431-442` — **CONFIRMED** (re-rated medium)

`OPERATION_LOCK` is an in-memory Tokio mutex, so each GUI/CLI process owns an independent instance and the zsh pipeline owns none. Concurrent setup/build/install/fix operations can therefore overwrite shared artifacts or worktrees while another frontend is consuming them. This conclusion is an inference from ordinary process isolation; the repository contains no shared file/advisory lock.
Evidence: `sabrage/crates/sabrage-core/src/stages/mod.rs:431-442` defines `pub static OPERATION_LOCK: LazyLock<Mutex<()>>` and acquires only that mutex; `stages/mod.rs:505-520` relies on it as the stage serialization boundary.

*Recommendation:* Add an inter-process lock at a stable shared path and make both native entry points and `demo.sh` participate. Preserve the current run launch-boundary release semantics while keeping setup/build/install/fixes mutually exclusive across processes.

*Verifier:* Factually correct as stated. Downgraded from high to medium because it is a missing guarantee rather than a broken one: the zsh pipeline has never had a cross-process lock either (two concurrent `./demo.sh build` runs collide the same way today), the single-instance plugin removes the most likely GUI-vs-GUI collision, and reaching the bad state requires the user to deliberately drive two front-ends at once. The blast radius is a corrupted build tree or a half-overlaid CrossOver install — recoverable by re-running the stage, not data loss.

*Fix sketch:* Add an advisory lock module (e.g. `stages::interprocess_lock`) that opens a file at a stable shared path — `~/Library/Application Support/Sabrage/operation.lock` is wrong for a shell participant; prefer a contract-declared path under the repo or `/tmp` — with `flock(LOCK_EX|LOCK_NB)` (via the existing `nix` dependency), writing pid + stage for a useful 'held by …' message. Wrap it around the existing `acquire_operation_lock()` so `OperationGuard` carries both, keeping `run`'s hand-the-guard-to-run::run release semantics (stages/mod.rs:507-517) so the file lock is released at the same launch boundary. demo.sh must participate through a `chk`-style helper in scripts/demo/lib.sh using `/usr/bin/shlock` or a mkdir-based lock on the same path, and the path itself belongs in contract/pipeline.toml so neither side spells it literally.

*Regression test:* sabrage/crates/sabrage-core/src/stages/mod.rs `mod tests`: a test that spawns a second process (the test binary re-exec'd, or a tiny helper) which tries to take the lock while the parent holds it and asserts it fails with the 'held by pid N' error; plus a test that the lock file is released once the guard drops and once `run`'s launch boundary drops it. A tier-1 parity test in sabrage-parity asserting the shell helper and the Rust module resolve the same contract-declared lock path.

*Cross-area files:* scripts/demo/lib.sh, scripts/demo/contract.gen.sh, contract/pipeline.toml, sabrage/crates/sabrage-cli/src/main.rs, sabrage/src-tauri/src/commands.rs, sabrage/PARITY.md

### A4-4 [high, conf 0.99] Helper restaging cannot repair a missing execute bit
`sabrage/crates/sabrage-core/src/fixes/helper.rs:133-178` — **CONFIRMED** (re-rated medium)

If staged and built helpers have identical bytes but the staged file lost its execute bit, the initial architecture/executable check fails correctly, but `copy_if_changed` returns `Unchanged` because it compares only bytes. A real apply then fails validation forever; dry-run is worse and reports `changed` plus “would be restaged” even though its recorded operation is a skip. Rebuilding does not repair this mode-only state.
Evidence: `sabrage/crates/sabrage-core/src/fixes/helper.rs:100-107` checks `helper_is_arm64`; `helper.rs:133-169` invokes byte-based `copy_if_changed` and unconditionally claims a dry-run restage; `sabrage/crates/sabrage-core/src/executor.rs:411-412` returns `Unchanged` when `cmp_files` succeeds; shell reference `scripts/demo/lib.sh:117-118` confirms executability is part of helper validity.

*Recommendation:* Make helper staging mode-aware through `Executor`: treat missing execute bits as a required copy/permission repair even when bytes match. Dry-run must plan that repair and must not report `changed` when the executor planned `Skip`.

*Verifier:* Real and unambiguous, but downgraded from high to medium: reaching the state requires an external `chmod -x` on the staged helper (std::fs::copy carries the source's mode, and CMake stages it executable), and run.sh's `ensure_helper_staged` (scripts/demo/run.sh via lib.sh:117-118 `helper_is_arm64` + lib.sh:109-115 `install_if_changed`'s `cmp -s`) has byte-for-byte the same dead end — so this is a faithful port of a shell defect, not a native regression. When it does happen it is unrecoverable through both the fix and `./demo.sh build` (a rebuild that produces identical bytes still won't restage), and the dry-run preview actively lies about it.

*Fix sketch:* Make helper staging mode-aware. Add a mode-carrying primitive to the Executor trait — e.g. `install_executable(src, dst) -> Result<Copied>` (or `set_executable(path)`) implemented on RealExecutor as copy-or-chmod and on DryRunExecutor as a planned `PlannedKind::Copy`/`Chmod` — and have `helper::restage_helper` call it instead of `copy_if_changed`, treating 'bytes equal but dst not executable' as a required repair rather than `Unchanged`. Then derive the dry-run report from what the executor actually planned: if the plan is a Skip, return `FixReport::unchanged` instead of the unconditional 'would be restaged' at helper.rs:166-170. Because this diverges from run.sh's `install_if_changed`, add a PARITY.md row for it.

*Regression test:* sabrage/crates/sabrage-core/src/fixes/helper.rs `mod tests` (which already has PermissionsExt imported): (1) staged and built byte-identical with staged at 0o644 -> real apply returns Ok(changed) and `helper_is_arm64(&staged)` is true afterwards, mode has the execute bit; (2) same fixture under a DryRunExecutor -> the recorded plan contains the repair action and the FixReport is not `changed: true` while the executor planned a skip; (3) the already-good case (staged arm64+executable) still returns `FixReport::unchanged` with no plan entry.

*Cross-area files:* sabrage/crates/sabrage-core/src/executor.rs, sabrage/PARITY.md

### A4-5 [high, conf 0.99] ADB failures are reported as a clean unchanged state
`sabrage/crates/sabrage-core/src/fixes/adb.rs:83-166` — **CONFIRMED** (re-rated medium)

Both list failures/timeouts and per-serial removal failures collapse into the same report as an actually clean forwarding table. A disconnected device or adb-server failure can leave the WiFi-breaking forward installed while the UI says “no stale adb port forwards to clear,” violating the honest-state rule. Shell silence after a failed `&&` does not justify a structured FixReport asserting cleanliness.
Evidence: `sabrage/crates/sabrage-core/src/fixes/adb.rs:91-94` maps query errors/timeouts to an empty vector; `adb.rs:149-151` silently discards non-zero removals; `adb.rs:162-166` then returns `FixReport::unchanged(..., "no stale adb port forwards to clear")`. Shell reference `scripts/demo/run.sh:116-120` is silent on removal failure but makes no structured cleanliness claim.

*Recommendation:* Distinguish clean, query-failed, fully-cleared, and partially-failed outcomes. Return an error or explicit warning with the affected serial/port on failure, and re-list after removals before claiming no stale forwards remain.

*Verifier:* The false-cleanliness claim is real and reproducible: two distinct failure modes (query failure/timeout, per-serial removal failure) both collapse into a structured report asserting the table is clean, which the shell reference never asserts (run.sh:113-124 simply prints nothing). Downgraded from high to medium because the scenarios in which stale forwards actually SURVIVE the false claim are narrow: adb forwards live in the adb server process, so a cold-start that blows the 5 s timeout implies an empty forward table anyway, and a device that disconnects has its forwards dropped with it. The realistic residue is 'list succeeded, removal then failed' (adb server dies mid-fix, device drops between list and remove) — latent, not an everyday path, but it is exactly the case where WiFi discovery stays broken while the UI says otherwise.

*Fix sketch:* 1) `list_forwards` returns a result shape (e.g. `Result<Vec<(String,String)>, String>`) instead of swallowing `Ok(Err(_))|Err(_)` into an empty vec. 2) `remove_adb_forwards_at` keeps a `failed: Vec<(serial, local)>` for every `!status.success()` branch. 3) Terminal report becomes four-way: query-failed -> `FixReport::unchanged` whose description says `could not query adb forwards (<reason>) — stale tcp:9943/9944 may still be installed` plus a warn row; all removals succeeded -> unchanged behaviour; some/all failed -> a warn row naming each `<serial> <port>` and a description that never claims cleanliness; genuinely empty list -> today's "no stale adb port forwards to clear". Optionally re-list after removals before claiming clean. Keep the per-serial `forward --remove` invariant and the verbatim `cleared stale adb forward ...` info text untouched.

*Regression test:* sabrage/crates/sabrage-core/src/fixes/adb.rs `#[cfg(test)] mod tests`, reusing `write_fake_adb`: (a) a fixture whose `-s ... --remove` exits non-zero asserts the report is NOT `"no stale adb port forwards to clear"` and names `SERIALX`/`tcp:9943`; (b) a fixture where adb exits non-zero for every argv asserts the description reports the query failure rather than cleanliness; (c) the existing `clean_forward_list_is_a_noop` keeps pinning the genuinely-clean string.

*Cross-area files:* sabrage/PARITY.md

### A4-6 [high, conf 0.93] Backend liveness check and atomic rewrite have a lost-update race
`sabrage/crates/sabrage-core/src/fixes/backend.rs:278-311` — **REFUTED** (re-rated low)

The fix reads the entire configuration before its one-shot wineserver probe and later replaces the pathname with a stale reconstructed snapshot. CrossOver can start, exit, or save GUI changes between those steps; atomic rename swaps the inode but does not make this a coordinated transaction. The launch variant skips the probe entirely and does not kill wineserver until later launch actions. The inferred impacts are lost CrossOver settings or a successful preflight followed by a reset back to `auto` and a black-window launch.
Evidence: `sabrage/crates/sabrage-core/src/fixes/backend.rs:281-305` reads first and then probes; `backend.rs:308-311` writes the stale whole-file snapshot; `backend.rs:270-275` selects `EditAnyway` for launch; `sabrage/crates/sabrage-core/src/executor.rs:674-681` implements the write as temporary-file rename.

*Recommendation:* Coordinate the edit with an inter-process lock and a compare-before-replace check over the bytes/metadata originally read. Fail if the file changed. For launch, order wineserver shutdown and stable-file verification so CrossOver cannot rewrite after the successful backend check.

*Verifier:* The mechanism described is a sub-second TOCTOU inherent to two uncoordinated writers of a file CrossOver owns, not a defect this code introduces or can close. The gap the reviewer names (read at :281, probe at :299) is one process scan wide; for the claimed 'lost CrossOver settings' the CrossOver GUI must write cxbottle.conf inside that window AND the probe must report not-live, i.e. no wineserver is holding the bottle at all — the long, realistic window (CrossOver alive, holding config in memory, writing it back on exit) is precisely what the refusal already covers, and it errs toward refusing when it cannot tell. The launch variant's 'skips the probe' is the documented divergence rationale, byte-for-byte the shell's own ordering, and its claimed consequence (successful preflight then a silent reset to auto and a black window) is what run.sh does; the native side instead re-checks after the auto-fix and aborts with run.sh's die text. Nothing reproducible; the reviewer's proposed inter-process lock has no counterpart CrossOver participates in.

### A4-7 [medium, conf 0.99] Noncanonical backend lines produce a false success report
`sabrage/crates/sabrage-core/src/fixes/backend.rs:90-100` — **CONFIRMED** (re-rated medium)

Any line starting with the key enters the rewrite branch, but only the exact spacing and quoting pattern is replaced. Inputs such as an unquoted value or different spacing are returned without a canonical target line; the caller nevertheless performs an atomic replacement, emits “forced to dxmt,” and returns `changed=true`. A standalone Fix therefore claims success while Doctor still fails, and launch aborts only because its separate re-check catches the lie.
Evidence: `sabrage/crates/sabrage-core/src/fixes/backend.rs:91-100` branches on `starts_with(KEY_PREFIX)` but rewrites only `VALUE_PREFIX`; `backend.rs:308-321` unconditionally emits and reports success. Shell reference `scripts/demo/run.sh:40-42` has the same narrow sed pattern, but `sabrage/PARITY.md:7` explicitly says bug-for-bug parity is not required.

*Recommendation:* Require the transformed bytes to contain the exact target line before reporting success. Canonicalize supported spacing/quoting variants or insert the canonical key in the correct section; otherwise return a clear failure.

*Verifier:* Reachable and reproducible; the false claim survives only on the STANDALONE fix path (doctor Fix button / `sabrage fix set-graphics-backend`), where nothing re-checks: the user is told 'bottle graphics backend forced to dxmt' while cxbottle.conf is byte-identical and doctor's anchored `bottle.gfx-dxmt` still fails. The launch path is protected (preflight.rs:620-649 re-evaluates and dies with run.sh's `could not force graphics backend to dxmt` text), and run.sh's sed has the same blind spot, so this is strictly a Sabrage-only over-claim on a path with no shell counterpart. Medium, not high: a real cxbottle.conf is written by CrossOver in the canonical `"KEY" = "value"` shape, so a non-canonical key line requires hand-editing or a third-party writer.

*Fix sketch:* In `fixes::backend::rewrite` (backend.rs:278-321), after `let (rewritten, _branch) = rewrite_graphics_backend(&conf);` assert the postcondition before writing: if `!rewritten.lines().any(|l| l == TARGET_LINE)`, return `ctx.fatal(format!("could not force graphics backend to dxmt in {}", conf_path.display()), None)` WITHOUT calling `write_atomic` — reusing the exact die text `stages::run::preflight::post_fix_die` already emits for the same condition, so both doors say the same thing. `rewrite_graphics_backend` itself stays as-is (its sed-fidelity tests are the contract); optionally add a canonicalizing pass for `"CX_GRAPHICS_BACKEND"` lines with alternate spacing/quoting before that guard.

*Regression test:* sabrage/crates/sabrage-core/src/fixes/backend.rs `#[cfg(test)] mod tests`, next to `set_graphics_backend_rewrites_and_reports_the_verbatim_description`: write `"CX_GRAPHICS_BACKEND" = auto\n` into the fixture bottle, assert `set_graphics_backend` (and `set_graphics_backend_for_launch`) return `Err` whose text contains `could not force graphics backend to dxmt`, that the file bytes are unchanged, and that no `FixReport { changed: true }` with `FORCED_DESCRIPTION` is produced.

### A4-8 [medium, conf 0.9] Session liveness only recognizes one exact wineserver binary
`sabrage/crates/sabrage-core/src/fixes/backend.rs:149-226` — **REFUTED** (re-rated low)

The “any wineserver” predicate actually scans only processes whose canonical executable equals the single CrossOver path selected in `Paths`. A session using another CrossOver installation or another Wine build can own the machine-global ALVR file yet remain invisible; if `paths.wineserver` is absent, the guard is skipped altogether. This is an inferred multi-install/non-CrossOver failure path.
Evidence: `sabrage/crates/sabrage-core/src/fixes/backend.rs:154-170` filters on `resolved != want`; `backend.rs:225-226` defines `any_wineserver_alive` as non-emptiness of that filtered scan; `sabrage/crates/sabrage-core/src/fixes/session_json.rs:69-81` conditionally invokes it for only `ctx.paths.wineserver`.

*Recommendation:* Fail closed when the relevant executable cannot be resolved, and detect live OXRSys/ALVR consumers across installed Wine/CrossOver binaries using process identity, environment, and persistent session state rather than one exact executable path.

*Verifier:* The multi-install / other-Wine-build path cannot own ALVR's session.json in this pipeline. `install` overlays DXMT + wineopenxr into the SAME `$CX` that lib.sh and `Paths::new` resolve with the identical probe order, and runtime discovery goes through the root-owned host manifest pointing at the oxrsys dylib; a session launched from a second CrossOver install or a stock Wine build has no bridge, never loads oxrsys, and therefore is not an ALVR/session.json consumer — so it is not a live writer the guard needs to see. The `paths.wineserver == None` fail-open is likewise unreachable in a state where it matters: it holds only when no CrossOver.app is installed at all, in which case no Wine session can be running (and doctor's `cx.present` fails first). The guard is also already fail-closed for the case it can see (backend.rs:186-193: unreadable/absent WINEPREFIX counts as live). No reproducible path; the reviewer's own text marks this as inferred.

### A4-9 [medium, conf 0.96] Second-resolution backup names can overwrite the only recovery copy
`sabrage/crates/sabrage-core/src/fixes/session_json.rs:83-127` — **CONFIRMED** (re-rated medium)

The destructive fix derives the backup name from whole epoch seconds and writes it with replacement semantics. Concurrent native processes, or a rapid restore/recreate/delete cycle, can choose the same path; the later atomic rename overwrites the earlier backup. This is especially risky because restoration is the documented recovery from the known black-screen outcome.
Evidence: `sabrage/crates/sabrage-core/src/fixes/session_json.rs:85-92` builds `session.json.{unix_timestamp}` and calls `write_atomic`; `session_json.rs:123-127` truncates time to seconds; `sabrage/crates/sabrage-core/src/executor.rs:680-681` renames over the destination.

*Recommendation:* Create backups with non-overwriting `create_new` semantics and collision suffixes or UUIDs. Do not delete the source until the uniquely named backup has been durably created.

*Verifier:* Overwrite is mechanically certain, not hypothetical. The reviewer's concurrency scenario is weaker than claimed (two racing processes back up identical bytes, so nothing of value is lost — stages::OPERATION_LOCK at stages/mod.rs:434 also serializes in-process callers), but the loss case is real whenever session.json is recreated between two applies inside one second: the second backup replaces the only copy of the pre-delete state, which is precisely the documented recovery from the 800x900 black screen this fix's own header warns about (session_json.rs:4-9, 107-112). Rated medium, not high: it needs a sub-second recurrence, so it is latent rather than a path a user hits normally.

*Fix sketch:* In `delete_session_json` (session_json.rs:86-91): keep `create_dir_all` first, then pick the backup name with a collision-avoiding probe instead of a bare timestamp — `session.json.<secs>`, then `-2`, `-3`, … — i.e. either a local copy of `next_backup_path`'s loop or (preferred) lift `config::runtime_toml::next_backup_path` to a `pub(crate)` helper parameterised by prefix and call it from both writers. Combine with A4-10: derive `backup_dir` from `ctx.paths.sabrage_appsup.join("backups")` so the probe is testable. Note the probe-then-write remains TOCTOU-racy across two processes because `Executor::write_atomic` renames over the destination; a fully atomic version needs a `write_new`/`create_new` primitive on the Executor trait (both impls) — worth doing only if cross-process concurrency is considered in scope.

*Regression test:* sabrage/crates/sabrage-core/src/fixes/session_json.rs `mod tests`: a `#[tokio::test]` that, with `paths.sabrage_appsup` injected at a scratch dir, writes session.json = b"first", applies the fix, rewrites session.json = b"second" and applies again without sleeping, then asserts the backups dir holds TWO files whose contents are exactly {b"first", b"second"} and whose names are `session.json.<secs>` and `session.json.<secs>-2`. Add the naming unit test next to it if the helper is lifted (mirror runtime_toml.rs:1938-1954).

*Cross-area files:* sabrage/crates/sabrage-core/src/config/runtime_toml.rs (only if next_backup_path is lifted/shared), sabrage/crates/sabrage-core/src/executor.rs (only for the optional create_new write primitive)

### A4-10 [medium, conf 0.99] Session tests unsafely mutate process-global HOME
`sabrage/crates/sabrage-core/src/fixes/session_json.rs:168-195` — **CONFIRMED** (re-rated low)

The module mutex serializes only callers that voluntarily take that same mutex; it cannot prevent unrelated parallel tests from calling `home_dir()` while HOME points at the fixture. Holding the override across an `await` widens the race and can redirect sibling tests toward the wrong application-support tree, producing nondeterministic or misleading results. The stated SAFETY justification is therefore false.
Evidence: `sabrage/crates/sabrage-core/src/fixes/session_json.rs:176-188` locks a module-local mutex, calls unsafe `set_var`, and awaits; `sabrage/crates/sabrage-core/src/paths.rs:28-31` reads HOME without that mutex. The production cause is `session_json.rs:86`, which calls global `sabrage_support_dir()` instead of using the injectable `ctx.paths.sabrage_appsup`.

*Recommendation:* Use `ctx.paths.sabrage_appsup.join("backups")` so tests can inject a scratch location without changing HOME. Remove the process-global environment override entirely.

*Verifier:* The stated safety justification is factually wrong (sibling threads do read HOME unsynchronised, and `setenv` concurrent with any `getenv` is UB regardless of which variable), and the crate's own injection pattern makes the override unnecessary. Downgraded to low: it is test-only — no production path changes behaviour — and the interference is latent enough that 100 parallel runs did not flake it.

*Fix sketch:* session_json.rs:86 → `let backup_dir = ctx.paths.sabrage_appsup.join("backups");`, dropping `use crate::privilege::sabrage_support_dir` (line 40). In `mod tests`, set `paths.sabrage_appsup = root.join("Sabrage")` inside `ctx_with_session_json` (same shape as stages/run/preflight.rs:942 and config/runtime_toml.rs:2099) and delete `HOME_MUTEX`, `with_home_override`, and both `unsafe { std::env::set_var }` blocks (session_json.rs:168-196); each call site then awaits `delete_session_json` directly and reads backups under the injected dir. Update the module/test doc comments that currently explain the HOME override.

*Regression test:* The existing session_json tests become the regression test once converted: `deletes_after_backing_up_and_reports_the_backup_location`, `dry_run_neither_backs_up_nor_deletes` and `a_preview_executor_beats_opts_dry_run_false` assert the backup lands under the INJECTED `ctx.paths.sabrage_appsup/backups` (and that the injected dir stays absent on a dry run), with no `std::env::set_var` anywhere in the crate — enforceable by a small test asserting `grep -R 'set_var' crates/sabrage-core/src` finds nothing, or simply by the absence of the helper.

*Codex next steps:* Run a two-process harness that overlaps GUI/CLI/demo setup, build, install, and fixes, then verify a shared lock serializes them and live-session policy rejects every forbidden action. · Add fake-adb cases for list timeout/non-zero status and partial/all removal failures; assert the report never labels these states clean and that a final `forward --list` confirms removal. · Create identical built/staged helper bytes with staged mode 0644; verify real apply repairs mode and dry-run plans the same repair without a false success. · Exercise backend fixtures covering noncanonical spacing, unquoted and duplicate keys, CRLF, and a concurrent writer/CrossOver save; verify exact target postconditions without losing unrelated bytes. · Test the destructive path with an alternate wineserver binary, colliding backup timestamps, and the real GUI confirmation; also run the full Rust suite in parallel to confirm no test changes process-global HOME.

## A5 — stages-setup-build-stop

**Codex verdict:** needs-attention — Do not ship: stale build state can permanently block the x64 build, helper staging can report success with an unusable artifact, and several dry-run/stop paths emit success for work that never happened or could not be verified.

### A5-1 [high, conf 0.99] Setup dry-run fabricates successful postconditions
`sabrage/crates/sabrage-core/src/stages/setup.rs:111-127` — **CONFIRMED** (re-rated medium)

A dry-run over a fresh or damaged checkout emits green rows saying the submodules and patch set are ready, and that DXMT was extracted with its marker written, although the commands were only planned and the predicates are false. This makes the preview actively misleading rather than merely hypothetical.
Evidence: `setup.rs:111-127` uses `openvr_header.is_file() || ctx.executor.is_dry_run()` and `patchset_present || ctx.executor.is_dry_run()` before calling `st.ok("submodules ready")` and `st.ok("ALVR checkout carries...")`; `setup.rs:198-204` similarly accepts `dxmt_files_ok(...) || exec.is_dry_run()`, plans the marker write, then emits `ok("extracted ... provenance marker written")`. The dry-run contract calls for an honest preview with read-only probes, not synthetic successful state (`sabrage/docs/design/design-core.md:361`).

*Recommendation:* When a postcondition is currently false under dry-run, emit an Info/Planned or Unverified row using future-tense text; reserve Ok for predicates that actually hold or mutations that completed. Add fresh and damaged-checkout event tests.

*Verifier:* The rows are Severity::Ok statements about checkout state ('submodules ready', 'patch set present', 'marker written') that are false at the moment they print; a dry run over a fresh or damaged checkout is exactly the preview a user runs before committing to setup. No machine state is harmed and the trailing plan section still discloses the truth, so this is misleading UI rather than wrong mutation — medium, not high.

*Fix sketch:* In `setup::setup_submodules` and `setup::setup_pinned`, split the two-way `predicate || is_dry_run()` conditions into three arms: predicate true -> `st.ok(...)` unchanged; predicate false and `is_dry_run()` -> `st.info("would initialize submodules …" / "would extract ext/dxmt-artifacts and write the provenance marker")` (future tense, Info severity, so no green completed-state claim); predicate false and real run -> existing `st.fatal(...)`. Keep the marker `write_atomic` call inside the dry-run arm so the plan still records the write.

*Regression test:* New `#[cfg(test)] mod tests` cases in sabrage/crates/sabrage-core/src/stages/setup.rs (or a stage-event integration test alongside the existing tests/ dir) that run `setup::run` with a `DryRunExecutor` and a `Paths` rooted at a scratch dir containing no `ext/ALVR/openvr/headers/openvr_driver.h`, no `connection.rs`, and no dxmt artifacts, collecting StageEvents through a Vec sink; assert no event with Severity::Ok carries 'submodules ready', 'carries the oxrsys patch set', or 'provenance marker written', that Info rows with 'would' text appear instead, and that the same run over a fully-set-up scratch tree still emits the three Ok rows.

*Cross-area files:* sabrage/PARITY.md

### A5-2 [high, conf 0.97] The x64 configure does not clear a cached encoder-helper enable
`sabrage/crates/sabrage-core/src/stages/build.rs:369-375` — **CONFIRMED** (re-rated medium)

A build-x64 tree whose cache contains `OXRSYS_BUILD_ENCODER_HELPER=ON` remains poisoned: CMake options retain cached values, so the x86_64 reconfigure reaches the thin-arm64 FATAL gate on every retry. The stage neither forces the required OFF value nor gives the documented cache-recovery remedy.
Evidence: `build.rs:369-375` passes architecture and ALVR arguments but no `-DOXRSYS_BUILD_ENCODER_HELPER=OFF`. The helper gate defaults OFF for non-arm64 but fatals when a cached ON survives (`ext/oxrsys/runtime/CMakeLists.txt:334-342`), while the repository's explicit arch gate requires `OXRSYS_BUILD_ENCODER_HELPER:BOOL=OFF` in build-x64 (`CLAUDE.md:100`). The shell reference has the same omission at `scripts/demo/build.sh:17-19`, but bug-for-bug parity is not required.

*Recommendation:* Pass `-DOXRSYS_BUILD_ENCODER_HELPER=OFF` explicitly when configuring build-x64 and add a regression that starts with an ON cache and verifies the stage repairs it to OFF.

*Verifier:* Unambiguous and reproducible: a cached ON survives every retry and the stage's only feedback is CMake's 'thin-arm64-only' FATAL with no cache remedy. It needs a pre-existing damaged/stale build-x64 tree (or a manual -DON configure) rather than any default path, so medium rather than high; recovery (delete build-x64) is available but undocumented at the point of failure.

*Fix sketch:* Add `"-DOXRSYS_BUILD_ENCODER_HELPER=OFF"` to the extra-args array of the build-x64 `configure_spec` call in `stages::build::run` (build.rs:369-375), which forces the cache entry to OFF and makes the tree self-repairing and CLAUDE.md's arch-gate invariant (2) true by construction. Optionally map a configure failure whose output contains 'thin-arm64-only' to a Fatal carrying the remedy 'delete ext/oxrsys/build-x64 and re-run build'. Land the same flag in scripts/demo/build.sh in the same commit (CLAUDE.md's both-sides rule) or record the divergence in sabrage/PARITY.md.

*Regression test:* Unit test in sabrage/crates/sabrage-core/src/stages/build.rs: hoist the build-x64 configure args into a `const OXRSYS_X64_CONFIGURE_ARGS: [&str; 5]` and assert it contains both `-DCMAKE_OSX_ARCHITECTURES=x86_64` and `-DOXRSYS_BUILD_ENCODER_HELPER=OFF`, plus an end-to-end assertion driving `build::run` under a DryRunExecutor and checking that the recorded PlannedKind::Spawn display string for the build-x64 configure contains the OFF flag. A cache-repair integration test (configure a scratch tree with ON, re-run the stage's arg list, assert the cache flips to OFF) covers the CMake side.

*Cross-area files:* scripts/demo/build.sh, sabrage/PARITY.md

### A5-3 [high, conf 0.99] Byte-identical but non-executable staged helpers cannot be repaired
`sabrage/crates/sabrage-core/src/stages/build.rs:430-447` — **CONFIRMED** (re-rated medium)

If the staged helper has the correct bytes but lost its execute bit, build validates only the source, treats the destination as unchanged, and completes successfully. Doctor then rejects the staged file, and the automatic restage follows the same byte-only comparison, so the user remains stuck until manually deleting or chmodding it.
Evidence: `build.rs:430-447` validates `oxr_helper_built` and then calls `copy_if_changed` without validating the staged result; `build.rs:529-543` only checks `is_file()` in the final sweep. `copy_if_changed` returns Unchanged solely from byte comparison (`executor.rs:404-418`), while the launch/doctor predicate requires an execute bit (`checks/build.rs:70-78,111-117`).

*Recommendation:* After staging, always validate `helper_is_arm64(oxr_helper_staged)`. Ensure the copy path also repairs destination mode bits when bytes match, and test a 0644 staged copy with identical bytes.

*Verifier:* Reachable dead-end with no in-tool recovery, and build actively reports success over an unusable artifact. The precondition (identical bytes, execute bit stripped) cannot arise from the pipeline itself — std::fs::copy preserves the source mode — so it needs an external cause (zip/scp restore of build-x64, a broad chmod), which makes it medium rather than high.

*Fix sketch:* In `stages::build::run`, after the `copy_if_changed` match (build.rs:444-469), add a real-run validation of the destination: `if !dry_run && !helper_is_arm64(&ctx.paths.oxr_helper_staged)` -> repair then re-check, or fatal with a remedy naming the staged path. Repair belongs in the copy primitive: make `RealExecutor::copy_if_changed` (and the dry-run recorder, for plan accuracy) treat 'bytes match but destination mode differs from source mode' as changed — chmod the destination to the source's mode and return Copied::Copied — so build, `fix.restage-helper`, and run's preflight self-heal all fix it once. Also use `helper_is_arm64` instead of `is_file()` for the helper entry in the final sweep (build.rs:533).

*Regression test:* sabrage/crates/sabrage-core/src/executor.rs tests: `copy_if_changed_repairs_destination_mode` — identical bytes, src 0755, dst 0644, assert Copied::Copied and dst mode 0755. sabrage/crates/sabrage-core/src/stages/build.rs tests: stage the helper copy over a scratch build-x64/runtime whose staged file has identical bytes at 0644 and assert the stage either repairs it (destination executable afterwards) or returns a Fatal, and never emits Ok('encoder helper built (arm64) and staged …') plus Ok('all build outputs present') for a non-executable staged helper.

*Cross-area files:* sabrage/crates/sabrage-core/src/executor.rs, sabrage/crates/sabrage-core/src/fixes/helper.rs

### A5-4 [high, conf 0.99] Build dry-run reports artifacts as built despite skipping validation
`sabrage/crates/sabrage-core/src/stages/build.rs:470-471` — **CONFIRMED** (re-rated low)

The dry-run output contains completed-action Ok rows even though no compiler ran and the code explicitly skips artifact checks. The closing message then admits that nothing was built, producing contradictory UI state in one invocation.
Evidence: `build.rs:391` emits `ok("oxrsys built")`; `build.rs:470-471` emits `ok("encoder helper built (arm64) and staged...")`; and `build.rs:517` emits `ok("ALVR dashboard built")` unconditionally. Only later does `build.rs:525-527` say the output sweep was skipped because `nothing was built`.

*Recommendation:* Render dry-run build rows as Info with “would build/would stage” language, and emit completed Ok rows only after real execution or a currently true postcondition.

*Verifier:* Same defect class as A5-1 but weaker: these rows narrate this stage's own actions in a mode the user explicitly asked to preview, the output itself says nothing was built, and no state claim about the checkout is made. Cosmetic honesty issue — low.

*Fix sketch:* In `stages::build::run`, gate the four narrative Ok rows on `dry_run`: emit `info("would build oxrsys (build-x64)")` / 'would build the encoder helper and stage it', 'would build wineopenxr', 'would build the ALVR dashboard' under a dry run and keep the existing Ok text on the real path — the same verb-swap convention build.rs already applies to the helper's copy outcome. Add the matching 'would' verbs to sabrage/PARITY.md's dry-run language row.

*Regression test:* sabrage/crates/sabrage-core/src/stages/build.rs tests: drive `build::run` with a DryRunExecutor over a scratch root (tool gates satisfied by a fake PATH of stub executables), collect events through a Vec sink, and assert no Severity::Ok event contains 'built' while the corresponding Info 'would build' rows are present; the real-run counterpart keeps asserting the Ok texts verbatim.

*Cross-area files:* sabrage/PARITY.md

### A5-5 [high, conf 0.98] Reap neither verifies process identity nor termination before claiming success
`sabrage/crates/sabrage-core/src/stages/stop.rs:366-383` — **CONFIRMED** (re-rated low)

The process snapshot includes a start time, but reap discards it and signals a bare PID after an asynchronous gap. If the matched process exits and its PID is recycled, an unrelated process can receive SIGTERM. Separately, a failed kill, an ignored TERM, and a dry-run all still produce `encoder helper killed`, so Stop can finish green with the helper alive.
Evidence: `stop.rs:366-383` snapshots `ProcInfo`, executes `/bin/kill -TERM <pid>`, discards the executor result with `let _ = ...`, performs no liveness check, and unconditionally emits `found_msg`. `ProcInfo::is_same_process` already provides the pid/start-time recycling guard (`process.rs:473-497`). The shell likewise swallows `pkill` errors at `scripts/demo/lib.sh:144-152`, but the native UI must not repeat a dishonest success.

*Recommendation:* Re-check `(pid,start_time)` immediately before signalling through an identity-aware executor primitive, wait briefly for exit, and emit Ok only when termination is verified. Under dry-run, say “would terminate” rather than “killed”.

*Verifier:* Real but cosmetic-to-latent, not high: the only deterministic, reachable defect is a dry-run row that asserts a kill that never happened (and an unverified-termination claim on a real run). The pid-recycling trust-boundary story the 'high' rating rests on is unreachable, and swallowing a failed kill is byte-faithful to `reap_stray`'s `pkill ... || true` (scripts/demo/lib.sh:144-152), the default rather than a divergence.

*Fix sketch:* In stages/stop.rs: (1) make `reap` dry-run aware the way fixes/adb.rs:153 already is - take the verb from `ctx.executor.is_dry_run()` (or pass a `(real_msg, dry_msg)` pair from `run`'s two call sites, stop.rs:156-171) so a dry run says 'would terminate the leftover encoder helper' instead of 'encoder helper killed'; (2) after the per-pid `/bin/kill`, re-check the snapshot with `ProcInfo::is_same_process()` (process.rs:488-497) in a short bounded poll (~1s in 50ms steps, skipped entirely under dry run) and emit `found_msg` as Ok only when every matched identity is gone, else `warn` naming the surviving pid; (3) optionally skip signalling a pid whose `is_same_process()` is already false - free, and it removes the theoretical race the reviewer names.

*Regression test:* sabrage/crates/sabrage-core/src/stages/stop.rs `#[cfg(test)] mod tests`: (a) a dry-run reap over `std::env::current_exe()` asserting the emitted row text is the 'would ...' variant and differs from the real-run text, alongside the existing planned-Spawn assertion; (b) a real-executor reap over a scratch copy of `/bin/sleep` placed at a unique temp path and spawned by the test (exact-path match so nothing else on the machine can match), asserting the Ok 'killed' row is emitted only after the child is really gone (`ProcInfo::is_same_process()` false), and a companion case where the child ignores SIGTERM asserting a Warn row instead of Ok.

*Cross-area files:* sabrage/PARITY.md

### A5-6 [medium, conf 0.98] Failed stop probes are converted into clean Ok rows
`sabrage/crates/sabrage-core/src/stages/stop.rs:294-302` — **REFUTED** (re-rated low)

Dependency failure is indistinguishable from a clean machine. A missing/unrunnable lsof becomes an empty listener set and `streaming ports free`; a SwitchAudioSource spawn or command failure becomes an empty device name and the Ok row `audio output: `. This can hide occupied ports or a failed audio restore precisely when diagnostics are most important.
Evidence: `stop.rs:294-302` maps every lsof spawn error to `String::new()`, which `stop.rs:325-331` renders as Ok. `stop.rs:430-445` discards SwitchAudioSource's exit status and maps spawn errors to empty stdout, while `audio_report("")` selects the Restored branch. The shell reference also ignores these failures at `scripts/demo/stop.sh:15-17,25-32`, but bug-for-bug parity is explicitly unnecessary.

*Recommendation:* Use a captured probe result that preserves spawn errors, status, and stderr. Emit Warn/Unknown when the probe cannot establish state; reserve Ok for a successful probe showing no listeners or a non-BlackHole device.

*Verifier:* Unreachable trigger conditions (a base-system binary; a which() guard immediately preceding the spawn), the named harm belongs to a different function that does report its failures, and the residual empty-name row is faithful-to-shell cosmetics.

### A5-7 [medium, conf 0.91] Stopping from another repo root cannot find the old helper
`sabrage/crates/sabrage-core/src/stages/stop.rs:156-162` — **CONFIRMED** (re-rated low)

The reap target is derived only from the current checkout. A helper launched from repo A has a different resolved executable path, so Stop invoked from repo B reports `no leftover encoder helper` even if that process survived the wineserver shutdown. This is especially plausible in the project's worktree workflow. Inference: persisted session state cannot close this gap because it records wine and dashboard identities but no helper identity.
Evidence: `stop.rs:156-162` reaps only `ctx.paths.oxr_helper_staged`, and `stop.rs:366-370` exact-matches that path before emitting the not-found Ok row. Exact resolved-path equality is enforced by `process.rs:500-529`; `session/state.rs:102-108` persists wine and dashboard but no helper.

*Recommendation:* Persist the observed helper identity with the session and reap it using pid/start-time validation. Until that exists, detect same-basename helpers outside the current root and warn instead of claiming none remain.

*Verifier:* The gap is real and deterministic on the two-checkout path (plausible in this repo's worktree workflow), and the Ok row is falsely reassuring - but it needs an already-doctor-FAIL machine state plus a helper that outlived `wineserver -k`, and the shell reference behaves the same, so it is latent rather than user-visible on a normal path.

*Fix sketch:* Minimal, contained in stages/stop.rs: when the helper reap finds nothing at `ctx.paths.oxr_helper_staged`, run a second read-only scan by basename (`process::find_processes_by_cmdline("oxrsys-encoder-helper")`, already public and used by `report_survivors`) and, if it matches a process whose resolved exe lies outside this repo root, emit `warn "leftover encoder helper from another checkout: <pid> <path> - stop it from that checkout"` instead of the Ok row. Deliberately no kill: PARITY.md's Stop rationale (a mutating kill may not rely on an argv/basename match) still holds, so this stays a report. The fuller variant - record the observed helper `ProcInfo` in `SessionState` at launch and reap it with `is_same_process()` validation - touches session/state.rs and stages/run.rs and is only worth it if the helper identity is wanted for the Session screen too.

*Regression test:* sabrage/crates/sabrage-core/src/stages/stop.rs `#[cfg(test)] mod tests`: copy a scratch executable named `oxrsys-encoder-helper` into a unique temp dir ('another checkout'), spawn it, point `ctx.paths.oxr_helper_staged` at a nonexistent path under a different fake root, run the helper reap and assert a Warn row naming that pid/path rather than Ok 'no leftover encoder helper'; the test kills its own child in teardown. Pair it with the existing not-found case (`dry_run_reap_plans_a_kill_per_match_and_reports_once`) to assert the Ok row still appears when no foreign helper exists.

*Cross-area files:* sabrage/PARITY.md

*Codex next steps:* Run setup dry-run against both an empty checkout fixture and deliberately incomplete DXMT/submodule fixtures; capture events and confirm no completed-action Ok row is emitted for false predicates. · Create a scratch oxrsys build tree with helper=ON and arm64, then reconfigure it as x86_64 through the native build stage; verify the fixed stage forces the cache to OFF and completes. · Copy the built helper to the staged path, chmod it 0644 without changing bytes, then run build, doctor, and launch preflight; all should self-heal and pass after the fix. · Exercise Stop with a test process that ignores TERM plus an injected identity mismatch and DryRunExecutor; verify no unrelated PID is signalled and no `killed` row appears without confirmed exit. · Launch from one checkout and stop from another using the same bottle, then simulate failing lsof and SwitchAudioSource probes; verify leftovers or unknown state produce warnings rather than clean Ok rows.

## A6 — install-privilege

**Codex verdict:** needs-attention — Do not ship: the privileged path escapes Executor guarantees, dry-run reports mutations as completed, layer 1 is not rollback-safe, a successful install can immediately fail launch preflight, and terminal-launched GUIs misdescribe where authorization occurs.

### A6-1 [high, conf 0.94] Privileged cancellation can outlive the staging file and bypass Executor guarantees
`sabrage/crates/sabrage-core/src/privilege.rs:347-389` — **CONFIRMED** (re-rated low)

The sole root mutation uses a separate raw-process implementation instead of the mandatory Executor path. Cancellation sends a kill but does not wait for the child or any privileged descendant to exit; the outer error then drops and unlinks the staging file. Inference: an elevated descendant may finish the write after the stage reports Cancelled, or attempt to read an already-deleted source. Dry-run cannot exercise this behavior because it takes a different implementation.
Evidence: `sabrage/crates/sabrage-core/src/privilege.rs:347-353` explicitly says the elevated child “does not go through Executor” and dry-run “never reaches this code”; `:366-386` branches to a separate planner versus raw elevation; `:521-526` and `:544-547` return immediately on cancellation; `:586-589` unlinks the staged file in Drop.

*Recommendation:* Add an Executor primitive supporting captured or inherited stdio and use one execution path for real and dry-run elevation. On cancellation, terminate and await/reap the complete child process tree before allowing StagedTemp to drop.

*Verifier:* The mechanism is real and reachable (Ctrl-C in the CLI or GUI Cancel between authorization and the privileged write), but three of the reviewer's framings do not survive: (a) 'bypasses Executor guarantees' is a declared, documented design decision (privilege.rs:347-353 and sabrage/PARITY.md's Install rows — osascript needs captured stderr to tell -128 from failure, sudo needs the real tty; spawn_streamed gives neither), not a defect; (b) there is no trust-boundary break — the staging dir is mode 0700 under ~/Library/Application Support/Sabrage and the name is a uuid created O_CREAT|O_EXCL 0600 (privilege.rs:566-585), so an early unlink cannot let another user substitute content: root's `install` either already opened the fd or fails with ENOENT; (c) the residual states are self-correcting — dest written correctly (the intended state), not written, or (narrowly) truncated, and every one of those is caught on the next run by host_manifest_is_current / verify_written and by run's `host.manifest` gate. What is genuinely wrong: Cancel does not actually cancel the privileged mutation, and the stage neither waits for nor reports it.

*Fix sketch:* privilege.rs: give StagedTemp a `defuse()` that suppresses the Drop unlink, and change the cancel arms of run_inheriting/run_capturing to (1) signal, then (2) await the child with a bounded tokio::time::timeout before returning Cancelled. In write_host_manifest_privileged, on SabrageError::Cancelled: if the wait timed out (the root child is unkillable by us), defuse the StagedTemp so the privileged read cannot hit ENOENT, emit a warn that the elevated write may still complete, and sweep stale `host-manifest-*.json` from sabrage_temp_dir() at the start of the next privileged write.

*Regression test:* sabrage/crates/sabrage-core/src/privilege.rs #[cfg(test)] mod tests: a tokio test driving run_inheriting (module-private, callable from the module's own tests) with argv `/bin/sh -c 'sleep 0.4; touch <marker>'` and a token cancelled immediately — asserts the call returns Cancelled only after the child is reaped, that no marker exists, and (second case, with a child that ignores SIGTERM/outlives the timeout) that the StagedTemp path still exists when write_host_manifest_privileged returns.

### A6-2 [high, conf 1] Dry-run claims that unperformed install mutations succeeded
`sabrage/crates/sabrage-core/src/stages/install.rs:249-261` — **CONFIRMED** (re-rated medium)

A stale layer-4 dry-run plans operations, returns `PrivilegedWrite::Written`, and causes the caller to emit the same OK row as a real write. Other layers similarly say “backed up,” “installed,” and “ActiveRuntime registered” after DryRunExecutor only records plans. This violates the honest-stub rule and makes the preview indistinguishable from completed work in the event log.
Evidence: `sabrage/crates/sabrage-core/src/privilege.rs:394-414` returns `Written` from the planner; `sabrage/crates/sabrage-core/src/stages/install.rs:256-261` renders that as `ok("host registration written")`; `install.rs:126-128`, `:222-223`, and `:300-307` use completed-action wording for other planned mutations. `sabrage/PARITY.md:68-71` requires dry-run language to describe what would happen.

*Recommendation:* Represent planned outcomes separately from completed outcomes and emit only “would …” rows during dry-run across all four layers. Add assertions that no dry-run event claims an install, backup, registry update, or privileged write completed.

*Verifier:* Partly refuted, partly confirmed. The two copy rows ('installed: <dst>' / 'unchanged: <dst>') ARE a declared divergence with rationale — PARITY.md:70 and executor.rs:131-136 ('the narrative says "install: <path>" either way, only the plan distinguishes would copy from would skip') — plus a trailing '-- plan (dry run)' section (executor.rs:191-212, sabrage-cli/src/main.rs:751-753, 919-923) and privilege's own info row 'would prompt for administrator authorization' (privilege.rs:112-114, 368-372) give the reader dry-run context. Not declared anywhere, and contradicting PARITY.md:69 and design-core.md §361 ('an honest preview'), are the three non-copy rows: 'backed up stock DXMT -> …', 'ActiveRuntime registered' (a green OK for a wine child that was never spawned, immediately after a warn about a registry write that never happened), and 'host registration written' (a green OK three lines after 'would prompt for administrator authorization'). Rated medium rather than high: nothing on the machine is wrong, the mode is sabrage-only, and the trailing plan states the truth — but the run's persisted event log does contain OK rows claiming mutations that never occurred.

*Fix sketch:* install.rs: pick the verb from ctx.executor.is_dry_run() for the three non-copy rows, reusing the established vocabulary — 'would back up stock DXMT -> <path>' (layer 1), 'would register ActiveRuntime' (layer 3, and skip the lazy-flush warn under dry run since no reg add ran), and, for layer 4, add a third PrivilegedWrite variant (e.g. `Planned`) returned by privilege::plan_privileged_write so install.rs renders 'would write host registration' instead of reusing Written. Then add the row to PARITY.md's CLI/GUI table (or extend the existing :69 row) so the declared policy and the code agree.

*Regression test:* sabrage/crates/sabrage-core/src/stages/install.rs tests: update run_dry_runs_all_four_layers_in_order_without_touching_the_machine and layer_four_stages_the_host_manifest_file_form_byte_for_byte (which currently assert the completed wording at :843 and :962) to assert the 'would …' wording, and add a test that walks every StageEvent::Line emitted by run() under DryRunExecutor and fails if any text matches the completed-action deny-list ("backed up", "registered", "written", "installed:") without a leading 'would'.

*Cross-area files:* sabrage/PARITY.md

### A6-3 [high, conf 0.99] Layer 1 can preserve a corrupt backup and leave a hybrid live DXMT overlay
`sabrage/crates/sabrage-core/src/stages/install.rs:105-133` — **CONFIRMED** (re-rated low)

Source completeness is checked up front, but destination mutation is not transactional. `cp -R` can create `dxmt.stock-backup` and then fail or be cancelled; every retry treats the mere directory as a valid completed backup. The five live overlay files are then copied sequentially, so failure on a later file leaves CrossOver running a mixture of stock and forked components. The backup failure is also explicitly excluded from TCC classification, so an App Management-shaped failure can lack the intended remedy. Impact includes black-window/runtime failures and loss of a trustworthy rollback copy.
Evidence: `sabrage/crates/sabrage-core/src/stages/install.rs:105-111` checks only source presence; `:118-133` accepts `is_dir()` as backup completion, runs non-atomic `cp -R`, and copies live files one by one; `:122-125` explicitly says backup failures bypass the TCC path. The shell has the same non-atomic shape at `scripts/demo/install.sh:12-22`, but `sabrage/PARITY.md:7` states bug-for-bug parity is not required.

*Recommendation:* Build the backup in a fresh sibling directory, verify it against the source, then atomically rename it or write a validated completion marker. Stage and commit the overlay transactionally or roll back already-replaced files on failure. Route backup permission failures through the same TCC-aware diagnostic path.

*Verifier:* Confirmed as written, but the severity claim does not hold up. (1) This is a faithful port of the reference implementation — scripts/demo/install.sh:16-17 is `BK=...; if [ ! -d "$BK" ]; then cp -R ...` and its file loop is equally sequential — so it is not a native-introduced defect; CLAUDE.md makes the shell the spec and PARITY.md:7 only *permits* diverging. (2) The hybrid-overlay half is caught before it can black-window a launch: run's preflight blocks on the overlay files (PARITY.md's 'Native preflight blocks on ALL four overlay files' row; contract slug overlay.dxmt-d3d11 native_gate=block), and the failed install is a loud Fatal, with the remaining copies resumable because they are hash-gated. (3) The TCC half is no worse than the shell (which prints cp's stderr and dies with no remedy at all), and the tcc_denied 'GUI permission panel' the reviewer invokes has no consumer yet (`grep -rn 'tcc_denied|TccDenied' sabrage/ui/src sabrage/src-tauri/src` → no matches). What genuinely remains: a silently-incomplete stock backup is accepted forever, and the recovery it exists for (restore stock DXMT after a CrossOver update) would then restore a partial tree — recoverable by reinstalling CrossOver, so material but minor.

*Fix sketch:* install.rs layer 1: copy into a sibling `dxmt.stock-backup.partial-<uuid>` and only rename it into place after verifying it against the source tree (or write a `.complete` marker inside it), and treat an existing dxmt.stock-backup without that marker/verification as incomplete rather than done. For the diagnosis gap, have the dir_copy failure path check privilege::is_inside_app_bundle(&dxmt_backup) and, on a ChildFailed whose tail is a permission error, emit the same App Management Fatal/remedy privilege::upgrade_write_error produces. Record the resulting divergence from install.sh's `[ ! -d "$BK" ]` in PARITY.md.

*Regression test:* sabrage/crates/sabrage-core/src/stages/install.rs tests, next to dxmt_backup_present_is_reported_not_replanned: a test where dxmt.stock-backup exists but is missing files present in lib/dxmt (or lacks the completion marker) asserts the backup is re-planned (one PlannedKind::DirCopy) and no 'stock DXMT backup already exists' row is emitted; plus a test that a dir_copy failure whose destination is inside a .app bundle yields SabrageError::TccDenied with the APP_MANAGEMENT_SETTINGS_URL remedy.

*Cross-area files:* sabrage/PARITY.md

### A6-4 [high, conf 0.93] Install can report success immediately before launch rejects the registry state
`sabrage/crates/sabrage-core/src/stages/install.rs:179-223` — **CONFIRMED** (re-rated low)

After `reg add` succeeds, a missing lazy-flushed `system.reg` entry produces only a warning, followed by `OK ActiveRuntime registered`. The launch preflight gates on that exact file content. Inference: whenever the documented lazy-flush warning occurs, an immediate Launch—or the next stage of `sabrage all`—can reject the install that just reported success.
Evidence: `sabrage/crates/sabrage-core/src/stages/install.rs:211-223` warns when the entry is not visible and then unconditionally emits success; `scripts/demo/install.sh:38-42` documents the same lazy-flush observation; `scripts/demo/run.sh:28-30` immediately blocks when `system.reg` lacks the entry, and `contract/pipeline.toml:305-309` makes `bottle.registry` a native blocking gate.

*Recommendation:* After successful `reg add`, perform a bounded wait/re-probe or a safe registry flush before completing install. Preserve the required final Warn-never-Fail behavior, but make immediate launch/all retry the lazy state rather than contradicting the preceding success.

*Verifier:* The code path is exactly as described and reachable, but it is inherited by design, not a native defect: scripts/demo/install.sh:42-43 emits the identical warn-then-ok pair, and the contract makes bottle.registry a blocking gate on BOTH sides, so demo.sh has the same install→run hazard. The consequence is also transient and non-destructive: nothing on disk is wrong, the registry does flush when wineserver exits, the warn row already tells the user, and the blocking gate's remedy (re-run install) resolves it. Because the behaviour is contract-declared and symmetric, changing it is a shared pipeline change that must land in both implementations in the same commit (CLAUDE.md, 'Sabrage ⇄ demo.sh parity'), not a native-only fix — which is a further reason to rate it low rather than high.

*Fix sketch:* install.rs layer 3: after a successful `reg add`, re-probe system.reg on a short bounded poll (or spawn `wineserver -w` for the bottle to force the flush) before emitting the row; keep Warn-never-Fail as the terminal state when the entry still is not visible. Land the mirror change in scripts/demo/install.sh and, if the gate semantics move, in contract/pipeline.toml, in the same commit, then re-run scripts/dev/parity.sh.

*Regression test:* sabrage/crates/sabrage-core/src/stages/install.rs tests: drive run() over a fixture whose system.reg gains the ActiveRuntime line only after the first probe (a background writer or an injected clock) and assert no warn row is emitted and ok("ActiveRuntime registered") still fires; a second fixture where it never appears asserts the warn fires exactly once and the stage still returns Ok.

*Cross-area files:* scripts/demo/install.sh, contract/pipeline.toml

### A6-5 [medium, conf 0.99] Terminal-launched GUI runs announce the wrong authorization mechanism
`sabrage/crates/sabrage-core/src/privilege.rs:151-168` — **CONFIRMED** (re-rated low)

The code detects `Sudo` whenever a controlling terminal exists, including `cargo tauri dev`, but emits a method-agnostic NeedsAdmin event. The GUI renders that event as “macOS will ask for your password,” although sudo is actually waiting in the launching terminal—often behind the GUI—making the stage appear hung.
Evidence: `sabrage/crates/sabrage-core/src/privilege.rs:151-168` selects sudo from stdin or `/dev/tty`; `:364-378` discards that method when emitting NeedsAdmin; the module itself confirms the `tauri dev` sudo path at `:70-74`. `sabrage/ui/src/components/GateModal.svelte:400-406` unconditionally promises a macOS password prompt.

*Recommendation:* Carry the detected AdminMethod in the event and render sudo-versus-dialog instructions accurately, or make frontend context—not inherited tty state—select osascript for GUI-originated installs.

*Verifier:* Mechanically real and reachable, but only on the dev-build path (`npm run tauri dev`, or running the unbundled binary from a terminal); the shipped .app is unaffected, and the impact is a misleading UI note plus a stage that looks hung until the developer finds the sudo prompt in their terminal. No wrong bytes, no privileged-write hazard.

*Fix sketch:* Add `method: AdminMethod` (serde "sudo"/"osascript") to `StageEvent::NeedsAdmin` in sabrage-core/src/events.rs; set it from the already-computed `method` at privilege.rs:364 when emitting at :374-378 (make `AdminMethod` Serialize). GateModal.svelte's `needsAdmin` row (:400-407) branches on it: osascript -> current text; sudo -> "sudo is waiting for your password in the terminal that launched Sabrage". Extend the ipc.ts:136 event type; sabrage-cli/src/main.rs:612 keeps rendering `reason` only (its constructor at :1704 and events.rs:701 need the new field). Alternative (larger): let the Tauri frontend force `Osascript` for GUI-originated installs instead of inheriting tty state.

*Regression test:* Rust: a unit test in sabrage-core/src/privilege.rs asserting the emitted NeedsAdmin carries the same method `AdminMethod::detect()` returned (drive `write_host_manifest_privileged` through the existing recording-ctx harness used at privilege.rs:1018-1068, with a stale dest). Frontend: a GateModal test/story asserting the needsAdmin row text differs for method="sudo" vs "osascript" and that the sudo variant mentions the launching terminal.

*Cross-area files:* sabrage/crates/sabrage-core/src/events.rs, sabrage/crates/sabrage-cli/src/main.rs, sabrage/ui/src/ipc.ts, sabrage/ui/src/components/GateModal.svelte

*Codex next steps:* Run a stale-manifest install under dry-run in both CLI and GUI and assert that no event uses completed-action wording. · Fault-inject cancellation and nth-operation failures into backup/overlay installation; verify no partial backup is accepted and no hybrid live overlay remains. · Use an injectable delayed/forking elevation runner to cancel during authorization, then verify all descendants exit before the staged file is removed and no post-cancel destination write occurs. · Launch `cargo tauri dev` from a terminal with a stale host manifest and compare the GateModal message with the actual sudo prompt location. · Exercise install followed immediately by run/all with a deliberately delayed `system.reg` flush and verify launch does not reject the just-completed registry update.

## A7 — run-preflight-actions

**Codex verdict:** needs-attention — Do not ship: launch refusal, runtime configuration, crash recovery, and external-game-path handling all have concrete correctness failures. Review was read-only; no builds or tests were run.

### A7-1 [high, conf 0.99] Live-session refusal occurs after permanent autofixes
`sabrage/crates/sabrage-core/src/stages/run/preflight.rs:600-684` — **CONFIRMED** (re-rated medium)

A second Launch can rewrite `cxbottle.conf` or restage the helper before discovering a recorded Live session and refusing. The backend variant deliberately edits despite a live wineserver because it assumes `wineserver-reset` follows, but the Live return prevents that reset; the supposedly refused launch therefore changes disk state and may race CrossOver's in-memory configuration.
Evidence: `preflight.rs:600-605` labels both autofixes permanent, and `preflight.rs:620,677-684` applies the non-refusing launch variant. The caller executes preflight before reconciliation at `stages/run/mod.rs:132-143`, contradicting `PARITY.md:101`, which promises reconciliation before anything permanent.

*Recommendation:* Move reconciliation immediately after the read-only bottle resolution and before preflight, or split preflight evaluation from its mutations and apply fixes only after Live has been ruled out. Add a Live-session regression test with stale backend/helper state and assert zero executor mutations.

*Verifier:* The ordering is unambiguous and executed on every launch; the mutation is real and unwound by nothing (run/mod.rs:17-25 'permanent and never unwound'). It is not high because both autofixes are no-ops unless their check actually fails, which on a live, previously-launched session is the unusual state.

*Fix sketch:* In stages/run/mod.rs::run, move the `session::reconcile::reconcile` block (mod.rs:142-143) to immediately after `require_bottle`/`phase.publish(Preflight)` and before `preflight::run`; keep the checkpoint between them. Nothing in reconcile depends on PreflightFacts (it takes only ctx), so the move is mechanical. Alternative if the row order must stay: split preflight::run into an evaluate pass that records pending FixActions and an apply pass invoked after the Live refusal — larger, and it changes the interleaving of Check/AutoFixed rows, so prefer the move. Leave PARITY.md:101 as written (the move makes it true).

*Regression test:* sabrage/crates/sabrage-core/src/stages/run/mod.rs tests (or a new tests/run_live_refusal.rs): fixture bottle with `"CX_GRAPHICS_BACKEND" = "auto"` in cxbottle.conf plus a session-state.json describing a live session (this test process's pid + observed start time); assert stages::run::run returns the already-running Fatal AND cxbottle.conf is byte-identical to the fixture (and, with a DryRunExecutor-style recorder, that zero mutating executor calls were made).

*Cross-area files:* sabrage/crates/sabrage-core/src/stages/run/mod.rs

### A7-2 [high, conf 0.99] Preflight validates different TOML assignments than the runtime uses
`sabrage/crates/sabrage-core/src/stages/run/preflight.rs:113-147` — **CONFIRMED** (re-rated medium)

`protocol` and `encoder_process` use the first physical assignment, while the runtime uses the last accepted assignment regardless of table. For example, `protocol="alvr"` followed by `protocol="oxrsys"` passes native ALVR gates and drives ALVR cleanup/guards, but the launched runtime selects the unsupported legacy backend. Likewise, first `inproc` then `native` skips helper validation even though the runtime requires it. This can produce a black window or an unsupervised backend.
Evidence: `preflight.rs:113-124` returns on the first matching line and `preflight.rs:138-146` uses those values as launch facts; shell parity comes from the same first-match `awk` at `run.sh:57,70`. The actual consumer is documented as table-blind and last-wins at `config/runtime_toml.rs:13-28`, confirmed by the full-file loop and repeated assignments in `ext/oxrsys/runtime/src/Config.cpp:309-326,407-413,435-441`.

*Recommendation:* Parse effective launch facts with the runtime's line-oriented, last-valid-assignment semantics and update `run.sh` to match; add duplicate-key tests across table boundaries for both protocol and encoder process.

*Verifier:* Real and demonstrated, but it takes a hand-edited file with a duplicate assignment of protocol/encoder_process — the deployed toml is written once from the template with one of each. Latent, hence medium rather than high.

*Fix sketch:* Replace preflight.rs::awk_first_quoted with runtime semantics: strip quote-aware '#' comments, skip blank/'[' lines, split on the first '=', keep the LAST assignment whose value is in the accepted set — i.e. reuse config::runtime_toml::read_lines_like_the_runtime (runtime_toml.rs:473) and derive TomlFacts from its RuntimeConfigValues, keeping the ${:-auto} default. Do the same for the two sibling copies (checks::config::parse_protocol, fixes::helper::parse_encoder_process, helper.rs:69-91) so doctor and the restage die-text agree. Same commit must change run.sh:57 and :70 to a last-valid-assignment awk (e.g. `awk -F'"' '/^[[:space:]]*protocol[[:space:]]*=/{v=$2} END{print v}'` restricted to accepted values) and re-run scripts/dev/parity.sh --bless for the shell fingerprint.

*Regression test:* sabrage/crates/sabrage-core/src/stages/run/preflight.rs unit tests next to awk_first_quoted_matches_the_shell_recipe (preflight.rs:793): a table-crossing duplicate fixture asserting the preflight facts equal config::runtime_toml::read(...).values for both keys, an invalid-later-value fixture asserting the earlier valid value survives, and a sabrage-parity case asserting run.sh's awk yields the same string for the same fixture.

*Cross-area files:* scripts/demo/run.sh, sabrage/crates/sabrage-core/src/checks/config.rs, sabrage/crates/sabrage-core/src/fixes/helper.rs, sabrage/crates/sabrage-core/src/config/runtime_toml.rs, sabrage/crates/sabrage-parity

### A7-3 [high, conf 0.98] Missing Z: drive is ignored before launching an external game path
`sabrage/crates/sabrage-core/src/stages/run/actions.rs:463-490` — **REFUTED** (re-rated low)

When `bs_dir` is outside the bottle's `drive_c`, the launch unconditionally hands Wine a `Z:` path. `game.present` can pass on the host while `bottle.zdrive` is excluded from preflight, so a bottle without `dosdevices/z:` reaches Wine only to fail resolving the executable—after wineserver reset and Goldberg staging have already run.
Evidence: `actions.rs:463-490` constructs `--cx-app` from `win_path`; `lib.sh:121-127` specifies that every outside path becomes `Z:`. Yet `contract/pipeline.toml:109-114` gates `bottle.zdrive` as `none`, despite its evaluator explicitly failing this configuration at `checks/bottle.rs:154-174`.

*Recommendation:* Make `bottle.zdrive` a launch-blocking gate whenever `bs_dir` is outside `drive_c`, in both native and shell pipelines, and stop before any permanent preparation action.

*Verifier:* No native-side defect and no divergence from the shell reference: the missing preflight is a symmetric, contract-declared decision with a doctor row covering it. Adding the gate is a pipeline feature request (contract + run.sh + native, one commit), not a bug in this area's code.

### A7-4 [medium, conf 0.99] Wired forwards are mutated before crash-recovery state is saved
`sabrage/crates/sabrage-core/src/stages/run/actions.rs:146-178` — **CONFIRMED** (re-rated low)

The two forwards are returned to the caller only after both commands complete, and only then are they persisted. A cancellation or executor error on the second command takes the `?` return path and bypasses rollback, while a crash after either successful command leaves no state record. The surviving forward can silently break later WiFi discovery and reconciliation cannot identify it.
Evidence: `actions.rs:146-178` mutates each port, but rollback exists only for a non-success exit status; errors from `run_child(...).await?` return immediately. The caller saves the returned vector afterward at `stages/run/mod.rs:158-164`, contrary to the write-before-mutate requirement and “recorded as they are made” rule in `session/state.rs:14-30`. `run.sh:106-110` only covers ordinary nonzero exits, not Sabrage's cancellation/crash-recovery contract.

*Recommendation:* Persist the intended per-serial forwards before creating the first one, then update/clear the record after rollback. Also route every executor error through rollback rather than using `?` inside the loop.

*Verifier:* Reproducible on a realistic gesture (Ctrl-C / Stop during a --wired launch), but the blast radius is one leftover forward: the very next non-wired launch removes exactly tcp:9943+9944 per-serial (actions.rs:116-129 / run.sh:113-124) and doctor WARNs about them, so it self-heals without user action. Latent record-keeping gap rather than user-visible breakage — low, not medium.

*Fix sketch:* In actions::adb_forward_hygiene, take a `&mut SessionState` + state path (or a callback) and persist the intended WiredForward for a port BEFORE running its `adb forward`, clearing the record after a successful rollback; and wrap the loop body so every error path — non-success status AND `run_child` Err — goes through one `rollback(&specs)` helper before returning (rollback must run on a fresh, non-cancelled executor, like teardown_ctx in run/mod.rs, or it is a no-op under cancellation). Caller mod.rs:158-164 then only needs the final save.

*Regression test:* sabrage/crates/sabrage-core/src/stages/run/actions.rs tests: (a) fake adb that sleeps on the second port + a token cancelled mid-flight — assert session-state.json already lists tcp:9943 when the Err returns, and that `forward --remove tcp:9943` was invoked; (b) fake adb that exits 127 for the second port — assert both removes are invoked (existing behaviour) and the record is cleared.

*Cross-area files:* sabrage/crates/sabrage-core/src/stages/run/mod.rs, sabrage/crates/sabrage-core/src/session/state.rs

### A7-5 [medium, conf 0.98] An already-Goldberg DLL is recorded as the original Steam DLL
`sabrage/crates/sabrage-core/src/stages/run/actions.rs:329-358` — **CONFIRMED** (re-rated medium)

If the live DLL already equals the configured Goldberg DLL and `.orig-steam` is absent, the action first copies Goldberg into `.orig-steam` and only afterward detects “goldberg already installed.” The Sabrage revert action later copies those same bytes back and reports that it restored the original, leaving the user on Goldberg under a false success message.
Evidence: `actions.rs:329-342` creates the backup before the comparison at `actions.rs:344-347`. The shell has the same ordering at `run.sh:147-149`, but native uniquely exposes a revert operation that claims restoration at `store/goldberg.rs:86-95`; bug-for-bug parity is not required.

*Recommendation:* Never synthesize an “original” backup when the current DLL is already the configured Goldberg payload. Track validated provenance or mark the original unavailable, and make revert refuse rather than claim success for an unverified backup.

*Verifier:* Reachable without exotic state: a hand-installed/copied Goldberg dll, or a user who deleted `.orig-steam` as junk, both leave live-dll == Goldberg with no backup. The first launch then mints a backup of Goldberg bytes, which is never refreshed (`if !backup.exists()`), and revert reports success while leaving the user on Goldberg. Not critical: in that state the real Steam dll was already gone before Sabrage touched anything, so nothing is destroyed by this code — the harm is a false success message plus a permanently poisoned backup.

*Fix sketch:* Harden the Sabrage-only revert rather than reorder the shell-parity artifact writes (creating/not creating `.orig-steam` is an artifact-byte difference from run.sh:147). In `store::goldberg::revert_original_steam_dll`, after the `backup.is_file()` test, compare the backup against the configured Goldberg payload (`util::cmp_files(&paths.gbe_dll, &backup)` — the fn needs the Paths/gbe path passed in, or a `contract().deps.gbe_dll_sha256` hash test like store/library.rs:373 already does) and return `RevertReport { restored: false, message: "the .orig-steam backup at … is itself the Goldberg dll — Sabrage never saw the real Steam steam_api64.dll on this machine, so there is nothing to restore" }` instead of copying. Optionally add an `st.warn` in `actions::goldberg_stage` when it mints a backup whose bytes already equal `ctx.paths.gbe_dll` (rows are not artifacts, so no parity impact), and give `store::library::validate` a state for "backup present but is Goldberg".

*Regression test:* sabrage/crates/sabrage-core/src/store/goldberg.rs `mod tests`, next to `a_present_backup_is_restored_and_left_in_place`: a case that writes live dll == backup == the configured Goldberg bytes and asserts `!report.restored`, a says-why message, and that the live dll is untouched. Plus a case in sabrage/crates/sabrage-core/src/stages/run/actions.rs tests asserting the goldberg_stage-then-revert round trip on an already-Goldberg install never claims a restore (the exact sequence reproduced above).

*Cross-area files:* sabrage/crates/sabrage-core/src/store/goldberg.rs, sabrage/crates/sabrage-core/src/store/library.rs

### A7-6 [medium, conf 0.97] Autofix errors bypass the promised Check and Fatal events
`sabrage/crates/sabrage-core/src/stages/run/preflight.rs:619-653` — **CONFIRMED** (re-rated low)

Any error returned while applying an autofix exits through `?` before emitting the slug's final `Check`. Backend write failures also propagate as raw I/O errors rather than the shell's `could not force graphics backend...` Fatal, so event-only consumers receive only a failed `StageFinished` and can leave the check unresolved without an actionable reason. Helper copy failures have the same missing-Check path.
Evidence: `preflight.rs:17-20` promises exactly one final `Check` per evaluated slug, but `preflight.rs:619-625` returns directly from `apply_fix(...).await?`; all Check/Fatal handling occurs later at `preflight.rs:646-653`. The backend write is a raw propagated error at `fixes/backend.rs:308-311`, whereas `run.sh:41-42` emits a specific die message.

*Recommendation:* Catch `apply_fix` errors, emit exactly one final failing Check, and convert executor failures to the appropriate run.sh-shaped Fatal while retaining the underlying I/O cause. Add failing-executor event-sequence tests for both autofixes.

*Verifier:* The documented invariant is genuinely broken — the failing slug's Check row silently vanishes from the GUI's gate list and from any event-only transcript — and it is reachable (a helper that was never built, an unwritable cxbottle.conf). Downgraded to low because both shipped consumers still show an actionable reason: ctx.fatal paths emit the run.sh-shaped Fatal event, and the Io paths surface `<path>: <errno>` through the invoke rejection / CLI stderr. The claim that consumers 'receive only a failed StageFinished' is refuted, as is 'without an actionable reason'; the run.sh:41-42 die-text divergence is real but only for genuine write failures (permission denied / ENOSPC), where the Io message names the same file plus the cause.

*Fix sketch:* In `preflight::autofix`, replace the `?` at preflight.rs:620 with a match on `apply_fix(...)`: on Err(e), emit exactly one final `Check` for the slug (the pre-fix Fail outcome, `.with_detail(e.to_string())`), then return `SabrageError::Fatal` — for `SabrageError::Fatal` propagate unchanged (its Fatal event is already out), for the other variants surface the cause as a stderr-shaped `StageEvent::Output` and call `die(ctx, spec, post_fix_die(ctx, slug).0, remedy)` so run.sh's `could not force graphics backend to dxmt in <conf>` / `./demo.sh build` text is preserved and the io cause is kept — the same `die_with_cause` shape stages/run/actions.rs:392-410 already uses. No change needed in fixes/backend.rs or fixes/helper.rs.

*Regression test:* sabrage/crates/sabrage-core/src/stages/run/preflight.rs `mod tests`, beside `a_failing_backend_row_is_fixed_rechecked_and_reported_once` (preflight.rs:1377): same fixture with the bottle directory chmod'ed 0o555 so write_atomic fails; assert exactly one Check for `bottle.gfx-dxmt` with status Fail, exactly one StageEvent::Fatal whose message is run.sh's `could not force graphics backend to dxmt in <conf>`, an Output row carrying the io cause, and Err(Fatal). A twin case for a HELPER_SLUGS row whose restage errors, asserting its Check is emitted once alongside the existing Fatal.

*Codex next steps:* Exercise a recorded Live session with stale backend/helper fixtures and assert reconciliation refuses before any executor plan or filesystem change. · Run duplicate protocol/encoder assignments through both native preflight and `ParseConfigToml`, including assignments under different tables, and compare effective values. · Inject cancellation and executor failure after the first wired forward; verify both ports are removed and `session-state.json` always describes any surviving mutation. · Launch-test an outside-`drive_c` fixture with no `dosdevices/z:` and assert preflight blocks before wineserver or Goldberg actions. · Add regressions for an already-Goldberg DLL with no backup followed by Revert, plus permission-denied autofix paths and their emitted event order.

## A8 — run-supervise-guards-logs

**Codex verdict:** needs-attention — No ship: teardown can erase recovery state while audio remains unrestored, failure paths can strand live-session ownership, shell-launched sessions are reported as idle, and log tailing has both unbounded-memory and silent-data-loss paths.

### A8-1 [high, conf 0.99] Teardown deletes the record even when an audio guard remains pending
`sabrage/crates/sabrage-core/src/stages/run/mod.rs:571-584` — **CONFIRMED** (re-rated medium)

When the recorded output device and every fallback fail, `AudioGuard` intentionally leaves `audio_restored=false`, but every completed teardown unconditionally removes `session-state.json`. The Mac can remain on BlackHole with no machine-readable recovery path, directly defeating the fallback change's stated guarantee.
Evidence: `guards.rs:401-412` returns `Ok(false)` after `audio_unrestorable_line`, and `guards.rs:374-375` saves that false flag; nevertheless `mod.rs:571-584` executes `held.release(...)` followed by `clear_state(...)` without checking `sess.has_pending_guards()`. `sabrage/PARITY.md:102` requires that the record be "only cleared once every guard that was recorded is released."

*Recommendation:* Replace unconditional clearing in Normal, Cancelled, and Failed teardown with a shared `finish_record` policy: clear only when `!sess.has_pending_guards()`; otherwise save the pending state and emit an explicit retry message.

*Verifier:* Reachable, but only through the narrow window the fallback exists for: the recorded device must be gone AND `fallback_output_device` must find nothing non-virtual (guards.rs:405-410, session/mod.rs:319-325) — e.g. AirPods gone with only BlackHole/Virtual-Desktop outputs left. The user still gets the remedy row naming the device and the exact SwitchAudioSource command (session/mod.rs:354-360), so what is lost is the machine-readable retry, not all recovery. That is a documented-guarantee break and a real degradation, not a routine user-visible failure — medium, not high.

*Fix sketch:* Add a `finish_record`-style policy to `stages::run::teardown` (mod.rs) and route the Normal / Cancelled / Failed arms through it instead of bare `clear_state`: keep + save + emit a `record kept` row when a guard is still pending, clear otherwise. DO NOT use the reviewer's literal `!sess.has_pending_guards()` gate: `run` records `wired_forwards` (mod.rs:163) and NEVER sets `guards.forwards_cleared` (only reconcile.rs:629 does — removing the forwards is deliberately not part of run's teardown, CLAUDE.md's permanent-mutation boundary), so that gate would keep a stale record after EVERY `--wired` run. Gate on the guards teardown actually releases — `(prev_audio_output.is_some() && !guards.audio_restored) || (dashboard.is_some() && !guards.dashboard_closed)` — or mirror reconcile's `RestoreMode` split. Keep the Normal arm best-effort (#202: warn, never `?`).

*Regression test:* New unit test in `sabrage/crates/sabrage-core/src/stages/run/mod.rs`'s `mod tests`, next to `a_normal_exit_survives_a_failed_state_save`: wrap the dry executor in a `FailSwitchTo`-style decorator (the one at guards.rs:~980 for the SwitchAudioSource restore), hold an `AudioGuard::armed_for_test` for a disconnected device, run `teardown(... Reason::Normal ...)`, and assert (a) the planned kinds contain no `PlannedKind::RemoveFile` for `session_state_path()`, (b) `sess.has_pending_guards()` is still true, (c) the unrestorable remedy row was printed, and (d) rc is still wine's. Add the `--wired` counter-test: `sess.wired_forwards` non-empty with audio restored must still CLEAR the record.

*Cross-area files:* sabrage/PARITY.md

### A8-2 [high, conf 0.98] A fallible guard release can abort teardown before later guards and LIVE_SESSION cleanup
`sabrage/crates/sabrage-core/src/stages/run/mod.rs:297-304` — **CONFIRMED** (re-rated medium)

Guard release short-circuits on the first error. A dashboard state-save failure prevents async audio restoration; on cancellation it also skips child reaping, state finalization, and `clear_live_session`. Detach similarly disarms both guards before a fallible save, so a save failure leaves supervision gone but the live handle present. In the GUI, subsequent Stop/Detach actions target tokens with no supervisor and can wait until timeout.
Evidence: `mod.rs:297-304` uses `d.release(ctx, sess).await?` before reaching audio; `mod.rs:597-604` uses `held.release(...).await?` before `clear_live_session`, while `mod.rs:517-530` performs `held.disarm(); ... state::save(...).await?; ... clear_live_session(...)`. By contrast, the shell INT trap at `scripts/demo/run.sh:180` sequences all cleanup operations before re-signalling itself.

*Recommendation:* Attempt every guard release independently, retain the first cleanup error without short-circuiting, and clear the run-id-matched live handle in a finally-style scope. For detach, persist the detached state before disarming guards, and do not abandon supervision if persistence fails.

*Verifier:* Trigger is a `session-state.json` write failure (disk full, unwritable ~/Library/Application Support/Sabrage) — unusual, hence latent. But the consequence is precisely the #202 symptom left unfixed on two arms: LIVE_SESSION keeps a handle whose supervisor is gone, so the next Stop/quit burns the 30 s timeout on an already-fired token, the record is left describing an exited session, and a cancel returns `Io` (exit 1) instead of `Cancelled` (exit 130), diverging from run.sh's INT trap, which completes `stop_wine; stop_dashboard; stop_helper; restore_audio` before re-signalling (scripts/demo/run.sh:180).

*Fix sketch:* In `teardown`: make the Cancelled arm best-effort like Normal — `if let Err(e) = held.release(...).await { warn }`, then always reap the child, `let _ = clear_state(...)` (or the new finish_record from A8-1), then `clear_live_session`, then `Err(SabrageError::Cancelled)`. Same for `Reason::DryRun` (mod.rs:611). In the Detached arm, persist first and disarm second: `sess.detached = true; let saved = state::save(...).await;` then `held.disarm()` only if the save succeeded (a failed save should keep supervision armed and surface a warn), and run `clear_live_session` unconditionally. Optionally hoist `session::clear_live_session(ctx.run_id)` into a scope guard so no arm can skip it. In `Guards::release`, `take()` and attempt every guard, keeping the first error instead of `?`-ing out.

*Regression test:* In `sabrage/crates/sabrage-core/src/stages/run/mod.rs` tests, clone `a_normal_exit_survives_a_failed_state_save` twice with the same `DenyWriteTo` executor: (1) `Reason::Cancelled { child: None }` — assert the returned error is `SabrageError::Cancelled` (exit 130), that `session::live_session()` is `None`, and that the warn names session-state.json; (2) `Reason::Detached { log }` — assert it returns `Ok(0)`, `live_session()` is `None`, and that a failed save did not silently leave a disarmed-but-unrecorded session (guards still armed or an explicit warn row).

### A8-3 [medium, conf 0.91] AudioGuard is persisted but not armed during the actual device mutation
`sabrage/crates/sabrage-core/src/stages/run/guards.rs:283-295` — **CONFIRMED** (re-rated medium)

There is a cancellation window after the previous device is saved but before the RAII guard records it. If `SwitchAudioSource` changes the output and cancellation wins before `run_child` returns, `acquire` returns `Cancelled`; its local guard has `previous_output=None`, the outer `held.audio` was never assigned, and teardown can remove the only recovery record. This conclusion depends on the explicit cancellation race in the child runner, but the mutation-before-status ordering makes the outcome plausible.
Evidence: `guards.rs:283-295` saves `state.prev_audio_output`, awaits the switch, and only then executes `guard.previous_output = Some(previous.clone())`; `mod.rs:356` assigns the returned guard only after `acquire(...).await?` completes. `process.rs:357-390` returns `SabrageError::Cancelled` whenever the cancellation branch wins, after signalling and waiting for the child.

*Recommendation:* Arm `guard.previous_output` immediately after the durable save and before spawning `SwitchAudioSource`. Treat cancellation or an otherwise ambiguous child error as potentially mutated state: restore through the armed guard or retain the pending record.

*Verifier:* Real but narrow: the cancel must land inside the few-ms window in which SwitchAudioSource has applied the switch and not yet been waited on. The outcome when it does is the worst one this file is designed to prevent — Mac on BlackHole 2ch, no in-memory guard, no on-disk record, and no printed remedy (the `audio: default output -> …` row is inside the same success branch). Cancellation during launch is an ordinary user action (Ctrl-C / Stop), so it is not unreachable, just timing-dependent.

*Fix sketch:* In `AudioGuard::acquire` (guards.rs), arm the guard immediately after the durable save and before spawning the switch: move `guard.previous_output = Some(previous.clone())` above the `run_child(&switch)` call, and clear it again (`guard.previous_output = None`) in the existing failure branch alongside `state.prev_audio_output = None` (guards.rs:307-315). That makes the designed `Drop` fallback (guards.rs:415-450) cover the cancelled early return. For a fully async undo, additionally stop `acquire` from swallowing the guard on error — either install it into `Guards` before mutating (pass `&mut Guards` instead of returning `Self`) or return `(AudioGuard, Result<()>)` so `guarded` can assign `held.audio` and let `teardown` run the normal `release`. Pair with A8-1 so a cancelled-mid-switch record is kept rather than cleared.

*Regression test:* In `sabrage/crates/sabrage-core/src/stages/run/guards.rs` tests (next to the FailSwitchTo cases): an executor decorator whose `run_child` returns `Err(SabrageError::Cancelled)` for the `-s BlackHole 2ch` spec only. Assert `AudioGuard::acquire` leaves a recoverable state — either it returns a guard whose `previous_output` is armed, or (minimal fix) that dropping the returned error path issues the synchronous restore — and that `state.prev_audio_output` is still `Some(previous)`. Add a `teardown` test asserting the record survives a cancel raised from inside `acquire`.

### A8-4 [medium, conf 0.97] The guarded audio action permanently overwrites BlackHole's volume
`sabrage/crates/sabrage-core/src/stages/run/guards.rs:301-310` — **REFUTED** (re-rated low)

The launch forces the per-device BlackHole volume to 100 but records and restores only the default output device. Users who configured BlackHole for another workflow permanently lose that setting, despite this mutation occurring after the guarded boundary. The shell has the same bug, but declared policy requires behavior parity rather than bug-for-bug parity.
Evidence: `guards.rs:301-310` runs `osascript -e "set volume output volume 100"`; restoration at `guards.rs:368-375` only switches output and saves `audio_restored`. The shell likewise mutates at `scripts/demo/run.sh:186-192`, while `restore_audio` at `scripts/demo/run.sh:157-162` restores only the device. `contract/pipeline.toml:413-414` describes the audio action, including volume 100, as guarded and restored on exit.

*Recommendation:* Probe and persist BlackHole's prior volume before setting it to 100, then restore that volume before switching back to the previous output. Apply the behavior to both native and shell launch paths, or explicitly declare a justified divergence.

*Verifier:* Not a Sabrage defect and not a parity violation. PARITY.md's rule is 'Bug-for-bug parity is NOT required' — permission to diverge, not a requirement to; and CLAUDE.md's both-sides-same-commit rule means a native-only change here would create an UNDECLARED divergence, the opposite of what the finding asks for. The residual effect is confined to the BlackHole 2ch virtual device's own output volume, which is exactly what the loopback path needs at 100 and is not a device a user listens on. If the project wants it, it is a shared enhancement (probe + persist the BlackHole volume in SessionState, restore it in both implementations plus the contract text), not a bug in this area.

### A8-5 [high, conf 0.99] A supported demo.sh session is falsely reported as idle
`sabrage/crates/sabrage-core/src/stages/run/mod.rs:388-405` — **CONFIRMED** (re-rated high)

Only a native run publishes `LIVE_SESSION`; `demo.sh run` writes a log and waits but creates no compatible session record. The monitor consequently leaves phase at Idle even when `runtime_status.json` is fresh and streaming, and the Session screen tells the user "No session running." That violates the honest-state rule and can invite a second launch over an active game.
Evidence: `mod.rs:388-405` creates and publishes the in-process `LiveSessionHandle` only inside native `guarded`; the shell path at `scripts/demo/run.sh:249-266` merely creates the log, spawns wine, and waits. `session/watcher.rs:278-310` derives Running only from `live_session()` or `session-state.json`, otherwise retaining Idle; runtime freshness at `watcher.rs:313-325` does not change that phase. The UI renders Idle as "No session running" at `Session.svelte:291` and `Session.svelte:405-406`.

*Recommendation:* Add an explicit external-session state derived conservatively from fresh streaming telemetry plus a verified Wine/runtime process, or make every supported launch route publish a shared durable identity. Never map a positively observed active stream to Idle.

*Verifier:* Both front-ends are first-class (CLAUDE.md "both front-ends stay alive"), so `./demo.sh run` in a terminal with Sabrage open is a realistic path, and on it the Session screen states the opposite of the observable truth (fresh streaming runtime_status.json) while disabling Stop.

*Fix sketch:* In session/watcher.rs::SessionMonitor::snapshot, after the runtime_status block, add a conservative external branch when base == Base::None: if runtime_status parses, is fresh, and names a live process (crate::process::is_alive on its `process_id`, which oxrsys already writes — see the deployed runtime_status.json), set a new SessionPhase::External (or Running with owned_by_this_process = false and pid from the status file) instead of Idle. Never derive it from file freshness alone. Then in Session.svelte let hasSession/canStop treat that phase as a session (Stop already works recordless — stop.rs:107), and reuse the existing "running outside this Sabrage instance" banner (Session.svelte:162). Optionally have stages::run::preflight refuse a launch on that same signal, mirroring PARITY.md:101's recorded-Live refusal.

*Regression test:* sabrage/crates/sabrage-core/src/session/watcher.rs #[cfg(test)] module (next to the existing snapshot tests): a monitor with no live handle and no session-state.json, whose oxr_appsup contains a fresh `state:"streaming"` runtime_status.json naming this test process's own pid, must NOT report Idle and must report owned_by_this_process == false; a stale (>3 s) or dead-pid status must still report Idle. Plus a UI test/story asserting the external phase shows the banner and enables Stop.

*Cross-area files:* sabrage/crates/sabrage-core/src/session/watcher.rs, sabrage/ui/src/screens/Session.svelte, sabrage/crates/sabrage-core/src/session/mod.rs, sabrage/PARITY.md

### A8-6 [high, conf 0.99] The line cap does not bound Tailer's memory or I/O
`sabrage/crates/sabrage-core/src/logs.rs:349-365` — **CONFIRMED** (re-rated medium)

Each poll reads all bytes from the current offset to EOF into one `Vec`, then splits every line into `pending`; the 2,000-line cap is applied only afterward. Opening an existing Wine console log from offset zero can therefore allocate and parse hundreds of megabytes or more in one poll, exhausting memory or stalling the app despite the documented cap.
Evidence: `logs.rs:349-365` performs `file.read_to_end(&mut buf)`, pushes the entire buffer through `ChunkSplitter`, and only then calls `drain_capped()`. `logs.rs:168-176` claims the cap prevents one burst from blocking the UI, but its only byte cap applies to from-end preload, not ordinary polls. `commands.rs:1116-1121` enables from-end mode only for `AlvrSession`, so WineConsole and OxrsysRuntime initially take the unbounded path.

*Recommendation:* Read fixed-size chunks with a per-poll byte budget, stop once the pending line budget is reached, and retain the cursor/splitter for the next poll. Also bound a single unterminated line so it cannot grow without limit.

*Verifier:* Downgraded from high to medium: the code path is genuinely unbounded and reachable (open the Logs pane on a past `--verbose` run), but every source Sabrage tails from offset 0 is bounded in the default configuration (51 KB wine logs, 5 MiB self-rotating oxrsys log), and the read happens off the UI thread — so "exhausts memory or stalls the app" is latent, not the normal case.

*Fix sketch:* In Tailer::poll (logs.rs:344-370): replace `file.read_to_end` with a bounded loop — read at most POLL_BYTE_BUDGET (e.g. 1 MiB) per poll via a fixed buffer, advance self.offset by exactly what was consumed, push into the splitter, and return early once `pending.len() >= MAX_LINES_PER_POLL` (the cursor + splitter already survive to the next poll, and LogBatch::truncated already tells the caller more is queued, so the 250 ms cadence drains the backlog). Additionally cap the splitter's unterminated-line buffer (emit/flush a synthetic break past e.g. 1 MiB) so one newline-less file cannot grow it without limit. Consider also opening WineConsole/OxrsysRuntime with from_end when the file already exceeds the preload cap.

*Regression test:* sabrage/crates/sabrage-core/src/logs.rs tests module, beside the existing Tailer tests: write a file far larger than the byte budget, poll once, and assert the tailer consumed at most POLL_BYTE_BUDGET bytes (self.offset <= budget, e.g. via a public accessor or by asserting that the total lines delivered across N polls only reaches the file's line count after ceil(size/budget) polls) while still eventually delivering every line in order; plus a test that a single 2 MiB line with no newline does not grow the splitter past its cap.

*Cross-area files:* sabrage/src-tauri/src/commands.rs

### A8-7 [medium, conf 0.98] Same-inode truncate-and-regrow silently loses the new log prefix
`sabrage/crates/sabrage-core/src/logs.rs:303-312` — **CONFIRMED** (re-rated low)

Rotation is detected only when the inode changes or the observed length is below the previous offset. ALVR opens `session_log.txt` with in-place truncation; if it writes back to the old offset or beyond between 250 ms polls, the inode is unchanged and `len < offset` is false. The tailer seeks to the stale offset, misses the beginning of the new session, and never reports rotation, mixing sessions in the UI.
Evidence: `logs.rs:303-312` defines rotation as `id != current_id || len < self.offset`; `logs.rs:353-358` then seeks to `self.offset`. The actual ALVR writer uses `.truncate(true)` at `ext/ALVR/alvr/server_core/src/logging_backend.rs:69-76`, preserving the inode, and the command polls every 250 ms at `commands.rs:1052-1053`.

*Recommendation:* Detect in-place rewrites even after regrowth, for example by retaining and rechecking a small byte signature immediately before the prior offset, using a filesystem truncate notification, or changing the writer to rotate via rename/recreate. Add a truncate-then-regrow-to-equal-and-larger-size regression test.

*Verifier:* Downgraded to low: real defect, deterministic once the race is hit, but it needs a narrow 250 ms window plus a prior offset small enough to be overtaken, and the consequence is cosmetic — a log pane that mixes two sessions and misses a prefix. No state corruption, no wrong pipeline action.

*Fix sketch:* In Tailer::poll, before trusting self.offset, verify continuity: keep a small signature (e.g. the last 64 bytes read, or a rolling hash of them, captured at each poll) and re-read those bytes at offset-len(sig); a mismatch means the file was rewritten in place — treat it exactly like the `len < self.offset` branch (reopen, offset = 0, splitter reset, rotated = true). Cheap enough at 250 ms cadence. Alternatively track (mtime, len) pairs and treat a shrink-then-grow between polls as rotation.

*Regression test:* sabrage/crates/sabrage-core/src/logs.rs tests module: `truncate_and_regrow_to_a_larger_size_between_polls_reports_rotation` — write N bytes, open from_end, truncate in place and write >N different bytes, poll once, assert batch.rotated == true and batch.lines starts with the new file's FIRST line; a sibling case regrowing to exactly N asserts the same.

*Codex next steps:* Fault-inject an unrestorable audio device through full Normal, Cancelled, and Failed teardown; verify `session-state.json` remains and the next reconcile retries it. · Inject dashboard/state-save failures during cancellation and detach; verify every guard is attempted, the live handle is always cleared, and no Stop action waits on an orphaned token. · Use a controllable `SwitchAudioSource` test double that mutates and then blocks while cancellation fires; verify the armed guard restores output and BlackHole's previous volume round-trips. · Run `scripts/demo/run.sh` while the GUI monitor is active and assert the Session screen reports an external active session rather than Idle and prevents an unsafe second launch. · Exercise Tailer with a very large Wine log under a memory/time budget and with ALVR-style same-inode truncate-and-regrow at equal and larger sizes; verify bounded polling and complete rotated output.

## A9 — session-reconcile-telemetry

**Codex verdict:** needs-attention — Do not ship: reconciliation can mutate an in-flight launch or lose the only crash-recovery record, while telemetry can attribute stale data to the current session. Nine material correctness and lifecycle risks remain.

### A9-1 [high, conf 0.99] Reconcile can tear down guards belonging to a launch still in progress
`sabrage/crates/sabrage-core/src/session/reconcile.rs:223-225` — **CONFIRMED** (re-rated high)

Before wine is spawned, a valid record has `wine: None`; reconciliation therefore calls it Dead. Its only ownership exemption is `LIVE_SESSION`, which is not published until after spawn, and the GUI reconciliation command does not take the operation lock. Inference: remounting the Session screen during Launching can restore audio, terminate the newly spawned dashboard, remove forwards, and clear the active launch's record.
Evidence: `let live = crate::session::live_session().map(|h| h.run_id);` is the sole ambient guard (`sabrage/crates/sabrage-core/src/session/reconcile.rs:223-225`), while `let Some(wine) = state.wine.as_ref() else { return Classification::Dead; }` (`reconcile.rs:196-199`). The record explicitly covers the pre-spawn window (`state.rs:97-105`), and the GUI directly awaits `reconcile(&ctx)` without `OPERATION_LOCK` (`sabrage/src-tauri/src/commands.rs:1030-1043`). Audio is persisted before mutation at `stages/run/guards.rs:283-294`, matching the shell guard boundary at `scripts/demo/run.sh:156-181`.

*Recommendation:* Serialize external reconciliation with `OPERATION_LOCK` using a holding-lock variant for the internal run call, and refuse mutation when a matching `RUN_PHASE` or verified owner identity is Preflight, Launching, or Stopping.

*Verifier:* The pre-spawn window is deliberately covered by the record (state.rs:97-105) but nothing marks it as owned: classify() sees wine:None as Dead, LIVE_SESSION is only published after the spawn, RUN_PHASE (session/mod.rs:242-258) is published for exactly that window but reconcile never consults it, and the Tauri reconcile command takes no lock.

*Fix sketch:* Give `reconcile_with` a third injected ambient input — `session::run_phase()` — and treat a published Preflight/Launching/Stopping phase the same way a matching live_run_id is treated: report the classification, restore nothing, keep the file (a new `Reconciled::Busy`, or reuse the report-only arm). Additionally have `commands::reconcile_session` take `stages::acquire_operation_lock()` for the call, with a `reconcile_holding_lock` variant used by `stages::run::run` (which already holds it at reconcile.rs's call site, mod.rs:141). Belt-and-braces: refuse to touch a record whose `owner_pid` is a live foreign process (see A9-3).

*Regression test:* sabrage/crates/sabrage-core/src/session/reconcile.rs tests module (next to `a_record_this_process_still_supervises_is_reported_but_never_touched`): save a pending record with wine:None, dashboard Some, prev_audio_output Some; publish a Launching RunPhaseInfo under `lock_session_globals()`; assert reconcile returns the report-only variant, `ctx.executor.planned()` is empty (no kill, no write, no RemoveFile) and the file still exists. Plus a sabrage/src-tauri test that `reconcile_session` blocks while OPERATION_LOCK is held (mirroring fixes/mod.rs:387's `apply_waits_for_the_operation_lock_then_proceeds`).

*Cross-area files:* sabrage/src-tauri/src/commands.rs, sabrage/crates/sabrage-core/src/stages/run/mod.rs, sabrage/crates/sabrage-core/src/stages/mod.rs

### A9-2 [high, conf 0.99] A retained recovery record is overwritten by the launch that was supposed to retry it
`sabrage/crates/sabrage-core/src/session/reconcile.rs:668-679` — **CONFIRMED** (re-rated medium)

When audio or another guard cannot be restored, `finish_record` saves the pending record and returns ordinary `Reconciled::Dead`. The run path blocks only `Reconciled::Live`, creates a fresh `SessionState`, and eventually overwrites the same global file. A missing `SwitchAudioSource` therefore causes the exact previous output-device name to be lost on the next launch, leaving no later recovery path.
Evidence: pending work is saved and reported with `state::save(...); ... Ok(())` (`sabrage/crates/sabrage-core/src/session/reconcile.rs:674-679`), but the caller still receives `Ok(Reconciled::Dead { state, restored })` (`reconcile.rs:282-285`). The launch caller rejects only `Reconciled::Live` (`sabrage/crates/sabrage-core/src/stages/run/mod.rs:137-164`). This contradicts the declared behavior that the record is kept until every guard is restored (`sabrage/PARITY.md:101-102`).

*Recommendation:* Return an explicit pending-recovery outcome and block a new launch until it is resolved or the user explicitly abandons it; alternatively store recovery records per run so a new session cannot overwrite the unresolved one.

*Verifier:* Real and unambiguous, but the entry condition is the tail: the reconcile audio restore must have failed AND `fallback_output_device` must have found nothing real (reconcile.rs:533-566), or the probe must be unavailable (`which("SwitchAudioSource")` → None, reconcile.rs:450 — plausible for a Finder-launched .app, whose own PATH lacks /opt/homebrew/bin, reconciling a record the CLI wrote). Latent rather than everyday, so medium, not high.

*Fix sketch:* Make finish_record's kept-record case visible: return a distinct outcome (e.g. `Reconciled::Dead { state, restored, pending: true }` or a `PendingRecovery` variant) and have `stages::run::run` (mod.rs:141) refuse — or, better, carry the unresolved fields forward: when the pending record's `prev_audio_output` is still un-restored, seed the new `SessionState` with it (and keep its guard flag false) instead of starting from `SessionState::new`, so the next teardown restores the ORIGINAL device. Alternatively keep unresolved records under `<sabrage_appsup>/recovery/<run_id>.json` so a new session cannot overwrite one.

*Regression test:* sabrage/crates/sabrage-core/src/session/reconcile.rs tests: extend `with_nothing_to_fall_back_to_the_record_is_kept_with_the_remedy` to assert the pending outcome is distinguishable from a clean Dead. Plus a test in sabrage/crates/sabrage-core/src/stages/run/mod.rs's test module: with a kept record on disk carrying prev_audio_output="AirPods Pro", drive `run` far enough to write its own record and assert the device name survives (or that the launch refuses with the pending-recovery message).

*Cross-area files:* sabrage/crates/sabrage-core/src/stages/run/mod.rs, sabrage/crates/sabrage-core/src/stages/run/guards.rs, sabrage/PARITY.md

### A9-3 [high, conf 0.99] Atomic writes still allow cross-process last-writer-wins corruption
`sabrage/crates/sabrage-core/src/session/state.rs:193-206` — **CONFIRMED** (re-rated medium)

There is one application-wide path, but save and clear carry no expected run ID or owner check. Two frontends that simultaneously pass the no-record check can atomically replace each other's state; a late teardown can then delete the newer run's record. Atomic rename prevents torn JSON, not lost updates. The stored `owner_pid` contract is never enforced anywhere outside its declaration.
Evidence: `save` unconditionally serializes and calls `write_atomic(path, &bytes)`, while `clear` unconditionally calls `remove_file(path)` (`sabrage/crates/sabrage-core/src/session/state.rs:193-206`). The field documentation says a live foreign `owner_pid` means reconciliation “must not touch its guards” (`state.rs:97-101`), but repository-wide usage only initializes/tests that field. The sole path is `<sabrage_appsup>/session-state.json` (`sabrage/crates/sabrage-core/src/paths.rs:336-345`).

*Recommendation:* Add interprocess serialization plus compare-and-swap semantics: every save/clear must verify the expected `run_id` and owner identity under a file lock. Prefer per-run recovery journals if multiple bottles must coexist.

*Verifier:* Every factual claim checks out and the contract is genuinely unenforced; downgraded from high because the damaging path needs two Sabrage front-ends running concurrently (the in-process races are covered by OPERATION_LOCK and the UI busy state — except the unlocked reconcile command, which is A9-1).

*Fix sketch:* Add an `expected: Option<RunId>` (or `owner: ProcInfo`) parameter to `state::save`/`state::clear` and refuse when the on-disk record names a different run whose `owner_pid` is still alive; take an advisory flock on `<sabrage_appsup>/session-state.lock` around the read-modify-write in save/clear/reconcile so two processes serialize; and teach `reconcile` (and `finish_stopped_session`) to skip a record whose `owner_pid` is a live foreign process, which is what state.rs:97-101 already promises.

*Regression test:* sabrage/crates/sabrage-core/src/session/state.rs tests: `a_save_for_a_different_run_does_not_clobber_a_live_owners_record` — write record A (owner_pid = a live foreign pid, e.g. this process's parent), attempt `save`/`clear` for run B, assert Err/no-op and that A's bytes are unchanged. Plus a reconcile test asserting a record whose owner_pid is alive and not us is reported and left untouched.

*Cross-area files:* sabrage/crates/sabrage-core/src/stages/run/mod.rs, sabrage/crates/sabrage-core/src/stages/run/guards.rs, sabrage/crates/sabrage-core/src/paths.rs

### A9-4 [high, conf 0.99] A failed adb removal is nevertheless persisted as completed
`sabrage/crates/sabrage-core/src/session/reconcile.rs:607-630` — **CONFIRMED** (re-rated low)

Each non-zero `adb forward --remove` is skipped, but after the loop the single completion flag is set unconditionally. A transient failure can leave tcp:9943 or tcp:9944 installed while the record is cleared, silently breaking subsequent WiFi discovery with no retry metadata.
Evidence: failure executes `continue` (`sabrage/crates/sabrage-core/src/session/reconcile.rs:618-623`), followed unconditionally by `state.guards.forwards_cleared = true; state::save(...)` (`reconcile.rs:629-630`). The shell only reports a stale-forward removal when the command succeeds (`scripts/demo/run.sh:114-120`), and parity requires per-serial removal of exactly these ports (`sabrage/PARITY.md:116-118`).

*Recommendation:* Track outstanding forwards individually, remove only successful or authoritatively absent entries, and leave `forwards_cleared` false whenever any removal has an indeterminate failure.

*Verifier:* The literal claim is true and reproducible, but the consequence the finding rests on ('silently breaking subsequent WiFi discovery with no retry metadata') does not follow. Stale forwards are cleared record-independently by every non-wired launch, from `adb forward --list`, not from the record: actions.rs:116-128 → `fixes::adb::remove_adb_forwards_at`, the exact counterpart of run.sh:114-124 — and doctor WARNs when the forwards are present (CLAUDE.md's --wired note). The shell reference is also misread: run.sh's `&&` at line 119-120 gates only the info line, not any state, and run.sh:108 removes both ports 'ignoring every failure'. Finally the state the finding needs — removal fails while the forward is still installed — is close to unreachable: `adb forward --remove` fails when the device is gone or the listener is absent, and in both cases so is the forward. Bookkeeping inaccuracy, not user-visible behaviour: low.

*Fix sketch:* In `restore_with`'s forwards block (reconcile.rs:607-630), drop each successfully removed entry from `state.wired_forwards` and only set `guards.forwards_cleared = true` when the vector is empty; leave the flag false (and the record kept, which finish_record already honours) when any removal failed indeterminately.

*Regression test:* sabrage/crates/sabrage-core/src/session/reconcile.rs tests, beside `without_adb_the_forwards_stay_pending`: with an executor that fails `adb … forward --remove`, assert `!state.guards.forwards_cleared`, that the un-removed port is still listed in `state.wired_forwards`, and that the record file survives (no RemoveFile planned).

### A9-5 [medium, conf 0.98] An unverifiable live PID is treated as safe to dismantle
`sabrage/crates/sabrage-core/src/session/reconcile.rs:196-209` — **CONFIRMED** (re-rated low)

The documented spawn fallback records `start_time = 0`. For a still-live PID this becomes IdentityMismatch, which restores audio and removes wired forwards even though the process may be the actual session. SafeOnly prevents signalling a stranger but is not safe for a live, unverifiable wired session; it can disconnect streaming and then clear the record while ignoring its dashboard.
Evidence: a live PID that fails `is_same_process()` becomes `Classification::IdentityMismatch` (`sabrage/crates/sabrage-core/src/session/reconcile.rs:203-206`), including the explicitly documented zero-start-time fallback (`reconcile.rs:25-28`). That branch invokes `RestoreMode::SafeOnly` and `finish_record` (`reconcile.rs:287-290`), whose completion rule considers only audio and forwards (`reconcile.rs:648-657`).

*Recommendation:* Separate confirmed identity mismatch from unverifiable identity. While a PID with unknown start time remains alive, retain the record and mutate nothing unless ownership is established or the user confirms recovery.

*Verifier:* The reviewer read the branch correctly and the doc comment (reconcile.rs:25-28) confirms the conflation is deliberate; but the precondition (a live process invisible to ProcInfo::observe for 200 ms) is a defensive fallback that does not occur on any realistic launch, and the damage (audio switched back, two forwards removed, record dropped) is recoverable, not irreversible. Latent under conditions I could not produce -> low, not medium.

*Fix sketch:* In session/reconcile.rs: give classify() a fourth answer for `wine.start_time == 0 && process::is_alive(wine.pid)` (call it Unverifiable), and in reconcile_with/finish_stopped_session_inner treat it like Classification::Live: restore nothing, keep the record. Minimal shape reuses Reconciled::Live so no serialized variant is added and stages::run's existing Live refusal (stages/run/mod.rs:142) also covers it; a distinct Reconciled variant would be more honest but then ui/src/ipc.ts + ui/src/stores/session.svelte.ts must learn it.

*Regression test:* sabrage/crates/sabrage-core/src/session/reconcile.rs tests: a `a_live_pid_with_no_observed_start_time_is_never_dismantled` case writing a state whose wine = ProcInfo{pid: std::process::id(), start_time: 0}, with prev_audio_output + wired_forwards set, asserting reconcile_with plans no SwitchAudioSource and no `adb forward --remove`, and that session_state_path() still exists afterwards.

### A9-6 [high, conf 0.99] The encoder chip can come from a previous session
`sabrage/crates/sabrage-core/src/session/watcher.rs:327-335` — **CONFIRMED** (re-rated medium)

A new monitor preloads 200 lines from the global runtime log and accepts every encoder-ready line without comparing its timestamp to the current session. Clearing happens only on a later transition into Idle/Exited; initial `last_phase` is already Idle. Reopening Sabrage before the current session negotiates an encoder can therefore show the previous session's HEVC/native-helper chip, potentially masking an in-process H.264 downgrade indefinitely if no new line arrives.
Evidence: `SessionMonitor::new` opens the tail with `RUNTIME_LOG_PRELOAD_LINES` (`sabrage/crates/sabrage-core/src/session/watcher.rs:205-218`), and polling assigns any parsed line directly to `self.encoder` (`watcher.rs:327-335`). The reset condition is only `now_idle_or_exited && !was_idle_or_exited`, after initialization with `last_phase: Idle` (`watcher.rs:428-444`).

*Recommendation:* Reset telemetry when the observed `run_id` changes and accept encoder lines only from a cursor/timestamp established for that session. Never publish an encoder while no current session has produced one.

*Verifier:* Real and reproducible, but the window is narrower than 'high': it needs the monitor to be constructed (app start / first Session-screen poll) while a session is already live and has not yet negotiated, with the previous session's line still inside the last 200 lines (also capped at 256 KiB by crate::logs). Within one app run the Running->Idle/Exited edge does clear the chip, so a second launch is correct. User-visible wrong data on an uncommon-but-real path -> medium.

*Fix sketch:* In session/watcher.rs: remember the run identity the chip belongs to. Add `encoder_run_id: Option<RunId>` next to `encoder`; in snapshot(), after phase/identity resolution, drop `self.encoder` whenever `status.run_id != self.encoder_run_id`, and stamp `encoder_run_id = status.run_id` when a poll parses a line. Additionally bound the preload: either open the tailer with preload 0 and re-open with preload only when the monitor has no session yet, or have parse_encoder_ready also return the spdlog `[YYYY-MM-DD HH:MM:SS.mmm]` prefix and reject lines older than status.started_at_unix_ms. Never publish an encoder whose run_id is None while a session is Running.

*Regression test:* sabrage/crates/sabrage-core/src/session/watcher.rs tests (beside the existing F16 'encoder-clears-on-exit' block): a `a_previous_sessions_encoder_line_is_not_published_for_a_new_run` case seeding the log with an encoder line, then SessionMonitor::new + set_live_session(new run_id) + snapshot, asserting `s.encoder.is_none()` until a line is appended after the session started; plus a case asserting the chip DOES appear for a line whose timestamp/run matches.

### A9-7 [medium, conf 0.98] Runtime freshness is unbounded against future skew and unscoped to the session
`sabrage/crates/sabrage-core/src/session/watcher.rs:100-105` — **CONFIRMED** (re-rated low)

Any timestamp in the future produces an age of zero, so a clock correction or corrupt future value remains fresh until wall time catches up. The parsed `process_id` is also ignored, allowing another bottle or shell-launched runtime writing the global file to mark this session fresh. Both paths can suppress Stalled and report the wrong runtime state.
Evidence: freshness is `now_unix_ms.saturating_sub(updated_at_unix_ms) <= ...` (`sabrage/crates/sabrage-core/src/session/watcher.rs:100-105`). Although `RuntimeStatus` contains `process_id` (`watcher.rs:43-49`), `snapshot` copies state/freshness solely from the timestamp and never compares that ID with `status.pid` (`watcher.rs:313-324`).

*Recommendation:* Reject timestamps beyond a small explicit future-skew allowance and bind telemetry to the current session using process ID plus session start time, or preferably a runtime-emitted launch nonce.

*Verifier:* Factually true of the code (no future-skew bound, process_id parsed and ignored), and a backward system-clock correction or a second runtime writing the shared global file does suppress Stalled. But writer and reader are the same Mac and the same clock, a second concurrent runtime is not a realistic configuration (one headset, one wineserver), and the worst outcome is a status pill that fails to say 'Stalled' — no mutation, no wrong action. Latent, display-only -> low.

*Fix sketch:* In session/watcher.rs::is_fresh, reject stamps beyond a small explicit allowance: `let skew = updated_at_unix_ms.saturating_sub(now_unix_ms); skew <= MAX_FUTURE_SKEW.as_millis() && now.saturating_sub(updated) <= RUNTIME_STATUS_MAX_AGE` with MAX_FUTURE_SKEW ~2 s documented as clock-skew tolerance. For session scoping, do NOT equate process_id with the recorded wine pid (unverified relation); instead ignore a status whose updated_at_unix_ms predates status.started_at_unix_ms, and record process_id only for display until oxrsys emits a launch nonce.

*Regression test:* sabrage/crates/sabrage-core/src/session/watcher.rs tests, next to the existing is_fresh cases: assert `is_fresh(now + 3_600_000, now) == false` and `is_fresh(now + 1_000, now) == true`; plus a snapshot case whose runtime_status.json predates started_at_unix_ms and asserts `runtime_fresh == false`.

### A9-8 [medium, conf 0.97] Newer recovery schemas can be silently downgraded and deleted
`sabrage/crates/sabrage-core/src/session/state.rs:174-200` — **CONFIRMED** (re-rated low)

The version field is parsed but never validated, and serde ignores unknown fields by default. An older binary can therefore load a newer record containing an additional guard, rewrite only the fields it understands, and then clear the record after its known guards complete. This loses forward-version cleanup metadata.
Evidence: `load` returns `serde_json::from_str(&text).map(Some)` with no `SESSION_STATE_VERSION` check (`sabrage/crates/sabrage-core/src/session/state.rs:174-185`), while `save` reserializes only `SessionState` via `serde_json::to_vec_pretty(state)` (`state.rs:193-200`). The schema exposes `version: u32` but has no flattened unknown-field storage (`state.rs:86-123`).

*Recommendation:* Refuse to mutate or clear records with a version newer than supported, and either preserve unknown fields during rewrites or introduce an explicit migration that understands every guard in that version.

*Verifier:* The gap is real and the doc promises a guard that does not exist, but no v2 writer exists today (the constant has never been bumped), so nothing can currently produce the record that triggers it. Pure forward-compat hygiene, worth fixing before the first bump precisely because a shipped v1 binary cannot be retrofitted -> low.

*Fix sketch:* In session/state.rs::load, after deserializing, return an error (or a new `Ok(Some(state))` flagged unsupported) when `state.version > SESSION_STATE_VERSION`; in session/reconcile.rs, treat that as 'report and keep' — warn one row naming the version and the file path, restore nothing, never call state::clear. Optionally add `#[serde(flatten)] extra: serde_json::Map<String, Value>` to SessionState so a same-major rewrite preserves unknown keys.

*Regression test:* sabrage/crates/sabrage-core/src/session/state.rs tests: `a_newer_schema_is_refused_not_downgraded` writing a record with `"version": 2` plus an unknown `"futureGuard"` key and asserting load() errors (or is flagged); and in session/reconcile.rs tests, a case asserting such a file leaves session-state.json on disk and plans no SwitchAudioSource / adb / kill.

### A9-9 [medium, conf 0.9] Stop and detach can race, with detach winning nondeterministically
`sabrage/crates/sabrage-core/src/session/reconcile.rs:767-790` — **CONFIRMED** (re-rated medium)

Detach unconditionally fires its independent token even when Stop has already fired cancellation. The supervisor waits on child, cancel, and detach in one unbiased `tokio::select!`; if both tokens are ready, detach may win, disarm the guards, and leave wine running while the Stop caller observes the live slot disappear and reports success. This is reachable through rapid Stop/Detach actions or an OS terminate event during teardown.
Evidence: `detach` immediately executes `handle.detach.cancel()` without checking or atomically arbitrating the cancel state (`sabrage/crates/sabrage-core/src/session/reconcile.rs:767-768`). The two tokens are independent (`sabrage/crates/sabrage-core/src/session/mod.rs:152-156`), and supervision selects them as separate branches (`sabrage/crates/sabrage-core/src/stages/run/mod.rs:424-443`). This conclusion is an inference from simultaneous readiness of those branches.

*Recommendation:* Replace the two-token race with one atomic lifecycle decision where Stop is terminal and cannot be superseded by Detach. Reject detach once stopping begins and cover simultaneous requests with a deterministic concurrency test.

*Verifier:* Both halves of the reviewer's inference hold: detach fires unconditionally after Stop, and the select is ~50/50 when both branches are ready. The reachable triggers are real (Session-screen Stop vs. the Detach action / `resolve_quit(Keep)` / `detach_on_terminate` on an AppKit terminate, which PARITY.md:104 declares as an answered-'keep running' path). Two windows exist: (a) both tokens ready before the supervise task is next polled — microseconds, needs near-simultaneous triggers; (b) detach first, then Stop while the `Reason::Detached` arm is still running (LIVE_SESSION is cleared only at the end of that arm) — Stop's cancel lands on a supervisor already past the select, the slot then clears, and Stop reports success over a still-running game. Not higher than medium: it needs the narrow interleave, and it is recoverable (the record is marked `detached`, so a later reconcile restores the guards, and a second Stop falls through to the Stop stage and kills wineserver). Not lower: the outcome is a user-visible false 'stopped' with the audio device left on BlackHole and the dashboard open.

*Fix sketch:* Make Stop terminal and detach subordinate, using the monotonicity of the cancel token. (1) `session::reconcile::detach` (reconcile.rs:767): return `Ok(())` (or a typed 'stopping' refusal) without firing `handle.detach` when `handle.cancel.is_cancelled()`. (2) `stages::run::guarded`'s supervise select (run/mod.rs:426-431): make the arbitration deterministic — `biased;` with the cancel branch first, and in the `Supervised::Detached` arm re-check `ctx.cancel.is_cancelled()` and downgrade to `Reason::Cancelled { child: Some(DetachedChild { identity, child: proc }) }` so a Stop that fired at any point wins the teardown. (3) optionally in `src-tauri/src/commands.rs::stop_session`, do not claim `ok: true` when the slot was released by a detach rather than by teardown (check the persisted record's `detached` flag, or have `stop_live_session_and_wait` report which happened).

*Regression test:* Two: a concurrency test in `sabrage/crates/sabrage-core/src/stages/run/mod.rs` tests (next to `detaching_marks_the_state_leaves_the_guards_and_keeps_the_file`, ~line 1288) that fires `cancel` and `detach` together — looped, so both select orderings are exercised — and asserts the outcome is always the Cancelled teardown (guards released, `session-state.json` cleared, `Err(SabrageError::Cancelled)`), never `detached: true`; and a unit test in `sabrage/crates/sabrage-core/src/session/reconcile.rs` tests asserting `detach()` on a handle whose `cancel` is already cancelled leaves `handle.detach` un-fired and does not write `detached: true` to an existing record.

*Cross-area files:* sabrage/crates/sabrage-core/src/stages/run/mod.rs, sabrage/src-tauri/src/commands.rs

*Codex next steps:* Barrier-test reconciliation while a launch is paused after audio/dashboard persistence but before `LIVE_SESSION`; assert no restore, signal, forward removal, or clear occurs. · Exercise an unresolved audio/adb guard followed by a new launch, and two concurrent frontend run IDs, verifying that no record is overwritten or cleared by a non-owner. · Inject one successful and one failed adb forward removal; verify only the successful entry is removed from persisted recovery state and retry remains possible. · Seed an old HEVC encoder line plus future- or wrong-process runtime status, start a new session without current telemetry, and verify the status remains unbound/waiting and eventually stalls correctly. · Repeatedly issue Stop and Detach simultaneously, and load a future-version state with an unknown guard; verify Stop always wins and unsupported recovery data is never rewritten or cleared.

## A10 — config-runtime-toml

**Codex verdict:** needs-attention — NO-SHIP: the GUI can disagree with the runtime’s effective values, live saves can rebuild the active encoder, and unsynchronized writes can irreversibly overwrite concurrent edits without a valid backup.

### A10-1 [critical, conf 0.98] Concurrent writers can lose the entire hand-maintained config without backing it up
`sabrage/crates/sabrage-core/src/config/runtime_toml.rs:1017-1072` — **CONFIRMED** (re-rated medium)

`write` snapshots the file and its existence, then performs several asynchronous operations before replacing it. It neither locks the document nor verifies that the destination still matches the snapshot. On the absent path, another Sabrage process, `setup.sh`, or an editor can create the file after the check; Sabrage then replaces it while retaining `exists = false`, so the lost document receives no backup. Existing-file races are also unsafe: the backup contains the stale snapshot rather than the concurrently modified bytes.
Evidence: `sabrage/crates/sabrage-core/src/config/runtime_toml.rs:1017-1028` captures `let exists = toml_path.is_file()` and unconditionally calls `write_atomic` with the template; `:1053-1072` backs up the earlier `base` and later replaces the config without a compare-and-swap. `sabrage/crates/sabrage-core/src/executor.rs:674-681` implements that write as a rename over the destination. The shell has the competing check/write sequence `[ -f "$TOML" ]` followed by `cat ... > "$TOML"` at `scripts/demo/setup.sh:42-50`.

*Recommendation:* Serialize config creation and editing with a cross-process lock shared by Sabrage entry points and setup, use no-clobber creation for the absent case, and verify a content hash/version immediately before replacement. If the file changed, abort and require the user to reload instead of overwriting it. Reserve backup names atomically as part of the same transaction.

*Verifier:* The read-modify-write is genuinely unguarded: no lock, no O_EXCL create, no content/mtime verification before the rename, and the plumbing that would allow it (modified_unix_ms) is unused. Re-rated critical→medium because the only frequent competing writer emits byte-identical template content, the GUI is single-instance, and the loss window is tens of milliseconds — unusual conditions, not a routine path.

*Fix sketch:* In `config::runtime_toml::write`: (1) take the absent path through a new `Executor::create_new` (O_EXCL) instead of `write_atomic`, and on AlreadyExists fall through to the existing-file branch rather than clobbering; (2) capture a hash of `base` (or the stat mtime+len) alongside it and re-read + compare immediately before the final `write_atomic`, returning `SabrageError::InvalidInput("<path> changed on disk since it was read — reload Settings and retry")` on mismatch; (3) hold an advisory lock file (`$OXR_APPSUP/.oxrsys-runtime.toml.lock`, flock) around the whole read→backup→write sequence and take the same lock in `checks`/`fixes` writers so the CLI and GUI serialise. Backup-name reservation should use the same O_EXCL create so `next_backup_path` stops being a check-then-create race.

*Regression test:* sabrage/crates/sabrage-core/src/config/runtime_toml.rs `#[cfg(test)] mod tests`: (a) `write` on an absent path where a file is created between the probe and the write — inject via a test Executor whose `write_atomic` for the toml path first writes competing bytes — asserts the competing content survives (or the call errors) and is never replaced without a backup; (b) `write` on an existing file whose bytes are mutated by the test between `read` and the final write asserts an Err naming the changed file and that the on-disk bytes are the concurrent ones; (c) a unit test that two backup reservations in the same second never collide.

*Cross-area files:* sabrage/crates/sabrage-core/src/executor.rs, sabrage/src-tauri/src/commands.rs, sabrage/ui/src/stores/config.svelte.ts

### A10-2 [high, conf 0.99] The readers discard the runtime’s previous valid assignment when the final assignment is invalid
`sabrage/crates/sabrage-core/src/config/runtime_toml.rs:341-359` — **CONFIRMED** (re-rated medium)

Both Rust readers inspect only the final physical occurrence. The C++ runtime instead processes assignments sequentially and ignores an invalid assignment without clearing an earlier valid value. For `protocol = "alvr"` followed later by `protocol = "bogus"`, the runtime remains on ALVR, while the GUI reports an absent value and substitutes the compiled-in `oxrsys` default. That produces a false legacy warning and an incorrect base for `buildPatch`.
Evidence: `sabrage/crates/sabrage-core/src/config/runtime_toml.rs:341-359` selects `occurrences.last()` and `continue`s when it is invalid; `:492-505` repeats this with `hits.last()` in the fallback. Conversely, `ext/oxrsys/runtime/src/Config.cpp:435-443` assigns protocol only when the value is whitelisted, and `:541-544` explicitly says malformed values “keep the last valid/default setting.”

*Recommendation:* Fold occurrences in physical order and update the effective value only for assignments the runtime accepts. Track the last physical occurrence separately as the patch target, and add duplicate-with-invalid-last fixtures for every editable key.

*Verifier:* The divergence is real and reproducible on all six editable keys, but it needs a duplicate-key file with an invalid final occurrence — an unusual hand-edit — and the UI still flags the invalid value rather than silently lying. Re-rated high→medium.

*Fix sketch:* In `harvest` and `read_lines_like_the_runtime`, fold occurrences in physical order: keep `effective` = the value of the last occurrence the runtime would ACCEPT (leave it at the default when none is acceptable), and keep `patch_target` = the last physical occurrence (what `apply_patch` must edit, so the edit still wins at runtime). Push an `InvalidValue` for every rejected occurrence as today. `assign`/`assign_raw` then run on `effective` instead of on the last occurrence.

*Regression test:* runtime_toml.rs tests + a new fixture `sabrage/crates/sabrage-core/tests/fixtures/phase4/oxrsys-runtime.shadowed-invalid-last.toml` carrying, for each of the six keys, a valid occurrence followed by an invalid one in a later table. Asserts: `view.values.<key>` equals the earlier valid value, `view.invalid` names the later occurrence, `view.shadowed` contains the key, and that `apply_patch` still rewrites the LAST physical occurrence.

### A10-3 [high, conf 0.99] The GUI accepts TOML string syntax that the runtime treats as invalid
`sabrage/crates/sabrage-core/src/config/runtime_toml.rs:510-544` — **CONFIRMED** (re-rated high)

The primary reader decodes full TOML strings, while the fallback and no-op comparator additionally treat single-quoted strings as runtime-equivalent. The C++ parser only recognizes double quotes and explicitly performs no escape processing. A standard TOML line such as `protocol = 'alvr'` is therefore displayed as ALVR with no invalid warning; saving another setting leaves it untouched, while the runtime silently remains on its `oxrsys` default. Valid multiline strings can likewise contain assignment-looking physical lines that the runtime consumes but `toml_edit` treats as string content.
Evidence: `sabrage/crates/sabrage-core/src/config/runtime_toml.rs:369-390` interprets decoded `Value::as_str()` values; `:510-540` tracks both quote styles and removes either pair; `:905-913` treats those spellings as an unchanged runtime value. In contrast, `ext/oxrsys/runtime/src/Config.cpp:257-275` states “no escape handling” and tracks only `"`, while `:290-298` removes only double quotes.

*Recommendation:* Use one raw, physical-line scanner matching `ParseConfigToml` as the source of effective values on every read; reserve `toml_edit` for deciding whether a file can be safely edited. Differential-test Rust against the C++ parser using single quotes, escapes, numeric underscores, multiline strings, arrays, and inline tables.

*Verifier:* One realistic hand-edit (single quotes are ordinary TOML) makes the GUI display and the runtime disagree, and — worse — makes the protocol fix a silent no-op that reports 'already has protocol = "alvr"'. Kept at high: user-visible wrong behaviour on a plausible path, only partly caught by a different parser elsewhere.

*Fix sketch:* Make one raw physical-line scanner that mirrors `ParseConfigToml` exactly (double-quote-only unquote, `"`-only comment state, no escape handling) the source of effective values in BOTH `read` branches; keep `toml_edit` solely for deciding round-trippability and for editing. Narrow `unquote` to double quotes, drop `'`/`\\` from `strip_comment`, and delete the literal-quote special case in `same_to_the_runtime` so `protocol = 'alvr'` is reported invalid and is rewritten as `protocol = "alvr"` on save. Also reject (parse_error) a document whose editable keys sit inside a multiline string, where physical lines and toml_edit disagree.

*Regression test:* runtime_toml.rs tests + fixture `sabrage/crates/sabrage-core/tests/fixtures/phase4/oxrsys-runtime.literal-quotes.toml`: asserts `read` reports `protocol` invalid (runtime would ignore it) with `values.protocol == None`, that `apply_patch` with `protocol = Alvr` rewrites the line to a basic string and reports `changed_keys == ["protocol"]`, and a table-driven differential test over single quotes, escapes, `0x50`, multiline strings, arrays and inline tables asserting the Rust scanner and the documented `ParseConfigToml` semantics agree.

*Cross-area files:* sabrage/ui/src/screens/Settings.svelte

### A10-4 [high, conf 0.99] Settings writes can reconfigure and rebuild the encoder during a live stream
`sabrage/crates/sabrage-core/src/config/runtime_toml.rs:999-1017` — **CONFIRMED** (re-rated high)

The general write path has no live-session guard, and its IPC caller explicitly permits live writes under the false premise that the runtime reads the file only at the next launch. The runtime actually polls the file every 250 ms; the ALVR frame path reads `encoder_process` and `video_codec` and retires the encoder when the resulting identity changes. A normal Settings save can therefore rebuild the encoder mid-stream despite the UI promising “Values take effect at the next launch.” A `live_session()` check alone would still miss sessions owned by another Sabrage process.
Evidence: `sabrage/crates/sabrage-core/src/config/runtime_toml.rs:999-1017` enters validation and I/O without any session check, although the separate protocol-fix path refuses a same-process live session at `:1151-1162`. `sabrage/src-tauri/src/commands.rs:1262-1283` explicitly allows the live write. `ext/oxrsys/runtime/src/Config.cpp:680-690` reloads every 250 ms, and `ext/oxrsys/runtime/src/AlvrStreamingBackend.cpp:371-380,450-459,581-587` reads the configuration per frame and rebuilds on identity drift. The contrary UI promise is at `sabrage/ui/src/screens/Settings.svelte:635-636`.

*Recommendation:* Refuse config writes while a reconciled session is active, checking persisted process identity so sessions owned by another process are covered, and repeat that check immediately before replacement. Alternatively, formally design and test safe live reconfiguration and change the UI contract; the current behavior cannot be described as next-launch-only.

*Verifier:* Bumping bitrate or switching the encoder while streaming is exactly why a user opens Settings mid-session; the save lands within 250 ms and rebuilds (or, for native-with-no-helper, stops) the encoder mid-stream while the UI says the change is inert until next launch. Kept at high — user-visible wrong behaviour on a realistic path — not critical, since the effect is a recoverable hitch/rebuild rather than data loss.

*Fix sketch:* Add a live-session guard to `config::runtime_toml::write` itself (not just `edit_protocol`): take an explicit `allow_live: bool`/`SessionGuard` argument, resolve it from the persisted session record (`crate::session::live_session()`, which reads the on-disk reconciled session, so another process's session counts), refuse with `edit_protocol`'s wording plus the `./demo.sh stop --bottle <name>` remedy, and re-check immediately before the final `write_atomic` (same place the A10-1 CAS goes). Update commands.rs:1262-1283 to stop documenting/permitting the live write, and have `read_runtime_config`/the Settings store surface `live` so the screen disables Save with "stop the session to change these" instead of "Values take effect at the next launch."

*Regression test:* runtime_toml.rs tests: `write` with a fake live session in `StageCtx`/paths asserts `Err` naming the bottle and leaves the file's bytes and mtime untouched; a second test asserts the guard also fires for a session record written by a different pid. Plus a UI-copy assertion in the Settings screen's existing test/snapshot that the next-launch sentence is gone.

*Cross-area files:* sabrage/src-tauri/src/commands.rs, sabrage/ui/src/screens/Settings.svelte, sabrage/ui/src/stores/config.svelte.ts, sabrage/crates/sabrage-core/src/session/mod.rs

### A10-5 [medium, conf 1.0] Any real edit normalizes unrelated CRLF, BOM, and trailing-newline bytes
`sabrage/crates/sabrage-core/src/config/runtime_toml.rs:786-797` — **CONFIRMED** (re-rated low)

The no-op path preserves raw input, but every actual edit serializes the complete `DocumentMut`. The code itself records that this renderer converts CRLF to LF, drops a BOM, and adds a final newline. Thus changing one value can rewrite every line and other bytes outside Sabrage’s stated six-value ownership boundary. A backup limits recovery risk but does not satisfy the format-preservation contract.
Evidence: `sabrage/crates/sabrage-core/src/config/runtime_toml.rs:786-797` says `toml_edit` performs those normalizations and uses `doc.to_string()` whenever `changed_keys` is nonempty. The public boundary promises that comments, spacing, and other bytes survive byte-for-byte at `sabrage/crates/sabrage-core/src/config/mod.rs:7-11`.

*Recommendation:* Patch the selected raw source range rather than serializing the whole document, preserving original line endings, BOM, and EOF state. Add actual-change tests for CRLF, BOM-prefixed, and non-newline-terminated fixtures; the existing no-op tests do not cover this path.

*Verifier:* The normalization is real and unguarded on the edit path, but the reviewer over-rates the blast radius. Both producers of this file write LF, no BOM, newline-terminated: scripts/demo/setup.sh's heredoc and Sabrage's create-from-template (contract/oxrsys-runtime.toml.template), so the fixture-realistic case (verified above) rewrites exactly the one edited line. Reaching the divergence requires a hand-edit on macOS with an editor configured for CRLF or a UTF-8 BOM, or a file truncated without its final newline. Nothing semantic is lost when it does happen - comments, ordering, unknown keys, and the provenance header all survive; only line-ending/BOM/EOF bytes change, and write() has already copied the original into ~/Library/Application Support/Sabrage/backups/ (runtime_toml.rs:1054-1067). So: latent, unusual conditions, recoverable = low, not medium.

*Fix sketch:* In `apply_patch` (sabrage/crates/sabrage-core/src/config/runtime_toml.rs:738-805), capture the input's byte shape before `text.parse::<DocumentMut>()`: `had_bom = text.starts_with('\u{feff}')`, `was_crlf = text.contains("\r\n")` (or majority-of-line-breaks), `had_final_newline = text.ends_with('\n')`. Strip the BOM before parsing. After `doc.to_string()` on the changed branch, restore the shape in a small helper `restore_byte_shape(rendered, had_bom, was_crlf, had_final_newline)`: re-prefix the BOM, map `\n` -> `\r\n` (after normalizing any stray `\r\n` to `\n` first), and drop the trailing newline when the input had none. Leave the existing `changed_keys.is_empty()` short-circuit as is. A cheaper alternative, if mixed line endings are judged not worth supporting, is to make `read` set `parse_error` (and therefore `write` refuse) for non-LF/BOM files - but that degrades the fix.edit-protocol path, so restoring the shape is preferable.

*Regression test:* sabrage/crates/sabrage-core/src/config/runtime_toml.rs `mod tests` (alongside the existing `an_unchanged_patch_writes_nothing_and_takes_no_backup`, ~line 1877): add `a_real_edit_preserves_crlf_bom_and_missing_final_newline` asserting that for each of (a) a CRLF fixture, (b) a BOM-prefixed fixture, (c) a fixture with no trailing newline, `apply_patch(text, &patch_bitrate(60))` returns `changed_keys == ["bitrate_mbps"]` and `out.text` equals `text` with only the `bitrate_mbps` value substituted - i.e. same CRLF count, same leading BOM byte, same EOF state - plus the existing LF fixture as the control.

*Codex next steps:* Build a differential corpus test that feeds identical bytes to C++ `ParseConfigToml`, Rust `read`, and `read_lines_like_the_runtime`; include invalid-last duplicates, single quotes, escapes, numeric underscores, multiline strings, arrays, and inline tables. · Run real value edits against CRLF, BOM-prefixed, and missing-final-newline fixtures and byte-diff the results, allowing only the selected value and the explicitly declared comment relocation to change. · Use barriers or fault injection to interleave two `write` calls and a setup-style creator after the existence check, snapshot read, and backup-name selection; verify that stale writes abort and every displaced document remains recoverable. · During an active ALVR stream, save `encoder_process` and `video_codec` from Settings and confirm whether the runtime logs a config reload and encoder identity rebuild; repeat from a second Sabrage process to validate the cross-process guard. · After fixes, run `cargo test -p sabrage-core`, the C++ config tests, `scripts/dev/parity.sh --live=off`, and `npm run check` from their documented workspace roots.

## A11 — ipc-boundary

**Codex verdict:** needs-attention — No-ship: stop/quit can report success before teardown completes, and unlocked IPC mutations can silently overwrite successful configuration or library updates.

### A11-1 [high, conf 0.99] Stop and quit report success even when teardown times out
`sabrage/src-tauri/src/commands.rs:691-705` — **CONFIRMED** (re-rated medium)

CONFIRMED. The 30-second wait has no result indicating whether the live-session slot cleared. Consequently, `stop_session` returns `ok: true`, while `resolve_quit("stop")` approves and exits even if teardown is still running or hung. Because `QuitApproved` is then true, the exit hook skips its detach fallback. The user can select “Stop and quit” yet leave the game or guards unsupervised.
Evidence: `sabrage/src-tauri/src/commands.rs:691-705` defines `async fn stop_live_session_and_wait()` returning `()` and ends with `let _ = ... .await`; its loop merely stops at the deadline. `commands.rs:785-791` then unconditionally returns `Ok(StageOutcome { ... ok: true ... })`. `commands.rs:856-860` unconditionally stores approval and calls `app.exit(0)`. `sabrage/src-tauri/src/lib.rs:251-253` passes that approval to `detach_on_terminate`. `sabrage/PARITY.md:104` promises that “Stop and quit” stops the session.

*Recommendation:* Return a completion result from the wait and re-check that the same live run disappeared. Propagate timeout/join/teardown failure from `stop_session`; in `resolve_quit`, set `QuitApproved` and exit only after confirmed teardown. Keep the dialog open with an actionable error otherwise.

*Verifier:* The code path is unambiguous and reachable: no completion signal exists, so 'timed out' and 'torn down' are indistinguishable to both callers, and the quit path burns its only fallback (detach_on_terminate) on the assumption teardown finished. Downgraded from high to medium because reaching the bad outcome needs an anomaly — an unbounded-child hang or an IO failure in guard release — not an ordinary teardown (wineserver is 5 s-bounded), and the on-disk session record plus reconcile make the leaked guards recoverable rather than permanent.

*Fix sketch:* Make `stop_live_session_and_wait()` return an outcome (`enum TeardownWait { Cleared, TimedOut, JoinFailed }`) by having the polling closure return `live_session().is_none_or(|h| h.run_id != run_id)` and mapping the `JoinHandle` result. `stop_session`'s live branch returns `Ok(StageOutcome{ ok: false, exit_code_equiv: 1 })` (or `Err` with the 'teardown still running' text) on anything but `Cleared`. `resolve_quit`'s `Stop` arm: on `Cleared` keep today's approve+exit; on `TimedOut` do NOT silently approve — fall back to the documented keep-running policy (`detach_live_session().await`, which marks the record `detached` and disarms the guards) and only then approve+exit, returning an `Err` string the QuitDialog can show, so the user is never left unable to quit. Separately, in `stages/run/mod.rs`'s teardown `Reason::Cancelled` arm, make `held.release`/`clear_state` best-effort (warn rows) exactly like the `Normal` arm, so the handle cannot leak and make every subsequent wait a guaranteed 30 s timeout.

*Regression test:* sabrage/src-tauri/src/commands.rs `mod tests`: extract the wait into a pure, injectable helper (`wait_for_slot_clear(is_clear: impl Fn() -> bool, deadline)`) and assert it reports `TimedOut` when the predicate never flips and `Cleared` when it flips mid-poll; plus a pure `quit_action_for(wait_outcome)` test asserting Stop+TimedOut yields 'detach then exit', Stop+Cleared yields 'approve then exit'. In sabrage-core, a test alongside stages/run/mod.rs's existing teardown tests asserting that a `Reason::Cancelled` teardown whose guard release fails still calls `clear_live_session` (`live_session().is_none()` afterwards).

*Cross-area files:* sabrage/crates/sabrage-core/src/stages/run/mod.rs

### A11-2 [high, conf 0.98] Phase-4 commands bypass serialization and can lose successful writes
`sabrage/src-tauri/src/commands.rs:1232-1242` — **CONFIRMED** (re-rated medium)

CONFIRMED. Runtime-config and JSON-store commands perform multi-step read/modify/write sequences through a bare executor without the process-wide operation lock or a file-specific transaction. Two invocations can read the same base and each resolve successfully, while the last atomic rename silently discards the other update. Concrete paths include `save_game` racing launch’s `record_last_session`, and `write_runtime_config` racing setup’s absent-file check: setup can overwrite the user’s successful patch with the template, violating the write-once contract.
Evidence: `sabrage/src-tauri/src/commands.rs:1240-1242` is only `RealExecutor::new(...)`, with no lock. `commands.rs:1523-1526` performs `let mut lib = library::load(...)`, `lib.upsert(entry)`, then `library::save(...)`; `commands.rs:1629-1641` independently loads, updates, and saves the same file. `commands.rs:1277-1283` invokes the config writer directly. Context `sabrage/crates/sabrage-core/src/stages/mod.rs:431-441` states “One mutating operation at a time” and acquires `OPERATION_LOCK`, while `stages/setup.rs:224-240` independently checks and writes the same TOML. `sabrage/PARITY.md:113` requires write-once template creation.

*Recommendation:* Serialize runtime-config writes with setup/fixes using the shared `OPERATION_LOCK`. Protect each complete settings/library read-modify-write transaction with a dedicated async mutex or revision/CAS scheme so concurrent saves cannot overwrite newer data.

*Verifier:* The absence of any lock is verifiable by grep and the interleavings are genuine (Tauri runs async commands concurrently on the multi-thread runtime; every one of these paths has await points between its read and its write). Downgraded from high to medium: every write goes through `write_atomic`, so the failure mode is a lost update, never a corrupt file; it needs two near-simultaneous operations (a Library/Settings save landing inside a launch teardown, a setup run, or an edit-protocol fix); the TOML path leaves a timestamped backup; and no machine state or PARITY byte-invariant is broken (a template-clobber still writes the same template bytes both front-ends agree on — only the user's patch is lost).

*Fix sketch:* In commands.rs, take the already-public `sabrage_core::stages::acquire_operation_lock().await` guard for the duration of `write_runtime_config` (that alone serializes it against setup and the edit-protocol fix, which already hold it). For `library.json`/`settings.json`, add a file-scoped `static LIBRARY_LOCK: tokio::sync::Mutex<()>` / `SETTINGS_LOCK` (or one `STORE_LOCK`) held across the whole load->mutate->save transaction in `save_game`, `remove_game`, `revert_original_steam_dll`'s read, `record_last_session`, and `save_settings`, so no two of them can straddle each other's write. Keep `get_library`/`get_settings` lock-free reads.

*Regression test:* sabrage/src-tauri/src/commands.rs `mod tests` (a `#[tokio::test]` under a temp `$HOME`-style Paths): spawn `save_game(entry_a)` and `record_last_session(id_b, ..)` concurrently with `tokio::join!` and assert BOTH mutations are present in the reloaded `library.json` (the test fails today by keeping only one). A second case: `write_runtime_config(patch)` concurrent with a `stages::setup` config write over an absent TOML asserts the patched key survives.

### A11-3 [medium, conf 0.96] Queued stages have no observable run ID and cannot be cancelled safely
`sabrage/src-tauri/src/commands.rs:529-535` — **CONFIRMED** (re-rated medium)

CONFIRMED. A command creates and registers its run ID before awaiting `sabrage_core::run_stage`, but core waits for `OPERATION_LOCK` before emitting the first `StageStarted`. The IPC cancellation contract exposes the ID only through that event. Therefore, a second invocation queued behind a build/install has no ID the frontend can cancel. If its channel disappears while queued, send failures are ignored and it later mutates the machine without an observer.
Evidence: `sabrage/src-tauri/src/commands.rs:529-535` registers the canceller and then awaits `run_stage`. Context `sabrage/crates/sabrage-core/src/stages/mod.rs:505-515` acquires the operation lock before emitting `StageStarted`. `sabrage/ui/src/ipc.ts:283-286` documents cancellation using the ID “from its first `stageStarted` event”. `commands.rs:368-375` explicitly drops `channel.send` failures and keeps the stage running.

*Recommendation:* Atomically reject concurrent mutations with a structured busy error, or add a pre-lock queued event carrying the run ID and make lock acquisition cancellation-aware. Do not allow an unobserved queued stage to begin after its channel has closed.

*Verifier:* Reproducible by UI construction (Hide + second stage), and the id genuinely does not exist on the wire until the lock is granted. Kept at medium: nothing is corrupted, the window closes as soon as the queued stage starts (Cancel then works), and the operator has to deliberately hide a running gate and start another mutation.

*Fix sketch:* Add a pre-lock event carrying the id: a `StageEvent::Queued { run_id, stage }` variant in sabrage/crates/sabrage-core/src/events.rs, emitted by commands.rs's `execute_stage_with_sink` through the sink immediately after `registry.register` and before `run_stage` (no core change needed — the command owns the sink); mirror it in ipc.ts's `StageEvent` union and have GateModal set `runId` from `queued` as well as `stageStarted`, so Cancel is live while queued. Alternatively/additionally reject up front: if `sabrage_core::operation_in_progress()`, return a structured busy error rather than silently queueing. Register `fix` with the `RunRegistry` the same way `execute_stage_with_sink` does.

*Regression test:* sabrage/src-tauri/src/commands.rs `mod tests`: a `#[tokio::test]` that holds `acquire_operation_lock()` and then drives `execute_stage_with_sink` with a collecting sink, asserting the sink observes a `Queued` event carrying a non-nil run id BEFORE the lock is released, and that `registry.cancel(that_id)` returns true and the stage settles as `Cancelled` once the lock frees. Frontend: a GateModal test asserting Cancel is enabled after a `queued` event.

*Cross-area files:* sabrage/ui/src/components/GateModal.svelte

### A11-4 [medium, conf 0.94] Quit interception has no guaranteed responder
`sabrage/src-tauri/src/lib.rs:218-240` — **CONFIRMED** (re-rated low)

CONFIRMED backend gap; inferred failure under renderer loss. Both Cmd-Q and window close are prevented solely because a live handle exists, then a transient frontend event is emitted with no readiness handshake, acknowledgement, timeout, or native fallback. If the webview is reloading, crashed, or failed before registering its listener, the event has no responder and repeated Cmd-Q/window-close attempts remain intercepted.
Evidence: `sabrage/src-tauri/src/lib.rs:218-226` calls `api.prevent_exit()` and ignores the result of `emit("app://quit-requested", ())`; `lib.rs:232-240` does the same for `prevent_close()`. The only responder is the frontend subscription in `sabrage/ui/src/ipc.ts:615-619`, `listen("app://quit-requested", ...)`; no owned code establishes readiness or acknowledges receipt.

*Recommendation:* Use a backend/native quit dialog, or track frontend readiness and require an acknowledgement with a bounded fallback. If no responder exists, apply the documented keep-running detach policy and exit rather than permanently preventing the request.

*Verifier:* The backend gap is real (an intercepted Cmd-Q with no responder stays intercepted), but it can only bite when the renderer dies or is mid-reload while a session this process launched is live — and a live handle essentially implies the webview already booted and subscribed, since only the frontend can start a launch. With Dock-menu Quit / Force Quit falling through to the documented detach path, the worst outcome is that Cmd-Q and the window close button appear inert until the user quits another way. That is minor, not user-visible-wrong-behaviour-on-a-realistic-path, so medium -> low.

*Fix sketch:* In lib.rs's two intercept arms, keep `prevent_exit`/`prevent_close` but arm a bounded fallback: record the pending-quit instant in managed state and, if no `resolve_quit` arrives within N seconds (or if `emit` returns Err), apply the documented keep-running answer — `commands::detach_live_session()` then `app.exit(0)` — instead of holding the quit forever. Cheapest variant: on the SECOND intercepted request within the timeout window, stop asking and take the detach-and-exit path.

*Regression test:* sabrage/src-tauri/src/commands.rs `mod tests`, alongside the existing `should_intercept_quit` cases: a pure `quit_intercept_decision(quit_approved, session_is_live, pending_since, now)` helper with tests asserting Intercept on the first request, and DetachAndExit once the pending request has gone unanswered past the fallback deadline (or on a repeat request), so the app can never be permanently unquittable.

### A11-5 [low, conf 0.99] Log-tail tasks and registry entries survive channel loss
`sabrage/src-tauri/src/commands.rs:1065-1139` — **CONFIRMED** (re-rated low)

CONFIRMED. Tail tasks remove registry entries only through explicit `stop_log_tail`. A task that exits after a send or poll error never unregisters itself, so the registry later claims a dead tail was stopped successfully. Worse, when a webview channel closes while the log is idle, no send is attempted, so the task cannot detect closure and continues polling every 250 ms for the app’s lifetime. Reloads can accumulate orphan tasks and file I/O.
Evidence: `sabrage/src-tauri/src/commands.rs:1071-1093` removes entries only in `TailRegistry::stop`. `commands.rs:1125-1139` registers the ID and spawns a loop whose `break` paths do not call registry cleanup; its `Ok(_) => {}` path performs no channel liveness check before sleeping. `commands.rs:1143-1147` nevertheless describes `false` for a tail “no longer tracked”.

*Recommendation:* Give each task a registry-owned cleanup guard so every exit removes its ID, and tie tails to webview/page lifecycle. Add a bounded channel-liveness mechanism so an idle source also notices renderer loss.

*Verifier:* The two mechanical claims are unambiguous in the code: no exit path unregisters, and an idle tail performs no liveness check. Both are reachable, but only via a webview reload — the frontend stops tails on every in-app navigation and the single window's close is app-quit — so consequences are a dev-time orphan thread doing 4 stat/read calls a second plus a few bytes of stale map entry, and a `true` return nobody reads. Real, latent, minor: low.

*Fix sketch:* In sabrage/src-tauri/src/commands.rs: hold the map behind an Arc (`tails: Arc<Mutex<HashMap<u64, Arc<AtomicBool>>>>`) so a clone can be moved into the blocking task, and add a `TailGuard { id, tails }` whose `Drop` removes the id (no flag store). start_log_tail constructs the guard right after `registry.register(...)` and moves it into the closure, so every exit — send error, poll error, stop flag — unregisters exactly once; stop_log_tail's `false` then means what its doc says. For idle liveness, either send a heartbeat batch every Nth idle poll (cheap, but see the eval caveat: it will not error on a reload), or — the reliable fix — bind tails to the page: take `webview: tauri::Webview` in start_log_tail, record the webview label with the id, and in lib.rs's builder add `.on_page_load(|webview, _| tails_for_label(webview.label()).stop_all())` so a reload/navigation kills that page's tails deterministically. lib.rs is inside this area.

*Regression test:* sabrage/src-tauri/src/commands.rs `#[cfg(test)] mod tests` (line 1646): a test that registers a stop flag, drops the returned TailGuard, and asserts `registry.stop(id) == false` (entry gone, no leak) while a still-held guard yields `true` and sets the flag. Plus a Logs.svelte-level assertion is not needed; if the on_page_load route is taken, add a test that stop_all_for_label clears every id registered under that label and sets each flag.

*Codex next steps:* Inject teardown lasting longer than 30 seconds, then invoke both `stop_session` and `resolve_quit("stop")`; verify neither reports success nor exits while the live run remains. · Use a barrier executor to race setup against `write_runtime_config`, and `save_game` against `record_last_session`/`remove_game`; verify every acknowledged update survives. · Hold `OPERATION_LOCK`, invoke a second stage, drop its channel, and attempt cancellation using only emitted events; verify it is rejected or cancellable and never starts later. · During a live session, remove or crash the frontend listener before exercising Cmd-Q and window close; verify a bounded native fallback remains available. · Start tails, reload the webview before new log data arrives, and inspect task/registry counts and polling activity; verify no orphan remains.

## A12 — ui-shell-session

**Codex verdict:** needs-attention — No-ship: the UI exposes a documented known-bad destructive repair, allows live-session mutations that can interrupt streaming, and contains several observable state/ordering failures.

### A12-1 [critical, conf 0.99] Doctor exposes a documented known-bad destructive repair without disclosing the failure mode
`sabrage/ui/src/screens/Doctor.svelte:59-63` — **CONFIRMED** (re-rated low)

The `delete-session-json` action deletes unrecoverable ALVR state even though the core explicitly documents that this remedy has produced an 800x900 black screen and that editing pins in place is the working recovery. The UI reduces this to a generic confirmation, so a user following Doctor can turn a recoverable warning into lost configuration and a broken client.
Evidence: `sabrage/ui/src/screens/Doctor.svelte:59-63` maps every destructive action to `confirmFix`, and `Doctor.svelte:211-217` says only `This cannot be undone` before calling `runFix`. The action's authoritative comment says `Known-bad remedy. Deleting the file has been observed to leave the client at an 800x900 black screen` at `sabrage/crates/sabrage-core/src/fixes/mod.rs:69-75`.

*Recommendation:* Do not expose this Fix until the in-place pin editor exists. If deletion must remain available, create a restorable backup and explicitly disclose the observed black-screen outcome and recovery procedure.

*Verifier:* Reachable and unambiguous: `cfg.session-pins` (contract/pipeline.toml:333-338) attaches `fix.delete-session-json`, CheckRow.svelte:20 renders a Fix button for any row whose `fix` maps to a modelled action, and the only destructive action in the registry is DeleteSessionJson (fixes/mod.rs:216-220), so the generic dialog text is written for exactly this one fix. Not critical: the state is backed up and restorable, and the operation is refused while a session is live, so no irreversible loss occurs — the defect is that the user consents without being told what the remedy is known to do.

*Fix sketch:* Give `FIX_META` in ui/src/ipc.ts an optional `consequence` string mirroring a new `FixDef` field in sabrage-core/src/fixes/mod.rs (single source of truth, same hand-mirroring convention commands.rs:57 already documents), populated for DeleteSessionJson with the known-bad outcome + backup location + "edit the pinned IP in place instead". In Doctor.svelte's `confirmFix` block (lines 207-218) render that string instead of the literal "This cannot be undone.", and replace that sentence with the accurate one ("the file is backed up to Application Support/Sabrage/backups first"). No change to the check's message/remedy strings (shell parity).

*Regression test:* sabrage/ui has no JS test harness (ui/package.json has only dev/build/check/tauri), so the enforceable half is Rust-side: extend the existing registry test in sabrage-core/src/fixes/mod.rs (the one at ~line 516 asserting every def is forbidden_while_session_live) with an assertion that every `destructive` FixDef carries a non-empty `consequence` mentioning the observed failure ("800x900"), so the string the GUI renders can never go missing; keep session_json.rs's `deletes_after_backing_up_and_reports_the_backup_location` as the backup guarantee.

*Cross-area files:* sabrage/ui/src/ipc.ts, sabrage/crates/sabrage-core/src/fixes/mod.rs

### A12-2 [high, conf 0.98] Doctor can remove the active ADB forwards from a live wired session
`sabrage/ui/src/screens/Doctor.svelte:31-63` — **CONFIRMED** (re-rated high)

Opening Doctor during a GUI wired session runs the forward check without carrying the launch's `wired` state. The active 9943/9944 forwards are therefore presented as stale, and the resulting Fix button remains enabled while the session is live. Applying it removes the stream's active forwards and can disconnect the headset.
Evidence: `sabrage/ui/src/screens/Doctor.svelte:31-33` passes only `bottle` to Doctor, while `Doctor.svelte:181-185` supplies `onFix` with no session-phase guard. `contract/pipeline.toml:368-373` attaches `fix.remove-adb-forwards` to the check; that fix executes `adb -s <serial> forward --remove <local>` at `sabrage/crates/sabrage-core/src/fixes/adb.rs:138-150`. The registry explicitly marks every fix forbidden while live at `sabrage/crates/sabrage-core/src/fixes/mod.rs:181-183`, but this UI does not enforce that contract.

*Recommendation:* Disable all Fix actions while any session is live and annotate why. Also enforce `forbidden_while_session_live` in the backend so other callers cannot bypass the UI guard.

*Verifier:* Realistic path with a live-stream consequence: user launches wired from Session.svelte (`wired` toggle, Session.svelte:363), hits stutter mid-session, opens Doctor (exactly when one opens Doctor), sees `net.adb-forwards` WARN calling the session's own active forwards stale, clicks Fix; the forwards backing the wired stream are removed while the game runs. The registry declares every fix forbidden while a session is live and neither the UI nor the command layer enforces it.

*Fix sketch:* Three layers. (1) sabrage-core/src/fixes/adb.rs: give `remove_adb_forwards` (the standalone-fix entry point only, NOT `remove_adb_forwards_at` as called from the run preflight) the same refusal shape session_json.rs uses — `ctx.fatal(...)` when a session is live — so the launch path keeps its hygiene step. (2) src-tauri/src/commands.rs `fix`: reject any `action.def().forbidden_while_session_live` while `live_session()` is Some or the managed SessionMonitor reports a running session, mirroring the existing `destructive && !confirmed` guard. (3) Doctor.svelte: import sessionStore, pass a `sessionLive` prop into CheckRow to disable the Fix button with a title explaining why (phases preflight/launching/running/stalled/stopping and any non-idle unowned session).

*Regression test:* sabrage/crates/sabrage-core/src/fixes/adb.rs `mod tests`: add `refuses_while_a_session_is_live`, built exactly like session_json.rs's `refuses_while_any_wineserver_is_alive` (set `ctx.paths.wineserver = current_exe()` so `any_wineserver_alive` reports true), asserting the standalone fix returns Err containing "refusing" and that no adb child was planned/spawned (assert `ctx.executor.planned()` is empty under a DryRunExecutor); plus a test that `remove_adb_forwards_at` with `step::RUN_ADB_FORWARDS` still proceeds, pinning that the run preflight is unaffected.

*Cross-area files:* sabrage/ui/src/ipc.ts, sabrage/src-tauri/src/commands.rs, sabrage/crates/sabrage-core/src/fixes/adb.rs, sabrage/crates/sabrage-core/src/fixes/mod.rs

### A12-3 [high, conf 0.95] Setup, Build, and Install remain executable during a live session
`sabrage/ui/src/components/StagesPanel.svelte:140-148` — **CONFIRMED** (re-rated medium)

The panel has no session-state guard; its real Run buttons are disabled only when Install lacks a bottle. Because the operation lock is deliberately released after Wine launches, these stages execute immediately and can replace build outputs, CrossOver overlays, bottle files, or host registration while a process using them is still running. This creates runtime/on-disk version skew and teardown races.
Evidence: `sabrage/ui/src/components/StagesPanel.svelte:140-148` enables Dry-run unconditionally and real Run based solely on `card.needsBottle && !selectedBottle`. The lock policy confirms that another operation can start during the live session at `sabrage/crates/sabrage-core/src/stages/mod.rs:25-42`; the corresponding whole-stage fixes are all declared forbidden while live at `sabrage/crates/sabrage-core/src/fixes/mod.rs:226-245`.

*Recommendation:* Keep dry-run available, but disable real Setup/Build/Install while the status is preflight, launching, running, stalled, stopping, or detached-live. Add the same refusal at the stage command boundary.

*Verifier:* The path is real and unguarded, but the trigger is an unusual user action (deliberately running Install/Build during a live game) and the outcome is not deterministic breakage — build overwriting the in-use x86_64 runtime dylib or install overwriting the CrossOver overlays under a mapped process is a plausible SIGBUS/skew, not a certainty, and install additionally requires a sudo prompt the user must approve. Latent hazard under unusual conditions ⇒ medium, not high. Dry-run is correctly harmless and should stay enabled.

*Fix sketch:* StagesPanel.svelte: import sessionStore, derive `sessionLive` from `sessionStore.status.phase` (anything other than `idle`, plus a non-idle session not owned by this process), and use it to disable each card's real Run button with an inline explanation while leaving Dry-run and Copy enabled. Back it with a refusal at the command boundary: in src-tauri/src/commands.rs `execute_stage`, reject Setup/Build/Install (never Stop, never Run — Run has its own reconcile refusal) while `live_session()` is Some or the managed SessionMonitor reports running, returning the same Fatal shape the GateModal already renders; put the policy predicate next to OPERATION_LOCK in sabrage-core/src/stages/mod.rs so the CLI inherits it too.

*Regression test:* sabrage/crates/sabrage-core/src/stages/mod.rs `mod tests`: assert the new predicate (e.g. `stage_forbidden_while_session_live(Stage)`) is true for Setup/Build/Install and false for Run/Stop, and that `FixAction::EVERY`'s whole-stage actions agree with it — pinning the registry's `forbidden_while_session_live` flags to an actually-enforced rule instead of dead metadata; plus a stages test that `run_stage(Stage::Build)` returns the refusal Fatal (and plans no mutation) when the liveness probe reports a live session.

*Cross-area files:* sabrage/src-tauri/src/commands.rs, sabrage/crates/sabrage-core/src/stages/mod.rs

### A12-4 [high, conf 1] The Pipeline Launch menu item never launches anything
`sabrage/ui/src/App.svelte:54-57` — **CONFIRMED** (re-rated low)

Selecting Pipeline → Launch or pressing its shortcut merely navigates to the Session screen. It never calls the launch path, supplies options, or tells Session that a launch was requested. This is silent wrong behavior for a command explicitly named Launch.
Evidence: `sabrage/ui/src/App.svelte:54-57` handles `id === "launch"` solely as `navigate("session")`, whereas the menu specification names `Launch ⌘R` at `sabrage/docs/design/design-app.md:208`.

*Recommendation:* Route the menu event through the same launch action used by Session/Library. If no valid bottle/options are selected, disable the menu item or navigate with a visible validation message instead of silently treating Launch as navigation.

*Verifier:* Reachable and user-visible: a menu command named Launch with the conventional run shortcut only changes screens. Harm is limited to a wasted keystroke — the user lands on Session with the Launch button one click away and nothing wrong happens — hence low, not high. Note the same shape applies to Run Doctor (navigating to an already-open Doctor screen re-runs nothing, since checks only fire in `onMount`), which is worth folding into the same fix.

*Fix sketch:* In App.svelte's `onMenu` handler, keep `navigate("session")` but also signal intent: hold a `launchRequest` counter/flag in App and pass it to `Session.svelte` as a prop; Session `$effect`s on it and calls its existing `doLaunch(false)` once its bottle/options have loaded, or — if `bottles.length === 0` or no bottle is selected — leaves a visible validation message instead of launching. Same treatment for `doctor` (bump a token Doctor watches to re-run `runChecks`). No new IPC and no duplicate launch logic: the menu goes through the same `doLaunch`/`sessionStore.launch` path Session and Library use.

*Regression test:* No JS test harness exists in sabrage/ui (package.json has dev/build/check/tauri only), so this is not unit-testable as the repo stands; the checkable half is Rust-side — a test in src-tauri/src/lib.rs asserting every Pipeline menu id maps to a `menu://…` topic the frontend handles (`doctor`/`launch`/`stop`) so a renamed or added item cannot silently become a no-op — with the behavioural assertion recorded as a manual step (⌘R from About starts a launch or shows the validation message).

### A12-5 [medium, conf 0.99] A failed Doctor rerun hides its error behind stale waiting rows
`sabrage/ui/src/stores/doctor.svelte.ts:55-80` — **CONFIRMED** (re-rated medium)

Each rerun retains every prior row as a waiting placeholder. If the new invocation rejects before reporting all slugs, those rows are never removed. Because the screen renders its error card only when the row array is empty, the user sees permanently dim historical results and no explanation that Doctor failed.
Evidence: `sabrage/ui/src/stores/doctor.svelte.ts:59-76` maps all old rows to `phase: "waiting"` and records an error on rejection without pruning them. `sabrage/ui/src/screens/Doctor.svelte:163-189` displays `doctorStore.error` only inside `rows.length === 0`; any retained row suppresses it.

*Recommendation:* Track rows by run generation and finalize or remove placeholders not emitted by the completed run. Render invocation errors independently of whether historical rows exist.

*Verifier:* Code path is unambiguous and reachable: no pruning or per-run generation exists in the store, and the screen gates the error card on an empty row array. Downgraded from 'no explanation at all' to 'explanation suppressed for the non-repo-root rejection, stale dim rows and a wrong summary in all cases' — hence medium, not high: it needs a doctor invocation that rejects after a prior successful run, and nothing is mutated or lost.

*Fix sketch:* In `createDoctorStore.run()` (sabrage/ui/src/stores/doctor.svelte.ts:55-80): capture `const gen = ++runGeneration` and a `const seen = new Set<string>()`; add every event slug to `seen`; in `finally`, drop (or mark a distinct `"stale"` phase) rows whose slug is not in `seen` when the promise rejected, so no row is left in `waiting` while `running === false`. Expose `error` unconditionally and render it in Doctor.svelte as a banner above the rows block (move the `error-card` out of the `rows.length === 0` branch, keeping the 'not run yet' / 'running' placeholders inside it), and make `summaryText` (Doctor.svelte:117-130) return a failure string when `doctorStore.error && !doctorStore.running` instead of 'Running checks…'.

*Regression test:* No UI test runner exists today (sabrage/ui/package.json has only dev/build/check scripts). Either (a) add vitest + @testing-library/svelte and a `sabrage/ui/src/stores/doctor.test.ts` that seeds two rows via a fake `runDoctor` that resolves, then reruns with a `runDoctor` that emits one slug and rejects, asserting: no row keeps `phase === "waiting"` after `run()` settles, `error` is non-null, and the rendered Doctor screen shows the error text while rows exist; or (b) if adding a runner is out of scope, assert the invariant in a Rust-side smoke of commands.rs::run_doctor's two rejection paths and cover the UI with a manual step in sabrage/docs. Preferred is (a).

*Cross-area files:* sabrage/ui/package.json, sabrage/ui/vite.config.ts

### A12-6 [medium, conf 0.97] Rapid Logs tab changes can complete out of order
`sabrage/ui/src/screens/Logs.svelte:146-158` — **CONFIRMED** (re-rated medium)

`switchTab` awaits `stopTail` and then mutates the selected tab without checking whether a newer click superseded it. Since `stopTail` clears `tailId` before awaiting IPC, a second switch can complete immediately; when the older stop later resolves, the older click overwrites the newer tab and starts its tail. The tail-generation guard prevents many leaks but does not protect navigation ordering.
Evidence: `sabrage/ui/src/screens/Logs.svelte:120-135` clears `tailId` before awaiting `stopLogTail`, and `Logs.svelte:146-158` resumes after that await with unconditional `tab = next` and `startTail(...)`.

*Recommendation:* Assign an action generation before awaiting `stopTail` and recheck it before changing `tab` or starting a tail, or serialize switches through a single latest-request-wins transition.

*Verifier:* Reproducible from the real control flow; only condition is a second tab click inside the `stop_log_tail` IPC round trip (a few ms), which rapid clicking hits. Self-correcting on the next click and nothing is mutated on disk, so medium rather than high.

*Fix sketch:* Add a navigation generation alongside `tailGeneration` in Logs.svelte: `let navGeneration = 0`. In `switchTab`/`openPastRun`/`backToPastRuns`, take `const myNav = ++navGeneration` BEFORE `await stopTail()` and return immediately after the await if `myNav !== navGeneration`, so only the newest click assigns `tab`/`openedPastRun` and calls `startTail`. Also set the intended `tab` optimistically (before the await) or compare `next` against a `pendingTab` rather than the stale `tab` in the :149 early-return, so a rapid A->B->A cannot be swallowed as a no-op.

*Regression test:* No UI test runner exists (sabrage/ui/package.json). With vitest added, `sabrage/ui/src/screens/Logs.test.ts` mounting Logs with mocked ipc (`stopLogTail` resolving on a 20ms timer, `startLogTail` on 5ms): click tab B then tab C 1ms later, flush timers, assert the active tab is C and the last `startLogTail` source is `alvrSession`; second case clicks B then A and asserts the active tab is A.

*Cross-area files:* sabrage/ui/package.json, sabrage/ui/vite.config.ts

### A12-7 [medium, conf 0.99] Copied demo.sh commands are not shell-safe
`sabrage/ui/src/lib/demo.ts:23-30` — **CONFIRMED** (re-rated low)

Arbitrary bottle and directory values are placed inside double quotes without escaping double quotes, backslashes, dollar expansions, or backticks. A copied path can therefore be parsed differently from the value sent over IPC; `$()` or backticks embedded in a path execute when pasted into zsh. The stage-panel variant also emits bottle names completely unquoted, so ordinary names containing spaces split into multiple arguments.
Evidence: `sabrage/ui/src/lib/demo.ts:28-30` constructs `"${bottle}"` and `"${bsDir}"` verbatim. `sabrage/ui/src/components/StagesPanel.svelte:66-71` interpolates `selectedBottle` with no quoting at all.

*Recommendation:* Use one tested POSIX/zsh shell-quoting helper for every dynamic argument in both command renderers, preferably single-quote encoding with embedded apostrophes represented as `'\''`.

*Verifier:* The string defects are real and reproducible, and the two renderers disagree with each other (demo.ts:25-27 quotes bottle names with spaces precisely because CrossOver allows them; StagesPanel does not). Re-rated to low, not medium: no trust boundary is crossed — every input is a path or bottle directory the user created on their own machine, the copied command is inert until the user pastes it, and the most likely real case (a space in the bottle name from StagesPanel) fails loudly at demo.sh's arg parser rather than doing something wrong. The `$`/backtick/`\`/`"` cases are silently wrong but require an unusual path.

*Fix sketch:* Add one exported `shQuote(v: string): string` to sabrage/ui/src/lib/demo.ts using single-quote encoding (`'` -> `'\''`), returning the value bare only when it matches /^[A-Za-z0-9_.\/:@%+=-]+$/ (keep the `<name>` placeholder unquoted). Use it for both `--bottle` and `--bs-dir` in `demoRunCommand` (demo.ts:26-30) and import it in StagesPanel.svelte's `demoCommand` (StagesPanel.svelte:64-71) so both renderers emit the same quoting. Note demo.ts's doc comment at :10-16 currently documents the verbatim-double-quote behaviour and must be updated with the change.

*Regression test:* No UI test runner exists. With vitest added, `sabrage/ui/src/lib/demo.test.ts` asserting `demoRunCommand` and StagesPanel's `demoCommand` for: a plain bottle stays bare; `Beat Saber` -> `'Beat Saber'` in BOTH renderers; a bsDir containing `$HOME`, a backtick, a double quote, a backslash and an apostrophe round-trips through `zsh -c 'printf %s'`-equivalent single-quote rules; and the `<name>` placeholder stays unquoted. If a runner cannot be added, hoist `shQuote` and cover it from the existing Rust side only if an equivalent renderer is added there — otherwise the assertion belongs in the UI test file above.

*Cross-area files:* sabrage/ui/package.json, sabrage/ui/vite.config.ts

*Codex next steps:* In an isolated wired session, open Doctor and verify whether `net.adb-forwards` offers Fix and whether applying it removes the active 9943/9944 forwards. · Seed a disposable ALVR `session.json`, trigger `cfg.session-pins`, inspect the confirmation, and verify the deletion/black-screen behavior plus whether any backup is recoverable. · While a supervised session is running, invoke Setup, Build, and Install from the panel and trace whether files mutate immediately after the launch-boundary lock release. · Exercise Pipeline → Launch/⌘R and rapid Logs tab clicks with an artificially delayed `stop_log_tail` response; assert that the requested action and final tab win. · Add UI tests for a Doctor rerun rejection with existing rows and table-driven shell quoting cases containing spaces, quotes, backslashes, `$()`, backticks, and newlines.

## A13a — store-rust

**Codex verdict:** needs-attention — Do not ship: the store layer can lose persisted updates, misreport an unlaunchable game as Ready, and falsely claim a Goldberg DLL was restored as the Steam original.

### A13a-1 [high, conf 0.99] A Goldberg DLL can be classified and restored as the Steam original
`sabrage/crates/sabrage-core/src/store/goldberg.rs:74-95` — **CONFIRMED** (re-rated medium)

If the live DLL is already Goldberg and `.orig-steam` is absent, validation calls it Original; the next run backs those Goldberg bytes up; Revert then copies them back and reports that the original was restored. The user still has Goldberg despite an explicit success claim.
Evidence: `sabrage/crates/sabrage-core/src/store/library.rs:370-375` returns `GoldbergState::Original` solely because `orig_steam_present` is false, before checking the pinned hash. `scripts/demo/run.sh:147-149` copies the current DLL to `.orig-steam` before comparing it with Goldberg. `sabrage/crates/sabrage-core/src/store/goldberg.rs:74-95` validates only that the backup is a file, copies it, and says `restored the original steam_api64.dll`. This failure follows directly when the starting DLL is already Goldberg.

*Recommendation:* Never label a DLL or backup as original without positive provenance. Add an Unverified/Backup state, reject a backup matching the pinned Goldberg hash, and persist verified backup provenance when the original is first captured. Revert must refuse or use neutral wording when provenance cannot be established.

*Verifier:* Code path is unambiguous and needs no timing. Severity lowered from high to medium: it requires an install that was already Goldberg-patched with no `.orig-steam` (third-party Goldberg from a Beat Saber modding guide, a manually deleted backup, a game dir copied without the suffixed backup). On the ordinary path (real Steam dll first) every label and the revert claim are correct, and Sabrage never destroys the real dll itself — the harm is a wrong UI label plus an explicit false success claim, not lost data. The backup-whatever-is-live step is parity-mandated (run.sh:147, PARITY.md decision 20 tolerates a non-pinned dll at run), so the fix must stay on the Sabrage-only surfaces (classification + revert), not in goldberg_stage.

*Fix sketch:* 1) store/library.rs `validate_with_bottle`: test the live dll against the pin BEFORE the backup-presence branch. New matrix: dll==pin && backup -> Applied; dll==pin && !backup -> a new `AppliedUnverified` variant ('Goldberg installed, no verified Steam original on this machine'); dll!=pin && !backup -> Original; dll!=pin && backup -> Modified. 2) store/goldberg.rs `revert_original_steam_dll`: hash `.orig-steam` against `contract().deps.gbe_dll_sha256`; on a match return restored:false with a message saying the backup is itself the Goldberg dll and no verified original exists; otherwise drop the word 'original' from the success string ('restored steam_api64.dll from the .orig-steam backup'). Factor the pin argument into a `pub(crate)` helper so tests can pass the sha of their own fixture bytes.

*Regression test:* store/library.rs unit tests: extend `goldberg_state_covers_all_four_variants` (line 866) with a fifth row asserting a pin-matching dll with no backup is NOT Original. store/goldberg.rs unit tests: new `refuses_when_the_backup_is_itself_the_pinned_goldberg_dll` asserting restored==false and that the message never contains 'restored the original'.

*Cross-area files:* sabrage/ui/src/ipc.ts, sabrage/ui/src/screens/Library.svelte, sabrage/ui/src/screens/EditGame.svelte

### A13a-2 [high, conf 0.98] Revert can race the run stage after its liveness check
`sabrage/crates/sabrage-core/src/store/goldberg.rs:59-86` — **CONFIRMED** (re-rated medium)

Revert does not acquire `OPERATION_LOCK`; its check-then-copy sequence can overlap a run that has applied Goldberg but has not yet published `LIVE_SESSION`. If Revert writes afterward, Wine launches with the real/backup DLL even though the run stage reported Goldberg installed. External CLI/demo.sh sessions are also invisible to this check.
Evidence: `sabrage/crates/sabrage-core/src/store/goldberg.rs:63-86` checks `live_session()` once and later copies without a lock. `sabrage/src-tauri/src/commands.rs:1574-1576` invokes it directly. In `sabrage/crates/sabrage-core/src/stages/run/mod.rs:169`, Goldberg is applied, but `set_live_session` does not occur until line 389 and the run lock remains held until line 422. That interval is a concrete same-process race window.

*Recommendation:* Acquire the process-wide operation lock at the outer command boundary, then repeat the liveness check while holding it and retain it through the copy. Add an exact-process or persisted-session probe before overwriting so sessions launched by another frontend are not treated as idle.

*Verifier:* Race is real in code, but the same-process interleaving the reviewer describes is mostly shadowed by the UI's phase gate; the realistic exposure is the cross-process one (demo.sh session + Sabrage's Revert button) plus the un-regated confirm button. Needs unusual timing or a second frontend, hence medium rather than high.

*Fix sketch:* store/goldberg.rs `revert_original_steam_dll`: take `crate::stages::acquire_operation_lock().await` at entry, then re-check liveness while holding it and hold it through the copy. Widen the liveness probe: accept `&Paths` and also reject when `session::state::load(&paths.session_state_path())` yields a state whose `wine.is_same_process()` holds (catches a `sabrage` CLI session), plus a pgrep-style probe for a wine child under this bs_dir for demo.sh sessions. commands.rs:1564 passes the paths; EditGame.svelte:356 gains `disabled={reverting || !canRevert}`.

*Regression test:* store/goldberg.rs unit tests: `refuses_while_a_persisted_session_is_live` — write a session-state.json under a scratch Paths whose wine identity is this test process, assert the call errors and the dll is untouched; and `waits_for_the_operation_lock`, modelled on fixes/mod.rs:387 `apply_waits_for_the_operation_lock_then_proceeds`.

*Cross-area files:* sabrage/src-tauri/src/commands.rs, sabrage/ui/src/screens/EditGame.svelte

### A13a-3 [high, conf 0.99] Concurrent library transactions silently overwrite each other
`sabrage/crates/sabrage-core/src/store/library.rs:145-172` — **CONFIRMED** (re-rated medium)

Atomic replacement prevents malformed JSON but does not make the load-modify-save transaction atomic. A game removal/edit racing session completion can resurrect the removed entry, discard the edit, or lose `lastSession`, depending only on which full snapshot renames last.
Evidence: `sabrage/crates/sabrage-core/src/store/library.rs:150-172` exposes independent load and whole-file save operations with no revision or serialization. Production performs separate load-modify-save sequences in `sabrage/src-tauri/src/commands.rs:1523-1525` (`save_game`), lines 1541-1544 (`remove_game`), and lines 1629-1640 (`record_last_session`), with no lock spanning each transaction.

*Recommendation:* Serialize every complete library read-modify-write transaction under one outer lock, preferably the required process-wide operation lock, and add a persisted revision/CAS check to reject stale snapshots. Test save, remove, and session-recording interleavings.

*Verifier:* Mechanism is unambiguous and the realistic trigger exists (record_last_session fires from a background task when the game exits, while the user is editing/removing rows in the Library screen). Lowered from high to medium: it needs the two to interleave inside a millisecond-scale load->save window, and the loss is one library entry, one edit, or one lastSession — silent, but re-doable, not machine state.

*Fix sketch:* store/library.rs: add a module-level `static LIBRARY_LOCK: LazyLock<tokio::sync::Mutex<()>>` and a `pub async fn transact<F: FnOnce(&mut Library) -> T>(executor, path, f) -> Result<T>` that holds it across load -> mutate -> save; optionally add a `revision: u64` to `Library`, bumped on save, with a stale-revision rejection for defence in depth. Convert commands.rs `save_game`, `remove_game` and `record_last_session` to call `transact` instead of their own load/save pairs.

*Regression test:* sabrage/crates/sabrage-core/src/store/library.rs unit tests: `concurrent_transactions_do_not_lose_writes` — spawn a removal and a record_last_session through `transact` with `tokio::join!` and assert the removed entry stays removed and no entry is resurrected, whichever order they run in.

*Cross-area files:* sabrage/src-tauri/src/commands.rs

### A13a-4 [high, conf 0.97] Settings autosaves are unordered whole-snapshot writes
`sabrage/crates/sabrage-core/src/store/settings.rs:131-140` — **CONFIRMED** (re-rated low)

Rapid changes launch independent saves of complete Settings snapshots. A later change can finish first and then be overwritten by an older request; an older failed request can also roll the frontend back past a newer successful one. Both controls may still flash Saved.
Evidence: `sabrage/crates/sabrage-core/src/store/settings.rs:133-140` unconditionally serializes and replaces the entire file without a revision. `sabrage/ui/src/stores/settings.svelte.ts:41-50` allows every save to assign its own response or rollback its captured `previous` state, with no queue or request generation. `sabrage/ui/src/screens/Settings.svelte:86-142` starts these saves independently from every field handler.

*Recommendation:* Queue settings saves in user-action order or replace the full-object API with serialized backend patches. Add a revision number so stale writes and stale rollback responses are ignored rather than silently winning.

*Verifier:* Ordering hazard is real, but reachability is thin: every handler is bound to a discrete `onchange` (no per-keystroke save — the bs-dir input commits on change only, Settings.svelte:763), the optimistic update at settings.svelte.ts:42 is synchronous so a later request's payload already contains the earlier change, and a local Tauri IPC + uuid-temp write completes in ~1 ms while two human input events are tens of ms apart. Overlap needs a stalled filesystem or synthetic input, and the surviving state is one stale field that the next edit corrects. Note the cited file is not where the defect lives: settings.rs's whole-file atomic write is the documented design; the ordering responsibility is in the UI store, outside this area's owned list.

*Fix sketch:* settings.svelte.ts: serialize saves on a module-level promise chain (`pending = pending.then(() => ipcSaveSettings(next))`) and carry a monotonically increasing request id so a response or a rollback from a superseded request is discarded. Optionally give `Settings` a `revision: u64` in store/settings.rs that `save` bumps and rejects when the incoming revision is older than the on-disk one, so the guard has a backend half that is testable in Rust.

*Regression test:* store/settings.rs unit tests: `save_rejects_a_stale_revision` — save revision 1, then save a snapshot still carrying revision 0 and assert the call errors and the file still holds the newer snapshot. (A frontend-ordering test would need a UI test runner; sabrage/ui has no vitest setup today.)

*Cross-area files:* sabrage/ui/src/stores/settings.svelte.ts

### A13a-5 [high, conf 1] Validation reports Ready when the required Steam DLL is absent
`sabrage/crates/sabrage-core/src/store/library.rs:367-414` — **CONFIRMED** (re-rated medium)

A game with a valid executable, version, and bottle is marked Ready even when no `steam_api64.dll` exists. The shell-equivalent launch is guaranteed to abort, so the primary status badge contradicts the launch contract.
Evidence: `sabrage/crates/sabrage-core/src/store/library.rs:367-378` computes `GoldbergState::NoDll`, but lines 406-414 derive status without considering it. `scripts/demo/run.sh:143-145` searches both allowed locations and dies when neither DLL exists. The test at `library.rs:798-825` creates no DLL yet explicitly asserts `GameStatus::Ready`, enshrining the mismatch.

*Recommendation:* Add a missing-DLL problem and make `GoldbergState::NoDll` yield NeedsAttention or NotFound. Add a parity test asserting that anything guaranteed to fail run.sh's DLL gate cannot receive Ready.

*Verifier:* Reproduced by code path and by the existing green unit test: validate() computes the NoDll fact and then discards it when deriving both `status` and `problems`, so the primary badge says Ready for a game whose launch is guaranteed to die at the Goldberg stage on both front-ends. Re-rated medium rather than high because the fact is still visible in the detail panel, the failure is a clean fatal with a correct remedy, and the triggering install shape is off the documented path.

*Fix sketch:* In validate_with_bottle (sabrage/crates/sabrage-core/src/store/library.rs): after computing `goldberg`, push a problem when it is GoldbergState::NoDll (text mirroring run.sh:145 / actions.rs:317, e.g. "steam_api64.dll not found under <bs_dir> — is this a complete Beat Saber install?"), and add `|| goldberg == GoldbergState::NoDll` to the NeedsAttention arm of the status ladder (keep NotFound/NeedsSetup ahead of it, since a missing exe or bottle is the more specific verdict). Update the doc comment on GameStatus/validate to state the invariant. Nothing in the GameValidity shape changes, so ipc.ts and the Svelte screens keep working.

*Regression test:* sabrage/crates/sabrage-core/src/store/library.rs #[cfg(test)] mod tests: (a) amend/duplicate a_fully_healthy_game_is_ready_with_no_problems so the Ready fixture also writes a steam_api64.dll (proving Ready still requires it), and (b) add `healthy_game_without_steam_dll_is_not_ready`, which builds the same fixture minus the dll and asserts goldberg == GoldbergState::NoDll, status != GameStatus::Ready (NeedsAttention), and that some problem string contains 'steam_api64.dll' — i.e. any state that would trip run.sh:145 / run::actions::goldberg_stage cannot show Ready.

### A13a-6 [medium, conf 0.99] Older builds silently destroy fields written by newer store schemas
`sabrage/crates/sabrage-core/src/store/settings.rs:48-140` — **CONFIRMED** (re-rated low)

Both stores accept unknown fields and then reserialize only the fields known to the old binary. In `library.json`, a future version number is not rejected, so an old build can preserve `version: 2` while deleting the v2 data it did not understand. Settings autosave makes the same loss occur on the first changed control.
Evidence: `sabrage/crates/sabrage-core/src/store/settings.rs:48-50` uses a closed typed struct with `#[serde(default)]`; lines 123 and 137 deserialize then serialize that reduced shape. `sabrage/crates/sabrage-core/src/store/library.rs:77-85` defines a versioned closed struct, while lines 156 and 169 parse and rewrite it without checking `version > LIBRARY_VERSION` or preserving unknown fields.

*Recommendation:* Preserve unknown fields with flattened maps, or reject newer schema versions before any mutation and implement explicit migrations. Add downgrade round-trip tests proving unknown top-level and per-entry fields either survive byte-for-byte or cause a safe refusal.

*Verifier:* Factually correct and reproduced, but latent: LIBRARY_VERSION is still 1 and no v2 schema exists, so it can only bite after a future schema bump followed by a downgrade to an older binary. The lost data is GUI-only state under ~/Library/Application Support/Sabrage (launch prefs, per-entry fields) — no machine state, no pipeline artifact, re-enterable by hand — hence low rather than the reviewer's medium. Note the modules' doc comments (settings.rs:9-17) declare only forward-tolerance of *older/trimmed* files; downgrade preservation was never claimed, which is why this reads as a missing guard rather than a broken promise.

*Fix sketch:* Cheapest correct fix: in library::load, reject `version > LIBRARY_VERSION` with a SabrageError (a clear 'this library.json was written by a newer Sabrage' message) before any caller can mutate and re-save; give Settings an analogous `schemaVersion` or at least a `#[serde(flatten)] extra: serde_json::Map<String, Value>` catch-all so unknown keys round-trip. If preservation is preferred over refusal, add the flatten catch-all to Settings, Library and GameEntry and serialize it back out (serde_json::Map keeps insertion order; pretty-printer output stays stable).

*Regression test:* sabrage/crates/sabrage-core/src/store/{settings,library}.rs test modules: `newer_library_version_is_refused_not_silently_rewritten` (write version = LIBRARY_VERSION+1, assert load() is Err and the file on disk is byte-unchanged) and `unknown_fields_survive_a_load_save_round_trip` for both stores (write a file with an unknown top-level key and an unknown per-entry key, load, save, assert both keys are still present in the re-read text).

*Codex next steps:* Force `save_game`, `remove_game`, and `record_last_session` through barrier-controlled concurrent transactions and verify removals, edits, and session history all survive. · Mock delayed settings IPC so save B completes before save A, and separately make A fail after B succeeds; compare the final UI state and settings.json with the latest user action. · Start with pinned Goldberg bytes and no `.orig-steam`; run validation, Goldberg backup creation, and Revert, then compare all hashes and displayed state/message. · Pause Revert after its liveness check, start a run through Goldberg preparation, release Revert, and assert locking prevents Wine from reaching launch with the backup DLL. · Add fixtures for an otherwise-valid game with no Steam DLL and for future-version settings/library JSON with unknown fields; assert launch-status parity and lossless round-trip or explicit refusal.

## A13b — ui-settings-library

**Codex verdict:** needs-attention — Do not ship: Library breaks after a completed run, Settings can falsely report saves or lose preferences, and EditGame can overwrite hidden session history or mutate the wrong/unverified DLL.

### A13b-1 [high, conf 0.99] Terminal session state blocks all subsequent Library launches
`sabrage/ui/src/screens/Library.svelte:63-107` — **CONFIRMED** (re-rated high)

CONFIRMED: `exited` intentionally remains published until the next launch, but Library treats every non-idle phase as both busy and running. After the first session ends, every Run button remains disabled; entries sharing the same bottle also display “Running,” hiding their real last-session value.
Evidence: `sabrage/ui/src/screens/Library.svelte:63-64` uses `sessionStore.status.phase !== "idle"` for `busy`, and `:100-107` uses the same predicate plus bottle equality for “Running.” `sabrage/crates/sabrage-core/src/stages/run/mod.rs:235-237,264-273` explicitly preserves `Exited` until the next launch.

*Recommendation:* Use an explicit live-phase predicate that excludes `exited`, and retain the launched `gameId` in session state rather than inferring identity from bottle equality.

*Verifier:* After the first launch in an app session the published phase settles on `exited` and stays there until the next launch, so `busy` stays true forever: every Run button in Library is dead (the only escape is launching from the Session screen, whose predicate excludes `exited`, or restarting the app). Simultaneously `isRunningFor` reports "Running" for the exited session's bottle, hiding the real Last-session timestamp on that row and on any other row using the same bottle.

*Fix sketch:* In Library.svelte replace the two `phase !== "idle"` tests with an explicit live set: `const LIVE: SessionPhase[] = ["preflight","launching","running","stalled","stopping","detached"]` (matching Session.svelte's intent), so `busy = sessionStore.launching || LIVE.includes(status.phase) || stageStore.gate !== null` and `isRunningFor` = `status.phase === "running" || status.phase === "stalled"` plus identity. For identity, stop inferring from `bottle`: have the session store remember the `gameId` of the launch it started (sessionStore.launch already receives `opts.gameId`) and compare that, falling back to bottle equality only for a session this process did not start.

*Regression test:* The UI has no test runner today (sabrage/ui/package.json has only dev/build/check — no vitest, no *.test.ts anywhere under sabrage/ui/src). Minimal honest option: extract `isLivePhase(phase)` into sabrage/ui/src/lib/session.ts typed as `Record<SessionPhase, boolean>` so `npm run check` fails when a new phase is added without a decision, and add a vitest suite `sabrage/ui/src/lib/session.test.ts` asserting isLivePhase("exited")===false and isLivePhase("running")===true plus a Library-level case that Run is enabled at phase exited — wiring vitest requires sabrage/ui/package.json (outside this area).

*Cross-area files:* sabrage/ui/src/stores/session.svelte.ts, sabrage/ui/src/lib/session.ts, sabrage/ui/package.json

### A13b-2 [high, conf 0.99] Revert remains disabled after the run that creates its backup
`sabrage/ui/src/screens/EditGame.svelte:186-186` — **CONFIRMED** (re-rated high)

CONFIRMED: Revert requires phase `idle`. During a run it is non-idle, and after settlement the backend deliberately leaves phase `exited`; the UI then falsely says “A session is live.” A clean run can create `.orig-steam`, but the user must restart the app before this feature becomes usable.
Evidence: `sabrage/ui/src/screens/EditGame.svelte:186` defines `canRevert` as `origSteamPresent && phase === "idle"`, while `:364-365` labels every other phase live. `sabrage/crates/sabrage-core/src/stages/run/mod.rs:235-237` says `Exited` survives until the next launch.

*Recommendation:* Disable only for genuinely live phases (`preflight`, `launching`, `running`, `stalled`, `stopping`, and live `detached`), not `exited`; add a regression test for run → exited → revert.

*Verifier:* Same root cause as A13b-1: `exited` is treated as live. From the first launch onward in an app session, Revert is greyed out and the screen states a falsehood ("A session is live") when no session exists; the user must quit and reopen Sabrage to use the feature.

*Fix sketch:* Use the same live-phase predicate as the A13b-1 fix: `canRevert = !!validity?.origSteamPresent && !isLivePhase(sessionStore.status.phase)` (live = preflight/launching/running/stalled/stopping/detached), and gate the "A session is live — stop it first." note on the same predicate so `exited`/`idle` never show it.

*Regression test:* No UI test runner exists (see A13b-1). With `isLivePhase` extracted to sabrage/ui/src/lib/session.ts, a vitest case in sabrage/ui/src/lib/session.test.ts asserting `exited` is not live covers both screens; a component-level test that EditGame's Revert is enabled at phase `exited` with origSteamPresent=true needs vitest + @testing-library/svelte wired into sabrage/ui/package.json (outside this area).

*Cross-area files:* sabrage/ui/src/lib/session.ts, sabrage/ui/package.json

### A13b-3 [high, conf 0.99] A failed settings load still enables controls and can report a fake save
`sabrage/ui/src/stores/settings.svelte.ts:22-30` — **CONFIRMED** (re-rated medium)

CONFIRMED: A corrupt first load leaves `settings` null but marks the store loaded. Controls become enabled, `update()` silently returns, and the caller flashes “Saved.” On a later failed reload, the store retains its prior snapshot, so an autosave can replace the corrupt file despite the advertised hard-error policy.
Evidence: `sabrage/ui/src/stores/settings.svelte.ts:22-30` retains the prior value and sets `loaded = true`; `:63-65` silently returns when null. `sabrage/ui/src/screens/Settings.svelte:86-90` flashes success after that no-op, and controls use only `!settingsStore.loaded` at `:660,675,684,699,718,744,764`. The backend calls this the screen that must surface corruption at `sabrage/src-tauri/src/commands.rs:1290-1295`.

*Recommendation:* Track successful loading separately, clear or quarantine stale state on failure, make `update()` reject without a fresh value, and require an explicit backup-and-reset recovery action before replacing corrupt JSON.

*Verifier:* With a corrupt settings.json the screen still presents enabled controls seeded from hardcoded local defaults, flashes "Saved" for changes it never persisted (default bottle, default BS dir, adb-probe toggle), and silently no-ops "Change checkout…" — the one recovery action a user with a broken settings file is likely to reach for. If the corruption instead appears after a successful load, the next autosave writes the pre-corruption snapshot over the file, contradicting store/settings.rs's documented "never a silent reset" policy.

*Fix sketch:* In settings.svelte.ts add a `loadOk` flag set only on success (keep `loaded` as "a load has been attempted"), null/quarantine `settings` on a failed load instead of retaining the previous snapshot, and make `update()` reject (`throw new Error("settings not loaded")`) rather than silently return. In Settings.svelte gate all settings controls on `settingsStore.loadOk` (not `loaded`), and replace the corrupt-file path with an explicit "back up settings.json and reset to defaults" action so replacing the file is a deliberate user choice; `persistSettings` should only `flashSaved()` when the save actually happened.

*Regression test:* Needs a UI test runner (absent). With vitest wired into sabrage/ui/package.json, a suite `sabrage/ui/src/stores/settings.test.ts` mocking ../ipc: (a) getSettings rejects -> `loadOk===false`, `settings===null`, and `update({})` rejects (no saveSettings call); (b) load OK then a rejecting reload -> `settings` cleared and no saveSettings call on a subsequent update.

*Cross-area files:* sabrage/ui/package.json

### A13b-4 [high, conf 0.98] Concurrent autosaves can silently lose a newer preference
`sabrage/ui/src/stores/settings.svelte.ts:41-65` — **CONFIRMED** (re-rated medium)

CONFIRMED: Each autosave shallow-merges against mutable optimistic state, sends the entire Settings object, and independently rolls back to its own captured predecessor. Rapid toggles can overlap; an older request completing last overwrites the newer file, while an older failure can roll the UI behind a successful newer write. The backend directly writes each supplied snapshot without serialization.
Evidence: `sabrage/ui/src/stores/settings.svelte.ts:41-50` captures `previous`, optimistically assigns, awaits IPC, then independently assigns or rolls back; `:63-65` constructs a full snapshot. `sabrage/ui/src/screens/Settings.svelte:654-725` wires multiple controls to independent autosaves. `sabrage/src-tauri/src/commands.rs:1301-1306` saves directly without `OPERATION_LOCK` or a revision check.

*Recommendation:* Serialize updates through one queue and merge patches server-side under the process mutation lock, or add a revision/CAS field so stale whole-object writes are rejected and reloaded.

*Verifier:* Overlapping autosaves are ordinary (a text field's blur-commit immediately followed by a checkbox click starts a second save while the first is in flight), and nothing serializes them: last-writer-wins on whole snapshots means an older request landing last silently reverts a newer preference, and an older failure rolls both the store and the controls behind a value already on disk, with no indication until the next reload.

*Fix sketch:* Serialize in the store: keep a `let chain: Promise<void>` and have `save`/`update` queue onto it so at most one saveSettings is in flight and each merge sees the settled value; drop the per-call `previous` rollback in favour of re-reading from disk (a `load()`) after a failure, so the UI converges on what was actually written. Optionally harden the backend by taking OPERATION_LOCK (or a settings-specific mutex) in save_settings, or adding a revision field so a stale whole-object write is rejected.

*Regression test:* Needs a UI test runner (absent). With vitest wired in, `sabrage/ui/src/stores/settings.test.ts` mocks saveSettings with per-call delays [40ms, 5ms], fires two overlapping `update()`s, and asserts the last write to reach the mock is the superset (and that a rejected first save leaves the store equal to the successful second save, not to `previous`). A Rust-side counterpart is possible only if serialization moves into save_settings (sabrage/src-tauri/src/commands.rs).

*Cross-area files:* sabrage/ui/package.json, sabrage/src-tauri/src/commands.rs

### A13b-5 [high, conf 0.98] Saving an edit can erase a newly recorded last session
`sabrage/ui/src/screens/EditGame.svelte:217-223` — **CONFIRMED** (re-rated medium)

CONFIRMED: EditGame clones and later submits the entire `GameEntry`, including hidden server-owned `lastSession`. If a session ends while the editor is open—or after opening Edit but before the post-run refresh finishes—the backend records the new session, then Save replaces the whole entry with the stale clone and deletes that record. Edit remains available while a run is busy.
Evidence: `sabrage/ui/src/screens/EditGame.svelte:79` clones `row.entry`, and `:217-223` sends it unchanged as a whole entry. `sabrage/ui/src/screens/Library.svelte:283-288` leaves Edit enabled during a session. `sabrage/crates/sabrage-core/src/store/library.rs:64-72` includes `last_session` in `GameEntry`, while `:99-103` replaces the entire entry; the independent writer is `sabrage/src-tauri/src/commands.rs:1622-1641`.

*Recommendation:* Accept an editable-fields DTO and preserve `id`, `appid`, `addedAtUnixMs`, and `lastSession` server-side. Add optimistic revision checking for concurrent edits.

*Verifier:* Reachable and unambiguous, but the lost data is only the last-session metadata row (timestamp/exit/log basename; the log file itself survives), it is not machine state, and it requires the user to open Edit before a run's record lands and Save afterwards — not the main Library->Run->return flow. Material but not 'user-visible wrong behaviour on the common path', hence medium rather than high.

*Fix sketch:* Make save_game field-selective instead of whole-entry: in commands.rs::save_game, load the library, look up the existing entry by id and copy the server-owned fields (`last_session`, and defensively `added_at_unix_ms`/`appid`) from the stored entry onto the incoming one before `upsert` — e.g. add `Library::upsert_editable(&mut self, incoming) -> &GameEntry` in store/library.rs that performs that merge, and have save_game call it. UI unchanged (wire shape stays GameEntry). Optional follow-up: a revision counter on GameEntry for concurrent-edit detection.

*Regression test:* sabrage/crates/sabrage-core/src/store/library.rs `mod tests`: `upsert_editable_preserves_server_owned_last_session` — upsert entry, record_last_session, upsert a stale clone taken before the record, assert `get(id).last_session` is still Some(recorded) while the edited name/bs_dir took effect. Mirror it at the command level in sabrage/src-tauri/src/commands.rs if the merge is factored into a pure fn there.

*Cross-area files:* sabrage/src-tauri/src/commands.rs, sabrage/crates/sabrage-core/src/store/library.rs

### A13b-6 [high, conf 0.99] Revert validates the draft path but mutates the persisted path
`sabrage/ui/src/screens/EditGame.svelte:193-200` — **CONFIRMED** (re-rated medium)

CONFIRMED: The form validates the current unsaved `entry.bsDir`, but Revert sends only `gameId`. The backend reloads the saved entry and mutates its old `bs_dir`. Changing the path and clicking Revert before Save can therefore overwrite `steam_api64.dll` in a different installation than the one displayed and validated.
Evidence: `sabrage/ui/src/screens/EditGame.svelte:100-105` validates `entry.bsDir`, while `:193-200` invokes `revertOriginalSteamDll(gameId)`. `sabrage/src-tauri/src/commands.rs:1564-1575` resolves that ID from `library.json` and passes the persisted `entry.bs_dir` to the mutation.

*Recommendation:* Disable Revert while path/bottle fields differ from the persisted row, display the exact target path, and have the backend compare an expected path/revision before mutating.

*Verifier:* Real target/display mismatch that writes a file. Bounded: it fires only when the draft path differs AND the *persisted* install also has a .orig-steam (otherwise goldberg.rs:73-84 returns restored:false and touches nothing), the write is recoverable (the next launch reinstalls Goldberg, .orig-steam is left in place — goldberg.rs:10-16), and the returned RevertReport.message names the actual backup path, so the user does see the wrong install afterwards. Medium, not high.

*Fix sketch:* Two halves. UI (EditGame.svelte): gate `canRevert` on the draft matching the persisted row (`entry.bsDir === savedEntry.bsDir && entry.bottle === savedEntry.bottle`, keeping a copy of the loaded row), show the resolved dll path in the confirmation text, and add a 'save your path change first' note when dirty. Backend (commands.rs::revert_original_steam_dll): take an `expected_bs_dir: String` alongside `game_id` and return an error when it differs from the persisted `entry.bs_dir` (fail closed rather than mutate the wrong install); ipc.ts revertOriginalSteamDll gains the argument.

*Regression test:* sabrage/src-tauri/src/commands.rs `mod tests`: factor the guard into a pure `fn revert_target(entry_bs_dir: &str, expected: &str) -> Result<&Path, String>` and assert it rejects a mismatch and accepts an exact match. UI side: a Vitest/component test (or, if none exists, extend the existing EditGame tests) asserting the Revert button is disabled once `entry.bsDir` is edited away from the loaded value even while `validity.origSteamPresent` is true.

*Cross-area files:* sabrage/src-tauri/src/commands.rs, sabrage/ui/src/ipc.ts

### A13b-7 [high, conf 0.99] The UI calls an unverified backup the original Steam DLL
`sabrage/ui/src/screens/EditGame.svelte:350-369` — **CONFIRMED** (re-rated low)

CONFIRMED: Both the state label and destructive confirmation claim “original,” but neither the backup nor a no-backup live DLL is authenticated. The run path copies whatever DLL existed into `.orig-steam`; if that DLL was already Goldberg or modified, Revert copies those bytes back and reports that the original was restored.
Evidence: `sabrage/ui/src/screens/EditGame.svelte:40-44` labels the no-backup state “original,” and `:350-362` asks to restore the original. `sabrage/crates/sabrage-core/src/store/library.rs:367-374` classifies any present DLL without a backup as `Original`. `scripts/demo/run.sh:147` and `sabrage/crates/sabrage-core/src/stages/run/actions.rs:329-342` back up the pre-existing bytes without verification; `sabrage/crates/sabrage-core/src/store/goldberg.rs:86-95` copies the backup and claims success.

*Recommendation:* Rename the operation to “restore backup” unless a trusted version-specific original hash/signature is verified. Surface source/target hashes and refuse to claim or classify bytes as original without evidence.

*Verifier:* The mislabel is real and cheap to fix on the hash side, but the consequence is wording/classification only: no data is destroyed, the Revert message already names the exact `.orig-steam` path it restored, and there is no trusted per-version hash of the genuine Steam dll available to authenticate 'original' in the strong sense the finding asks for. Also note the whole action is a declared Sabrage-only divergence (PARITY.md:108) with no shell counterpart. Low, not high.

*Fix sketch:* library.rs validate_with_bottle: consult the pin on the no-backup branch too — if `file_sha256_matches(&api, &contract().deps.gbe_dll_sha256)` while `!orig_steam_present`, classify as `Applied` (or a new `AppliedNoBackup`) instead of `Original`. Rewording: EditGame.svelte GOLDBERG_LABEL + the confirm text (and Library.svelte's copy) say 'restore the .orig-steam backup' rather than 'the original', and goldberg.rs's success message drops the 'original' claim in favour of 'restored steam_api64.dll from the .orig-steam backup'. Surface RevertReport.dllPath next to the message.

*Regression test:* sabrage/crates/sabrage-core/src/store/goldberg.rs / store/library.rs tests: `a_pinned_goldberg_dll_without_a_backup_is_not_reported_original` — write the contract-pinned gbe dll (or stub the hash via the existing fixture helper) at the steam_api path with no `.orig-steam`, assert `validate(...).goldberg != GoldbergState::Original`. Plus a message assertion in goldberg.rs's existing `restores_the_backup…` test that the success text does not claim 'original'.

*Cross-area files:* sabrage/crates/sabrage-core/src/store/library.rs, sabrage/crates/sabrage-core/src/store/goldberg.rs

### A13b-8 [medium, conf 0.99] Unknown settings fields are destroyed by any autosave
`sabrage/ui/src/stores/settings.svelte.ts:63-65` — **CONFIRMED** (re-rated low)

CONFIRMED: Loading deliberately ignores unknown JSON fields, while every UI update serializes the known typed Settings object. Running an older Sabrage build against a newer settings schema and toggling one field irreversibly strips all newer fields.
Evidence: `sabrage/ui/src/stores/settings.svelte.ts:63-65` saves a reconstructed whole object. `sabrage/crates/sabrage-core/src/store/settings.rs:12-17` says unknown fields are ignored, and `:133-140` serializes only `Settings`; `sabrage/ui/src/ipc.ts:765-768` explicitly documents the result as `unknown-field-stripped`.

*Recommendation:* Preserve unknown keys with a flattened raw map or perform field-level JSON patching. Add a schema version and refuse writes from an older binary when safe preservation is impossible.

*Verifier:* Mechanism is exactly as described, but it is a documented, deliberate forward-compat choice on a GUI-only preferences file with six re-settable fields (repoRoot, defaultBottle, defaultBsDir, launch flags, allowAdbProbes, runtimeConfigEditAcknowledged). It requires downgrading the app — there is no multi-writer or cross-machine sync — and nothing here is a parity artifact (settings.json has no demo.sh counterpart), so the worst case is that a few preferences fall back to defaults. Latent and minor: low, not medium.

*Fix sketch:* store/settings.rs: add `#[serde(flatten)] pub extra: serde_json::Map<String, serde_json::Value>` to `Settings` (skipped when empty) so load captures and save re-emits unknown keys verbatim, and add a `version: u32` field so a future load can refuse/warn on a newer schema. commands.rs get_settings/save_settings pass the field through untouched; ipc.ts's `Settings` gets an opaque passthrough member (or the UI keeps the object it loaded and spreads onto it, which settings.svelte.ts already does via `{ ...settings, ...patch }` — that alone preserves the extras once they exist on the wire).

*Regression test:* sabrage/crates/sabrage-core/src/store/settings.rs `mod tests`: `unknown_fields_survive_a_load_modify_save_round_trip` — write a settings.json containing a `"futureField": {"a":1}` plus known keys, `load`, flip `allow_adb_probes`, `save` through RealExecutor into a scratch dir, then re-read the raw JSON and assert `futureField` is byte-identically present and the flipped field took effect.

*Cross-area files:* sabrage/crates/sabrage-core/src/store/settings.rs, sabrage/src-tauri/src/commands.rs, sabrage/ui/src/ipc.ts

### A13b-9 [medium, conf 0.98] Debounced validation can apply an obsolete result to a newer draft
`sabrage/ui/src/screens/EditGame.svelte:100-110` — **REFUTED** (re-rated low)

CONFIRMED: Debouncing cancels only pending timers, not an already-running validation request. There is no request generation or captured-argument check before assigning `validity`, and either request can independently clear `validating`. A slow old path can therefore overwrite the result for the currently displayed path and influence the Revert affordance.
Evidence: `sabrage/ui/src/screens/EditGame.svelte:100-110` assigns every response directly to shared state; `:113-126` only clears `validateTimer`. The backend may read the full `globalgamemanagers` fallback at `sabrage/crates/sabrage-core/src/util/mod.rs:113-123`, making response reordering plausible.

*Recommendation:* Capture the validated path/bottle with a monotonically increasing request ID and apply success, failure, and loading completion only if that request is still current.

*Verifier:* Sync (blocking) Tauri command: handled inline on the main thread, so concurrent invokes serialize and resolve in call order; the newest debounced request is always the last one applied.

### A13b-10 [medium, conf 0.99] Goldberg validity remains stale for the entire running session
`sabrage/ui/src/screens/Library.svelte:93-95` — **CONFIRMED** (re-rated low)

CONFIRMED: Run applies Goldberg before Wine launches, but Library refreshes only when the launch promise settles, which occurs after the game exits—potentially hours later. A user who remains on Library sees “original … applied at next launch” even though this launch already changed the DLL.
Evidence: `sabrage/ui/src/screens/Library.svelte:93-95` attaches refresh only to promise settlement, while `sabrage/ui/src/ipc.ts:421-429` states that promise does not resolve until session end. Goldberg is copied before launch at `sabrage/crates/sabrage-core/src/stages/run/actions.rs:329-358`, and the stale state is rendered at `sabrage/ui/src/screens/Library.svelte:265-267`.

*Recommendation:* Refresh the affected row on the launch-local `launched` event, after Goldberg preparation has completed, and refresh again on settlement for last-session data.

*Verifier:* Reachable one-click path (Run → Hide, stay on Library) leaves the row's freshly-computed-at-fetch validity stale for hours, but nothing acts on it — purely a misleading label.

*Fix sketch:* In Library.svelte, add an `$effect` that watches `sessionStore.launchedAt` (already exposed by sabrage/ui/src/stores/session.svelte.ts:41-47, 193-195) and calls `void libraryStore.refresh()` when it transitions to a new non-null value, keeping the existing settlement refreshes at :93-95 for lastSession data. Guard against refetch loops with a local `lastRefreshedLaunchAt` scalar, and skip while `libraryStore.loading`.

*Regression test:* No JS test harness exists (sabrage/ui/package.json has no test script and no vitest dep), so this needs one added: a vitest + @testing-library/svelte spec at sabrage/ui/src/screens/Library.test.ts that mounts Library with a stubbed ipc layer, sets `sessionStore.launchedAt` to a new timestamp without settling the launch promise, and asserts `getLibrary` was invoked a second time and the rendered Goldberg line flips from 'original …' to 'applied'.

*Cross-area files:* sabrage/ui/package.json, sabrage/ui/vite.config.ts

*Codex next steps:* Add a session-state UI test that drives `running → exited` and verifies Library Run is enabled, only the launched game is identified, and EditGame Revert is not labeled as blocked by a live session. · Exercise corrupt settings on both first load and reload-after-success; assert controls cannot report “Saved” and the corrupt bytes remain untouched until explicit recovery. · Use controllable deferred `saveSettings` promises to complete two autosaves in reverse order and fail the older request; verify the final UI and persisted object retain both patches. · Open EditGame from a stale row, advance `lastSession`, change `bsDir`, then Save/Revert; verify metadata survives and no DLL outside the displayed target is touched. · Create Goldberg fixtures where the initial DLL is already Goldberg or modified and `.orig-steam` is absent/present; verify the UI never calls unverified bytes original and validation generations discard late responses.

## A14 — cli

**Codex verdict:** needs-attention — No-ship: the CLI diverges from shell argument semantics, corrupts streamed progress output, mishandles per-stream color detection, and can strand detached or privileged child work after a second signal.

### A14-1 [high, conf 0.99] Empty option values override shell fallback semantics
`sabrage/crates/sabrage-cli/src/main.rs:505-516` — **CONFIRMED** (re-rated medium)

An explicit empty `--bs-dir` or `--bottle` is retained as `Some("")`. Consequently, `--bs-dir ""` resolves to the current directory instead of the bottle default, while `--bottle ""` produces a different missing-bottle path. This is realistic for wrappers that always interpolate optional variables and can probe or launch an unintended directory.
Evidence: `sabrage/crates/sabrage-cli/src/main.rs:505-516` stores values unconditionally with `out.bottle = Some(v.clone())` and `PathBuf::from(v)`; `main.rs:667-672` applies those empty values over the environment. `sabrage/crates/sabrage-core/src/paths.rs:443-445` preserves the empty override. In contrast, `demo.sh:32-35` exports the value and `scripts/demo/lib.sh:92-99` uses `[ -n "${WINEVR_BOTTLE:-}" ]` and `${WINEVR_BS_DIR:-<default>}`, both of which treat empty as absent.

*Recommendation:* When an explicitly supplied value is empty, clear the corresponding environment-derived option to `None`; add tests covering empty CLI values both with and without preset `WINEVR_BOTTLE`/`WINEVR_BS_DIR`.

*Verifier:* Real divergence, but its worst realistic outcome is a differently-worded abort, not a wrong launch. `--bottle ""`: shell dies with `CrossOver bottle name required…`, native takes the other branch of require_bottle (stages/mod.rs:372-395) and dies `bottle '' not found at …/Bottles/`. The sharper case is `sabrage setup --bottle ""`: stages/setup.rs:290 runs `require_bottle` whenever `bottle_name.is_some()`, so the stage FAILS where `./demo.sh setup --bottle ""` skips require_bottle and only warns — a stage-level outcome flip. `--bs-dir ""` yields a cwd-relative game path, which normally just fails the game.present gate with a nonsense path in the message; launching something unintended additionally requires cwd to hold a `Beat Saber.exe`. Realistic only via a wrapper that always interpolates an unset variable, hence medium, not high.

*Fix sketch:* Keep the parse as-is (StageArgs/DoctorArgs must still distinguish 'not given' from 'given empty'), and collapse empty at the merge, where the shell's `export X=""` semantics live: in merge_stage_options (main.rs:660-690) set `opts.bottle_name = (!b.is_empty()).then(|| b.clone())` and `opts.bs_dir_override = (!d.as_os_str().is_empty()).then(|| d.clone())` — i.e. an explicit empty CLI value CLEARS the env-derived value rather than overriding it with Some(""). Apply the same two lines to doctor's merge onto CheckOptions (main.rs:667-672).

*Regression test:* sabrage/crates/sabrage-cli/src/main.rs unit tests, next to the existing merge_stage_options tests: assert `merge_stage_options(StageOptions{bottle_name:Some("Steam".into()), bs_dir_override:Some("/preset".into()), ..Default::default()}, &StageArgs{bottle:Some(String::new()), bs_dir:Some(PathBuf::new()), ..Default::default()})` yields `bottle_name == None` and `bs_dir_override == None` (empty clears a preset WINEVR_* exactly as `${VAR:-}` does), plus the same assertion with no preset, and the doctor-side merge equivalent.

### A14-2 [high, conf 0.97] Second-signal termination bypasses all child and guard cleanup
`sabrage/crates/sabrage-cli/src/main.rs:1131-1135` — **REFUTED** (re-rated low)

A second INT/TERM terminates the process through the kernel without unwinding. During `run`, this strands the detached wine process and potentially leaves audio, dashboard, and adb guards active. During the administrator prompt, it also bypasses the `kill_on_drop` child and staged-file destructors, so an orphaned `osascript` authorization/write can outlive the CLI. A second SIGTERM additionally exits 143 despite PARITY.md's unqualified promise that signal cancellation exits 130.
Evidence: `sabrage/crates/sabrage-cli/src/main.rs:1131-1135` restores `SIG_DFL` and calls `raise(LAST_SIGNAL...)`; the file itself admits at `main.rs:1038-1045` that wine remains running and guards may remain. `sabrage/crates/sabrage-core/src/privilege.rs:512-525` relies on `kill_on_drop(true)`, while `privilege.rs:586-588` relies on `Drop` to delete the privileged staging file—neither runs after default signal termination. `sabrage/PARITY.md:74` says cancellation exits 130 regardless of initiating signal, and `scripts/demo/run.sh:176-181` performs wine/guard cleanup before re-raising.

*Recommendation:* Replace the abrupt second-signal action with a bounded hard-cancel path that explicitly terminates every registered child/process group and preserves or restores guards before exiting. Add real-process tests for double SIGINT/SIGTERM during wine supervision and the administrator prompt.

*Verifier:* Not a defect: this is the declared escape hatch, matching the shell's own kernel-default disposition after `trap - INT`, with a designed reconcile path for the stranded guards. The privilege sub-claim is minor and not a trust-boundary break: the orphaned `osascript` completes the very elevated write the user just authorized, and the leaked StagedTemp (privilege.rs:551-590) is a 0600 randomly-named copy of the non-secret host-manifest JSON under ~/Library/Application Support/Sabrage/tmp. The only residual is a wording nit in PARITY.md:74 (it could say the 130 promise covers a completed cancellation, not an impatient double-tap) — and PARITY.md is outside this area's owned file.

### A14-3 [high, conf 0.99] Output passthrough converts carriage-return progress into newline spam
`sabrage/crates/sabrage-cli/src/main.rs:594-600` — **CONFIRMED** (re-rated medium)

`StageEvent::Output` is not passed through byte-for-byte. The core removes whether each chunk ended in LF, CR, or EOF, and the CLI then appends LF with `println!`. Curl and build-tool progress that should repaint one terminal line therefore becomes many permanent lines; a final unterminated chunk also gains a newline.
Evidence: `sabrage/crates/sabrage-cli/src/main.rs:594-600` retains only `chunk`, and `main.rs:645-650` prints every chunk with `println!`. `sabrage/crates/sabrage-core/src/process.rs:178-210` splits on both `\n` and `\r` while discarding the delimiter. The shell runs `curl --progress-bar` directly at `scripts/demo/lib.sh:163`, preserving its carriage-return behavior.

*Recommendation:* Preserve an output chunk's terminator—or carry raw bytes—through `StageEvent::Output`, then render with `print!` and the original delimiter while retaining `--quiet` suppression.

*Verifier:* Reproducible and on the standard first-run path (`sabrage setup` fetches the pinned DXMT/Goldberg binaries), but the damage is console noise only — no wrong state, no wrong exit code, and `--quiet` already suppresses it. Downgraded from high to medium: user-visible cosmetic parity break, not wrong behaviour.

*Fix sketch:* Carry the terminator through the event: add a delimiter to ChunkSplitter's `out` callback (process.rs push/finish — `\n`, `\r`, or none-at-EOF) and a field to StageEvent::Output (events.rs:~200), default `\n` for existing emitters (privilege.rs, stages/build.rs, stages/install.rs, stages/run/actions.rs). In this area's file, have stage_event_lines carry it on RenderedLine and render_stage_event use `print!("{s}{term}")` + `stdout().flush()` instead of `println!`, keeping the `quiet` suppression untouched.

*Regression test:* sabrage/crates/sabrage-cli/src/main.rs unit tests beside the existing Output tests (main.rs:1752-1780): assert a CR-terminated chunk renders as `"…\r"` (no LF) and an EOF chunk renders with no terminator, while an LF chunk is unchanged; plus a sabrage-core test in process.rs asserting ChunkSplitter reports `\r` for `b"a\rb\n"` and none for the EOF flush.

*Cross-area files:* sabrage/crates/sabrage-core/src/process.rs, sabrage/crates/sabrage-core/src/events.rs, sabrage/ui/src/ipc.ts

### A14-4 [medium, conf 0.99] Flags before the command produce the wrong parity diagnostic
`sabrage/crates/sabrage-cli/src/main.rs:132-150` — **CONFIRMED** (re-rated low)

Both implementations reject flags before the command, but they emit different bytes. Rust dispatches on the first token before parsing the tail; the shell records that token as `CMD`, then parses all remaining tokens before dispatch. For example, `--bottle Steam run` prints the full Rust usage but `demo.sh` reports `error: unknown argument 'Steam'`, both with exit 2.
Evidence: `sabrage/crates/sabrage-cli/src/main.rs:132-150` immediately enters the default usage branch when `args[0]` is not a command. `demo.sh:28-42` assigns and shifts `CMD` before running its argument loop, and only dispatches at `demo.sh:46-57`.

*Recommendation:* Mirror the shell's sequencing: capture the first token as the command, validate the remaining arguments before unknown-command fallback, and add byte/stream/exit assertions for flags preceding the command.

*Verifier:* Real byte divergence, but both sides reject with exit 2 in every arrangement I could construct; only the diagnostic differs (full usage on stdout vs `error: unknown argument 'Steam'` on stderr). Where the tail is all valid flags (`sabrage --verbose --bottle X`) both sides print usage and exit 2, so no outcome flips. Cosmetic/diagnostic only, hence low, not high.

*Fix sketch:* In main() (main.rs:126-152) mirror the shell's sequencing: keep `args[0]` as the command, and in the `_` arm first run `parse_stage_args(&args[1..])` — on `Err(msg)` print `msg` to stderr and exit 2 (the shell's first-bad-argument-wins text), otherwise fall through to the usage/exit-2 fallback. Extract that arm into a small pure helper (e.g. `fn unknown_command_outcome(args: &[String]) -> Result<(), String>`) so it is testable without `exit`.

*Regression test:* sabrage/crates/sabrage-cli/src/main.rs unit tests next to unknown_argument_message_matches_demo_sh_verbatim (main.rs:1246-1258): assert the new helper returns `"error: unknown argument 'Steam'"` for `["--bottle","Steam","run"]` (stderr, exit 2) and `Ok(())` — i.e. usage-on-stdout fallback — for `["--verbose","--bottle","X"]`.

### A14-5 [medium, conf 0.99] Color gating uses stdout state for stderr fatal output
`sabrage/crates/sabrage-cli/src/main.rs:465-476` — **CONFIRMED** (re-rated low)

Fatal rows are routed to stderr but color selection is based exclusively on stdout. With stdout attached to a terminal and stderr redirected, ANSI escapes leak into the error file; with stdout piped and stderr on a terminal, fatal output loses color. This contradicts the ledger's claim that native output strips color for non-terminals.
Evidence: `sabrage/crates/sabrage-cli/src/main.rs:465-476` explicitly calls only `stdout().is_terminal()`. `main.rs:624-629` routes `Fatal` rows to `RenderedLine::Stderr`. `sabrage/PARITY.md:13` states that console colors are gated on isatty plus `NO_COLOR` and stripped for non-terminals.

*Recommendation:* Compute color eligibility separately for stdout and stderr, applying the destination stream's `is_terminal()` result together with `NO_COLOR`.

*Verifier:* Real but cosmetic: the only effects are ANSI bytes in a redirected error log (identical to demo.sh's own behavior) or a colorless FATAL when stdout is piped. No content change — PARITY.md:13's byte-identical claim is about strip-ANSI-equal content, which still holds.

*Fix sketch:* Split `use_colors()` into `use_colors_for(stream)`: keep the `NO_COLOR` short-circuit, then test `std::io::stdout().is_terminal()` / `std::io::stderr().is_terminal()`. Thread two booleans (or a small `Colors { stdout: bool, stderr: bool }`) through `cmd_stage`/`cmd_all` into `stage_event_lines`, choosing `stderr` for the `Fatal` arm (main.rs:625-629) and `stdout` elsewhere; doctor (main.rs:277) keeps the stdout value. Update main.rs:465-471's doc comment.

*Regression test:* In sabrage/crates/sabrage-cli/src/main.rs's `mod tests` (near the existing `stage_event_lines` Fatal test at ~main.rs:1789): assert that with stdout-colors=false and stderr-colors=true a `StageEvent::Fatal` renders `RenderedLine::Stderr("\u{1b}[31mFATAL\u{1b}[0m msg")` while a `StageEvent::Line{Severity::Fail}` in the same call renders an uncolored stdout row, and the mirrored case; plus a `use_colors_for` test asserting NO_COLOR forces false for both streams.

### A14-6 [medium, conf 0.95] Doctor still requires a multithreaded Tokio runtime
`sabrage/crates/sabrage-cli/src/main.rs:123-141` — **REFUTED** (re-rated low)

Despite being described as synchronous, `doctor` is dispatched only after the `#[tokio::main]` wrapper constructs a multithreaded runtime. Under thread/resource exhaustion, runtime construction can fail before doctor prints any diagnostic, unnecessarily coupling the diagnostic command to async runtime availability.
Evidence: `sabrage/crates/sabrage-cli/src/main.rs:123-141` wraps the entire dispatcher in `#[tokio::main] async fn main()` and calls `cmd_doctor` from inside it. `sabrage/crates/sabrage-cli/Cargo.toml:14-15` says Tokio is only for async stage entry points and that doctor stays synchronous. Inference: the attribute expands around the whole function, so runtime construction precedes the match body.

*Recommendation:* Use a synchronous top-level `main` for help/version/doctor and construct a Tokio runtime only when dispatching an async stage command.

*Verifier:* Structurally true, functionally inert: the only failure mode requires resource exhaustion that already defeats every check doctor performs, and the cited Cargo.toml comment says doctor is synchronous (it is), not that no runtime is constructed. Restructuring `main` into a sync shell plus per-command `Runtime::new()` is churn with no observable benefit.

*Codex next steps:* Capture stdout, stderr, and exit bytes for shell/native argument matrices covering flags before commands, empty values, `--flag=value`, and relative paths. · Pass a fixture child's CR-delimited progress and unterminated final output through the CLI renderer; compare captured bytes with direct shell execution and repeat with `--quiet`. · Run a PTY/redirection matrix for stdout and stderr, with and without `NO_COLOR`, and assert that only terminal-bound rows contain ANSI escapes. · Send single and double SIGINT/SIGTERM during a fixture detached launch, between `all` stages, and during an authorization-prompt fixture; verify exit status, child liveness, staged files, and guard state. · Exercise `doctor` where worker-thread creation is denied, or unit-test a synchronous dispatcher that proves no Tokio runtime is constructed for doctor.
