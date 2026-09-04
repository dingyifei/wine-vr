//! Generator for `scripts/demo/contract.gen.sh`, the zsh mirror of
//! `contract/pipeline.toml` that lib.sh sources so the shell needs no TOML parser.
//!
//! Two tripwires cover it: the `# contract-sha256:` header lets doctor's
//! `meta.contract-sync` catch a contract edited without regenerating, and
//! [`check`] catches a hand-edited generated file. The committed bytes are
//! pinned by sabrage-parity's
//! `tests::contract_gen_parity::generate_matches_the_committed_contract_gen_sh`.
//!
//! The contract subset below is re-declared rather than imported from
//! `sabrage-core`: a field added to `pipeline.toml` but not here is visibly
//! absent from the shell instead of silently coupled through a shared struct.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

/// Repo-relative path of the generated file.
pub const OUTPUT_REL_PATH: &str = "scripts/demo/contract.gen.sh";

/// Repo-relative paths of the three contract inputs, in the order their bytes
/// are concatenated for the `contract-sha256` header.
/// Reference: scripts/demo/doctor.sh section 0, same order and recipe.
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
    gbe_dll_asset: String,
    gbe_dll_sha256: String,
    dxmt_tgz_asset: String,
    dxmt_tgz_sha256: String,
}

#[derive(Debug, Deserialize)]
struct Game {
    appid: u64,
    depot: u64,
    manifest: String,
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

    let dxmt_files = c
        .dxmt
        .files
        .iter()
        .map(|f| zsh_word(f))
        .collect::<Vec<_>>()
        .join(" ");
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
DEPS_URL={deps_url}
DXMT_TGZ_ASSET={dxmt_asset}
DXMT_TGZ_SHA256={dxmt_sha}
GBE_DLL_ASSET={gbe_asset}
GBE_DLL_SHA256={gbe_sha}

# ---- Beat Saber depot pin ------------------------------------------------------
BS_APPID={appid}
BS_DEPOT={depot}
BS_MANIFEST={manifest}
BS_DIR_LEAF={bs_dir_leaf}

# ---- host OpenXR loader registration -------------------------------------------
HOST_XR_JSON={host_xr_json}

# ---- DXMT artifact set (presence gates key on ALL of these) --------------------
DXMT_FILES=({dxmt_files})

# ---- streaming ports -----------------------------------------------------------
WIRED_PORTS=({wired_ports})
LEGACY_REVERSE_PORTS=({legacy_ports})   # 9947 deliberately absent
",
        hash = hash,
        deps_url = zsh_scalar(&c.deps.url),
        dxmt_asset = zsh_scalar(&c.deps.dxmt_tgz_asset),
        dxmt_sha = zsh_scalar(&c.deps.dxmt_tgz_sha256),
        gbe_asset = zsh_scalar(&c.deps.gbe_dll_asset),
        gbe_sha = zsh_scalar(&c.deps.gbe_dll_sha256),
        appid = c.game.appid,
        depot = c.game.depot,
        manifest = zsh_word(&c.game.manifest),
        bs_dir_leaf = zsh_scalar(&c.game.bs_dir_leaf),
        host_xr_json = zsh_scalar(&c.paths.host_xr_json),
        dxmt_files = dxmt_files,
        wired_ports = wired_ports,
        legacy_ports = legacy_ports,
    ))
}

// A value here is shell code unless quoted, and sabrage-core reads the same TOML
// literally — an unquoted `$(...)`/backtick/`$VAR`/space/glob silently diverges the
// two front-ends (tests::hostile_contract_values_are_emitted_as_zsh_literals).
// Encoding stays minimal so the committed contract.gen.sh is byte-identical
// (tests::zsh_encoders_are_minimal_for_ordinary_contract_values).

/// Wrap `s` in single quotes, which zsh treats as fully literal, closing and
/// re-opening around each embedded `'` (`'\''`).
fn zsh_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// A scalar assignment's right-hand side, quotes included.
///
/// Keeps the historical `"…"` form for values that are literal inside double
/// quotes (no `"`, `\`, `$`, backtick, and no control characters); anything
/// else becomes a single-quoted literal.
fn zsh_scalar(s: &str) -> String {
    if s.chars()
        .all(|c| !matches!(c, '"' | '\\' | '$' | '`') && !c.is_control())
    {
        format!("\"{s}\"")
    } else {
        zsh_single_quote(s)
    }
}

/// One shell *word* (an array element, or a bare assignment like `BS_MANIFEST=`).
///
/// Bare only for the conservative set that survives zsh's expansions untouched:
/// no whitespace (word splitting), no `*?[]{}~` (globbing/brace expansion), no
/// `$`/backtick (substitution), and never a leading `=` or `~` (zsh's `EQUALS`
/// and tilde expansions fire on the first character). Everything else is
/// single-quoted, which keeps it exactly one word whatever it contains.
fn zsh_word(s: &str) -> String {
    let body_ok = s.chars().all(|c| {
        c.is_ascii_alphanumeric()
            || matches!(c, '.' | '_' | '-' | '/' | ':' | '+' | '@' | '%' | ',')
    });
    let head_ok = matches!(s.chars().next(), Some(c) if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/'));
    if body_ok && head_ok {
        s.to_string()
    } else {
        zsh_single_quote(s)
    }
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
mod tests;
