//! Parity tests between the native pipeline (`sabrage-core` / `sabrage-contract-gen`)
//! and the zsh reference implementation (`demo.sh`, `scripts/demo/*.sh`,
//! `contract/`).
//!
//! This crate carries **no runtime surface** — see `Cargo.toml`: every
//! dependency is a dev-dependency, and everything below is a tier-1 hermetic
//! `cargo test` per `docs/design/design-parity.md` §4 ("always-on pure tests,
//! no env gate, no machine state" beyond reading the repo tree the crate is
//! built from). Tier 2 (the live doctor diff) and the pre-push hook are
//! `scripts/dev/parity.sh`'s job, not this crate's.
//!
//! Every test reads its shell/contract inputs from the **working checkout on
//! disk** via [`tests::repo_root`], never from a compiled-in copy: the whole
//! point of this crate is to catch "the checkout and the generated/compiled
//! artifact disagree," which comparing two compiled-in copies of the same
//! `include_str!` would defeat by construction.

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    /// The repo root: this crate's manifest dir is `sabrage/crates/sabrage-parity`,
    /// so three `..` hops land on `sabrage-parity` → `crates` → `sabrage` → repo
    /// root — the same depth `sabrage-contract-gen::compiled_repo_root()` uses
    /// (it lives at the same `crates/<name>` depth).
    pub(crate) fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .canonicalize()
            .expect("repo root resolves")
    }

    // ── (1) contract-gen regen ──────────────────────────────────────────────

    /// `generate() == committed scripts/demo/contract.gen.sh`, both as a
    /// compile-time byte comparison (this test's own copy, independent of
    /// `sabrage-contract-gen`'s in-crate version of the same assertion) and as
    /// a live `--check` against the working checkout.
    mod contract_gen_parity {
        use super::repo_root;

        #[test]
        fn generate_matches_the_committed_contract_gen_sh() {
            let generated = sabrage_contract_gen::generate();
            // Compiled in independently of sabrage-contract-gen's own copy of
            // this comparison, at this crate's own include-path depth
            // (src/lib.rs -> sabrage-parity -> crates -> sabrage -> repo root).
            let committed = include_str!("../../../../scripts/demo/contract.gen.sh");
            assert_eq!(
                generated, committed,
                "sabrage-contract-gen::generate() no longer reproduces the committed \
                 scripts/demo/contract.gen.sh byte-for-byte — run: \
                 cargo run -p sabrage-contract-gen -- --write"
            );
        }

        /// Executable shell lines only: full-line comments (the header prose in
        /// demo.sh/lib.sh that *documents* the default install leaf) are not
        /// hard-coded values.
        fn executable_lines(text: &str) -> String {
            text.lines()
                .filter(|l| !l.trim_start().starts_with('#'))
                .collect::<Vec<_>>()
                .join("\n")
        }

        /// A contract scalar the shell hard-codes is a scalar the two
        /// front-ends can silently disagree about: editing it in
        /// `pipeline.toml` and regenerating moves only the generated file's
        /// `# contract-sha256:` header, so `--check`, `--regen` and doctor's
        /// `meta.contract-sync` all report "in sync" while native setup fetches
        /// one asset and setup.sh fetches another (or the two resolve BS_DIR to
        /// different directories).
        ///
        /// `sabrage-contract-gen` emits these three; the shell must source
        /// them. `sabrage-core` already reads them from the contract
        /// (`stages::setup`, `paths::resolve_bs_dir`).
        #[test]
        fn the_shell_sources_the_emitted_asset_and_install_leaf_scalars() {
            let root = repo_root();
            let generated = sabrage_contract_gen::generate();
            for var in ["GBE_DLL_ASSET", "DXMT_TGZ_ASSET", "BS_DIR_LEAF"] {
                assert!(
                    generated.contains(&format!("\n{var}=")),
                    "contract.gen.sh must emit {var} for the shell to source"
                );
            }

            let read = |rel: &str| {
                std::fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
            };
            let setup = executable_lines(&read("scripts/demo/setup.sh"));
            let lib = executable_lines(&read("scripts/demo/lib.sh"));
            let doctor = executable_lines(&read("scripts/demo/doctor.sh"));

            for (name, text, var) in [
                ("setup.sh", &setup, "GBE_DLL_ASSET"),
                ("setup.sh", &setup, "DXMT_TGZ_ASSET"),
                ("lib.sh", &lib, "BS_DIR_LEAF"),
                ("doctor.sh", &doctor, "BS_DIR_LEAF"),
            ] {
                assert!(
                    text.contains(var),
                    "{name} must use ${var} (sourced from contract.gen.sh) instead of a literal"
                );
            }

            for (name, text, literal) in [
                (
                    "setup.sh",
                    &setup,
                    sabrage_core::contract().deps.gbe_dll_asset.as_str(),
                ),
                (
                    "setup.sh",
                    &setup,
                    sabrage_core::contract().deps.dxmt_tgz_asset.as_str(),
                ),
                (
                    "lib.sh",
                    &lib,
                    sabrage_core::contract().game.bs_dir_leaf.as_str(),
                ),
                (
                    "doctor.sh",
                    &doctor,
                    sabrage_core::contract().game.bs_dir_leaf.as_str(),
                ),
            ] {
                assert!(
                    !text.contains(literal),
                    "{name} hard-codes the contract value {literal:?} — source the generated \
                     variable instead, so changing contract/pipeline.toml changes both sides"
                );
            }
        }

        #[test]
        fn check_reports_in_sync_against_the_live_checkout() {
            let report = sabrage_contract_gen::check(&repo_root())
                .expect("contract/ files under repo_root are readable");
            assert!(
                report.in_sync,
                "scripts/demo/contract.gen.sh is stale relative to contract/ — run: \
                 cargo run -p sabrage-contract-gen -- --write"
            );
        }
    }

    // ── (2) shell fingerprint tripwire ──────────────────────────────────────

    /// `sabrage/parity/shell.fingerprint` pins the sha256 of `demo.sh` plus
    /// every `scripts/demo/*.sh` (excluding the GENERATED `contract.gen.sh`,
    /// which is covered by `contract_gen_parity` instead). Any edit to a
    /// tracked shell file — content OR the tracked file set itself — turns
    /// this test red until `scripts/dev/parity.sh --bless` re-signs it, which
    /// (per design-parity.md §4 tier 1.2) only re-blesses after the rest of
    /// the suite passes. The coupling from "shell edited" to "cargo test red"
    /// is mechanical, not honor-system.
    mod shell_fingerprint {
        use super::repo_root;
        use std::path::{Path, PathBuf};

        const FINGERPRINT_REL: &str = "sabrage/parity/shell.fingerprint";
        const BLESS_HINT: &str = "verify the divergence is intentional (see \
             docs/design/design-parity.md §4), then run: scripts/dev/parity.sh --bless";

        /// `demo.sh` + every `scripts/demo/*.sh`, `contract.gen.sh` excluded
        /// (it is GENERATED and already covered by `contract_gen_parity`),
        /// sorted the same way the fingerprint file's rows are sorted.
        fn discover_tracked_shell_files(root: &Path) -> Vec<PathBuf> {
            let mut files = vec![PathBuf::from("demo.sh")];
            let dir = root.join("scripts/demo");
            let mut names: Vec<String> = std::fs::read_dir(&dir)
                .expect("scripts/demo directory exists")
                .filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| n.ends_with(".sh") && n != "contract.gen.sh")
                .collect();
            names.sort();
            files.extend(
                names
                    .into_iter()
                    .map(|n| PathBuf::from("scripts/demo").join(n)),
            );
            files.sort();
            files
        }

        /// Parse `"<sha256>  <repo-relative-path>"` rows (shasum's own output
        /// shape), tolerant of any run of whitespace between the two columns.
        fn parse_fingerprint(text: &str) -> Vec<(String, PathBuf)> {
            text.lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| {
                    let mut parts = l.splitn(2, char::is_whitespace);
                    let hash = parts.next().expect("hash column present").to_string();
                    let path = parts.next().expect("path column present").trim_start();
                    (hash, PathBuf::from(path))
                })
                .collect()
        }

        #[test]
        fn shell_fingerprint_is_current() {
            // Bless-mode contract with scripts/dev/parity.sh: when set, this
            // assertion skips itself (loudly) so --bless can re-sign after an
            // intentional shell edit — every OTHER tier-1 test still gates it.
            if std::env::var_os("PARITY_SKIP_FINGERPRINT").is_some_and(|v| !v.is_empty()) {
                eprintln!(
                    "shell_fingerprint_is_current: skipped (PARITY_SKIP_FINGERPRINT set — bless mode)"
                );
                return;
            }
            let root = repo_root();
            let fingerprint_text = std::fs::read_to_string(root.join(FINGERPRINT_REL))
                .unwrap_or_else(|e| panic!("{FINGERPRINT_REL} is committed: {e}"));
            let recorded = parse_fingerprint(&fingerprint_text);
            let recorded_paths: Vec<PathBuf> = recorded.iter().map(|(_, p)| p.clone()).collect();

            let mut sorted = recorded_paths.clone();
            sorted.sort();
            assert_eq!(
                recorded_paths, sorted,
                "{FINGERPRINT_REL} rows must be sorted by path — {BLESS_HINT}"
            );

            let tracked = discover_tracked_shell_files(&root);
            assert_eq!(
                recorded_paths, tracked,
                "{FINGERPRINT_REL}'s file set does not match demo.sh + scripts/demo/*.sh \
                 (contract.gen.sh excluded) — a shell file was added, removed, or renamed. {BLESS_HINT}"
            );

            let mismatches: Vec<String> = recorded
                .iter()
                .filter_map(|(want_hash, rel)| {
                    let got_hash = sabrage_core::util::sha256_file(&root.join(rel))
                        .unwrap_or_else(|e| panic!("reading {}: {e}", rel.display()));
                    (&got_hash != want_hash).then(|| rel.display().to_string())
                })
                .collect();
            assert!(
                mismatches.is_empty(),
                "shell file(s) changed since the fingerprint was last blessed: {}. {BLESS_HINT}",
                mismatches.join(", ")
            );
        }
    }

    // ── (3) slug coverage: doctor.sh chk/tap call sites <-> contract slugs ──

    /// Token-scans `doctor.sh` for every slug it emits via `chk <status>
    /// <slug> …` or `tap <slug> <status>` (literal calls), plus the four
    /// `for _x in slug:value …; do` loops whose body reuses the loop
    /// variable's `${_x%%:*}` prefix as the chk/tap slug argument (toolchain,
    /// submodules, build outputs, overlay files — sections 4/6/9/10), and
    /// checks the resulting slug set against the contract.
    ///
    /// # The counting rule
    ///
    /// A literal read of "no slug appears in two different `chk` call lines"
    /// does not hold in doctor.sh as written: legitimate checks routinely
    /// span 2-5 separate `chk`/`tap` source lines for one slug (e.g.
    /// `cfg.session-pins` has three distinct `chk` lines — one per branch of
    /// an if/elif/else — plus a `tap … skipped` in a sibling branch;
    /// `host.manifest` has five). What doctor.sh actually keeps disjoint is
    /// **which of its 20 numbered sections** (`# 0.` … `# 16b.`, doctor.sh's
    /// own organizing device) a slug's occurrences fall in: every slug's
    /// emissions live in exactly one section, and every section's slug set is
    /// disjoint from every other section's. That is the rule this test
    /// enforces — a slug found in two different sections is the realistic
    /// failure mode (a copy-pasted slug, or two checks accidentally sharing
    /// one), and it is what "each slug exactly once as a chk-or-tap-emitting
    /// row set" cashes out to once loops and if/elif/else branching are taken
    /// into account. Verified against the current file: the 20 sections'
    /// slug sets are pairwise disjoint and their union is exactly the 48
    /// contract slugs with `group != "run-only"`.
    mod slug_coverage {
        use super::repo_root;
        use regex::Regex;
        use std::collections::{BTreeMap, BTreeSet};

        /// Physical doctor.sh lines, with `\`-continued lines merged into one
        /// logical line (needed only for the overlay loop header, section 10,
        /// which spans four physical lines this way).
        fn logical_lines(text: &str) -> Vec<String> {
            let mut out = Vec::new();
            let mut buf = String::new();
            for line in text.lines() {
                let trimmed_end = line.trim_end();
                if let Some(stripped) = trimmed_end.strip_suffix('\\') {
                    buf.push_str(stripped);
                    buf.push(' ');
                } else {
                    buf.push_str(trimmed_end);
                    out.push(std::mem::take(&mut buf));
                }
            }
            if !buf.is_empty() {
                out.push(buf);
            }
            out
        }

        /// The executable part of a shell line: everything before the first
        /// **comment** `#` — one that starts a word (line start, or after
        /// whitespace) and is not inside a single- or double-quoted string.
        ///
        /// Without this the scanner credits a commented-out `chk` line as a
        /// live emission, which is precisely how a deleted check keeps its
        /// coverage. `${_f#$ROOT/}`, `$#` and a `#` inside a quoted message are
        /// not comments and must survive.
        fn strip_comment(line: &str) -> &str {
            let mut in_single = false;
            let mut in_double = false;
            let mut prev_ws = true;
            for (i, c) in line.char_indices() {
                match c {
                    '\'' if !in_double => in_single = !in_single,
                    '"' if !in_single => in_double = !in_double,
                    '#' if !in_single && !in_double && prev_ws => return &line[..i],
                    _ => {}
                }
                prev_ws = c == ' ' || c == '\t';
            }
            line
        }

        /// `# 0.` / `# 9b.` / `# 16b.` … doctor.sh's own section numbering.
        /// Read from the RAW line: section headers are comments, and
        /// [`strip_comment`] would eat them.
        fn section_header(line: &str) -> Option<String> {
            let re = Regex::new(r"^#\s+(\d+[a-z]?)\.").unwrap();
            re.captures(line.trim_start()).map(|c| c[1].to_string())
        }

        fn is_slug_shaped(s: &str) -> bool {
            let re = Regex::new(r"^[a-z][a-z0-9]*(?:[._-][a-z0-9]+)*$").unwrap();
            re.is_match(s)
        }

        /// What one scan of doctor.sh found.
        pub(super) struct Scan {
            /// slug -> the set of doctor.sh sections it was emitted from.
            pub by_slug: BTreeMap<String, BTreeSet<String>>,
            /// Slugs in **first-emission order** — the contract calls its own
            /// order load-bearing ("Order = doctor.sh order"), so a set
            /// comparison alone cannot prove the two agree.
            pub order: Vec<String>,
            /// `slug:value` loop headers whose body emits nothing: the header
            /// still names slugs, but no `chk`/`tap` call consumes the loop
            /// variable, so those slugs are NOT covered.
            pub header_only_loops: Vec<String>,
        }

        fn record(scan: &mut Scan, slug: &str, section: &str) {
            let sections = scan.by_slug.entry(slug.to_string()).or_default();
            if sections.is_empty() {
                scan.order.push(slug.to_string());
            }
            sections.insert(section.to_string());
        }

        /// Does the loop body starting at `rest` actually emit for `var`?
        ///
        /// Both halves are required: the `${_x%%:*}` slug extraction (directly
        /// in the `chk` argument, or via the intermediate `_slug=` assignment
        /// section 10 uses) **and** at least one `chk`/`tap` call. Deleting the
        /// body's `chk` lines while keeping the `slug:value` header — the
        /// realistic way a check evaporates — then reads as uncovered.
        fn loop_body_emits(rest: &[String], var: &str, call_re: &Regex) -> bool {
            let prefix = format!("${{{var}%%:*}}");
            let mut saw_extraction = false;
            let mut saw_call = false;
            let mut depth = 0usize;
            for line in rest {
                let logical = strip_comment(line);
                let trimmed = logical.trim();
                if trimmed == "done" {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                    continue;
                }
                if trimmed.ends_with("; do") || trimmed == "do" {
                    depth += 1;
                }
                if logical.contains(&prefix) {
                    saw_extraction = true;
                }
                if call_re.is_match(logical) {
                    saw_call = true;
                }
            }
            saw_extraction && saw_call
        }

        pub(super) fn scan_doctor_slugs(text: &str) -> Scan {
            let chk_re =
                Regex::new(r"\bchk\s+(?:ok|warn|fail|info)\s+([a-z][a-z0-9._-]*)\b").unwrap();
            let tap_re =
                Regex::new(r"\btap\s+([a-z][a-z0-9._-]*)\s+(?:ok|warn|fail|info|skipped)\b")
                    .unwrap();
            // `for _x in <list>; do` — the loop headers in sections 4/6/9/10.
            let loop_re = Regex::new(r"\bfor\s+(_\w+)\s+in\s+(.+?)\s*;\s*do\b").unwrap();
            // Any chk/tap invocation, slug-shaped argument or not: inside a
            // loop the argument is `${_t%%:*}` / `"$_slug"`, which the two
            // slug-extracting regexes above deliberately do not match.
            let call_re = Regex::new(r"\b(?:chk|tap)\s").unwrap();

            let lines = logical_lines(text);
            let mut scan = Scan {
                by_slug: BTreeMap::new(),
                order: Vec::new(),
                header_only_loops: Vec::new(),
            };
            let mut current_section: Option<String> = None;

            for (i, raw) in lines.iter().enumerate() {
                if let Some(sec) = section_header(raw) {
                    current_section = Some(sec);
                }
                let logical = strip_comment(raw);
                let Some(section) = current_section.clone() else {
                    continue; // nothing before "# 0." emits a slug
                };

                for cap in chk_re.captures_iter(logical) {
                    record(&mut scan, &cap[1], &section);
                }
                for cap in tap_re.captures_iter(logical) {
                    record(&mut scan, &cap[1], &section);
                }
                for cap in loop_re.captures_iter(logical) {
                    let var = cap[1].to_string();
                    // Each whitespace-delimited word is `slug:value`; take
                    // only the part before the FIRST colon (values like
                    // overlay's `"$DXMT_ART/…/d3d11.dll:$CX/…/d3d11.dll"`
                    // contain further colons that must not be mistaken for a
                    // second slug).
                    let slugs: Vec<String> = cap[2]
                        .split_whitespace()
                        .filter_map(|w| w.split_once(':').map(|(slug, _)| slug))
                        .filter(|slug| is_slug_shaped(slug))
                        .map(str::to_string)
                        .collect();
                    if slugs.is_empty() {
                        continue;
                    }
                    if loop_body_emits(&lines[i + 1..], &var, &call_re) {
                        for slug in slugs {
                            record(&mut scan, &slug, &section);
                        }
                    } else {
                        scan.header_only_loops
                            .push(format!("for {var} in {}", slugs.join(" ")));
                    }
                }
            }
            scan
        }

        /// The contract's doctor-row slugs, in contract order.
        fn contract_row_slugs() -> Vec<String> {
            sabrage_core::contract()
                .checks
                .iter()
                .filter(|c| c.group != "run-only")
                .map(|c| c.slug.clone())
                .collect()
        }

        #[test]
        fn doctor_slug_coverage_matches_the_contract() {
            let root = repo_root();
            let text = std::fs::read_to_string(root.join("scripts/demo/doctor.sh"))
                .expect("scripts/demo/doctor.sh reads");

            let scan = scan_doctor_slugs(&text);

            assert!(
                scan.header_only_loops.is_empty(),
                "doctor.sh loop header(s) name slugs but the loop body emits no \
                 chk/tap for them: {:?}",
                scan.header_only_loops
            );

            let cross_section: Vec<String> = scan
                .by_slug
                .iter()
                .filter(|(_, sections)| sections.len() > 1)
                .map(|(slug, sections)| {
                    format!(
                        "{slug} (sections {})",
                        sections.iter().cloned().collect::<Vec<_>>().join(", ")
                    )
                })
                .collect();
            assert!(
                cross_section.is_empty(),
                "slug(s) emitted from more than one doctor.sh section — likely a \
                 copy-pasted slug: {}",
                cross_section.join("; ")
            );

            let found_slugs: BTreeSet<String> = scan.by_slug.keys().cloned().collect();
            let contract_order = contract_row_slugs();
            let contract_slugs: BTreeSet<String> = contract_order.iter().cloned().collect();

            let missing: Vec<&String> = contract_slugs.difference(&found_slugs).collect();
            assert!(
                missing.is_empty(),
                "contract slug(s) with no chk/tap emission found in doctor.sh: {missing:?} \
                 (add a `chk`/`tap` call in doctor.sh, or move the check to the run-only \
                 group if it truly has no doctor row)"
            );

            let unknown: Vec<&String> = found_slugs.difference(&contract_slugs).collect();
            assert!(
                unknown.is_empty(),
                "doctor.sh emits slug(s) the contract does not declare: {unknown:?} \
                 (add a [[check]] to contract/pipeline.toml, or fix the typo)"
            );

            // The contract calls its own order load-bearing (pipeline.toml:
            // "Order = doctor.sh order"), and tier 2 compares two tap channels
            // as unordered maps — so this is the only gate on it.
            assert_eq!(
                scan.order, contract_order,
                "doctor.sh's first-emission order must equal the contract's check order"
            );
        }

        // ── the scanner's own behaviour, on inline fixtures ──────────────────

        fn slugs_of(fixture: &str) -> Vec<String> {
            scan_doctor_slugs(fixture).order
        }

        #[test]
        fn a_commented_out_chk_line_contributes_no_slug() {
            let live = "# 5. rust\nchk ok rust.x64-target \"ok\"\n";
            let commented = "# 5. rust\n#chk ok rust.x64-target \"ok\"\n";
            let indented = "# 5. rust\n  # chk ok rust.x64-target \"ok\"\n";
            assert_eq!(slugs_of(live), vec!["rust.x64-target".to_string()]);
            assert!(slugs_of(commented).is_empty());
            assert!(slugs_of(indented).is_empty());
        }

        #[test]
        fn a_trailing_comment_does_not_hide_the_call_before_it() {
            let fixture = "# 5. rust\nchk ok rust.x64-target \"ok\"  # trailing note\n";
            assert_eq!(slugs_of(fixture), vec!["rust.x64-target".to_string()]);
        }

        #[test]
        fn a_hash_inside_a_parameter_expansion_or_a_string_is_not_a_comment() {
            let fixture =
                "# 9. build\nchk ok build.oxr-dylib \"built: ${_f#$ROOT/} # not a comment\"\n";
            assert_eq!(slugs_of(fixture), vec!["build.oxr-dylib".to_string()]);
        }

        #[test]
        fn a_loop_header_whose_body_emits_nothing_covers_no_slug() {
            let covered = "# 4. toolchain\nfor _t in tool.cmake:cmake tool.ninja:ninja; do\n  \
                           chk ok ${_t%%:*} \"${_t#*:}\"\ndone\n";
            let gutted = "# 4. toolchain\nfor _t in tool.cmake:cmake tool.ninja:ninja; do\n  \
                          command -v ${_t#*:} >/dev/null\ndone\n";
            assert_eq!(
                slugs_of(covered),
                vec!["tool.cmake".to_string(), "tool.ninja".to_string()]
            );
            let scan = scan_doctor_slugs(gutted);
            assert!(scan.order.is_empty(), "{:?}", scan.order);
            assert_eq!(scan.header_only_loops.len(), 1);
        }

        #[test]
        fn a_loop_that_extracts_the_slug_into_a_variable_still_counts() {
            // Section 10's shape: `_slug="${_o%%:*}"` then `chk ok "$_slug" …`.
            let fixture = "# 10. overlay\nfor _o in overlay.woxr-dll:a:b; do\n  \
                           _slug=\"${_o%%:*}\"\n  chk ok \"$_slug\" \"current\"\ndone\n";
            assert_eq!(slugs_of(fixture), vec!["overlay.woxr-dll".to_string()]);
        }

        #[test]
        fn the_scan_records_first_emission_order_not_a_set() {
            let fixture = "# 0. meta\nchk ok meta.contract-sync \"a\"\n# 1. system\n\
                           chk ok sys.arch \"b\"\ntap sys.arch ok\nchk ok sys.macos27 \"c\"\n";
            assert_eq!(
                slugs_of(fixture),
                vec![
                    "meta.contract-sync".to_string(),
                    "sys.arch".to_string(),
                    "sys.macos27".to_string(),
                ],
                "a re-emitted slug keeps its first position, and order is preserved"
            );
            let swapped = "# 0. meta\nchk ok sys.arch \"b\"\n# 1. system\n\
                           chk ok meta.contract-sync \"a\"\n";
            assert_ne!(slugs_of(swapped), slugs_of(fixture));
        }
    }

    // ── (4) run.sh preflight/launch-action tags <-> contract gates ──────────

    mod run_sh_tags {
        use super::repo_root;
        use regex::Regex;
        use sabrage_core::Gate;
        use std::collections::BTreeMap;

        /// `require_bottle()` (lib.sh) enforces `bottle.named` and
        /// `bottle.exists` by calling `die()` unconditionally at the top of
        /// run.sh (line 6, one line before the `# preflight:`-tagged block
        /// begins) — before the tagged preflight section, so neither carries
        /// a `# preflight:` tag of its own. The contract still declares
        /// `shell_gate = "block"` for both (pipeline.toml's own comments say
        /// so: "require_bottle dies before the run preflight proper"). Both
        /// lib.sh and run.sh are out of scope for this crate to edit, so this
        /// is the one place the tag scan is deliberately told about a real,
        /// but untagged, blocking check rather than reporting it as a false
        /// mismatch.
        const REQUIRE_BOTTLE_BLOCKS: [&str; 2] = ["bottle.named", "bottle.exists"];

        /// The three preflight tag kinds and the `shell_gate` each one claims.
        /// `none` has no tag by construction: an untagged check is not in the
        /// launch preflight at all.
        const TAG_GATES: [(&str, Gate); 3] = [
            ("preflight:", Gate::Block),
            ("preflight-warn:", Gate::Warn),
            ("preflight-autofix:", Gate::Autofix),
        ];

        /// All slugs on `# <tag> <slug> [<slug> …]` lines, in file order
        /// (multiple slugs on one line, as `cfg.protocol.supported
        /// cfg.protocol.legacy-oxrsys` did before the gates split them, expand
        /// to multiple entries).
        fn extract_tagged(text: &str, tag: &str) -> Vec<String> {
            let re = Regex::new(&format!(r"(?m)^#\s*{}\s+(.+)$", regex::escape(tag))).unwrap();
            re.captures_iter(text)
                .flat_map(|c| {
                    c[1].split_whitespace()
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .collect()
        }

        /// slug -> the gate its tag claims, for every tagged (or documented
        /// untagged) preflight in run.sh.
        ///
        /// Merging the three tag kinds into one "is gating at all" bag — which
        /// this test used to do — cannot tell `warn` from `block` in either
        /// direction: turning `game.version`'s `warn` into a `die` left the tag
        /// and the whole comparison unchanged. The map is the fix.
        fn tag_gate_map(text: &str) -> (BTreeMap<String, Gate>, Vec<String>) {
            let mut map: BTreeMap<String, Gate> = BTreeMap::new();
            let mut conflicts: Vec<String> = Vec::new();
            for (tag, gate) in TAG_GATES {
                for slug in extract_tagged(text, tag) {
                    if let Some(prev) = map.insert(slug.clone(), gate) {
                        if prev != gate {
                            conflicts.push(format!(
                                "{slug} is tagged both {} and {}",
                                prev.as_str(),
                                gate.as_str()
                            ));
                        }
                    }
                }
            }
            for slug in REQUIRE_BOTTLE_BLOCKS {
                if let Some(prev) = map.insert(slug.to_string(), Gate::Block) {
                    conflicts.push(format!(
                        "{slug} carries a {} tag as well as the documented require_bottle block",
                        prev.as_str()
                    ));
                }
            }
            (map, conflicts)
        }

        /// The contract's shell-side launch preflight: slug -> gate, `none`
        /// excluded (an untagged, non-gating check).
        fn contract_shell_gates() -> BTreeMap<String, Gate> {
            sabrage_core::contract()
                .checks
                .iter()
                .filter(|c| c.shell_gate != Gate::None)
                .map(|c| (c.slug.clone(), c.shell_gate))
                .collect()
        }

        #[test]
        fn preflight_tags_match_the_contracts_shell_gates_gate_for_gate() {
            let root = repo_root();
            let text = std::fs::read_to_string(root.join("scripts/demo/run.sh"))
                .expect("scripts/demo/run.sh reads");

            let (tagged, conflicts) = tag_gate_map(&text);
            assert!(
                conflicts.is_empty(),
                "contradictory run.sh tags: {conflicts:?}"
            );

            let want = contract_shell_gates();
            assert_eq!(
                tagged, want,
                "run.sh's `# preflight:` (block) / `# preflight-warn:` (warn) / \
                 `# preflight-autofix:` (autofix) tags, plus the documented require_bottle \
                 exception ({REQUIRE_BOTTLE_BLOCKS:?}), must equal the contract's shell_gate \
                 map slug-for-slug AND gate-for-gate"
            );
            // Cardinality, stated separately so a same-size swap still reads
            // as one obvious failure above rather than as a length mismatch.
            assert_eq!(tagged.len(), want.len());
        }

        /// A tag is a comment: nothing binds `# preflight-warn: game.version`
        /// to the `warn` two lines below it. This ties each tag *group* to the
        /// verb its block actually uses, which is what makes the map above
        /// evidence about behaviour rather than about comments.
        ///
        /// A "block" is the run of lines from a tag group (consecutive
        /// `# preflight*:` lines) to the next tag group, or to end of file.
        /// `autofix` groups are exempt: their blocks legitimately contain both
        /// verbs (the fix's own failure path dies).
        #[test]
        fn each_preflight_tag_group_uses_the_verb_its_gate_claims() {
            let root = repo_root();
            let text = std::fs::read_to_string(root.join("scripts/demo/run.sh"))
                .expect("scripts/demo/run.sh reads");
            let die_re = Regex::new(r"\bdie\b").unwrap();
            let warn_re = Regex::new(r"\bwarn\b").unwrap();
            for group in tag_groups(&text) {
                let has_die = die_re.is_match(&group.body);
                let has_warn = warn_re.is_match(&group.body);
                if group.gates.contains(&Gate::Autofix) {
                    continue;
                }
                if group.gates.contains(&Gate::Block) {
                    assert!(
                        has_die,
                        "block-tagged {:?} but its run.sh block never calls die:\n{}",
                        group.slugs, group.body
                    );
                }
                if group.gates.contains(&Gate::Warn) {
                    assert!(
                        has_warn,
                        "warn-tagged {:?} but its run.sh block never calls warn:\n{}",
                        group.slugs, group.body
                    );
                    if !group.gates.contains(&Gate::Block) {
                        assert!(
                            !has_die,
                            "warn-only {:?} but its run.sh block calls die — the contract says \
                             the launch continues:\n{}",
                            group.slugs, group.body
                        );
                    }
                }
            }
        }

        struct TagGroup {
            slugs: Vec<String>,
            gates: Vec<Gate>,
            body: String,
        }

        /// Consecutive `# preflight*:` tag lines and the run.sh lines they
        /// govern (up to the next tag group, or EOF).
        fn tag_groups(text: &str) -> Vec<TagGroup> {
            let tag_re =
                Regex::new(r"^#\s*(preflight:|preflight-warn:|preflight-autofix:)\s+(.+)$")
                    .unwrap();
            let lines: Vec<&str> = text.lines().collect();
            let mut groups: Vec<TagGroup> = Vec::new();
            let mut i = 0usize;
            while i < lines.len() {
                let Some(cap) = tag_re.captures(lines[i].trim_start()) else {
                    i += 1;
                    continue;
                };
                let mut slugs: Vec<String> = Vec::new();
                let mut gates: Vec<Gate> = Vec::new();
                let mut cap = Some(cap);
                while let Some(c) = cap {
                    let gate = TAG_GATES
                        .iter()
                        .find(|(t, _)| *t == &c[1])
                        .map(|(_, g)| *g)
                        .expect("the regex only matches the three known tags");
                    gates.push(gate);
                    slugs.extend(c[2].split_whitespace().map(str::to_string));
                    i += 1;
                    cap = lines.get(i).and_then(|l| tag_re.captures(l.trim_start()));
                }
                let start = i;
                while i < lines.len() && tag_re.captures(lines[i].trim_start()).is_none() {
                    i += 1;
                }
                groups.push(TagGroup {
                    slugs,
                    gates,
                    body: lines[start..i].join("\n"),
                });
            }
            groups
        }

        #[test]
        fn launch_action_tags_match_the_contracts_order() {
            let root = repo_root();
            let text = std::fs::read_to_string(root.join("scripts/demo/run.sh"))
                .expect("scripts/demo/run.sh reads");

            let tagged = extract_tagged(&text, "launch-action:");
            let contract_ids: Vec<String> = sabrage_core::contract()
                .launch_actions
                .iter()
                .map(|a| a.id.clone())
                .collect();

            assert_eq!(
                tagged, contract_ids,
                "run.sh's `# launch-action:` tags, in file order, must exactly equal the \
                 contract's [[launch_action]] id order"
            );
        }

        // ── the tag scan's own behaviour, on inline fixtures ─────────────────

        #[test]
        fn the_scan_separates_block_warn_and_autofix() {
            let fixture = "# preflight: a.block\n[ -f x ] || die \"gone\"\n\
                           # preflight-warn: b.warn\ncase $v in x) : ;; *) warn \"odd\" ;; esac\n\
                           # preflight-autofix: c.fix\nfix_it || die \"could not fix\"\n";
            let (map, conflicts) = tag_gate_map(fixture);
            assert!(conflicts.is_empty());
            assert_eq!(map.get("a.block"), Some(&Gate::Block));
            assert_eq!(map.get("b.warn"), Some(&Gate::Warn));
            assert_eq!(map.get("c.fix"), Some(&Gate::Autofix));
        }

        #[test]
        fn a_warn_tagged_block_that_dies_is_rejected() {
            let honest = "# preflight-warn: b.warn\ncase $v in x) : ;; *) warn \"odd\" ;; esac\n";
            let lying = "# preflight-warn: b.warn\ncase $v in x) : ;; *) die \"odd\" ;; esac\n";
            let die_re = Regex::new(r"\bdie\b").unwrap();
            let warn_re = Regex::new(r"\bwarn\b").unwrap();

            let g = &tag_groups(honest)[0];
            assert!(warn_re.is_match(&g.body) && !die_re.is_match(&g.body));

            let g = &tag_groups(lying)[0];
            assert!(
                die_re.is_match(&g.body),
                "the warn->die mutation must be visible in the group's body"
            );
        }

        #[test]
        fn a_block_tagged_group_that_only_warns_is_rejected() {
            let lying = "# preflight: a.block\ncase $v in x) : ;; *) warn \"odd\" ;; esac\n";
            let g = &tag_groups(lying)[0];
            assert!(!Regex::new(r"\bdie\b").unwrap().is_match(&g.body));
        }

        #[test]
        fn consecutive_tags_share_one_block() {
            let fixture = "# preflight: a.block\n# preflight-warn: b.warn\n\
                           case $v in a) : ;; b) warn \"legacy\" ;; *) die \"invalid\" ;; esac\n";
            let groups = tag_groups(fixture);
            assert_eq!(groups.len(), 1);
            assert_eq!(groups[0].slugs, vec!["a.block", "b.warn"]);
            assert_eq!(groups[0].gates, vec![Gate::Block, Gate::Warn]);
        }
    }

    // ── (5) artifact goldens ─────────────────────────────────────────────────

    /// Byte-exact rendering checks, each built from the template/contract
    /// file read fresh off disk rather than sabrage-core's compiled-in copy —
    /// sabrage-core's own unit tests already cover the compiled-in-vs-itself
    /// case; this crate's job is to also catch a stale `include_str!` (edited
    /// the on-disk template, forgot to rebuild).
    mod artifact_goldens {
        use super::repo_root;
        use std::path::Path;

        #[test]
        fn render_host_manifest_matches_the_on_disk_template() {
            let root = repo_root();
            let template =
                std::fs::read_to_string(root.join("contract/active_runtime.x86_64.json.template"))
                    .expect("contract/active_runtime.x86_64.json.template reads");
            let dylib = Path::new("/repo/ext/oxrsys/build-x64/runtime/liboxrsys-runtime.dylib");
            let expected = template
                .trim_end_matches('\n')
                .replace("@OXR_DYLIB@", &dylib.to_string_lossy());

            assert_eq!(sabrage_core::util::render_host_manifest(dylib), expected);
            assert_eq!(
                sabrage_core::util::host_manifest_file_bytes(dylib),
                format!("{expected}\n")
            );
        }

        /// The dylib path lands inside a JSON string literal, so both
        /// front-ends escape it (`util::json_escape_string` here,
        /// `${OXR_DYLIB//\\/\\\\}` + `${.../\"/\\\"}` in install.sh) before the
        /// `@OXR_DYLIB@` substitution. The golden above proves an ordinary
        /// path is unchanged by that; this one proves a path containing `"` or
        /// `\` still produces JSON whose decoded `library_path` is the path we
        /// asked for, rather than the invalid/misdirected manifest a raw
        /// replace writes into a root-owned file.
        #[test]
        fn render_host_manifest_json_escapes_the_dylib_path() {
            for raw in [
                "/Users/me/my \"vr\" repo/ext/oxrsys/build-x64/runtime/liboxrsys-runtime.dylib",
                "/Users/me/a\\b/ext/oxrsys/build-x64/runtime/liboxrsys-runtime.dylib",
                "/Users/me/\"\\\"/liboxrsys-runtime.dylib",
            ] {
                let path = Path::new(raw);
                for rendered in [
                    sabrage_core::util::render_host_manifest(path),
                    sabrage_core::util::host_manifest_file_bytes(path),
                    sabrage_core::privilege::host_manifest_bytes(path),
                ] {
                    let parsed: serde_json::Value = serde_json::from_str(&rendered)
                        .unwrap_or_else(|e| panic!("not valid JSON: {rendered}: {e}"));
                    assert_eq!(
                        parsed["runtime"]["library_path"].as_str(),
                        Some(raw),
                        "the decoded library_path must be the dylib path itself"
                    );
                }
            }
        }

        /// The bytes install layer 4 actually **writes** — not merely the ones
        /// `util` can render.
        ///
        /// install.sh writes the manifest with `print -- "$WANT"`, so the live
        /// `/usr/local/share/openxr/1/active_runtime.x86_64.json` ends
        /// `7d 0a 7d 0a` (`}\n}\n`). The comparison form
        /// (`render_host_manifest`, what `$(<file)` yields) is that minus the
        /// final newline, and the two are one byte apart: writing the wrong one
        /// ships a host manifest that differs from `./demo.sh install`'s on the
        /// single most drift-sensitive artifact in the pipeline. The Phase-2
        /// review found exactly that — the util helpers were right and the
        /// install call site passed the other one.
        ///
        /// So this golden pins the **write path's own** byte source, read off
        /// the on-disk template, and the shape that makes the mistake
        /// unexpressible: `write_host_manifest_privileged` takes the dylib
        /// path, never pre-rendered content (see the compile-time tripwire
        /// below). Driving layer 4 end to end needs an async runtime this crate
        /// deliberately does not depend on; that half lives in sabrage-core's
        /// `layer_four_stages_the_host_manifest_file_form_byte_for_byte`, which
        /// runs `install::run` under a recording dry-run executor.
        #[test]
        fn the_privileged_write_stages_the_file_form_of_the_host_manifest() {
            let root = repo_root();
            let template =
                std::fs::read_to_string(root.join("contract/active_runtime.x86_64.json.template"))
                    .expect("contract/active_runtime.x86_64.json.template reads");
            let dylib = Path::new("/repo/ext/oxrsys/build-x64/runtime/liboxrsys-runtime.dylib");
            let comparison_form = template
                .trim_end_matches('\n')
                .replace("@OXR_DYLIB@", &dylib.to_string_lossy());
            let file_form = format!("{comparison_form}\n");

            assert_eq!(
                sabrage_core::privilege::host_manifest_bytes(dylib),
                file_form,
                "install layer 4 must stage `print -- \"$WANT\"`'s bytes"
            );
            assert!(file_form.ends_with("}\n"), "{file_form:?}");
            assert_eq!(
                file_form.len(),
                comparison_form.len() + 1,
                "exactly one newline apart from the currency-test form"
            );
            assert_ne!(
                sabrage_core::privilege::host_manifest_bytes(dylib),
                comparison_form,
                "the comparison form must never be what lands on disk"
            );
        }

        /// Compile-time half of the golden above: the privileged write takes a
        /// **path**, so no caller can hand it the newline-less comparison form.
        /// If the signature ever goes back to accepting rendered content, this
        /// stops compiling and the parity suite goes red before anything runs.
        #[allow(dead_code)]
        async fn write_path_takes_a_dylib_path_not_content(
            ctx: &sabrage_core::StageCtx,
            oxr_dylib: &Path,
            dest: &Path,
        ) -> sabrage_core::Result<sabrage_core::PrivilegedWrite> {
            sabrage_core::privilege::write_host_manifest_privileged(ctx, oxr_dylib, dest).await
        }

        #[test]
        fn toml_template_matches_the_on_disk_contract_file() {
            let root = repo_root();
            let on_disk =
                std::fs::read_to_string(root.join("contract/oxrsys-runtime.toml.template"))
                    .expect("contract/oxrsys-runtime.toml.template reads");
            assert_eq!(sabrage_core::util::toml_template(), on_disk);
        }

        #[test]
        fn win_path_table() {
            use sabrage_core::util::win_path;
            let prefix = Path::new("/Users/me/Library/Application Support/CrossOver/Bottles/Steam");

            // Inside drive_c -> C:\, separators flipped.
            assert_eq!(
                win_path(
                    Some(prefix),
                    &prefix.join("drive_c/windows/system32/wineopenxr.dll")
                ),
                "C:\\windows\\system32\\wineopenxr.dll"
            );
            assert_eq!(
                win_path(Some(prefix), &prefix.join("drive_c/openxr")),
                "C:\\openxr"
            );

            // The literal `drive_c` directory (no trailing slash / nothing
            // after it) does NOT match the trailing-slash glob and falls
            // through to Z: — this is string matching, not path-component
            // matching (design-core §10 parity decision 22).
            let dc = prefix.join("drive_c");
            assert_eq!(
                win_path(Some(prefix), &dc),
                format!("Z:{}", dc.display().to_string().replace('/', "\\"))
            );

            // Outside the bottle -> Z: + the whole path.
            assert_eq!(
                win_path(Some(prefix), Path::new("/games/Beat Saber 1294")),
                "Z:\\games\\Beat Saber 1294"
            );
            // No prefix at all -> Z:.
            assert_eq!(win_path(None, Path::new("/games/bs")), "Z:\\games\\bs");
        }

        #[test]
        fn steam_appid_txt_content_has_no_trailing_newline() {
            // run.sh: `printf '%s' "$BS_APPID" > "$APIDIR/steam_appid.txt"` —
            // `stages::run::actions::goldberg_stage` writes this file for
            // real now, from the exact expression this test pins:
            // `contract().game.appid.to_string()`, rendered with no trailing
            // newline (`printf '%s'`, never `print`/`println`). This golden
            // does not re-exercise the write itself — sabrage-core's own
            // `goldberg_stage` tests already do that against a fixture
            // executor — it only pins the *value and shape* the contract
            // must keep producing, so a contract change (a different appid
            // type, or the game section growing a formatted-string field)
            // that would silently change the on-disk bytes fails here first.
            let appid = sabrage_core::contract().game.appid.to_string();
            assert_eq!(appid, "620980");
            assert!(!appid.ends_with('\n'));
        }
    }

    // ── (6) contract sanity ──────────────────────────────────────────────────

    mod contract_sanity {
        #[test]
        fn slugs_are_unique() {
            let c = sabrage_core::contract();
            let mut seen = std::collections::BTreeSet::new();
            for slug in c.check_slugs() {
                assert!(seen.insert(slug), "duplicate contract slug: {slug}");
            }
        }

        #[test]
        fn every_autofix_gate_has_a_fix_action() {
            let c = sabrage_core::contract();
            for check in &c.checks {
                if check.shell_gate == sabrage_core::Gate::Autofix
                    || check.native_gate == sabrage_core::Gate::Autofix
                {
                    assert!(
                        check.fix.is_some(),
                        "{}: autofix gate declared with no [fix] action",
                        check.slug
                    );
                }
            }
        }

        #[test]
        fn run_only_checks_are_all_blocking() {
            let c = sabrage_core::contract();
            let run_only: Vec<_> = c.checks.iter().filter(|c| c.group == "run-only").collect();
            assert!(!run_only.is_empty(), "contract has no run-only checks");
            for check in run_only {
                assert_eq!(
                    check.shell_gate,
                    sabrage_core::Gate::Block,
                    "{}: run-only check must be shell_gate = block",
                    check.slug
                );
                assert_eq!(
                    check.native_gate,
                    sabrage_core::Gate::Block,
                    "{}: run-only check must be native_gate = block",
                    check.slug
                );
            }
        }

        #[test]
        fn cfg_protocol_is_split_into_supported_and_legacy_oxrsys() {
            let c = sabrage_core::contract();
            assert!(c.check("cfg.protocol.supported").is_some());
            assert!(c.check("cfg.protocol.legacy-oxrsys").is_some());
            // The pre-split slug must not linger.
            assert!(c.check("cfg.protocol").is_none());
        }

        #[test]
        fn legacy_reverse_ports_are_the_explicit_four_element_list() {
            let c = sabrage_core::contract();
            assert_eq!(c.ports.legacy_reverse, vec![9944, 9945, 9946, 9948]);
            // 9947 is deliberately never present, and this is never a range.
            assert!(!c.ports.legacy_reverse.contains(&9947));
        }

        /// Hermetic mirror of sabrage-core's registry invariants so CI (which
        /// runs only this crate + contract-gen on ubuntu) gates them: the
        /// strict registry must build, cover the contract in order, and leave
        /// **no** slug unbound — run-only preflights included. A mis-registered
        /// evaluator would otherwise ship CI-green and panic at runtime in the
        /// CLI and app.
        ///
        /// Phase 3 removed the `NO_DOCTOR_ROW_GROUP` exemption `build_registry`
        /// used to grant the three `run-only` slugs (they have no doctor
        /// *row*, but `checks::run_only` binds a real evaluator for each, which
        /// `stages::run::preflight` resolves through this same registry) — see
        /// `checks::mod`'s own doc comment on `build_registry`. This test
        /// pins that: `strict = true` now means every contract slug, with no
        /// group-shaped carve-out left.
        #[test]
        fn strict_registry_builds_and_covers_the_contract_in_order() {
            use sabrage_core::checks::build_registry;
            let reg = build_registry(true).expect(
                "strict registry builds — every contract slug, run-only preflights included, \
                 must have a bound evaluator",
            );
            let bound: Vec<&str> = reg.checks().iter().map(|c| c.slug()).collect();
            let declared = sabrage_core::contract().check_slugs();
            assert_eq!(bound, declared, "registry order must equal contract order");
            for c in reg.checks() {
                assert!(
                    c.eval.is_some(),
                    "contract slug {} has no bound evaluator",
                    c.slug()
                );
            }
        }
    }

    // ── (7) run launch preflight <-> contract, in both directions ────────────

    /// `run_sh_tags` (module 4) ties run.sh's `# preflight:`/`# launch-action:`
    /// tags to the contract. This module closes the other side of the same
    /// loop: the **native** slug/id lists, read from `sabrage-core` itself
    /// rather than re-derived, must equal the same contract data. Together the
    /// two prove shell tags == contract == native, not just shell == contract.
    mod run_launch_preflight_parity {
        use sabrage_core::{contract, Gate};

        /// `stages::run::preflight::preflight_slugs()` — the list the launch
        /// preflight actually walks — must be exactly the contract's
        /// native-gating check slugs, in contract order.
        #[test]
        fn preflight_slugs_equal_the_contracts_native_gating_checks_in_order() {
            let want: Vec<&str> = contract()
                .checks
                .iter()
                .filter(|c| c.native_gate != Gate::None)
                .map(|c| c.slug.as_str())
                .collect();
            assert_eq!(
                sabrage_core::stages::run::preflight::preflight_slugs(),
                want,
                "preflight_slugs() must be exactly contract().checks with native_gate != none, \
                 in contract order"
            );
        }

        /// `stages::run::actions::LAUNCH_ACTION_IDS` — the Rust constant the
        /// launch stage actually executes in order — must equal the contract's
        /// `[[launch_action]]` ids, in order, and there must be exactly 7 of
        /// them: run.sh's seven `# launch-action:` tags (checked against the
        /// contract by `run_sh_tags::launch_action_tags_match_the_contracts_order`),
        /// the contract's own `[[launch_action]]` table, and this constant.
        #[test]
        fn launch_action_ids_equal_the_contracts_order_and_there_are_exactly_seven() {
            let want: Vec<&str> = contract()
                .launch_actions
                .iter()
                .map(|a| a.id.as_str())
                .collect();
            assert_eq!(
                want.len(),
                7,
                "the contract must declare exactly 7 launch actions"
            );
            assert_eq!(
                sabrage_core::stages::run::actions::LAUNCH_ACTION_IDS.to_vec(),
                want,
                "LAUNCH_ACTION_IDS must equal contract().launch_actions' ids, in order"
            );
        }
    }

    // ── (8) launch artifact/behavior goldens ──────────────────────────────────

    mod launch_goldens {
        use sabrage_core::executor::PlannedKind;
        use sabrage_core::{contract, Bottle, Paths, StageCtx, StageOptions};
        use std::path::{Path, PathBuf};
        use std::sync::Arc;

        /// A dependency-free `block_on`.
        ///
        /// `sabrage-parity` carries no async runtime at all — every dependency
        /// in `Cargo.toml` is a `dev-dependency`, and none of them is `tokio`
        /// (adding one is a `Cargo.toml` edit outside this crate's ownership
        /// this phase). Every `DryRunExecutor` method
        /// [`sabrage_core::stages::run::actions::goldberg_stage`] drives
        /// (`copy_if_changed`/`write_atomic`/`create_dir_all`) is
        /// `Box::pin(async move { … Ok(…) })` with no real `.await` inside — it
        /// resolves on the very first poll — so a hand-rolled poll loop over
        /// [`std::task::Waker::noop`] (stable since 1.85) drives it to
        /// completion with no dependency at all.
        fn block_on<F: std::future::Future>(fut: F) -> F::Output {
            use std::task::{Context, Poll, Waker};
            let waker = Waker::noop();
            let mut cx = Context::from_waker(waker);
            let mut fut = Box::pin(fut);
            loop {
                match fut.as_mut().poll(&mut cx) {
                    Poll::Ready(v) => return v,
                    Poll::Pending => std::thread::yield_now(),
                }
            }
        }

        fn scratch(tag: &str) -> PathBuf {
            let p = std::env::temp_dir().join(format!(
                "sabrage-parity-launch-{tag}-{}",
                std::process::id()
            ));
            std::fs::remove_dir_all(&p).ok();
            std::fs::create_dir_all(&p).unwrap();
            p
        }

        fn silent_sink() -> sabrage_core::EventSink {
            Arc::new(|_| {})
        }

        // ── steam_appid.txt bytes ────────────────────────────────────────────

        /// run.sh:150 — `printf '%s' "$BS_APPID" > "$APIDIR/steam_appid.txt"`:
        /// the appid digits, and nothing else — no trailing newline. Driven
        /// through the real `actions::goldberg_stage` under `--dry-run` (never
        /// a copy of its recipe), so a call-site regression — util's helper is
        /// right and the call site passes something else, exactly the bug the
        /// Phase-2 review found on the host manifest — would turn this red:
        /// the plan's `Write` action for `steam_appid.txt` records `"<n>
        /// bytes"`, and `n` must equal the appid string's own length with no
        /// byte added or dropped.
        #[test]
        fn steam_appid_txt_is_written_as_exactly_the_appid_digits_with_no_trailing_newline() {
            let root = scratch("goldberg");
            let bs_dir = root.join("BeatSaber");
            std::fs::create_dir_all(&bs_dir).unwrap();
            // The game-root fallback `steam_api_path` checks second — no
            // plugin subdirectory needed for this golden.
            std::fs::write(bs_dir.join("steam_api64.dll"), b"steam").unwrap();

            let paths = Paths::new(&root);
            std::fs::create_dir_all(paths.gbe_dll.parent().unwrap()).unwrap();
            std::fs::write(&paths.gbe_dll, b"goldberg-bytes").unwrap();

            let opts = StageOptions {
                bs_dir_override: Some(bs_dir),
                dry_run: true,
                ..StageOptions::default()
            };
            let ctx = StageCtx::new(paths, opts, silent_sink(), Default::default());

            block_on(sabrage_core::stages::run::actions::goldberg_stage(&ctx))
                .expect("a dry run plans goldberg-stage cleanly, never errors");

            let appid = contract().game.appid.to_string();
            assert_eq!(appid, "620980");
            assert!(!appid.ends_with('\n'), "printf '%s', never print/println");

            let plan = ctx.executor.planned();
            let appid_write = plan
                .iter()
                .find(|a| {
                    a.kind == PlannedKind::Write
                        && a.dst
                            .as_deref()
                            .is_some_and(|d| d.ends_with("steam_appid.txt"))
                })
                .expect("steam_appid.txt is planned by goldberg_stage");
            assert_eq!(
                appid_write.reason,
                format!("{} bytes", appid.len()),
                "steam_appid.txt must be exactly the appid digits — no trailing newline, \
                 no other byte"
            );

            std::fs::remove_dir_all(&root).ok();
        }

        // ── steam_appid.txt bytes, as they land on disk ──────────────────────

        /// A synchronous [`Executor`] that really writes, into the test's own
        /// scratch tree.
        ///
        /// The plan a dry run records carries no content — `write_atomic`
        /// becomes `PlannedAction { reason: "<n> bytes" }` and the bytes are
        /// dropped — so the golden above can only pin the payload's *length*:
        /// `b"999999"` and `b"62098\n"` both record `"6 bytes"`. Reading the
        /// file back is the only way to pin the bytes, and
        /// [`sabrage_core::executor::RealExecutor`] cannot be driven from this
        /// crate (its primitives are `tokio::fs`, and sabrage-parity has no
        /// async runtime — adding one is a `Cargo.toml` edit outside this
        /// crate's ownership). `std::fs` behind the same trait gives the real
        /// on-disk bytes with no dependency at all; every primitive the run
        /// stage does not use panics rather than pretending to succeed.
        #[derive(Debug)]
        struct SyncFsExecutor;

        impl sabrage_core::executor::Executor for SyncFsExecutor {
            fn with_step(
                &self,
                _step: sabrage_core::events::StepId,
            ) -> std::sync::Arc<dyn sabrage_core::executor::Executor> {
                std::sync::Arc::new(SyncFsExecutor)
            }

            fn copy_if_changed<'a>(
                &'a self,
                src: &'a Path,
                dst: &'a Path,
            ) -> sabrage_core::executor::BoxFuture<
                'a,
                sabrage_core::Result<sabrage_core::executor::Copied>,
            > {
                Box::pin(async move {
                    if sabrage_core::util::cmp_files(src, dst) {
                        return Ok(sabrage_core::executor::Copied::Unchanged);
                    }
                    std::fs::copy(src, dst).expect("scratch copy succeeds");
                    Ok(sabrage_core::executor::Copied::Copied)
                })
            }

            fn write_atomic<'a>(
                &'a self,
                path: &'a Path,
                bytes: &'a [u8],
            ) -> sabrage_core::executor::BoxFuture<'a, sabrage_core::Result<()>> {
                Box::pin(async move {
                    std::fs::write(path, bytes).expect("scratch write succeeds");
                    Ok(())
                })
            }

            fn remove_dir_all<'a>(
                &'a self,
                path: &'a Path,
            ) -> sabrage_core::executor::BoxFuture<'a, sabrage_core::Result<()>> {
                Box::pin(async move {
                    std::fs::remove_dir_all(path).ok();
                    Ok(())
                })
            }

            fn remove_file<'a>(
                &'a self,
                path: &'a Path,
            ) -> sabrage_core::executor::BoxFuture<'a, sabrage_core::Result<()>> {
                Box::pin(async move {
                    std::fs::remove_file(path).ok();
                    Ok(())
                })
            }

            fn create_dir_all<'a>(
                &'a self,
                path: &'a Path,
            ) -> sabrage_core::executor::BoxFuture<'a, sabrage_core::Result<()>> {
                Box::pin(async move {
                    std::fs::create_dir_all(path).expect("scratch mkdir succeeds");
                    Ok(())
                })
            }

            fn dir_copy<'a>(
                &'a self,
                _src: &'a Path,
                _dst: &'a Path,
            ) -> sabrage_core::executor::BoxFuture<'a, sabrage_core::Result<()>> {
                unimplemented!("dir_copy is not part of the goldberg stage")
            }

            fn download<'a>(
                &'a self,
                _url: &'a str,
                _dest: &'a Path,
                _sha256: &'a str,
                _label: &'a str,
            ) -> sabrage_core::executor::BoxFuture<
                'a,
                sabrage_core::Result<sabrage_core::executor::Downloaded>,
            > {
                unimplemented!("download is not part of the goldberg stage")
            }

            fn tar_xzf<'a>(
                &'a self,
                _archive: &'a Path,
                _into_dir: &'a Path,
            ) -> sabrage_core::executor::BoxFuture<'a, sabrage_core::Result<()>> {
                unimplemented!("tar_xzf is not part of the goldberg stage")
            }

            fn touch<'a>(
                &'a self,
                _path: &'a Path,
            ) -> sabrage_core::executor::BoxFuture<'a, sabrage_core::Result<()>> {
                unimplemented!("touch is not part of the goldberg stage")
            }

            fn run_child<'a>(
                &'a self,
                _spec: &'a sabrage_core::process::ChildSpec,
            ) -> sabrage_core::executor::BoxFuture<'a, sabrage_core::Result<std::process::ExitStatus>>
            {
                unimplemented!("this golden spawns no children")
            }

            fn spawn_detached<'a>(
                &'a self,
                _spec: &'a sabrage_core::process::ChildSpec,
                _stdio: sabrage_core::executor::DetachedStdio,
            ) -> sabrage_core::executor::BoxFuture<
                'a,
                sabrage_core::Result<Option<sabrage_core::executor::DetachedChild>>,
            > {
                unimplemented!("this golden spawns no children")
            }
        }

        /// run.sh:150 — `printf '%s' "$BS_APPID" > "$APIDIR/steam_appid.txt"`,
        /// read back off disk: the appid digits and **nothing else**.
        ///
        /// The dry-run golden above can only see the payload's length; this one
        /// goes red for any six-byte impostor (`999999`, `62098\n`) as well as
        /// for a wrong length.
        #[test]
        fn steam_appid_txt_lands_on_disk_as_exactly_the_appid_digits() {
            let root = scratch("goldberg-bytes");
            let bs_dir = root.join("BeatSaber");
            std::fs::create_dir_all(&bs_dir).unwrap();
            let api = bs_dir.join("steam_api64.dll");
            std::fs::write(&api, b"steam").unwrap();

            let paths = Paths::new(&root);
            std::fs::create_dir_all(paths.gbe_dll.parent().unwrap()).unwrap();
            std::fs::write(&paths.gbe_dll, b"goldberg-bytes").unwrap();

            let opts = StageOptions {
                bs_dir_override: Some(bs_dir.clone()),
                dry_run: false,
                ..StageOptions::default()
            };
            let ctx = StageCtx::with_executor(
                paths,
                opts,
                silent_sink(),
                Default::default(),
                Arc::new(SyncFsExecutor),
                sabrage_core::events::RunId::default(),
            );

            block_on(sabrage_core::stages::run::actions::goldberg_stage(&ctx))
                .expect("goldberg_stage completes against a complete fixture tree");

            let written = std::fs::read(bs_dir.join("steam_appid.txt"))
                .expect("goldberg_stage wrote steam_appid.txt beside steam_api64.dll");
            assert_eq!(
                written,
                contract().game.appid.to_string().into_bytes(),
                "steam_appid.txt must be exactly the contract's appid digits — no trailing \
                 newline, no other byte"
            );
            // Belt and braces: the Goldberg dll really went over the api dll,
            // so the assertion above is about a stage that actually ran.
            assert_eq!(std::fs::read(&api).unwrap(), b"goldberg-bytes");

            std::fs::remove_dir_all(&root).ok();
        }

        // ── wine_env ─────────────────────────────────────────────────────────

        /// run.sh:242-248, table form. The load-bearing branch is `WINEDEBUG`:
        /// the caller's preset wins in **both** the verbose and non-verbose
        /// arms (`${WINEDEBUG:-…}`), and an inherited empty string is treated
        /// like unset (zsh's `:-`, not `-`).
        #[test]
        fn wine_env_table() {
            use sabrage_core::stages::run::actions::wine_env;
            let runtime_json = Path::new("/repo/ext/oxrsys/build-x64/runtime/oxrsys-runtime.json");
            let appid = 620980u64;

            fn get(env: &[(String, String)], key: &str) -> String {
                env.iter()
                    .find(|(k, _)| k == key)
                    .unwrap_or_else(|| panic!("{key} missing from wine_env"))
                    .1
                    .clone()
            }

            let env = wine_env(false, None, appid, runtime_json);
            assert_eq!(get(&env, "WINEDEBUG"), "-all");
            assert_eq!(
                get(&env, "XR_RUNTIME_JSON"),
                runtime_json.display().to_string()
            );
            assert_eq!(get(&env, "CX_GRAPHICS_BACKEND"), "dxmt");
            assert_eq!(get(&env, "SteamAppId"), "620980");
            assert_eq!(get(&env, "SteamGameId"), "620980");

            assert_eq!(
                get(&wine_env(true, None, appid, runtime_json), "WINEDEBUG"),
                "fixme-all,+openxr"
            );

            // Caller-set WINEDEBUG wins in BOTH branches.
            assert_eq!(
                get(
                    &wine_env(false, Some("+d3d11"), appid, runtime_json),
                    "WINEDEBUG"
                ),
                "+d3d11"
            );
            assert_eq!(
                get(
                    &wine_env(true, Some("+d3d11"), appid, runtime_json),
                    "WINEDEBUG"
                ),
                "+d3d11"
            );

            // An inherited empty value is treated like unset.
            assert_eq!(
                get(&wine_env(false, Some(""), appid, runtime_json), "WINEDEBUG"),
                "-all"
            );
            assert_eq!(
                get(&wine_env(true, Some(""), appid, runtime_json), "WINEDEBUG"),
                "fixme-all,+openxr"
            );
        }

        // ── wine_spec argv ───────────────────────────────────────────────────

        /// `"$WINE" --bottle "$WINEVR_BOTTLE" --no-update --cx-app "$BS_WIN"`.
        #[test]
        fn wine_spec_argv_matches_run_shs_command_line() {
            let root = scratch("winespec");
            let mut paths = Paths::new(&root);
            paths.wine = Some(root.join("CrossOver/bin/wine"));

            let opts = StageOptions {
                bottle_name: Some("Steam".to_string()),
                bs_dir_override: Some(root.join("BeatSaber")),
                ..StageOptions::default()
            };
            let ctx = StageCtx::new(paths, opts, silent_sink(), Default::default());
            let bottle = Bottle::unvalidated("Steam");

            let spec = sabrage_core::stages::run::actions::wine_spec(&ctx, &bottle);
            assert_eq!(
                spec.program,
                ctx.paths.wine.clone().unwrap().into_os_string()
            );

            let want_win = sabrage_core::util::win_path(
                Some(&bottle.prefix),
                &ctx.bs_dir.join("Beat Saber.exe"),
            );
            let want_args: Vec<std::ffi::OsString> =
                ["--bottle", "Steam", "--no-update", "--cx-app"]
                    .into_iter()
                    .map(std::ffi::OsString::from)
                    .chain(std::iter::once(std::ffi::OsString::from(want_win)))
                    .collect();
            assert_eq!(
                spec.args, want_args,
                "argv must be exactly: --bottle <name> --no-update --cx-app <win path>"
            );

            std::fs::remove_dir_all(&root).ok();
        }

        // ── wine_log_candidate ───────────────────────────────────────────────

        /// `date +%Y%m%d-%H%M%S` for attempt 0; Sabrage's own `-{n+1}` suffix
        /// on a collision (declared divergence — PARITY.md).
        #[test]
        fn wine_log_candidate_matches_run_shs_date_stamp_and_the_dash_two_collision_suffix() {
            use chrono::TimeZone;
            let logs_dir = Path::new("/repo/logs");
            let now = chrono::Local
                .with_ymd_and_hms(2026, 8, 29, 10, 11, 12)
                .unwrap();

            let p0 = sabrage_core::logs::wine_log_candidate(logs_dir, now, 0);
            assert_eq!(p0, logs_dir.join("beatsaber-20260829-101112.log"));
            let re = regex::Regex::new(r"^beatsaber-\d{8}-\d{6}\.log$").unwrap();
            assert!(
                re.is_match(p0.file_name().unwrap().to_str().unwrap()),
                "attempt 0's name must match run.sh's `date +%Y%m%d-%H%M%S` shape exactly"
            );

            let p1 = sabrage_core::logs::wine_log_candidate(logs_dir, now, 1);
            assert_eq!(p1, logs_dir.join("beatsaber-20260829-101112-2.log"));
        }
    }

    // ── (9) run.sh die/warn/banner text pinned verbatim ───────────────────────

    /// `stages::run` reproduces a long list of run.sh's `die`/`warn`/`info`/
    /// banner strings verbatim, scattered as `&str` literals across
    /// `preflight.rs`, `actions.rs`, `guards.rs` and `mod.rs` — most of them
    /// not contract-derived, so nothing above catches one going stale. Several
    /// of the native functions that own this text are `pub(crate)`
    /// (`banner_events`, `bs_win_path`), so this crate cannot call them
    /// directly; instead every fragment below is copied **from the native
    /// source** (confirmed against the actual `&str` literal at the call site,
    /// not just the doc comment) and pinned here as a substring of the on-disk
    /// `run.sh` — this module never calls native code at all.
    ///
    /// # What each half is actually gated by
    ///
    /// Editing `run.sh`'s wording without updating the native literal turns
    /// **this module's** tests red, because they pin the fragment as a
    /// substring of the on-disk file.
    ///
    /// The other direction — editing a native literal without touching
    /// `run.sh` — is NOT gated by sabrage-core's own frozen-text unit tests
    /// (`guards::tests::the_guard_texts_are_run_shs_verbatim`,
    /// `mod::tests::the_closing_lines_are_run_shs_verbatim`,
    /// `actions::tests::the_banner_is_run_shs_nine_lines_in_order`, …), which
    /// this module's header used to claim: tier 1 selects `sabrage-parity` +
    /// `sabrage-contract-gen` only (`scripts/dev/parity.sh`,
    /// `.github/workflows/parity.yml`), and Cargo does not run a
    /// dev-dependency's `#[cfg(test)]` harness. Those tests exist but nothing
    /// runs them in the gate, and CI (ubuntu) cannot add `-p sabrage-core`
    /// because much of that suite is macOS-shaped.
    ///
    /// So the native half is pinned **here**, by calling the native renderer
    /// wherever it is `pub`:
    /// [`native_run_only_die_text_is_verbatim_in_run_sh`] evaluates the real
    /// `checks::run_only` evaluators and asserts their own message text is in
    /// `run.sh`, and modules (5)/(8) do the same for the host manifest, the
    /// toml template, `win_path`, `wine_env`, `wine_spec` and
    /// `steam_appid.txt`. What remains substring-only is the text owned by
    /// `pub(crate)` functions (`stages::run::actions::banner_events`,
    /// `stages::run::mod`'s exit/teardown lines, `stages::run::guards`'
    /// audio/dashboard rows, `stages::run::preflight`'s die strings): making
    /// those `pub` (or adding `pub fn banner_lines()`-shaped accessors) is a
    /// sabrage-core change this crate cannot make.
    mod run_sh_text_parity {
        use super::repo_root;

        fn run_sh() -> String {
            std::fs::read_to_string(repo_root().join("scripts/demo/run.sh"))
                .expect("scripts/demo/run.sh reads")
        }

        fn assert_verbatim(text: &str, fragment: &str, native_site: &str) {
            assert!(
                text.contains(fragment),
                "run.sh no longer contains {fragment:?}, which {native_site} reproduces verbatim"
            );
        }

        /// The native half, called for real: `checks::run_only`'s evaluators
        /// carry `run.sh`'s `die` sentences in `message` (they have no doctor
        /// row, so run.sh IS their prose source). Editing one of those literals
        /// without editing `run.sh` turns this red — no dependency-crate unit
        /// test involved.
        #[test]
        fn native_run_only_die_text_is_verbatim_in_run_sh() {
            use sabrage_core::checks::{CheckCtx, CheckOptions, CheckStatus};

            let text = run_sh();
            let scratch = std::env::temp_dir()
                .join(format!("sabrage-parity-run-only-{}", std::process::id()));
            std::fs::remove_dir_all(&scratch).ok();
            std::fs::create_dir_all(&scratch).unwrap();

            // A fixture root with nothing built and no machine tools: never the
            // real CrossOver or adb.
            let mut paths = sabrage_core::Paths::new(&scratch);
            paths.wine = None;
            paths.adb = None;

            let defs = sabrage_core::checks::run_only::defs();
            let eval = |slug: &str, opts: CheckOptions| {
                let ctx = CheckCtx::new(paths.clone(), opts);
                let f = defs
                    .iter()
                    .find(|(s, _)| *s == slug)
                    .unwrap_or_else(|| panic!("{slug} is bound"))
                    .1;
                f(&ctx)
            };

            let bridge = eval("run.bridge-built", CheckOptions::new());
            assert_eq!(bridge.status, CheckStatus::Fail);
            assert_verbatim(&text, &bridge.message, "checks::run_only::run_bridge_built");

            // `--wired` with no adb at all: run.sh's first --wired die.
            let wired = eval(
                "run.wired-adb",
                CheckOptions {
                    wired: true,
                    ..CheckOptions::new()
                },
            );
            assert_eq!(wired.status, CheckStatus::Fail);
            assert_verbatim(&text, &wired.message, "checks::run_only::run_wired_adb");

            std::fs::remove_dir_all(&scratch).ok();
        }

        #[test]
        fn preflight_die_and_warn_text_is_verbatim_in_run_sh() {
            let text = run_sh();
            for (fragment, site) in [
                (
                    "bridge not built — ./demo.sh build",
                    "checks::run_only::run_bridge_built",
                ),
                (
                    "Goldberg dll missing — ./demo.sh setup",
                    "stages::run::preflight::block_die",
                ),
                (
                    "CrossOver DXMT overlay stale (CrossOver update?)",
                    "stages::run::preflight::block_die",
                ),
                (
                    "bottle wineopenxr.dll stale/missing",
                    "stages::run::preflight::block_die",
                ),
                (
                    "bottle OpenXR manifest missing",
                    "stages::run::preflight::block_die",
                ),
                (
                    "bottle ActiveRuntime registry key missing",
                    "stages::run::preflight::block_die",
                ),
                (
                    "host OpenXR registration missing",
                    "stages::run::preflight::block_die",
                ),
                (
                    "could not force graphics backend to dxmt in",
                    "stages::run::preflight::post_fix_die",
                ),
                (
                    "the Meta gate may block startup",
                    "stages::run::preflight::gate (game.version)",
                ),
                (
                    "encoder_process=inproc — in-process x86_64 encode (native helper disabled)",
                    "stages::run::preflight::emit_encoder_notice",
                ),
                (
                    "the runtime treats unknown values as auto",
                    "stages::run::preflight::emit_encoder_notice",
                ),
                (
                    "--wired needs adb (Android platform-tools) on PATH or under ~/Library/Android/sdk",
                    "checks::run_only::run_wired_adb",
                ),
                (
                    "--wired: no Quest over adb — connect USB and check 'adb devices'",
                    "checks::run_only::run_wired_adb",
                ),
            ] {
                assert_verbatim(&text, fragment, site);
            }
        }

        #[test]
        fn launch_action_text_is_verbatim_in_run_sh() {
            let text = run_sh();
            for (fragment, site) in [
                (
                    "kill the listed wineserver(s) manually, then re-run",
                    "stages::run::actions::wineserver_reset",
                ),
                (
                    "wineserver still alive after 5s:",
                    "stages::run::actions::wineserver_reset (RUN_WINESERVER_WAIT = 5s)",
                ),
                (
                    "resetting wineserver for bottle '",
                    "stages::run::actions::wineserver_reset",
                ),
                ("wineserver down", "stages::run::actions::wineserver_reset"),
                (
                    "steam_api64.dll not found under",
                    "stages::run::actions::goldberg_stage",
                ),
                (
                    "backup of original steam_api64.dll failed",
                    "stages::run::actions::goldberg_stage",
                ),
                (
                    "goldberg already installed",
                    "stages::run::actions::goldberg_stage",
                ),
                (
                    "goldberg install failed",
                    "stages::run::actions::goldberg_stage",
                ),
                (
                    "cleared adb reverse tunnels (ALVR manages its own)",
                    "stages::run::actions::adb_reverse_cleanup",
                ),
                (
                    "wired mode: adb forward",
                    "stages::run::actions::adb_forward_hygiene",
                ),
                (
                    "a later non-wired run clears these two",
                    "stages::run::actions::adb_forward_hygiene",
                ),
                (
                    "cleared stale adb forward",
                    "fixes::adb (fix.remove-adb-forwards)",
                ),
                (
                    "encoder helper: reaped (left over from the runtime)",
                    "stages::run::mod::HELPER_REAPED_LINE",
                ),
            ] {
                assert_verbatim(&text, fragment, site);
            }
        }

        #[test]
        fn audio_and_dashboard_guard_text_is_verbatim_in_run_sh() {
            let text = run_sh();
            for (fragment, site) in [
                (
                    "audio routing disabled (--no-audio) — sound stays on the Mac",
                    "stages::run::guards",
                ),
                (
                    "could not switch output to BlackHole 2ch — audio stays on the Mac",
                    "stages::run::guards",
                ),
                (
                    "BlackHole 2ch not present (brew install blackhole-2ch + reboot) — audio stays on the Mac",
                    "stages::run::guards",
                ),
                (
                    "audio: default output -> BlackHole 2ch (was:",
                    "stages::run::guards",
                ),
                (
                    "ALVR dashboard disabled (--no-dashboard)",
                    "stages::run::guards",
                ),
                (
                    "dashboard: ALVR server dashboard opening (connects once the game is up)",
                    "stages::run::guards",
                ),
                (
                    "alvr_dashboard not built — ./demo.sh build (continuing without the dashboard)",
                    "stages::run::guards",
                ),
                ("dashboard: closed", "stages::run::guards"),
            ] {
                assert_verbatim(&text, fragment, site);
            }
        }

        /// run.sh:252-260's six-line banner block.
        #[test]
        fn the_launch_banner_lines_are_verbatim_in_run_sh() {
            let text = run_sh();
            for fragment in [
                "launching Beat Saber through the bridge",
                "   put the headset ON and open the ALVR client; first frame can take ~30s.",
                "   pause in-game = X/A button or the Quest system button",
                "   (the left-menu-button pause is a Beat Saber/Unity limitation on every OpenXR runtime)",
                "   stop: Ctrl-C here, or ./demo.sh stop --bottle",
                "from another shell",
                "   exe: ",
                "   log: ",
            ] {
                assert_verbatim(&text, fragment, "stages::run::actions::banner_events");
            }
            assert_verbatim(
                &text,
                "wine exited with status",
                "stages::run::mod::wine_exit_line",
            );
            assert_verbatim(
                &text,
                "interrupted: stopping wine",
                "stages::run::mod::run (INT teardown section)",
            );
        }
    }
    // ── (10) repo-root spelling ──────────────────────────────────────────────

    /// Both front-ends embed the repo root as an absolute string inside the
    /// root-owned host manifest and compare those bytes **literally**
    /// (`install.sh`'s `[ "$(cat "$HOST_XR_JSON")" = "$WANT" ]`,
    /// `util::host_manifest_is_current` here). Two spellings of one checkout
    /// therefore mean two different manifests: alternating between Sabrage and
    /// `./demo.sh install` each sees the other's file as stale and prompts for
    /// sudo again.
    ///
    /// The shared spelling contract is zsh's **logical** `pwd`: absolute,
    /// `.`/`..` folded textually, symlinks preserved as the user spelled them.
    /// demo.sh gets it from `ROOT="$(cd "$(dirname "$0")" && pwd)"`;
    /// `paths::resolve_repo_root` mirrors it (`logical_absolute`, deliberately
    /// not `canonicalize`). This module pins both halves — the shell's `pwd`
    /// staying logical, and the native resolver's two behaviours — so neither
    /// side can drift to a physical spelling on its own.
    mod repo_root_spelling {
        use super::repo_root;

        #[test]
        fn demo_shs_root_is_the_logical_pwd() {
            let text = std::fs::read_to_string(repo_root().join("demo.sh")).expect("demo.sh reads");
            let line = text
                .lines()
                .find(|l| l.trim_start().starts_with("ROOT="))
                .expect("demo.sh assigns ROOT");
            assert_eq!(
                line.trim(),
                r#"ROOT="$(cd "$(dirname "$0")" && pwd)""#,
                "demo.sh's repo-root spelling is the logical `pwd`; `pwd -P` (or any other \
                 physical resolution) would resolve symlinks and diverge from \
                 paths::resolve_repo_root's logical_absolute — see sabrage/PARITY.md"
            );
        }

        #[test]
        fn the_native_resolver_preserves_a_symlinked_spelling_and_folds_dotdot() {
            let base = std::env::temp_dir().join(format!(
                "sabrage-parity-rootspelling-{}",
                std::process::id()
            ));
            std::fs::remove_dir_all(&base).ok();
            let physical = base.join("physical");
            std::fs::create_dir_all(&physical).unwrap();
            let link = base.join("checkout");
            std::os::unix::fs::symlink(&physical, &link).unwrap();

            let spelled = sabrage_core::resolve_repo_root(Some(link.to_str().unwrap()))
                .expect("an explicit root resolves");
            assert_eq!(
                spelled, link,
                "a symlinked checkout keeps its symlink spelling, exactly as `cd <link> && pwd` \
                 reports it"
            );

            // `..` is folded textually, so `<link>/anything/..` is `<link>` —
            // even though `<link>/anything` does not exist.
            let dotted = link.join("build/..");
            let folded = sabrage_core::resolve_repo_root(Some(dotted.to_str().unwrap()))
                .expect("an explicit root resolves");
            assert_eq!(folded, link);

            // The symptom the spelling contract exists to prevent: the same
            // checkout, spelled two ways, must produce one manifest.
            let a = sabrage_core::Paths::new(&spelled);
            let b = sabrage_core::Paths::new(&folded);
            assert_eq!(
                sabrage_core::util::host_manifest_file_bytes(&a.oxr_dylib),
                sabrage_core::util::host_manifest_file_bytes(&b.oxr_dylib),
                "two spellings of one checkout must write identical host-manifest bytes"
            );

            std::fs::remove_dir_all(&base).ok();
        }
    }
}
