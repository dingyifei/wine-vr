# Code standards: comments and tests

Adopted 2026-09-01. Applies to this repository's own code: `sabrage/` (Rust, Svelte, TypeScript),
`demo.sh` and `scripts/demo/` (zsh), and `docs/`. It does **not** apply inside `ext/*` (separate forks
with their own rules, e.g. `ext/oxrsys/AGENTS.md` mandates MPL-2.0 headers). Investigation-era `src/`,
`tools/` and `scripts/dev/` are held to §2 (no false comments) only. `CLAUDE.md` and `sabrage/PARITY.md`
take precedence where they are more specific; this document takes precedence over taste.

The standard exists because the codebase grew to 1.67 test lines and 0.67 comment lines per line of
code (`sabrage/docs/reviews/2026-08-31-simplification.md`), a third of it written by five review-fix
commits. Every rule cites its source by number; the list is in `docs/code-standards-sources.md`, and a
rule marked *house rule* is ours, not derived.

**Scope of enforcement.** A violation *in the lines you change* is fixed in that change. A violation you
merely notice elsewhere is left alone; §4 governs it. The one exception is a comment proven false (2.3),
which is fixed wherever it is found. Disputes over what a rule means are settled by the repository
owner, in the PR, in one line; the resolution is then written into this document in the same change. A
rule that is repeatedly wrong is amended or deleted here: this file is code, and rule 2.2 applies to it.

## 1. Comments

**1.1 A comment carries what the code cannot.** Intent, rationale, invariants, units, and non-obvious
consequences belong in comments, next to what they describe. Anything the reader can see from the code
beside the comment does not. "Comment repeats code" is a defect, not a courtesy [1, 2, 3, 5, 8].

**1.2 Doc comments are contracts.** A `///` or `//!` block states, in this order: a one-line summary;
what the item returns or does; `# Errors` / `# Panics` / `# Safety` where applicable. An example is
required only for items another crate calls (`sabrage-cli`, `src-tauri`, `sabrage-parity`) and only
where the call is not obvious from the signature; a required example on every `pub fn` is the
boilerplate 1.4 forbids. A doc comment never describes the current algorithm, because the algorithm
changes and the contract should not [4, 9]. A Svelte or TypeScript header is a prose block at the top
of `<script>` (the header of `sabrage/ui/src/screens/Library.svelte`): what the component owns, which store it
drives, and which invariant it shares with another screen.

**1.3 Implementation comments say *why*, or *what* for a block. Never *how*.** If a block needs a
comment to explain how it works, rewrite the block [8, 12]. Allowed (a block-*what* a reader cannot get
by skimming): `// Collapse duplicate slugs, keeping the first: doctor's output order is the contract.`
Forbidden (a restatement): `// loop over checks and insert into the map`. Test: if deleting the comment
costs the reader nothing but reading time, it is a restatement.

**1.4 Allowed and forbidden kinds** [7, 6].

| allowed | forbidden |
|---|---|
| intent and rationale ("why this and not the obvious thing") | restating the code |
| clarification of a non-obvious value or call | mandated boilerplate (a header on every fn) |
| **warning of consequences**, the landmine: "`arm64e` alone must NOT satisfy" | journal or changelog entries, bylines, dates |
| `TODO(<id>)` with an id in the 3.6 vocabulary | position markers (except the machine-read headers of 1.8) |
| a link to the spec, design note, `PARITY.md` row, or finding that explains the code | commented-out code |
| | **rejected-alternative narration** ("we tried X, it didn't work, then Y…"), a house rule; [2] permits a one-line "why not X", which is exactly the exception below |

Rejected-alternative narration is allowed in exactly one case: X is the obvious next edit *and* no test
can practically catch it (a timing, ordering, or performance property, or a test that would cost more
than the bug). Prefer the test; write the one-line warning only when you have decided against the test,
and say why in the same line. Everything catchable follows 1.5 instead. A TODO already in the tree with
no id is not adopted: give it an id in the change that touches it, delete it, or restate it as a
`Limitation:` comment describing what the code does not do.

**1.5 Landmines are short and point at their enforcement.** A warning-of-consequences comment is at
most three lines and names the test function, or cites the ledger as `PARITY.md § <exact heading>,
"<first words of the row>"`. The failure mode is a citation of a heading that does not exist (one was found in
`config/runtime_toml.rs` when this standard was adopted). If nothing would catch the mistake, write the test first; the comment
then points at it [1, 6].

**1.6 Do not transcribe the shell.** `scripts/demo/*.sh` is in this repository and is the source of
truth for what the pipeline does. A Rust module that mirrors a script references it (`Reference:
scripts/demo/stop.sh`, with step ids if useful) instead of re-listing its steps; the doc comment still
states the contract, what `stop()` guarantees on return. It is the *sequence* that lives in the script
and is cited, not copied [1].

**1.7 History lives in git and in `sabrage/docs/reviews/`.** A comment may cite a finding, issue, or
design note to explain why the code is the way it is; the *reason* belongs in the code, not only in
the commit log [1 ch. 16]. It may not narrate what the code used to do, who changed it, or when;
version control records that at finer grain and does not go stale [7, 10]. One exception: a regression
test's own doc comment states the defect it pins (the A14-3 test in `process.rs`, the A8-4 test in `logs.rs`), because that is
the test's specification. The exception is limited to the labelled assertion of 3.6; production code
says what is true now and cites the id for the rest.

**1.8 Some comments are code.** A comment that a program reads or prints is load-bearing and is
changed only together with its reader; §1 and 2.4 do not apply to it, 2.2 does. In this repository:
the `# preflight:` / `# preflight-warn:` / `# preflight-autofix:` / `# launch-action:` tags in
`scripts/demo/run.sh` (parsed by `sabrage-parity` `run_sh_tags`), the `# N. <section>` headers in
`scripts/demo/doctor.sh` (parsed by `slug_coverage::section_header`), generated-file headers
(`# GENERATED …`, `# contract-sha256:`), `contract/pipeline.toml`'s per-row comments, the header
block of `scripts/dev/parity.sh` (printed verbatim by `usage()`), and lines 2–22 of `demo.sh` (2–20
printed verbatim by the unknown-command branch's `sed -n '2,20p'`; 21–22 cited by the PARITY.md row on
`--wired`/`--verbose`). *House rule.*

## 2. False or stale comments

**2.1 A wrong comment is a defect, and a worse one than a missing comment** [11, 7, 13].

**2.2 Fix at the source. Rewrite or delete; never append.** When a comment is found false or stale,
rewrite it to describe the current code, or delete it if the code now speaks for itself. Do not add
`EDIT:`, `UPDATE:`, "actually…", "as of 2026-…", or a second paragraph that contradicts the first; the
reader must never reconcile two statements. The history of the correction is the commit message
[11, 12, 7, 10]. In the shell pipeline the follow-through is part of the fix: any edit to `demo.sh` or
`scripts/demo/*.sh`, comment-only included, invalidates `sabrage/parity/shell.fingerprint`, so run
`scripts/dev/parity.sh`, then `--bless`, and commit the new fingerprint in the same commit; never bless
past a test that is red for any other reason. In generated files the source is the generator: a false
comment in `scripts/demo/contract.gen.sh` is fixed in `contract/` and regenerated with
`scripts/dev/parity.sh --regen`; the generated file is never hand-edited.

**2.3 A comment proven false during review is fixed in the change that proved it**, even when the
change is otherwise unrelated. A false comment is a defect under 2.1, not a style cleanup, so the
file-a-bug-and-TODO route that [6] allows for pre-existing style issues does not apply; leaving it for
"a later cleanup" is how it becomes permanent [11, 7; house rule against [6]'s default]. If you can show
the comment is false but cannot determine what is true, delete it and say so in the commit message:
deleting a false comment is always safe. Leave a `TODO(<id>)` in its place only if the missing
explanation is itself load-bearing.

**2.4 Check the diffs.** Before committing, re-verify every comment inside a diff hunk, the doc comment
on any item whose signature or behaviour changed, and the block comment enclosing the hunk. Reviewers
do the same and additionally read the comments and TODOs in the changed files, not beyond them
[1 ch. 16, 6].

## 3. Tests

**3.1 One behaviour per test, through the public API of the unit under test, named for the
behaviour.** A behaviour is a guarantee the system makes about how it responds to inputs in a state. A
test name that mirrors a method name is a warning sign; a test that calls the system under test twice
and asserts after each call is two tests, unless the behaviour under test *is* what happens between the
calls (idempotence, caching, ordering, reconnect), in which case the sequence is the single behaviour
and the name says so (`second_install_makes_no_changes`) [15, 16, 18, 21]. A private helper is tested
in its own module (`checks/config.rs`'s `parse_protocol`), never made `pub` to satisfy this rule;
visibility is widened only when a test outside the crate must call it, and that widening is annotated
with the finding that required it (the `pub (A1-3)` pattern in `stages/run/actions.rs`).

**3.2 Assert results and state, not interactions.** Verify only the arguments the behaviour depends
on. Fakes only at true boundaries; in this repository that is the `Executor` trait (`DryRunExecutor`,
plus per-test doubles such as `stages::install::tests::TestExecutor`), a `Paths` rooted at a scratch
directory, and `StageCtx::for_fixture`; no mock-expectation DSLs [19, 20, 21]. Several near-identical
`impl Executor` doubles are a consolidation target, not a violation. A third fake means a third real
boundary: propose it with the boundary, do not add one to make an awkward test easier.

**3.3 No logic in tests.** Inputs and expected outputs are literals. When several tests differ only in
their literals, they become one table: a `&[(label, input, expected)]` slice iterated in the test, each
row labelled so a failure names the case (`sabrage-core` has no dev-dependencies today; a
parameterised-test crate is a dependency decision, not a style choice). When setup or assertions differ,
they stay separate functions; a table whose rows need branches is logic in a test [17, 21, 26].

**3.4 Definition: a test is redundant when every fact it pins is still pinned by a named surviving
test.** A **change-detector test** fails on refactors without detecting a defect [14]. Both are deleted
or rewritten. Line coverage never justifies a test; a unique fact or a unique mutant kill does [27].
Confirm redundancy two ways where you can and one way always: (a) *tooling*: `cargo mutants` on the
crate, run locally on macOS (it is not wired into CI, which is ubuntu and cannot run `-p sabrage-core`),
kills the same mutants before and after the deletion; (b) *reading*: the deleter names, in the commit
message, the surviving test that carries each fact of the deleted one, *about the same unit*. (b) is
mandatory; when (a) cannot be run, the deletion needs the owner's sign-off. Equal mutant kills never
justify a deletion on their own: `checks::config::parse_protocol` (doctor.sh's last-match `awk`) and
`stages::setup::parse_protocol_awk` (setup.sh's first-match `awk`) have tests that share eight literals
and differ in exactly one assertion (`"second"` vs `"first"`), so they look like duplicates and pin
opposite semantics of two different units.

**3.5 Layers: one owner per byte-fact, and it is `sabrage-parity` when CI can hold it.** CI runs
`-p sabrage-parity -p sabrage-contract-gen` on ubuntu (`.github/workflows/parity.yml`); local tier 1
adds `-p sabrage-core` (`TIER1_PKGS` in `scripts/dev/parity.sh`), which CI cannot. So a byte-fact whose golden is
hermetic lives in `sabrage-parity`: host XR JSON, the toml template, `steam_appid.txt`, the `win_path`
table, registry order, slug cardinality, launch banner/env/argv, the shell fingerprint. A byte-fact
that today lives only in `sabrage-core` (the `.sha256` marker in `stages/setup.rs`, the `cxbottle.conf`
`CX_GRAPHICS_BACKEND` line in `fixes/backend.rs`) stays there until an equivalent parity test exists; a
cut may never leave a byte unasserted. A core test may assert the same bytes as the observable result
of the behaviour it exercises, and `sabrage-parity` deliberately re-pins literals core already
unit-tests, for the reason the note at the top of its `run_sh_text_parity` module gives. What is forbidden is a *third*
copy that exercises nothing. A higher-layer failure with no lower-layer failure means a lower-layer
test is missing, not that the higher one should be duplicated [24, 21]. A regression label (3.6) for a
golden-byte finding lives on the parity row that pins those bytes; it does not create a second
assertion elsewhere.

**3.6 Regression rule.** Every bug fix and every accepted review finding ships with **exactly one**
assertion that failed before the fix and passes after it, labelled in the shape the tree already uses,
`/// <id> regression: <one line>` (the A3b-1 test in `checks/config.rs`), or as the table-row label string. Round 1
and round 2 share an id space (`sabrage/docs/reviews/2026-08-30-codex-round1.md` and `-round2.md` both
define `A1-1`), so **new** labels for any id that exists in more than one round carry the round:
`r2:A14-3`. Existing bare labels are not rewritten; a bare id resolves through `sabrage/docs/reviews/`.
Outside the review rounds the id is `issue:#<n>`, or `bug:<yyyy-mm-dd>-<slug>` for a bug found without
one; never a commit SHA, which does not exist when the label is written (a SHA is acceptable only when
back-filling a label onto a fix that already shipped). When a behaviour table exists, the regression is
a row in it, not a new function. The label survives any later merge or table-drive; the assertion may
move, never vanish [25, 21]. Where the accepted resolution is a declared divergence rather than code,
the "assertion" is a row in `sabrage/PARITY.md` plus its contract gate. The UI has no test runner
(`sabrage/ui/package.json`), so a frontend fix ships its assertion on the Rust side of the IPC boundary
where one exists, and otherwise ships `npm run check` plus a one-line note in the commit; adding a
runner is an owner decision.

**3.7 Never test the platform.** No tests of std, serde, derive output, tokio, or `toml_edit`
behaviour. Never assert a fact created by the test's own setup rather than by the code under test (the
scratch directory the fixture made exists; the plan is empty because nothing was called); asserting
that the *code under test* created a directory is the point of the test. A test that cannot fail is
deleted [14, 21]. A helper used by a single suite is not tested on its own; a helper shared across
suites is test infrastructure and, like production code, carries its own tests [21 ch. 12, 17]. In
particular the `sabrage-parity` shell scanners (`slug_coverage`, `run_sh_tags`) *decide* whether a gate
fired at all, so their fixture tests (the `slug_coverage` and `run_sh_tags` self-tests) are required, and a change to a
scanner ships with the fixture that would have caught the old behaviour.

**3.8 Tier-1 tests are hermetic and deterministic.** Tier 1 is everything `cargo test` runs; tier 2 is
the live differ in `scripts/dev/parity.sh`, which needs a machine and a bottle and never runs in CI.
Tier-1 tests use scratch directories under `temp_dir()` named uniquely per process (`std::process::id()`
plus a uuid, as the `scratch()` helper in `privilege.rs` does) and removed at entry as well as exit, so a panicked run cannot
poison the next; a `Drop` guard (`impl Drop for Fixture` in `stages/run/preflight.rs`) when the test mutates process-global
state. Never the real `HOME`, never real `adb` / `wine` / `osascript`, no waiting on the clock. Tests
that need a real subprocess spawn `/bin/sleep` or the test binary itself and kill what they spawned;
spawning `sleep` is not sleeping. These are *medium* tests in [21 ch. 11]'s taxonomy (they touch disk
and spawn processes), held to its hermeticity rule: "a test should contain all of the information
necessary to set up, execute, and tear down its environment." Any single test over ~2 s or a crate
suite over ~30 s is a defect: narrow it to the behaviour it pins, or mark it `#[ignore]` with a
one-line reason and a tier-2 entry point; never leave it slow and unmarked [21 ch. 11, 22].

**3.9 Smells that block review** [23, 22]: assertion roulette (many unlabelled asserts, no way to tell
which failed); entry-point mirrors (the same fixture and assertions re-run through a second entry point
that merely delegates to the first); and the violations of 3.1, 3.3 and 3.8.

## 4. Applying this to existing code

Existing comments and tests are brought under this standard by scans that produce a report first and
change code only after the owner's go. A comment cut supplies its replacement text (1.5, 1.6), never
"shorten it". A test cut names the surviving test that carries each fact and, for deletions, shows an
unchanged mutant-kill set when a run is available (3.4); finding labels are preserved by a mechanical
before/after check of the *set* of ids per file (3.6). Golden and parity tests are never cut, only
de-duplicated toward the layer CI actually runs, and never below one assertion per byte (3.5). Two
mechanical checks are still to be added: a tier-1 test that every `PARITY.md §` citation names a real
heading (1.5), and the finding-label set check (3.6).

## Sources

The numbered list is `docs/code-standards-sources.md`. The six that settle most arguments: Ousterhout,
*A Philosophy of Software Design* ch. 13 and 16 [1]; Google's code-review guide [6]; *Clean Code* ch. 4
[7]; *Software Engineering at Google* ch. 11–12 [21]; "Change-Detector Tests Considered Harmful" [14];
the cargo-mutants and PIT introductions [27].
