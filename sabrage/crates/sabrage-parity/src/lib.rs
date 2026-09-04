//! Parity tests between the native pipeline (`sabrage-core` / `sabrage-contract-gen`)
//! and the zsh reference implementation (`demo.sh`, `scripts/demo/*.sh`,
//! `contract/`).
//!
//! This crate carries **no runtime surface** — see `Cargo.toml`: every
//! dependency is a dev-dependency, and every test in `tests` is a tier-1 hermetic
//! `cargo test` per `docs/design/design-parity.md` §4 ("always-on pure tests,
//! no env gate, no machine state" beyond reading the repo tree the crate is
//! built from). Tier 2 (the live doctor diff) and the pre-push hook are
//! `scripts/dev/parity.sh`'s job, not this crate's.
//!
//! Every test reads its shell/contract inputs from the **working checkout on
//! disk** via [`tests::repo_root`], never from a compiled-in copy — with one
//! deliberate exception: the contract-gen parity test also compiles in its
//! own `include_str!` of the committed `scripts/demo/contract.gen.sh`, so the
//! compiled generator is compared against the checked-in bytes (the one place
//! those bytes are pinned). Everywhere else the point of this crate is to
//! catch "the checkout and the generated/compiled artifact disagree," which
//! comparing two compiled-in copies of the same `include_str!` would defeat
//! by construction.

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    /// The repo root, resolved from this crate's manifest dir.
    ///
    /// `sabrage-contract-gen` sits at the same `crates/<name>` depth, so both
    /// resolve the same directory — pinned by
    /// `tests::contract_gen_parity::check_reports_in_sync_against_the_live_checkout`.
    pub(crate) fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .canonicalize()
            .expect("repo root resolves")
    }

    /// `generate() == committed scripts/demo/contract.gen.sh`, as a compile-time
    /// byte comparison against this crate's own `include_str!` of the checked-in
    /// file, plus a live `--check` against the working checkout. This module is
    /// the only place those bytes are pinned.
    mod contract_gen_parity {
        use super::repo_root;

        #[test]
        fn generate_matches_the_committed_contract_gen_sh() {
            let generated = sabrage_contract_gen::generate();
            // include_str! resolves relative to this file, so the comparison is
            // against the bytes checked in on disk, never a value re-derived
            // from contract/.
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

        /// The three contract scalars `sabrage-contract-gen` emits must be
        /// sourced by the shell, never re-typed: editing one in `pipeline.toml`
        /// and regenerating moves only the generated file's `# contract-sha256:`
        /// header, so `--check`, `--regen` and doctor's `meta.contract-sync` all
        /// report "in sync" while native setup fetches one asset and setup.sh
        /// another (or the two resolve BS_DIR to different directories).
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

        /// `--check` against the working checkout reports in sync — and the
        /// root it is checked against is the same directory the contract-gen
        /// binary falls back to when `--repo-root` is omitted
        /// (`compiled_repo_root`, `main.rs`), which nothing else exercises.
        #[test]
        fn check_reports_in_sync_against_the_live_checkout() {
            let compiled = sabrage_contract_gen::compiled_repo_root()
                .canonicalize()
                .expect("sabrage-contract-gen::compiled_repo_root() resolves");
            assert_eq!(
                compiled,
                repo_root(),
                "sabrage-contract-gen::compiled_repo_root() no longer resolves to the repo \
                 root — the contract-gen binary's default --repo-root (main.rs) is wrong"
            );
            let report = sabrage_contract_gen::check(&repo_root())
                .expect("contract/ files under repo_root are readable");
            assert!(
                report.in_sync,
                "scripts/demo/contract.gen.sh is stale relative to contract/ — run: \
                 cargo run -p sabrage-contract-gen -- --write"
            );
        }
    }

    /// `sabrage/parity/shell.fingerprint` pins the sha256 of `demo.sh` plus
    /// every `scripts/demo/*.sh` (`contract.gen.sh` excluded — `contract_gen_parity`
    /// covers it). Any edit to a tracked shell file — its content OR the tracked
    /// file set itself — turns this test red until `scripts/dev/parity.sh --bless`
    /// re-signs it, which per docs/design/design-parity.md §4 tier 1.2 only
    /// happens after the rest of the suite passes.
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
        /// whitespace) and is not inside a single- or double-quoted string, so
        /// `${_f#$ROOT/}`, `$#` and a quoted `#` all survive.
        ///
        /// Without this the scanner credits a commented-out `chk` line as a live
        /// emission, so a deleted check keeps its coverage; see
        /// `tests::slug_coverage::a_hash_starts_a_comment_only_when_it_begins_an_unquoted_word`.
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
        /// The two halves must be *joined*, not merely both present: a `chk`/`tap`
        /// call counts only when it carries the loop item's slug, inline
        /// (`${_x%%:*}`) or through the assigned variable. Independent counting
        /// credits every header slug once the body contains any unrelated emission
        /// — the shape a deleted per-item check leaves behind (r1:A1-7). See
        /// `tests::slug_coverage::a_loop_credits_its_header_slugs_only_when_the_body_emits_for_them`.
        fn loop_body_emits(rest: &[String], var: &str, call_re: &Regex) -> bool {
            let extraction = format!("${{{var}%%:*}}");
            // Shell text that provably carries this loop item's slug.
            let mut carriers = vec![Regex::new(&regex::escape(&extraction)).unwrap()];
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
                // Assignments first: one logical line can both extract and
                // emit (`_slug="${_o%%:*}"; chk ok "$_slug" …`).
                for (idx, _) in logical.match_indices(&extraction) {
                    if let Some(name) = assigned_variable(&logical[..idx]) {
                        let n = regex::escape(&name);
                        carriers.push(Regex::new(&format!(r"\$(?:\{{{n}\}}|{n}\b)")).unwrap());
                    }
                }
                if call_re.is_match(logical) && carriers.iter().any(|re| re.is_match(logical)) {
                    return true;
                }
            }
            false
        }

        /// The variable an extraction is being assigned to, given everything
        /// on the line before it: `_slug="` / `local _slug=` → `_slug`.
        /// `None` when the extraction is not the right-hand side of an
        /// assignment (e.g. it is an argument, or part of a larger word).
        fn assigned_variable(before: &str) -> Option<String> {
            let head = before.trim_end_matches('"').strip_suffix('=')?;
            let name: String = head
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            (!name.is_empty()).then_some(name)
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
                .filter(|c| c.group != sabrage_core::checks::NO_DOCTOR_ROW_GROUP)
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

        fn slugs_of(fixture: &str) -> Vec<String> {
            scan_doctor_slugs(fixture).order
        }

        /// A `#` starts a comment only when it begins a word outside quotes;
        /// everything before such a `#` is still live shell, and a `#` inside
        /// an expansion or a quoted string is not one at all.
        #[test]
        fn a_hash_starts_a_comment_only_when_it_begins_an_unquoted_word() {
            let cases: &[(&str, &str, &[&str])] = &[
                (
                    "live chk line",
                    "# 5. rust\nchk ok rust.x64-target \"ok\"\n",
                    &["rust.x64-target"],
                ),
                (
                    "r1:A1-7 regression: a full-line #-comment contributes no slug",
                    "# 5. rust\n#chk ok rust.x64-target \"ok\"\n",
                    &[],
                ),
                (
                    "r1:A1-7 regression: an indented #-comment contributes no slug",
                    "# 5. rust\n  # chk ok rust.x64-target \"ok\"\n",
                    &[],
                ),
                (
                    "a trailing comment does not hide the call before it",
                    "# 5. rust\nchk ok rust.x64-target \"ok\"  # trailing note\n",
                    &["rust.x64-target"],
                ),
                (
                    "a hash inside a parameter expansion or a quoted string is not a comment",
                    "# 9. build\nchk ok build.oxr-dylib \"built: ${_f#$ROOT/} # not a comment\"; chk ok build.woxr-dll \"ok\"\n",
                    &["build.oxr-dylib", "build.woxr-dll"],
                ),
            ];
            for (label, fixture, expected) in cases {
                let slugs = slugs_of(fixture);
                let got: Vec<&str> = slugs.iter().map(String::as_str).collect();
                assert_eq!(&got[..], *expected, "{label}");
            }
        }

        /// A loop header's slugs are credited only when the body emits a
        /// `chk`/`tap` that carries the loop item itself; any other body shape
        /// leaves the header as a `header_only_loops` entry instead.
        #[test]
        fn a_loop_credits_its_header_slugs_only_when_the_body_emits_for_them() {
            let cases: &[(&str, &str, &[&str], usize)] = &[
                (
                    "covered loop: inline ${_t%%:*} in the chk argument",
                    "# 4. toolchain\nfor _t in tool.cmake:cmake tool.ninja:ninja; do\n  chk ok ${_t%%:*} \"${_t#*:}\"\ndone\n",
                    &["tool.cmake", "tool.ninja"],
                    0,
                ),
                (
                    "r1:A1-7 regression: a loop header whose body emits nothing covers no slug",
                    "# 4. toolchain\nfor _t in tool.cmake:cmake tool.ninja:ninja; do\n  command -v ${_t#*:} >/dev/null\ndone\n",
                    &[],
                    1,
                ),
                (
                    "section 10's shape: `_slug=\"${_o%%:*}\"` on its own line, then `chk ok \"$_slug\" …`",
                    "# 10. overlay\nfor _o in overlay.woxr-dll:a:b; do\n  _slug=\"${_o%%:*}\"\n  chk ok \"$_slug\" \"current\"\ndone\n",
                    &["overlay.woxr-dll"],
                    0,
                ),
                (
                    "A1-7 / r2:A1-6 regression: extraction line kept and the per-item chk deleted — only the unrelated static emission is real",
                    "# 10. overlay\nfor _o in overlay.woxr-dll:a:b overlay.woxr-so:c:d; do\n  _slug=\"${_o%%:*}\"\n  tap net.ports ok\ndone\n",
                    &["net.ports"],
                    1,
                ),
                (
                    "same shape, but with the extraction inline in the argument of an emission for a *different* slug",
                    "# 4. toolchain\nfor _t in tool.cmake:cmake; do\n  chk ok rust.x64-target \"${_t#*:}\"\n  echo \"${_t%%:*}\"\ndone\n",
                    &["rust.x64-target"],
                    1,
                ),
                (
                    "the same line may both extract and emit",
                    "# 10. overlay\nfor _o in overlay.woxr-dll:a:b; do\n  _slug=\"${_o%%:*}\"; chk ok \"$_slug\" \"current\"\ndone\n",
                    &["overlay.woxr-dll"],
                    0,
                ),
                (
                    "a variable holding the value half `${_o#*:}` is not a slug carrier",
                    "# 10. overlay\nfor _o in overlay.woxr-dll:a:b; do\n  _pair=\"${_o#*:}\"\n  chk ok \"$_pair\" \"current\"\ndone\n",
                    &[],
                    1,
                ),
                (
                    "near miss: `$_slugx` is not the carrier variable `$_slug` — carriers match whole variable names",
                    "# 10. overlay\nfor _o in overlay.woxr-dll:a:b; do\n  _slug=\"${_o%%:*}\"\n  chk ok \"$_slugx\" \"current\"\ndone\n",
                    &[],
                    1,
                ),
            ];
            for (label, fixture, expected_order, expected_header_only) in cases {
                let scan = scan_doctor_slugs(fixture);
                let order: Vec<&str> = scan.order.iter().map(String::as_str).collect();
                assert_eq!(&order[..], *expected_order, "{label}");
                assert_eq!(
                    scan.header_only_loops.len(),
                    *expected_header_only,
                    "{label}: {:?}",
                    scan.header_only_loops
                );
            }
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

        /// All slugs on `# <tag> <slug> [<slug> …]` lines, in file order; a line
        /// naming several slugs expands to one entry per slug.
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
        /// Per gate, not one "is gating at all" bag: a bag cannot tell `warn` from
        /// `block` in either direction, so turning `game.version`'s `warn` into a
        /// `die` would leave the comparison unchanged.
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
        ///
        /// A group carrying **two different gates** cannot be judged from the
        /// bag of verbs its body contains — one `die` and one `warn` satisfy
        /// both claims however they are wired, so swapping them (making the
        /// legacy protocol fatal and an unsupported one a mere warning) reads
        /// as green (round-1 finding A1-6). Every slug in a mixed group is
        /// therefore anchored to the line it actually emits, via
        /// [`VERB_ANCHORS`].
        #[test]
        fn each_preflight_tag_group_uses_the_verb_its_gate_claims() {
            let root = repo_root();
            let text = std::fs::read_to_string(root.join("scripts/demo/run.sh"))
                .expect("scripts/demo/run.sh reads");
            let problems: Vec<String> = tag_groups(&text)
                .iter()
                .flat_map(group_verb_errors)
                .collect();
            assert!(problems.is_empty(), "{}", problems.join("\n"));
        }

        /// Per-slug message anchors: the pinned run.sh text a slug's branch
        /// emits. Required for every slug in a mixed-gate tag group, and
        /// enforced wherever else it is declared.
        const VERB_ANCHORS: [(&str, &str); 2] = [
            (
                "cfg.protocol.legacy-oxrsys",
                "protocol=oxrsys (legacy USB path)",
            ),
            ("cfg.protocol.supported", "is not valid for the demo"),
        ];

        fn anchor_for(slug: &str) -> Option<&'static str> {
            VERB_ANCHORS
                .iter()
                .find(|(s, _)| *s == slug)
                .map(|(_, needle)| *needle)
        }

        /// Everything wrong with one tag group's verbs — a list rather than
        /// assertions so the mutation test below can run it over a modified
        /// run.sh in memory.
        fn group_verb_errors(group: &TagGroup) -> Vec<String> {
            let die_re = Regex::new(r"\bdie\b").unwrap();
            let warn_re = Regex::new(r"\bwarn\b").unwrap();
            let mut out: Vec<String> = Vec::new();
            if group.gates.contains(&Gate::Autofix) {
                return out;
            }
            let has_die = die_re.is_match(&group.body);
            let has_warn = warn_re.is_match(&group.body);

            // Mixed group: the group-level verb bag proves nothing about which
            // slug got which verb, so bind each slug to its own emitting line.
            let mixed = group.gates.iter().any(|g| *g != group.gates[0]);
            for (slug, gate) in &group.slug_gates {
                let Some(needle) = anchor_for(slug) else {
                    if mixed {
                        out.push(format!(
                            "{slug} shares a mixed-gate tag group {:?} but has no VERB_ANCHORS \
                             entry — its gate cannot be told from the group's verbs; add the \
                             line it emits",
                            group.gates
                        ));
                    }
                    continue;
                };
                let Some(line) = group.body.lines().find(|l| l.contains(needle)) else {
                    out.push(format!(
                        "{slug}'s pinned message {needle:?} is gone from its run.sh block:\n{}",
                        group.body
                    ));
                    continue;
                };
                let (want, unwanted) = match gate {
                    Gate::Warn => (&warn_re, &die_re),
                    _ => (&die_re, &warn_re),
                };
                if !want.is_match(line) || unwanted.is_match(line) {
                    out.push(format!(
                        "{slug} is tagged {} but the line emitting its message uses the other \
                         verb:\n{line}",
                        gate.as_str()
                    ));
                }
            }

            if group.gates.contains(&Gate::Block) && !has_die {
                out.push(format!(
                    "block-tagged {:?} but its run.sh block never calls die:\n{}",
                    group.slugs, group.body
                ));
            }
            if group.gates.contains(&Gate::Warn) {
                if !has_warn {
                    out.push(format!(
                        "warn-tagged {:?} but its run.sh block never calls warn:\n{}",
                        group.slugs, group.body
                    ));
                }
                if !group.gates.contains(&Gate::Block) && has_die {
                    out.push(format!(
                        "warn-only {:?} but its run.sh block calls die — the contract says the \
                         launch continues:\n{}",
                        group.slugs, group.body
                    ));
                }
            }
            out
        }

        /// Swapping the two verbs of the mixed protocol group in memory — the
        /// change a group-level verb bag cannot see — must be reported once per
        /// half.
        #[test]
        fn swapping_the_verbs_of_a_mixed_tag_group_is_caught() {
            let root = repo_root();
            let text = std::fs::read_to_string(root.join("scripts/demo/run.sh"))
                .expect("scripts/demo/run.sh reads");
            let swapped = text
                .replace(
                    "oxrsys) warn \"protocol=oxrsys (legacy USB path)",
                    "oxrsys) die \"protocol=oxrsys (legacy USB path)",
                )
                .replace(
                    "*) die \"oxrsys-runtime.toml protocol=",
                    "*) warn \"oxrsys-runtime.toml protocol=",
                );
            assert_ne!(swapped, text, "the mutation no longer matches run.sh");

            let mixed: Vec<TagGroup> = tag_groups(&swapped)
                .into_iter()
                .filter(|g| g.slugs.iter().any(|s| s.starts_with("cfg.protocol.")))
                .collect();
            assert_eq!(mixed.len(), 1, "the protocol tag group moved");
            let problems = group_verb_errors(&mixed[0]);
            assert_eq!(
                problems.len(),
                2,
                "both halves of the verb swap must be reported, got: {problems:?}"
            );
        }

        struct TagGroup {
            slugs: Vec<String>,
            gates: Vec<Gate>,
            /// Per **slug** gate — `gates` is per tag *line*, and one line can
            /// name several slugs (`# preflight-autofix: build.helper-staged
            /// build.helper-arm64`), so the two are not index-parallel.
            slug_gates: Vec<(String, Gate)>,
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
                let mut slug_gates: Vec<(String, Gate)> = Vec::new();
                let mut cap = Some(cap);
                while let Some(c) = cap {
                    let gate = TAG_GATES
                        .iter()
                        .find(|(t, _)| *t == &c[1])
                        .map(|(_, g)| *g)
                        .expect("the regex only matches the three known tags");
                    gates.push(gate);
                    for slug in c[2].split_whitespace() {
                        slugs.push(slug.to_string());
                        slug_gates.push((slug.to_string(), gate));
                    }
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
                    slug_gates,
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

    /// Byte-exact rendering checks, each built from the template/contract file
    /// read fresh off disk rather than sabrage-core's compiled-in copy — plus
    /// the pure pins that live here per 3.5 (`win_path_table` and the
    /// write-signature shim), which read nothing. sabrage-core pins the
    /// compiled-in templates and their digests; this module is where a
    /// *rendered* artifact is compared against the on-disk bytes, so a stale
    /// `include_str!` shows up here as a byte diff on the artifact itself.
    mod artifact_goldens {
        use super::repo_root;
        use std::path::{Path, PathBuf};

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

        /// r1:A1-8 regression: the dylib path is JSON-escaped, so a path
        /// containing `"` or `\` decodes back to itself.
        ///
        /// Both front-ends escape before the `@OXR_DYLIB@` substitution
        /// (`util::json_escape_string` here, parameter expansion in install.sh); a
        /// raw replace writes an invalid or misdirected root-owned manifest.
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

        /// The bytes install layer 4 actually **writes**: the file form, which is
        /// `render_host_manifest` plus one trailing newline (install.sh's
        /// `print -- "$WANT"`), never the newline-less comparison form. The two
        /// are one byte apart on the most drift-sensitive artifact in the
        /// pipeline, and `write_host_manifest_privileged` takes the dylib path
        /// rather than pre-rendered content so the mistake is unexpressible (the
        /// compile-time tripwire below). Driving layer 4 end to end needs an async
        /// runtime this crate does not depend on; that half is sabrage-core's
        /// `stages::install::tests::layer_four_stages_the_host_manifest_file_form_byte_for_byte`.
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

            // The `drive_c` match is a string prefix ending in a slash, not
            // path-component containment (design-core §10 parity decision 22):
            // the bare `drive_c` directory falls through to Z:, and a sibling
            // whose name merely starts with "drive_c" is never inside it.
            let cases: &[(&str, Option<&Path>, PathBuf, &str)] = &[
                (
                    "inside drive_c -> C: with separators flipped",
                    Some(prefix),
                    prefix.join("drive_c/windows/system32/wineopenxr.dll"),
                    "C:\\windows\\system32\\wineopenxr.dll",
                ),
                (
                    "spaces and parentheses survive on the C: branch",
                    Some(prefix),
                    prefix.join(
                        "drive_c/Program Files (x86)/Steam/steamapps/common/Beat Saber 1294",
                    ),
                    "C:\\Program Files (x86)\\Steam\\steamapps\\common\\Beat Saber 1294",
                ),
                (
                    "immediate child of drive_c",
                    Some(prefix),
                    prefix.join("drive_c/openxr"),
                    "C:\\openxr",
                ),
                (
                    "the literal drive_c directory misses the trailing-slash glob",
                    Some(prefix),
                    prefix.join("drive_c"),
                    "Z:\\Users\\me\\Library\\Application Support\\CrossOver\\Bottles\\Steam\\drive_c",
                ),
                (
                    "a drive_cache sibling is not inside drive_c",
                    Some(prefix),
                    prefix.join("drive_cache/x"),
                    "Z:\\Users\\me\\Library\\Application Support\\CrossOver\\Bottles\\Steam\\drive_cache\\x",
                ),
                (
                    "outside the bottle -> Z: plus the whole path",
                    Some(prefix),
                    PathBuf::from("/games/Beat Saber 1294"),
                    "Z:\\games\\Beat Saber 1294",
                ),
                (
                    "no prefix at all -> Z:",
                    None,
                    PathBuf::from("/games/bs"),
                    "Z:\\games\\bs",
                ),
                (
                    "empty prefix behaves like no prefix (zsh `[ -n \"${PREFIX:-}\" ]`)",
                    Some(Path::new("")),
                    PathBuf::from("/games/bs"),
                    "Z:\\games\\bs",
                ),
            ];
            for (label, pre, input, want) in cases {
                assert_eq!(win_path(*pre, input), *want, "{label}");
            }
        }

        #[test]
        fn steam_appid_txt_content_has_no_trailing_newline() {
            // run.sh writes this file with `printf '%s' "$BS_APPID"`, so the
            // contract value must render with no trailing newline. The write
            // itself is covered by sabrage-core's `goldberg_stage` tests; this
            // pins the value and its shape, so a contract change that would alter
            // the on-disk bytes fails here first.
            let appid = sabrage_core::contract().game.appid.to_string();
            assert_eq!(appid, "620980");
            assert!(!appid.ends_with('\n'));
        }
    }

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

        /// Hermetic mirror of sabrage-core's registry invariants so CI (which runs
        /// only this crate + contract-gen on ubuntu) gates them: the strict
        /// registry must build, cover the contract in order, and leave **no** slug
        /// unbound — run-only preflights included, because `checks::run_only`
        /// binds a real evaluator for each even though they have no doctor row. A
        /// mis-registered evaluator would otherwise ship CI-green and panic at
        /// runtime in the CLI and app.
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

    mod launch_goldens {
        use sabrage_core::executor::PlannedKind;
        use sabrage_core::{contract, Bottle, Paths, StageCtx, StageOptions};
        use std::path::{Path, PathBuf};
        use std::sync::Arc;

        /// A dependency-free `block_on`.
        ///
        /// `sabrage-parity` carries no async runtime: every `Cargo.toml` entry is
        /// a dev-dependency and none is tokio. Every `DryRunExecutor` method that
        /// [`sabrage_core::stages::run::actions::goldberg_stage`] drives resolves
        /// on the first poll — none of them actually awaits — so a hand-rolled
        /// loop over [`std::task::Waker::noop`] drives it to completion with no
        /// dependency.
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

        /// run.sh's `printf '%s' "$BS_APPID" > "$APIDIR/steam_appid.txt"`
        /// (`# launch-action: goldberg-stage`): the appid digits, and nothing else
        /// — no trailing newline. Driven through the real `actions::goldberg_stage`
        /// under `--dry-run`, never a copy of its recipe, so a call-site regression
        /// turns this red: the plan's `Write` action records `"<n> bytes"`, and `n`
        /// must equal the appid string's own length.
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

        /// A synchronous [`Executor`] that really writes, into the test's own
        /// scratch tree.
        ///
        /// A dry-run plan carries no content — `write_atomic` records only
        /// `"<n> bytes"` — so the golden above can pin the payload's length but not
        /// its bytes. [`sabrage_core::executor::RealExecutor`] cannot be driven from
        /// this crate (`tokio::fs` primitives, no async runtime), so `std::fs`
        /// behind the same trait gives the real on-disk bytes; every primitive the
        /// run stage does not use panics rather than pretending to succeed.
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

        /// The same `printf '%s' "$BS_APPID"` write (run.sh, `# launch-action:
        /// goldberg-stage`), read back off disk: the appid digits and **nothing
        /// else**. The dry-run golden above sees only the payload's length; this
        /// one goes red for a six-byte impostor (`999999`, `62098\n`) as well.
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

        /// run.sh's five wine exports, table form (`# launch-action:
        /// launch-wine`). The load-bearing branch is `WINEDEBUG`: the caller's
        /// preset wins in **both** the verbose and non-verbose arms
        /// (`${WINEDEBUG:-…}`), and an inherited empty string is treated like
        /// unset (zsh's `:-`, not `-`).
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

            assert_eq!(
                wine_env(false, None, appid, runtime_json),
                vec![
                    (
                        "XR_RUNTIME_JSON".to_string(),
                        "/repo/ext/oxrsys/build-x64/runtime/oxrsys-runtime.json".to_string()
                    ),
                    ("CX_GRAPHICS_BACKEND".to_string(), "dxmt".to_string()),
                    ("WINEDEBUG".to_string(), "-all".to_string()),
                    ("SteamAppId".to_string(), "620980".to_string()),
                    ("SteamGameId".to_string(), "620980".to_string()),
                ],
                "wine_env's quiet form is exactly run.sh's five exports, in this order"
            );

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

        /// `date +%Y%m%d-%H%M%S` for attempt 0; Sabrage's own `-{n+1}` suffix on a
        /// collision — a declared divergence (PARITY.md § Run (launch), "The wine
        /// console log is a plain file").
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
            let p3 = sabrage_core::logs::wine_log_candidate(logs_dir, now, 3);
            assert_eq!(p3, logs_dir.join("beatsaber-20260829-101112-4.log"));
        }
    }

    /// `stages::run` reproduces a long list of run.sh's `die`/`warn`/`info`/
    /// banner strings verbatim, scattered as `&str` literals across
    /// `preflight.rs`, `actions.rs`, `guards.rs` and `mod.rs` — most of them
    /// not contract-derived, so nothing above catches one going stale.
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
    /// `mod::tests::the_closing_lines_are_run_shs_verbatim`, …): tier 1
    /// selects `sabrage-parity` + `sabrage-contract-gen` only
    /// (`scripts/dev/parity.sh`, `.github/workflows/parity.yml`), and Cargo
    /// does not run a dev-dependency's `#[cfg(test)]` harness. Those tests
    /// exist but nothing runs them in the gate, and CI (ubuntu) cannot add
    /// `-p sabrage-core` because much of that suite is macOS-shaped.
    ///
    /// So the native half is pinned **here**, by calling the native renderer
    /// wherever it is `pub`. A1-3 made `pub` every function this module needs
    /// that was previously `pub(crate)` (`stages::run::actions::banner_events`
    /// / `bs_win_path`, `stages::run::mod::wine_exit_line` /
    /// `INT_TEARDOWN_LINE` / `HELPER_REAPED_LINE`, `stages::run::guards`'
    /// audio/dashboard line constants and functions,
    /// `stages::run::preflight::block_die` / `post_fix_die`), plus one new
    /// fixture constructor, [`sabrage_core::stages::StageCtx::for_fixture`],
    /// so this crate never has to depend on `tokio_util` just to build a
    /// `StageCtx`. `checks::run_only`'s die text is covered by
    /// [`native_run_only_die_text_is_verbatim_in_run_sh`], which calls that
    /// module's evaluators for real and pins the `--wired`-with-no-adb die
    /// by exact equality. What remains substring-only:
    /// `stages::run::actions::wineserver_reset` / `goldberg_stage` /
    /// `adb_reverse_cleanup` / `adb_forward_hygiene` and
    /// `fixes::adb::remove_adb_forwards_at`'s text, and
    /// `stages::run::preflight::emit_encoder_notice`'s two lines and the
    /// `game.version` warn row — those functions are not `pub` and are out of
    /// this pass's scope; their fragments are still copied from source below.
    mod run_sh_text_parity {
        use super::repo_root;
        use sabrage_core::paths::{Bottle, Paths};
        use sabrage_core::stages::{StageCtx, StageOptions};
        use std::path::Path;

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

        /// A [`StageCtx`] fixture over a fresh scratch root — never the real
        /// machine ([`StageCtx::for_fixture`] always picks a `DryRunExecutor`).
        fn ctx(tag: &str) -> StageCtx {
            let scratch =
                std::env::temp_dir().join(format!("sabrage-parity-{tag}-{}", std::process::id()));
            std::fs::remove_dir_all(&scratch).ok();
            std::fs::create_dir_all(&scratch).unwrap();
            StageCtx::for_fixture(Paths::new(&scratch), StageOptions::default())
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
            assert_eq!(
                wired.message,
                "--wired needs adb (Android platform-tools) on PATH or under ~/Library/Android/sdk"
            );
            assert_verbatim(&text, &wired.message, "checks::run_only::run_wired_adb");

            std::fs::remove_dir_all(&scratch).ok();
        }

        #[test]
        fn preflight_die_and_warn_text_is_verbatim_in_run_sh() {
            let text = run_sh();

            // `block_die` and `post_fix_die` are called for real. The fixture
            // outcome's message is what `block_die`'s run-only arm (and its
            // `_ =>` fallback) echo, so it doubles as the expected text for the
            // three run-only slugs below.
            use sabrage_core::checks::CheckOutcome;
            use sabrage_core::stages::run::preflight::{block_die, post_fix_die};
            let mut c = ctx("preflight-die");
            // `block_die`'s install remedy interpolates `opts.bottle_name`, and
            // `post_fix_die`'s `bottle.gfx-dxmt` arm the bottle's cxbottle.conf
            // path. With neither set the first renders doctor's `<name>`
            // placeholder and the second an empty path, and the equalities
            // below would pin those instead of the interpolations.
            c.opts.bottle_name = Some("FixtureBottle".to_string());
            c.bottle = Some(Bottle::unvalidated("FixtureBottle"));
            let outcome = CheckOutcome::fail_bare("x", "impl message");

            // Shell-backed rows: run.sh carries the sentence and `block_die`
            // renders it whole. Where run.sh interpolates `$WINEVR_BOTTLE` into
            // the tail, the fragment matched against the file stops where the
            // interpolation starts.
            for (slug, fragment, rendered) in [
                (
                    "dep.goldberg",
                    "Goldberg dll missing — ./demo.sh setup",
                    "Goldberg dll missing — ./demo.sh setup",
                ),
                (
                    "overlay.dxmt-d3d11",
                    "CrossOver DXMT overlay stale (CrossOver update?)",
                    "CrossOver DXMT overlay stale (CrossOver update?) — ./demo.sh install --bottle FixtureBottle",
                ),
                (
                    "bottle.woxr-dll",
                    "bottle wineopenxr.dll stale/missing",
                    "bottle wineopenxr.dll stale/missing — ./demo.sh install --bottle FixtureBottle",
                ),
                (
                    "bottle.manifest",
                    "bottle OpenXR manifest missing",
                    "bottle OpenXR manifest missing — ./demo.sh install --bottle FixtureBottle",
                ),
                (
                    "bottle.registry",
                    "bottle ActiveRuntime registry key missing",
                    "bottle ActiveRuntime registry key missing — ./demo.sh install --bottle FixtureBottle",
                ),
                (
                    "host.manifest",
                    "host OpenXR registration missing",
                    "host OpenXR registration missing — ./demo.sh install --bottle FixtureBottle",
                ),
            ] {
                assert_verbatim(&text, fragment, "stages::run::preflight::block_die");
                assert_eq!(block_die(&c, slug, &outcome).0, rendered, "{slug}");
            }

            // Native-only rows: run.sh `cmp`s only `d3d11.dll`, so neither slug
            // has a shell counterpart (the divergence is contract data).
            // `overlay.dxmt-winemetal` shares the d3d11 arm's sentence, already
            // matched against run.sh above; only `overlay.woxr-dll`'s sentence
            // exists on this side alone.
            for (slug, rendered) in [
                (
                    "overlay.dxmt-winemetal",
                    "CrossOver DXMT overlay stale (CrossOver update?) — ./demo.sh install --bottle FixtureBottle",
                ),
                (
                    "overlay.woxr-dll",
                    "CrossOver wineopenxr overlay stale (CrossOver update?) — ./demo.sh install --bottle FixtureBottle",
                ),
            ] {
                assert_eq!(block_die(&c, slug, &outcome).0, rendered, "{slug}");
            }

            // The three run-only slugs have no doctor row, so `block_die`
            // passes the check's own message through untouched.
            for slug in ["run.wine-exec", "run.bridge-built", "run.wired-adb"] {
                assert_eq!(block_die(&c, slug, &outcome).0, "impl message", "{slug}");
            }

            assert_eq!(
                post_fix_die(&c, "bottle.gfx-dxmt").0,
                format!(
                    "could not force graphics backend to dxmt in {}",
                    c.bottle.as_ref().unwrap().conf_path().display()
                ),
                "bottle.gfx-dxmt"
            );
            assert_verbatim(
                &text,
                "could not force graphics backend to dxmt in",
                "stages::run::preflight::post_fix_die",
            );
            assert_eq!(
                post_fix_die(&c, "build.helper-staged").0,
                format!(
                    "encoder helper restage failed validation at {} — ./demo.sh build",
                    c.paths.oxr_helper_staged.display()
                ),
                "build.helper-staged"
            );
            assert_verbatim(
                &text,
                "encoder helper restage failed validation at",
                "stages::run::preflight::post_fix_die",
            );

            for (fragment, site) in [
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
            ] {
                assert_verbatim(&text, fragment, site);
            }
            // The real constant, not a copy — anchored as the whole `print`
            // argument so a truncated constant cannot pass as a substring.
            assert_verbatim(
                &text,
                &format!(
                    "print \"{}\"",
                    sabrage_core::stages::run::HELPER_REAPED_LINE
                ),
                "stages::run::HELPER_REAPED_LINE",
            );
        }

        #[test]
        fn audio_and_dashboard_guard_text_is_verbatim_in_run_sh() {
            use sabrage_core::stages::run::guards::{
                audio_switched_line, blackhole_not_present_line, blackhole_switch_failed_line,
                AUDIO_DISABLED_LINE, DASHBOARD_CLOSED_LINE, DASHBOARD_DISABLED_LINE,
                DASHBOARD_NOT_BUILT_LINE, DASHBOARD_OPENING_LINE,
            };

            let text = run_sh();
            // Every fragment here is the real constant or the real render, not
            // a copy — `site` names the function that owns it.
            for (fragment, site) in [
                (
                    AUDIO_DISABLED_LINE.to_string(),
                    "stages::run::guards::AUDIO_DISABLED_LINE",
                ),
                (
                    blackhole_switch_failed_line(),
                    "stages::run::guards::blackhole_switch_failed_line",
                ),
                (
                    blackhole_not_present_line(),
                    "stages::run::guards::blackhole_not_present_line",
                ),
                (
                    DASHBOARD_DISABLED_LINE.to_string(),
                    "stages::run::guards::DASHBOARD_DISABLED_LINE",
                ),
                (
                    DASHBOARD_OPENING_LINE.to_string(),
                    "stages::run::guards::DASHBOARD_OPENING_LINE",
                ),
                (
                    DASHBOARD_NOT_BUILT_LINE.to_string(),
                    "stages::run::guards::DASHBOARD_NOT_BUILT_LINE",
                ),
                (
                    DASHBOARD_CLOSED_LINE.to_string(),
                    "stages::run::guards::DASHBOARD_CLOSED_LINE",
                ),
            ] {
                assert_verbatim(&text, &fragment, site);
            }
            // `audio_switched_line` interpolates the previous device, so only
            // its static prefix can be a substring of run.sh's `<dev>`-free
            // template.
            let rendered = audio_switched_line("MacBook Pro Speakers");
            assert!(
                rendered.starts_with("audio: default output -> BlackHole 2ch (was: "),
                "{rendered}"
            );
            assert_verbatim(
                &text,
                "audio: default output -> BlackHole 2ch (was:",
                "stages::run::guards::audio_switched_line",
            );
        }

        /// run.sh's nine-line launch banner (`# launch-action: launch-wine`) —
        /// every line, in order, including the two blank lines that frame it.
        #[test]
        fn the_launch_banner_lines_are_verbatim_in_run_sh() {
            use sabrage_core::events::StageEvent;
            use sabrage_core::stages::run::actions::{banner_events, bs_win_path};

            let text = run_sh();
            let c = ctx("banner");
            let bottle = Bottle::unvalidated("Steam");
            let bs_win = bs_win_path(&c, &bottle);
            let log = Path::new("/repo/logs/beatsaber-20260829-101112.log");
            let events = banner_events(c.run_id, &bottle.name, &bs_win, log);

            let mut rendered: Vec<String> = Vec::new();
            for ev in &events {
                match ev {
                    StageEvent::Section { title, .. } => rendered.push(format!("-- {title}")),
                    StageEvent::Text { text, .. } => rendered.push(text.clone()),
                    _ => {}
                }
            }
            assert_eq!(
                rendered,
                vec![
                    "".to_string(),
                    "-- launching Beat Saber through the bridge".to_string(),
                    "   put the headset ON and open the ALVR client; first frame can take ~30s."
                        .to_string(),
                    "   pause in-game = X/A button or the Quest system button".to_string(),
                    "   (the left-menu-button pause is a Beat Saber/Unity limitation on every OpenXR runtime)"
                        .to_string(),
                    "   stop: Ctrl-C here, or ./demo.sh stop --bottle Steam from another shell"
                        .to_string(),
                    format!("   exe: {bs_win}"),
                    format!("   log: {}", log.display()),
                    "".to_string(),
                ],
                "banner_events must emit run.sh's nine lines in order, blank line first and last"
            );
            // The four fully static lines — no interpolation, so the real
            // rendered string must appear in run.sh verbatim.
            for fragment in [
                "-- launching Beat Saber through the bridge",
                "   put the headset ON and open the ALVR client; first frame can take ~30s.",
                "   pause in-game = X/A button or the Quest system button",
                "   (the left-menu-button pause is a Beat Saber/Unity limitation on every OpenXR runtime)",
            ] {
                assert_verbatim(&text, fragment, "stages::run::actions::banner_events");
            }
            // The three interpolated lines are pinned in full by the equality
            // above; only their static fragments can be matched against run.sh.
            assert_verbatim(
                &text,
                "   stop: Ctrl-C here, or ./demo.sh stop --bottle",
                "stages::run::actions::banner_events",
            );
            assert_verbatim(
                &text,
                "from another shell",
                "stages::run::actions::banner_events",
            );
            assert_verbatim(&text, "   exe: ", "stages::run::actions::banner_events");
            assert_verbatim(&text, "   log: ", "stages::run::actions::banner_events");

            // `wine_exit_line` and `INT_TEARDOWN_LINE`, called/read for real.
            use sabrage_core::stages::run::{wine_exit_line, INT_TEARDOWN_LINE};
            let exit_line = wine_exit_line(0, log);
            assert!(
                exit_line.starts_with("wine exited with status 0 (log: ")
                    && exit_line.ends_with(')'),
                "{exit_line}"
            );
            assert_verbatim(
                &text,
                "wine exited with status",
                "stages::run::wine_exit_line",
            );
            assert_verbatim(&text, "(log: ", "stages::run::wine_exit_line");
            assert_verbatim(&text, INT_TEARDOWN_LINE, "stages::run::INT_TEARDOWN_LINE");
        }
    }

    /// `tap` and `chk` in scripts/demo/lib.sh return 0 with the tap disabled:
    /// lib.sh is sourced by `set -e` stages, and a bare `[ -n … ] && …` tail
    /// would end the stage whenever `WINEVR_DOCTOR_TAP` is unset.
    mod lib_sh_contract {
        use super::repo_root;

        const SCRIPT: &str = "set -e; source \"$WINEVR_ROOT/scripts/demo/lib.sh\"; tap demo.slug ok; chk ok demo.slug \"msg\"; exit 0";

        #[test]
        fn lib_sh_tap_and_chk_return_zero_when_tap_disabled() {
            let root = repo_root();
            let home = std::env::temp_dir()
                .join(format!("sabrage-parity-lib-sh-home-{}", std::process::id()));
            std::fs::remove_dir_all(&home).ok();
            std::fs::create_dir_all(&home).unwrap();
            let out = std::process::Command::new("zsh")
                .args(["-c", SCRIPT])
                .env("WINEVR_ROOT", &root)
                .env("HOME", &home)
                .env_remove("WINEVR_DOCTOR_TAP")
                .output()
                .expect("zsh runs");
            std::fs::remove_dir_all(&home).ok();
            assert!(
                out.status.success(),
                "tap/chk with the tap off must not end a set -e stage: status {:?}, stderr {}",
                out.status,
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }

    /// Both front-ends embed the repo root as an absolute string inside the
    /// root-owned host manifest and compare those bytes **literally**
    /// (`install.sh`'s `[ "$(cat "$HOST_XR_JSON")" = "$WANT" ]`,
    /// `util::host_manifest_is_current` here), so two spellings of one checkout
    /// mean two manifests and a sudo prompt per alternation (r1:A2-6).
    ///
    /// The shared contract is zsh's **logical** `pwd`: absolute, `.`/`..` folded
    /// textually, symlinks preserved as the user spelled them — demo.sh's
    /// `ROOT="$(cd "$(dirname "$0")" && pwd)"` and `paths::resolve_repo_root`'s
    /// `logical_absolute`, deliberately not `canonicalize`. This module pins both
    /// halves.
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

        /// r1:A2-6 regression: the resolver returns the symlink spelling, and
        /// `Paths` derives the host-manifest dylib path from that spelling
        /// rather than the physical target — the thrash this module documents.
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
            assert_ne!(
                spelled, physical,
                "the physical target of the symlink is explicitly not what comes back"
            );
            let manifest_dylib = sabrage_core::Paths::new(&spelled).oxr_dylib;
            assert!(
                manifest_dylib.starts_with(&link),
                "the dylib path the host manifest embeds is derived from the symlinked spelling"
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

    // ── (11) PARITY.md section citations ─────────────────────────────────────

    /// A section citation (`PARITY.md`, a section sign, a heading) names a real
    /// `## ` heading of sabrage/PARITY.md, and the words it quotes appear
    /// verbatim in that section.
    mod parity_md_citations {
        use super::repo_root;
        use std::path::{Path, PathBuf};

        /// Markdown emphasis dropped and whitespace collapsed, so a citation may
        /// quote `the wired forwards row` for a row that renders `the **wired**
        /// forwards row` across two wrapped lines.
        fn normalize(text: &str) -> String {
            text.replace('*', " ")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        }

        /// PARITY.md as `(heading title, section body)` pairs, split on `## `.
        fn sections(markdown: &str) -> Vec<(String, String)> {
            let mut out: Vec<(String, String)> = Vec::new();
            for line in markdown.lines() {
                match line.strip_prefix("## ") {
                    Some(title) => out.push((title.trim().to_string(), String::new())),
                    None => {
                        if let Some(last) = out.last_mut() {
                            last.1.push_str(line);
                            last.1.push('\n');
                        }
                    }
                }
            }
            out
        }

        /// One error string per citation in `flat_text` that names a heading
        /// PARITY.md does not have, follows a heading with words that are not
        /// a `, "<quote>"`, leaves that quote unterminated, or quotes words
        /// the section does not contain.
        fn check_citations(flat_text: &str, headings: &[(String, String)]) -> Vec<String> {
            const MARKER: &str = "PARITY.md \u{a7} ";
            const WINDOW: usize = 400;
            // A heading ends at punctuation or the end of the text, never
            // inside a run of words: `Setup / install` is no citation of `Setup`.
            const BOUNDARY: &[char] = &[',', '.', ';', ':', ')', ']'];
            let excerpt = |s: &str| s.chars().take(80).collect::<String>();
            let mut errors = Vec::new();
            let mut rest = flat_text;
            while let Some(at) = rest.find(MARKER) {
                let tail = &rest[at + MARKER.len()..];
                rest = tail;
                let mut cite: String = tail.chars().take(WINDOW).collect();
                if let Some(next) = cite.find(MARKER) {
                    cite.truncate(next);
                }
                let cite = cite.trim_end();
                // Longest title first, so `Run (launch)` wins over `Run` when both exist.
                let mut candidates: Vec<&(String, String)> = headings.iter().collect();
                candidates.sort_by_key(|(title, _)| std::cmp::Reverse(title.len()));
                let matched = candidates.into_iter().find(|(title, _)| {
                    cite.strip_prefix(title.as_str())
                        .is_some_and(|after| after.is_empty() || after.starts_with(BOUNDARY))
                });
                let Some((title, body)) = matched else {
                    errors.push(format!(
                        "no such PARITY.md heading in citation: {:?}",
                        excerpt(cite)
                    ));
                    continue;
                };
                let after = &cite[title.len()..];
                if let Some(quote) = after.strip_prefix(", \"") {
                    match quote.split_once('"') {
                        Some((words, _)) => {
                            if !normalize(body).contains(&normalize(words)) {
                                errors.push(format!(
                                    "PARITY.md \u{a7} {title} does not contain the quoted words {words:?}"
                                ));
                            }
                        }
                        None => errors.push(format!(
                            "unterminated quote in citation of PARITY.md \u{a7} {title}: {:?}",
                            excerpt(after)
                        )),
                    }
                } else if after.starts_with([',', ':']) && after[1..].trim_start().starts_with('"')
                {
                    errors.push(format!(
                        "malformed quote after PARITY.md \u{a7} {title} (expected `, \"...\"`): {:?}",
                        excerpt(after)
                    ));
                }
            }
            errors
        }

        /// Every line stripped of its leading comment marker and joined with a
        /// single space, so a citation that wraps across comment lines is still
        /// one run of text.
        fn flatten(source: &str) -> String {
            source
                .lines()
                .map(|line| {
                    let line = line.trim_start();
                    for marker in ["///", "//!", "//", "#", "*"] {
                        if let Some(body) = line.strip_prefix(marker) {
                            return body.trim();
                        }
                    }
                    line.trim()
                })
                .collect::<Vec<_>>()
                .join(" ")
        }

        fn collect(dir: &Path, extensions: &[&str], out: &mut Vec<PathBuf>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name();
                if path.is_dir() {
                    // Build output and the shell fixtures under tests/ are not
                    // sources that cite PARITY.md.
                    if name != "target" && name != "fixtures" {
                        collect(&path, extensions, out);
                    }
                } else if path
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| extensions.contains(&e))
                {
                    out.push(path);
                }
            }
        }

        fn cited_files(root: &Path) -> Vec<PathBuf> {
            let mut files = Vec::new();
            collect(&root.join("sabrage/crates"), &["rs"], &mut files);
            collect(&root.join("sabrage/src-tauri"), &["rs"], &mut files);
            collect(&root.join("sabrage/ui/src"), &["ts", "svelte"], &mut files);
            collect(&root.join("scripts/demo"), &["sh"], &mut files);
            files.push(root.join("demo.sh"));
            files.sort();
            files
        }

        /// Vacuous in a tree with no section citations; the table test below
        /// proves the checker on literals.
        #[test]
        fn parity_md_section_citations_name_real_headings() {
            let root = repo_root();
            let markdown =
                std::fs::read_to_string(root.join("sabrage/PARITY.md")).expect("PARITY.md reads");
            let headings = sections(&markdown);
            let mut errors: Vec<String> = Vec::new();
            for file in cited_files(&root) {
                let Ok(source) = std::fs::read_to_string(&file) else {
                    continue;
                };
                for error in check_citations(&flatten(&source), &headings) {
                    errors.push(format!("{}: {error}", file.display()));
                }
            }
            assert!(
                errors.is_empty(),
                "PARITY.md citations that no longer resolve:\n{}",
                errors.join("\n")
            );
        }

        #[test]
        fn citation_checker_accepts_verbatim_quotes_and_rejects_mistyped_or_malformed_ones() {
            let headings = vec![
                (
                    "Doctor / checks".to_string(),
                    "| Console colors gated on isatty | zsh bakes ANSI constants into every row |"
                        .to_string(),
                ),
                (
                    "Run preflight".to_string(),
                    "| The **wired** forwards row | removed in preflight |".to_string(),
                ),
            ];
            // The section sign is spelled `\u{a7}` in every literal below so
            // that the tree scan above does not read these fixtures as real
            // citations in this file. The third column is the error each
            // citation must produce, in order; empty means accepted.
            let cases: &[(&str, &str, &[&str])] = &[
                (
                    "exact heading with an exact quote",
                    "PARITY.md \u{a7} Doctor / checks, \"Console colors gated on isatty\"",
                    &[],
                ),
                (
                    "quote omitting the row's bold asterisks",
                    "PARITY.md \u{a7} Run preflight, \"The wired forwards row\"",
                    &[],
                ),
                (
                    "bare heading closing a sentence",
                    "see PARITY.md \u{a7} Run preflight. The next sentence quotes \"nothing\".",
                    &[],
                ),
                (
                    "mistyped heading",
                    "PARITY.md \u{a7} Doctor / cheks, \"Console colors gated on isatty\"",
                    &["no such PARITY.md heading"],
                ),
                (
                    "real heading with trailing words",
                    "PARITY.md \u{a7} Doctor / checks and more, \"Console colors gated on isatty\"",
                    &["no such PARITY.md heading"],
                ),
                (
                    "paraphrased quote",
                    "PARITY.md \u{a7} Doctor / checks, \"ANSI colors are gated on a tty\"",
                    &["does not contain the quoted words"],
                ),
                (
                    "colon instead of the comma",
                    "PARITY.md \u{a7} Doctor / checks: \"Console colors gated on isatty\"",
                    &["malformed quote"],
                ),
                (
                    "unterminated quote",
                    "PARITY.md \u{a7} Doctor / checks, \"Console colors gated on isatty",
                    &["unterminated quote"],
                ),
                (
                    "quote longer than 120 characters that the section lacks",
                    "PARITY.md \u{a7} Run preflight, \"the encoder helper is restaged from build-helper-arm64 before every launch and the stale forward pair on ports 9943 and 9944 is removed in preflight\"",
                    &["does not contain the quoted words"],
                ),
            ];
            for (label, citation, expected) in cases {
                let errors = check_citations(citation, &headings);
                assert_eq!(
                    errors.len(),
                    expected.len(),
                    "{label}: {citation} -> {errors:?}"
                );
                for (error, want) in errors.iter().zip(expected.iter()) {
                    assert!(error.contains(want), "{label}: {error:?} lacks {want:?}");
                }
            }
        }
    }
}
