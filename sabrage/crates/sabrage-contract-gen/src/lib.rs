//! Generator for `scripts/demo/contract.gen.sh` — the zsh mirror of
//! `contract/pipeline.toml`.
//!
//! # Why a generator at all
//!
//! lib.sh must keep its runtime surface as dumb as it is today: no jq, no
//! hand-rolled TOML awk, no `set -u` ordering hazards. So the scalars the shell
//! needs are emitted into a whole generated file, committed, and `source`d. A
//! whole file (rather than BEGIN/END markers inside lib.sh) leaves no ambiguity
//! about what is hand-editable, and hand edits are caught by `--check`.
//!
//! # The two tripwires
//!
//! * **zsh side, no Rust required**: the `# contract-sha256:` header records the
//!   digest of the three `contract/` files. doctor's `meta.contract-sync` check
//!   recomputes it with `shasum` and FAILs on a mismatch, catching "edited the
//!   contract, forgot to regenerate".
//! * **Rust side**: [`check`] diffs a fresh generation against the committed
//!   file, catching the inverse — a hand-edited generated file.
//!
//! # Deliberately not depending on `sabrage-core`
//!
//! The generator re-declares the small subset of contract fields the shell
//! actually consumes. That keeps it honest: if a field is added to
//! `pipeline.toml` and to `sabrage-core` but not here, the shell simply does not
//! get it, which is a visible, intended outcome — not a silent coupling through
//! a shared struct. serde ignores unknown fields, so extra contract sections
//! cost nothing.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

/// Repo-relative path of the generated file.
pub const OUTPUT_REL_PATH: &str = "scripts/demo/contract.gen.sh";

/// Repo-relative paths of the three contract inputs, in the order the
/// `contract-sha256` recipe concatenates them (doctor.sh section 0:
/// `cat pipeline.toml oxrsys-runtime.toml.template active_runtime.x86_64.json.template`).
pub const CONTRACT_FILES: [&str; 3] = [
    "contract/pipeline.toml",
    "contract/oxrsys-runtime.toml.template",
    "contract/active_runtime.x86_64.json.template",
];

/// `contract/pipeline.toml`, compiled in.
///
/// Path depth: this file is `sabrage/crates/sabrage-contract-gen/src/lib.rs`, and
/// `include_str!` resolves relative to it, so the repo root is four levels up.
pub const PIPELINE_TOML: &str = include_str!("../../../../contract/pipeline.toml");
/// `contract/oxrsys-runtime.toml.template`, compiled in (hash input only).
pub const RUNTIME_TOML_TEMPLATE: &str =
    include_str!("../../../../contract/oxrsys-runtime.toml.template");
/// `contract/active_runtime.x86_64.json.template`, compiled in (hash input only).
pub const HOST_MANIFEST_TEMPLATE: &str =
    include_str!("../../../../contract/active_runtime.x86_64.json.template");

// ── contract subset ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct Contract {
    deps: Deps,
    game: Game,
    paths: Paths,
    ports: Ports,
    dxmt: Dxmt,
}

#[derive(Debug, Deserialize)]
struct Deps {
    url: String,
    gbe_dll_sha256: String,
    dxmt_tgz_sha256: String,
}

#[derive(Debug, Deserialize)]
struct Game {
    appid: u64,
    depot: u64,
    manifest: String,
    #[allow(dead_code)]
    bs_dir_leaf: String,
}

#[derive(Debug, Deserialize)]
struct Paths {
    host_xr_json: String,
}

#[derive(Debug, Deserialize)]
struct Ports {
    stream: Vec<u16>,
    legacy_reverse: Vec<u16>,
}

#[derive(Debug, Deserialize)]
struct Dxmt {
    files: Vec<String>,
}

// ── errors ────────────────────────────────────────────────────────────────────

/// Everything that can go wrong generating or checking the file.
#[derive(Debug)]
pub enum GenError {
    /// `contract/pipeline.toml` did not parse.
    Parse(toml::de::Error),
    /// A contract file or the committed output could not be read/written.
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl fmt::Display for GenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GenError::Parse(e) => write!(f, "contract/pipeline.toml: {e}"),
            GenError::Io { path, source } => write!(f, "{}: {source}", path.display()),
        }
    }
}

impl std::error::Error for GenError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            GenError::Parse(e) => Some(e),
            GenError::Io { source, .. } => Some(source),
        }
    }
}

fn read(path: &Path) -> Result<String, GenError> {
    std::fs::read_to_string(path).map_err(|source| GenError::Io {
        path: path.to_path_buf(),
        source,
    })
}

// ── generation ────────────────────────────────────────────────────────────────

/// The `contract-sha256` header value: sha256 over the three contract files'
/// bytes, concatenated in [`CONTRACT_FILES`] order.
///
/// Identical recipe to doctor.sh section 0 and to `sabrage_core::util::contract_hash`.
pub fn contract_sha256_from(pipeline: &str, runtime_tmpl: &str, host_tmpl: &str) -> String {
    let mut h = Sha256::new();
    h.update(pipeline.as_bytes());
    h.update(runtime_tmpl.as_bytes());
    h.update(host_tmpl.as_bytes());
    hex::encode(h.finalize())
}

/// The `contract-sha256` of the compiled-in contract.
pub fn contract_sha256() -> String {
    contract_sha256_from(PIPELINE_TOML, RUNTIME_TOML_TEMPLATE, HOST_MANIFEST_TEMPLATE)
}

/// Generate `contract.gen.sh` from explicit contract bytes.
///
/// The prose, section banners, and the `# 9947 deliberately absent` note are
/// fixed literals; only the values are substituted. That is on purpose — the
/// generated file is meant to read like a hand-written shell fragment, and its
/// layout is part of what `--check` pins.
pub fn generate_from(
    pipeline: &str,
    runtime_tmpl: &str,
    host_tmpl: &str,
) -> Result<String, GenError> {
    let c: Contract = toml::from_str(pipeline).map_err(GenError::Parse)?;
    let hash = contract_sha256_from(pipeline, runtime_tmpl, host_tmpl);

    let dxmt_files = c.dxmt.files.join(" ");
    let wired_ports = join_nums(&c.ports.stream);
    let legacy_ports = join_nums(&c.ports.legacy_reverse);

    Ok(format!(
        "\
# GENERATED from contract/ — DO NOT EDIT. Regenerate: scripts/dev/parity.sh --regen
# contract-sha256: {hash}
# Shared scalar contract between demo.sh (this file is sourced by lib.sh) and
# sabrage-core (which parses contract/pipeline.toml directly). Values here MUST
# match contract/pipeline.toml — doctor's meta.contract-sync check verifies the
# header hash above against a live recompute of the contract/ files.

# ---- pinned dependency sources -----------------------------------------------
DEPS_URL=\"{deps_url}\"
DXMT_TGZ_SHA256=\"{dxmt_sha}\"
GBE_DLL_SHA256=\"{gbe_sha}\"

# ---- Beat Saber depot pin ------------------------------------------------------
BS_APPID={appid}
BS_DEPOT={depot}
BS_MANIFEST={manifest}

# ---- host OpenXR loader registration -------------------------------------------
HOST_XR_JSON=\"{host_xr_json}\"

# ---- DXMT artifact set (presence gates key on ALL of these) --------------------
DXMT_FILES=({dxmt_files})

# ---- streaming ports -----------------------------------------------------------
WIRED_PORTS=({wired_ports})
LEGACY_REVERSE_PORTS=({legacy_ports})   # 9947 deliberately absent
",
        hash = hash,
        deps_url = c.deps.url,
        dxmt_sha = c.deps.dxmt_tgz_sha256,
        gbe_sha = c.deps.gbe_dll_sha256,
        appid = c.game.appid,
        depot = c.game.depot,
        manifest = c.game.manifest,
        host_xr_json = c.paths.host_xr_json,
        dxmt_files = dxmt_files,
        wired_ports = wired_ports,
        legacy_ports = legacy_ports,
    ))
}

fn join_nums(v: &[u16]) -> String {
    v.iter()
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Generate from the compiled-in contract.
///
/// Infallible by construction: the contract is validated when this crate is
/// built, and `include_str!` makes any edit to it trigger a rebuild.
pub fn generate() -> String {
    generate_from(PIPELINE_TOML, RUNTIME_TOML_TEMPLATE, HOST_MANIFEST_TEMPLATE)
        .expect("compiled-in contract/pipeline.toml is valid")
}

/// Generate from the contract files **on disk** under `repo_root`.
///
/// This is what `--write`/`--check` use: the point of the tool is to compare a
/// working checkout against its own generated file, so a compiled-in copy would
/// defeat it.
pub fn generate_from_repo(repo_root: &Path) -> Result<String, GenError> {
    let pipeline = read(&repo_root.join(CONTRACT_FILES[0]))?;
    let runtime_tmpl = read(&repo_root.join(CONTRACT_FILES[1]))?;
    let host_tmpl = read(&repo_root.join(CONTRACT_FILES[2]))?;
    generate_from(&pipeline, &runtime_tmpl, &host_tmpl)
}

/// Absolute path of the committed generated file under `repo_root`.
pub fn output_path(repo_root: &Path) -> PathBuf {
    repo_root.join(OUTPUT_REL_PATH)
}

/// Outcome of a `--check` run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckReport {
    /// True when the committed file already matches a fresh generation.
    pub in_sync: bool,
    /// What generation produces.
    pub generated: String,
    /// What is committed (empty when the file is missing).
    pub committed: String,
}

/// Diff a fresh generation against the committed file. Never writes.
pub fn check(repo_root: &Path) -> Result<CheckReport, GenError> {
    let generated = generate_from_repo(repo_root)?;
    let path = output_path(repo_root);
    let committed = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(source) => return Err(GenError::Io { path, source }),
    };
    Ok(CheckReport {
        in_sync: generated == committed,
        generated,
        committed,
    })
}

/// Write the generated file. Returns `true` when the bytes changed.
pub fn write(repo_root: &Path) -> Result<bool, GenError> {
    let report = check(repo_root)?;
    if report.in_sync {
        return Ok(false);
    }
    let path = output_path(repo_root);
    std::fs::write(&path, report.generated.as_bytes())
        .map_err(|source| GenError::Io { path, source })?;
    Ok(true)
}

/// The repo root this crate was compiled from: three levels above the crate
/// manifest (`sabrage-contract-gen/` → `crates/` → `sabrage/` → repo root).
///
/// The default for the binary when `--repo-root` is not given; only meaningful
/// for a dev build run from the checkout.
pub fn compiled_repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The committed file, compiled in so the test needs no filesystem at all.
    const COMMITTED: &str = include_str!("../../../../scripts/demo/contract.gen.sh");

    #[test]
    fn generate_reproduces_the_committed_file_byte_for_byte() {
        let generated = generate();
        assert_eq!(
            generated, COMMITTED,
            "generated contract.gen.sh differs from the committed file:\n\
             --- generated ---\n{generated}\n--- committed ---\n{COMMITTED}"
        );
    }

    #[test]
    fn header_hash_is_the_documented_recipe() {
        // Same bytes doctor.sh's `cat … | shasum -a 256` sees.
        let hash = contract_sha256();
        assert_eq!(hash.len(), 64);
        assert!(COMMITTED.contains(&format!("# contract-sha256: {hash}\n")));
    }

    #[test]
    fn generated_file_ends_with_exactly_one_newline() {
        let g = generate();
        assert!(g.ends_with(")   # 9947 deliberately absent\n"));
        assert!(!g.ends_with("\n\n"));
    }

    #[test]
    fn check_against_the_working_checkout_is_in_sync() {
        let report = check(&compiled_repo_root()).expect("contract files readable");
        assert!(
            report.in_sync,
            "scripts/demo/contract.gen.sh is stale — run: cargo run -p sabrage-contract-gen -- --write"
        );
    }
}
