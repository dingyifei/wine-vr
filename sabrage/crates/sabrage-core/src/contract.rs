//! The shared parity contract, compiled in.
//!
//! `contract/pipeline.toml` is the single source for pins, the depot triple,
//! the host-manifest path, the DXMT artifact set, the port lists, the
//! **ordered check registry**, and the launch-action registry. The zsh side
//! consumes it through the GENERATED `scripts/demo/contract.gen.sh`;
//! sabrage-core parses the TOML directly.
//!
//! The three contract files are baked in with `include_str!` rather than read
//! from `repo_root`: `Sabrage.app` is installed somewhere unrelated to the
//! repo and `repo_root` is user-configurable, so the check registry is part
//! of the binary's identity, not of machine state. Editing a contract file
//! retriggers a rebuild, which is the tripwire the parity design wants.
//! [`crate::util::contract_hash`] reads the three files from `repo_root` at
//! runtime because `meta.contract-sync` compares the *on-disk* contract
//! against the *on-disk* generated shell file.
//!
//! All three includes live in this one module so the repo-root depth is
//! stated exactly once.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

/// Raw bytes of `contract/pipeline.toml` (compile-time).
pub const PIPELINE_TOML: &str = include_str!("../../../../contract/pipeline.toml");

/// Raw bytes of `contract/oxrsys-runtime.toml.template` (compile-time).
///
/// setup.sh does `cat template > "$TOML"`, so the file it writes is these bytes
/// verbatim — no trailing-newline munging. See [`crate::util::toml_template`].
pub const RUNTIME_TOML_TEMPLATE: &str =
    include_str!("../../../../contract/oxrsys-runtime.toml.template");

/// Raw bytes of `contract/active_runtime.x86_64.json.template` (compile-time).
///
/// install.sh reads it with `$(<…)`, which strips trailing newlines — see
/// [`crate::util::render_host_manifest`] for the exact rendering rule.
pub const HOST_MANIFEST_TEMPLATE: &str =
    include_str!("../../../../contract/active_runtime.x86_64.json.template");

/// Placeholder substituted by [`crate::util::render_host_manifest`].
pub const HOST_MANIFEST_PLACEHOLDER: &str = "@OXR_DYLIB@";

/// Repo-relative path of the generated shell mirror of this contract.
pub const CONTRACT_GEN_REL_PATH: &str = "scripts/demo/contract.gen.sh";

/// Repo-relative paths of the three contract files, in the order the
/// `meta.contract-sync` hash recipe concatenates them (doctor.sh section 0).
pub const CONTRACT_FILES: [&str; 3] = [
    "contract/pipeline.toml",
    "contract/oxrsys-runtime.toml.template",
    "contract/active_runtime.x86_64.json.template",
];

/// How a check's failure is treated by the launch preflight, per side.
///
/// Mirrors the contract's gate vocabulary verbatim:
/// * `block` — launch aborts on failure (`die` on the zsh side, [`crate::error::SabrageError::Fatal`] here)
/// * `warn` — failure prints a warning, launch continues
/// * `autofix` — failure triggers an automatic permanent fix, then a re-check
/// * `none` — doctor-only; not part of the launch preflight on that side
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Gate {
    Block,
    Warn,
    Autofix,
    None,
}

impl Gate {
    /// True when this gate participates in the launch preflight at all.
    pub fn is_gating(self) -> bool {
        !matches!(self, Gate::None)
    }

    /// The contract spelling (`"block"` / `"warn"` / `"autofix"` / `"none"`).
    pub fn as_str(self) -> &'static str {
        match self {
            Gate::Block => "block",
            Gate::Warn => "warn",
            Gate::Autofix => "autofix",
            Gate::None => "none",
        }
    }
}

/// `[deps]` — pinned dependency sources fetched by `demo.sh setup`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Deps {
    /// Release base URL (`DEPS_URL`).
    pub url: String,
    /// Goldberg dll asset filename.
    pub gbe_dll_asset: String,
    /// Pinned sha256 of the Goldberg dll (`GBE_DLL_SHA256`).
    pub gbe_dll_sha256: String,
    /// DXMT artifact tarball asset filename.
    pub dxmt_tgz_asset: String,
    /// Pinned sha256 of the DXMT tarball (`DXMT_TGZ_SHA256`), also the content of
    /// the `.sha256` provenance marker `setup` writes into `ext/dxmt-artifacts/`.
    pub dxmt_tgz_sha256: String,
}

/// `[game]` — the Beat Saber 1.29.4 depot pin and default install leaf.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Game {
    /// `BS_APPID` (620980).
    pub appid: u64,
    /// `BS_DEPOT` (620981).
    pub depot: u64,
    /// `BS_MANIFEST` — a 19-digit id, kept a string so it never round-trips
    /// through a float or overflows on a 32-bit target.
    pub manifest: String,
    /// Default install directory leaf under the bottle's Steam library
    /// (`"Beat Saber 1294"`).
    pub bs_dir_leaf: String,
}

/// `[paths]` — path literals both front-ends must agree on.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ContractPaths {
    /// `HOST_XR_JSON` — the root-owned host OpenXR registration.
    pub host_xr_json: String,
}

/// `[ports]` — streaming / dashboard endpoints.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Ports {
    /// `WIRED_PORTS` — the two ports `--wired` forwards, and the pair doctor's
    /// `net.ports` / `net.adb-forwards` checks look at.
    pub stream: Vec<u16>,
    /// `LEGACY_REVERSE_PORTS` — explicit list (9947 is deliberately absent;
    /// never treat this as a range).
    pub legacy_reverse: Vec<u16>,
    /// Embedded ALVR dashboard address, `"127.0.0.1:8082"`.
    pub dashboard_addr: String,
}

/// `[dxmt]` — the complete artifact set `install` deploys.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Dxmt {
    /// Paths relative to `ext/dxmt-artifacts/`; presence gates key on ALL of them.
    pub files: Vec<String>,
}

/// One `[[check]]` entry: the stable slug plus its per-side launch gates.
///
/// Check *logic* and message/remedy prose are impl-owned and deliberately absent
/// from the contract — the parity harness joins on `slug` + status only.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CheckSpec {
    /// Stable dotted slug, the join key for everything (`"build.helper-arm64"`).
    pub slug: String,
    /// Grouping label (`"system"`, `"bottle-bridge"`, `"run-only"`, …). Maps to a
    /// module under `checks/` — see [`crate::checks`] for the exact mapping.
    pub group: String,
    /// How `run.sh`'s preflight treats a failure of this check.
    pub shell_gate: Gate,
    /// How the native run preflight treats a failure. May deliberately differ
    /// from `shell_gate`; the divergence is recorded in the contract, not in code.
    pub native_gate: Gate,
    /// True when the tier-2 live differ may only compare presence, not status
    /// (adb / lsof / session state legitimately change between two doctor runs).
    #[serde(default)]
    pub volatile: bool,
    /// Optional `FixId` this check's remedy maps to (`"fix.run-install"`, …).
    #[serde(default)]
    pub fix: Option<String>,
}

/// One `[[launch_action]]`: an unconditional ordered preparation step in `run.sh`
/// (NOT check-shaped — no pass/fail, no remedy). Order here == execution order.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LaunchAction {
    /// Stable id, matching run.sh's `# launch-action:` tag.
    pub id: String,
    /// One-line description of what the step does.
    pub what: String,
}

/// The parsed `contract/pipeline.toml`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Contract {
    pub deps: Deps,
    pub game: Game,
    pub paths: ContractPaths,
    pub ports: Ports,
    pub dxmt: Dxmt,
    /// Ordered check registry; order is doctor.sh's and load-bearing (section 3
    /// resolves bottle context later checks consume; run-only preflights last).
    /// Doctor-row order (run-only slugs excluded) is pinned by
    /// `sabrage-parity::tests::slug_coverage::doctor_slug_coverage_matches_the_contract`.
    #[serde(default, rename = "check")]
    pub checks: Vec<CheckSpec>,
    /// Ordered launch-action registry.
    #[serde(default, rename = "launch_action")]
    pub launch_actions: Vec<LaunchAction>,
}

impl Contract {
    /// Parse a contract from TOML text. Used by the compile-time [`CONTRACT`] and
    /// by anything that wants to diff an on-disk contract against the baked one.
    pub fn parse(text: &str) -> Result<Contract, toml::de::Error> {
        toml::from_str(text)
    }

    /// The check spec for `slug`, if the contract declares it.
    pub fn check(&self, slug: &str) -> Option<&CheckSpec> {
        self.checks.iter().find(|c| c.slug == slug)
    }

    /// All check slugs, in contract (= doctor) order.
    pub fn check_slugs(&self) -> Vec<&str> {
        self.checks.iter().map(|c| c.slug.as_str()).collect()
    }

    /// Slug → spec, for O(log n) lookups when binding a whole registry.
    pub fn checks_by_slug(&self) -> BTreeMap<&str, &CheckSpec> {
        self.checks.iter().map(|c| (c.slug.as_str(), c)).collect()
    }

    /// Checks whose `native_gate` participates in the launch preflight.
    pub fn native_preflight(&self) -> Vec<&CheckSpec> {
        self.checks
            .iter()
            .filter(|c| c.native_gate.is_gating())
            .collect()
    }

    /// The `DepotDownloader …` remedy string doctor's `game.present` row prints.
    ///
    /// Byte-identical to lib.sh's `DEPOT_CMD` / doctor.sh's `$DEPOT_CMD`, including
    /// the quoting of `-dir`. `tests::depot_command_matches_lib_sh` pins this side's
    /// literal only — nothing reads lib.sh, so a shell-side edit has to be mirrored
    /// here by hand.
    pub fn depot_command(&self, bs_dir: &Path) -> String {
        format!(
            "DepotDownloader -app {} -depot {} -manifest {} -username <steam-user> -dir \"{}\"",
            self.game.appid,
            self.game.depot,
            self.game.manifest,
            bs_dir.display()
        )
    }
}

/// The compiled-in contract. Panics on first use if `contract/pipeline.toml` is
/// malformed — which can only happen at build time, so the panic is a build-time
/// error in practice.
pub static CONTRACT: LazyLock<Contract> = LazyLock::new(|| {
    Contract::parse(PIPELINE_TOML).expect("contract/pipeline.toml is not valid contract TOML")
});

/// `&'static` accessor for the compiled-in contract.
pub fn contract() -> &'static Contract {
    &CONTRACT
}

/// The `contract-sha256` of the contract **this binary was compiled from** —
/// the same `cat pipeline.toml runtime-template host-template | shasum -a 256`
/// recipe [`crate::util::contract_hash`] recomputes from `repo_root` on disk,
/// and the same value `scripts/demo/contract.gen.sh` records in its
/// `# contract-sha256:` header.
///
/// The on-disk half of `meta.contract-sync` only proves a checkout is
/// self-consistent: a binary built from checkout X, pointed at checkout Y via
/// `repo_root`, still executes **X's** registry, pins, ports, and templates.
/// The parity harness cannot see this either because tier 2 rebuilds the CLI
/// from the checkout it diffs. `meta.contract-sync` compares this value
/// against `util::contract_hash(repo_root)` to detect the skew; different is
/// a Fail, pinned by
/// `checks::meta::tests::fails_when_the_binary_was_compiled_from_a_different_contract`.
pub static COMPILED_CONTRACT_SHA256: LazyLock<String> = LazyLock::new(|| {
    crate::util::contract_sha256_from(&[
        PIPELINE_TOML,
        RUNTIME_TOML_TEMPLATE,
        HOST_MANIFEST_TEMPLATE,
    ])
});

#[cfg(test)]
mod tests;
