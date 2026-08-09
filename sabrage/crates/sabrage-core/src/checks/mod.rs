//! The check engine: one contract-ordered registry, bound to evaluator functions.
//!
//! # Contract
//!
//! * `contract/pipeline.toml` owns the **slug list, its order, and the per-side
//!   gates**. Nothing here may add, remove, or reorder checks.
//! * This crate owns **check logic and message/remedy prose**. The parity harness
//!   joins the two front-ends on `slug` + status and never compares prose — but
//!   every message and remedy string an evaluator prints must still match
//!   `scripts/demo/doctor.sh` **verbatim**, because docs/troubleshooting.md
//!   quotes those lines.
//! * Evaluators are **read-only probes**. No filesystem mutation may appear
//!   anywhere in check code — auto-fixes live in the (future) fix registry and
//!   run from the preflight, never from doctor.
//!
//! # Group → module mapping
//!
//! Contract `group` values do not map 1:1 to module names; three are folded or
//! renamed. The registry does not care (binding is by slug), but keep new
//! evaluators in the module their group points at:
//!
//! | contract `group` | module |
//! |---|---|
//! | `meta` | [`meta`] |
//! | `system`, `crossover` | [`system`] |
//! | `bottle` | [`bottle`] |
//! | `toolchain` | [`toolchain`] |
//! | `sources` | [`sources`] |
//! | `pinned` | [`pinned`] |
//! | `game` | [`game`] |
//! | `build` | [`build`] |
//! | `overlay` | [`overlay`] |
//! | `bottle-bridge` | [`bridge`] |
//! | `host` | [`host`] |
//! | `config` | [`config`] |
//! | `headset` | [`headset`] |
//! | `audio` | [`audio`] |
//! | `network` | [`network`] |
//! | `run-only` | [`run_only`] |

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::contract::{contract, CheckSpec};
use crate::paths::{resolve_bs_dir, Bottle, Paths};

pub mod audio;
pub mod bottle;
pub mod bridge;
pub mod build;
pub mod config;
pub mod game;
pub mod headset;
pub mod host;
pub mod meta;
pub mod network;
pub mod overlay;
pub mod pinned;
pub mod run_only;
pub mod sources;
pub mod system;
pub mod toolchain;

// ── outcome types ─────────────────────────────────────────────────────────────

/// Result of one check.
///
/// `Pass`/`Warn`/`Fail`/`Info`/`Skipped` are the five statuses the zsh tap
/// channel emits (`chk ok|warn|fail|info`, plus explicit `tap <slug> skipped`).
/// `NotImplemented` has no zsh counterpart — it exists only while Phase 1 is
/// filling in evaluators and is reported to the tap as `skipped`
/// (see [`crate::tap`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Warn,
    Fail,
    Info,
    Skipped,
    NotImplemented,
}

impl CheckStatus {
    /// Does this status increment doctor's `FAILCOUNT`? Only `Fail` does —
    /// `lib.sh`'s `fail()` is the sole site that bumps the counter.
    pub fn counts_as_fail(self) -> bool {
        matches!(self, CheckStatus::Fail)
    }

    /// Does this status increment the warn tally?
    pub fn counts_as_warn(self) -> bool {
        matches!(self, CheckStatus::Warn)
    }
}

/// One check's verdict, in the shape both the CLI renderer and the GUI consume.
///
/// `message` and `remedy` are the demo.sh strings verbatim; `detail` is the
/// sabrage-only explainability field (expected-vs-actual hash, parsed
/// `library_path`, `lipo -archs` output) that zsh has no room for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckOutcome {
    pub slug: String,
    pub status: CheckStatus,
    pub message: String,
    pub remedy: Option<String>,
    pub detail: Option<String>,
    /// True for rows doctor.sh only taps (`tap <slug> ok`) without printing —
    /// the CLI console renderer suppresses them for byte-compatibility with
    /// the shell; the tap channel and the GUI (which deliberately shows every
    /// row) are unaffected. Not serialized: IPC/tap consumers never need it.
    #[serde(skip)]
    pub quiet: bool,
}

impl CheckOutcome {
    fn new(slug: &str, status: CheckStatus, message: impl Into<String>) -> CheckOutcome {
        CheckOutcome {
            slug: slug.to_string(),
            status,
            message: message.into(),
            remedy: None,
            detail: None,
            quiet: false,
        }
    }

    /// `chk ok <slug> <msg>`.
    pub fn pass(slug: &str, message: impl Into<String>) -> CheckOutcome {
        CheckOutcome::new(slug, CheckStatus::Pass, message)
    }

    /// `tap <slug> ok` — a Pass that doctor.sh does NOT print a console row
    /// for (silent-when-clean tap-only sites: `cx.present`, `bottle.named`,
    /// `game.present`, and whichever `cfg.protocol.*` slug took the silent
    /// branch). The CLI renderer skips it; tap output and the GUI keep it.
    pub fn silent_pass(slug: &str, message: impl Into<String>) -> CheckOutcome {
        CheckOutcome {
            quiet: true,
            ..CheckOutcome::new(slug, CheckStatus::Pass, message)
        }
    }

    /// `chk warn <slug> <msg>`.
    pub fn warn(slug: &str, message: impl Into<String>) -> CheckOutcome {
        CheckOutcome::new(slug, CheckStatus::Warn, message)
    }

    /// `chk fail <slug> <msg> <remedy>` — the remedy is what demo.sh prints after
    /// `remedy:` and must match doctor.sh verbatim.
    pub fn fail(slug: &str, message: impl Into<String>, remedy: impl Into<String>) -> CheckOutcome {
        CheckOutcome {
            remedy: Some(remedy.into()),
            ..CheckOutcome::new(slug, CheckStatus::Fail, message)
        }
    }

    /// A FAIL with no remedy line (doctor's `fail` accepts one argument too).
    pub fn fail_bare(slug: &str, message: impl Into<String>) -> CheckOutcome {
        CheckOutcome::new(slug, CheckStatus::Fail, message)
    }

    /// `chk info <slug> <msg>` — an unstyled informational row.
    pub fn info(slug: &str, message: impl Into<String>) -> CheckOutcome {
        CheckOutcome::new(slug, CheckStatus::Info, message)
    }

    /// `tap <slug> skipped` — a precondition the check depends on is absent.
    pub fn skipped(slug: &str, reason: SkipReason) -> CheckOutcome {
        CheckOutcome::new(slug, CheckStatus::Skipped, reason.0)
    }

    /// No evaluator is bound to this slug yet (Phase 1 scaffolding only).
    pub fn not_implemented(slug: &str) -> CheckOutcome {
        CheckOutcome::new(
            slug,
            CheckStatus::NotImplemented,
            "no evaluator bound yet (Phase 1)",
        )
    }

    /// Attach the explainability detail line.
    pub fn with_detail(mut self, detail: impl Into<String>) -> CheckOutcome {
        self.detail = Some(detail.into());
        self
    }
}

/// Why a check was skipped. Kept as a distinct type so evaluators can hand back a
/// *reason* rather than an unexplained blank row (doctor prints nothing for some
/// skips; sabrage always says why — design-core §10 divergence 11).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkipReason(pub String);

impl SkipReason {
    pub fn new(reason: impl Into<String>) -> SkipReason {
        SkipReason(reason.into())
    }
}

impl From<&str> for SkipReason {
    fn from(s: &str) -> SkipReason {
        SkipReason(s.to_string())
    }
}

// ── context ───────────────────────────────────────────────────────────────────

/// The `WINEVR_*` mirror, as far as the check layer needs it.
///
/// `from_env()` reads the same variables demo.sh does, so `sabrage doctor` is
/// drop-in env-compatible; any non-empty value is `true` for the boolean flags,
/// matching `[ -n "${WINEVR_NO_AUDIO:-}" ]`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckOptions {
    /// `WINEVR_BOTTLE` / `--bottle`.
    pub bottle_name: Option<String>,
    /// `WINEVR_BS_DIR` / `--bs-dir`.
    pub bs_dir_override: Option<PathBuf>,
    /// `WINEVR_WIRED` / `--wired`. Gates the `run.wired-adb` preflight.
    pub wired: bool,
    /// `WINEVR_NO_AUDIO` / `--no-audio`.
    pub no_audio: bool,
    /// `WINEVR_NO_DASHBOARD` / `--no-dashboard`.
    pub no_dashboard: bool,
    /// `WINEVR_VERBOSE` / `--verbose`.
    pub verbose: bool,
    /// Sabrage-only: may probes shell out to `adb` (which starts its daemon)?
    /// Defaults to `true` so doctor parity holds; the GUI can turn it off.
    pub allow_adb_probes: bool,
}

impl CheckOptions {
    /// Defaults with adb probing enabled (doctor parity).
    pub fn new() -> CheckOptions {
        CheckOptions {
            allow_adb_probes: true,
            ..Default::default()
        }
    }

    /// Read the `WINEVR_*` environment exactly as demo.sh does.
    pub fn from_env() -> CheckOptions {
        fn flag(name: &str) -> bool {
            std::env::var_os(name).is_some_and(|v| !v.is_empty())
        }
        fn value(name: &str) -> Option<String> {
            std::env::var(name).ok().filter(|v| !v.is_empty())
        }
        CheckOptions {
            bottle_name: value("WINEVR_BOTTLE"),
            bs_dir_override: value("WINEVR_BS_DIR").map(PathBuf::from),
            wired: flag("WINEVR_WIRED"),
            no_audio: flag("WINEVR_NO_AUDIO"),
            no_dashboard: flag("WINEVR_NO_DASHBOARD"),
            verbose: flag("WINEVR_VERBOSE"),
            allow_adb_probes: true,
        }
    }
}

/// Everything an evaluator is allowed to look at.
///
/// Built once per doctor/preflight run so the bottle is resolved exactly once —
/// mirroring doctor.sh section 3, whose position in the file is load-bearing
/// because later sections consume `BOTTLE_OK`/`PREFIX`/`BS_DIR`.
#[derive(Debug, Clone)]
pub struct CheckCtx {
    /// The typed lib.sh path set.
    pub paths: Paths,
    /// `Some` only when a bottle was named **and** its `cxbottle.conf` exists —
    /// i.e. doctor's `BOTTLE_OK=1`.
    pub bottle: Option<Bottle>,
    /// The resolved Beat Saber directory (`BS_DIR`).
    pub bs_dir: PathBuf,
    /// True when a bottle name was supplied at all, regardless of whether it
    /// exists. doctor needs both bits: no name ⇒ `bottle.named` FAILs and the
    /// rest of section 3 is skipped; a name that does not resolve ⇒
    /// `bottle.exists` FAILs instead.
    pub bottle_requested: bool,
    /// The `WINEVR_*` mirror.
    pub opts: CheckOptions,
}

impl CheckCtx {
    /// Resolve the bottle and `BS_DIR` from `opts` and build the context.
    pub fn new(paths: Paths, opts: CheckOptions) -> CheckCtx {
        let bottle_requested = opts.bottle_name.is_some();
        // doctor.sh sets $PREFIX as soon as a NAME is given — before the
        // cxbottle.conf existence test — so BS_DIR must derive from the named
        // (unvalidated) bottle even when it does not exist; `bottle` (= the
        // shell's BOTTLE_OK) separately carries the existence bit.
        let named = opts.bottle_name.as_deref().map(Bottle::unvalidated);
        let bs_dir = resolve_bs_dir(named.as_ref(), opts.bs_dir_override.as_deref());
        let bottle = named.filter(Bottle::exists);
        CheckCtx {
            paths,
            bottle,
            bs_dir,
            bottle_requested,
            opts,
        }
    }

    /// Convenience constructor: `repo_root` + environment.
    pub fn from_env(repo_root: impl Into<PathBuf>) -> CheckCtx {
        CheckCtx::new(Paths::new(repo_root), CheckOptions::from_env())
    }

    /// The bottle name to interpolate into remedy strings.
    ///
    /// doctor.sh does `WINEVR_BOTTLE="${WINEVR_BOTTLE:-<name>}"` after section 3
    /// precisely so remedies like `./demo.sh install --bottle $WINEVR_BOTTLE`
    /// stay readable when no bottle was given. Same placeholder here.
    pub fn bottle_label(&self) -> &str {
        self.opts.bottle_name.as_deref().unwrap_or("<name>")
    }

    /// The bottle prefix, or an empty path when no bottle resolved (doctor's
    /// `PREFIX="${PREFIX:-}"`).
    pub fn prefix(&self) -> &Path {
        self.bottle
            .as_ref()
            .map(|b| b.prefix.as_path())
            .unwrap_or(Path::new(""))
    }
}

// ── registry ──────────────────────────────────────────────────────────────────

/// A check evaluator: a synchronous, read-only probe.
///
/// Sync is deliberate for v1. Every doctor probe is a `stat`, a small read, a
/// digest, or a short subprocess; the async machinery in design-core §3 belongs
/// to the *stage* layer, where long-running children and cancellation actually
/// exist. Making these `async` now would buy nothing and force `BoxFuture` into
/// every signature the Phase 1 evaluator agent writes.
pub type Evaluator = fn(&CheckCtx) -> CheckOutcome;

/// One contract check joined to its evaluator.
#[derive(Clone, Copy)]
pub struct BoundCheck {
    /// The contract entry — slug, group, both gates, `volatile`, `fix`.
    pub spec: &'static CheckSpec,
    /// `None` while Phase 1 is still filling the group modules.
    pub eval: Option<Evaluator>,
}

impl BoundCheck {
    pub fn slug(&self) -> &'static str {
        self.spec.slug.as_str()
    }

    /// Evaluate, or produce a [`CheckStatus::NotImplemented`] outcome.
    pub fn evaluate(&self, ctx: &CheckCtx) -> CheckOutcome {
        match self.eval {
            Some(f) => f(ctx),
            None => CheckOutcome::not_implemented(self.slug()),
        }
    }
}

impl std::fmt::Debug for BoundCheck {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BoundCheck")
            .field("slug", &self.slug())
            .field("bound", &self.eval.is_some())
            .finish()
    }
}

/// The contract check list, bound to evaluators, in contract order.
#[derive(Debug, Clone)]
pub struct Registry {
    checks: Vec<BoundCheck>,
}

/// The contract group whose checks have no doctor row: run-only preflights
/// (pipeline.toml "run-only preflights (no doctor row)"). Doctor never
/// evaluates or taps them — the ONE policy site for that rule.
pub const NO_DOCTOR_ROW_GROUP: &str = "run-only";

impl Registry {
    /// All checks, in contract (= doctor) order — including run-only
    /// preflights. Launch preflight walks this; doctor must not.
    pub fn checks(&self) -> &[BoundCheck] {
        &self.checks
    }

    /// The doctor-visible subset: contract order minus [`NO_DOCTOR_ROW_GROUP`].
    /// doctor.sh never emits these slugs, so neither may the native doctor
    /// (console, tap, or fail count) — tier-2 parity depends on it.
    pub fn doctor_checks(&self) -> impl Iterator<Item = &BoundCheck> {
        self.checks
            .iter()
            .filter(|c| c.spec.group != NO_DOCTOR_ROW_GROUP)
    }

    pub fn len(&self) -> usize {
        self.checks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.checks.is_empty()
    }

    /// Look one check up by slug.
    pub fn get(&self, slug: &str) -> Option<&BoundCheck> {
        self.checks.iter().find(|c| c.slug() == slug)
    }

    /// Slugs with no evaluator yet.
    pub fn unbound(&self) -> Vec<&'static str> {
        self.checks
            .iter()
            .filter(|c| c.eval.is_none())
            .map(|c| c.slug())
            .collect()
    }

    /// The subset the native launch preflight runs, in contract order.
    ///
    /// Order note: run.sh's preflight order differs from doctor's. This returns
    /// **contract order**; the run stage owns its own ordered slice and is the
    /// place to encode that difference.
    pub fn native_preflight(&self) -> Vec<&BoundCheck> {
        self.checks
            .iter()
            .filter(|c| c.spec.native_gate.is_gating())
            .collect()
    }
}

/// Why a registry could not be built strictly.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    /// Contract slugs with no evaluator bound.
    #[error("contract slugs with no evaluator: {}", .0.join(", "))]
    MissingEvaluators(Vec<String>),
    /// Evaluators registered for slugs the contract does not declare.
    #[error("evaluators for unknown slugs: {}", .0.join(", "))]
    UnknownSlugs(Vec<String>),
    /// The same slug registered by more than one evaluator.
    #[error("duplicate evaluator registrations: {}", .0.join(", "))]
    DuplicateEvaluators(Vec<String>),
}

/// Every evaluator the group modules expose, concatenated.
///
/// Order here is irrelevant — [`build_registry`] re-orders by the contract.
fn all_defs() -> Vec<(&'static str, Evaluator)> {
    let mut defs = Vec::new();
    for group in [
        meta::defs as fn() -> Vec<(&'static str, Evaluator)>,
        system::defs,
        bottle::defs,
        toolchain::defs,
        sources::defs,
        pinned::defs,
        game::defs,
        build::defs,
        overlay::defs,
        bridge::defs,
        host::defs,
        config::defs,
        headset::defs,
        audio::defs,
        network::defs,
        run_only::defs,
    ] {
        defs.extend(group());
    }
    defs
}

/// Join the contract's ordered check list with the evaluator map.
///
/// `strict = true` (the release contract) rejects any mismatch: a contract slug
/// with no evaluator, an evaluator for a slug the contract does not declare, or
/// two evaluators claiming the same slug. That is the mechanical enforcement of
/// "adding a check to only one place must fail" — the parity design's tier-1
/// coverage test in both directions.
///
/// `strict = false` is the Phase 1 escape hatch: unknown and duplicate
/// registrations are still errors (those are always bugs), but missing bindings
/// are tolerated and their checks report [`CheckStatus::NotImplemented`].
pub fn build_registry(strict: bool) -> Result<Registry, RegistryError> {
    let defs = all_defs();

    let mut dupes: BTreeSet<String> = BTreeSet::new();
    let mut map: std::collections::BTreeMap<&'static str, Evaluator> =
        std::collections::BTreeMap::new();
    for (slug, eval) in defs {
        if map.insert(slug, eval).is_some() {
            dupes.insert(slug.to_string());
        }
    }
    if !dupes.is_empty() {
        return Err(RegistryError::DuplicateEvaluators(
            dupes.into_iter().collect(),
        ));
    }

    let specs = &contract().checks;
    let declared: BTreeSet<&str> = specs.iter().map(|s| s.slug.as_str()).collect();
    let unknown: Vec<String> = map
        .keys()
        .filter(|s| !declared.contains(*s))
        .map(|s| s.to_string())
        .collect();
    if !unknown.is_empty() {
        return Err(RegistryError::UnknownSlugs(unknown));
    }

    let checks: Vec<BoundCheck> = specs
        .iter()
        .map(|spec| BoundCheck {
            spec,
            eval: map.get(spec.slug.as_str()).copied(),
        })
        .collect();

    if strict {
        // Run-only preflights have no doctor evaluator BY DESIGN (their
        // launch-preflight implementations land in Phase 2), so strictness
        // covers every group except NO_DOCTOR_ROW_GROUP.
        let missing: Vec<String> = checks
            .iter()
            .filter(|c| c.eval.is_none() && c.spec.group != NO_DOCTOR_ROW_GROUP)
            .map(|c| c.slug().to_string())
            .collect();
        if !missing.is_empty() {
            return Err(RegistryError::MissingEvaluators(missing));
        }
    }

    Ok(Registry { checks })
}

/// The registry, built strictly: every doctor-visible contract slug must have
/// a bound evaluator (run-only preflights excepted — Phase 2 binds those on
/// the launch path). "Added a slug to the contract, forgot the evaluator" is
/// an immediate hard error here, which is the point.
pub fn registry() -> Registry {
    build_registry(true).expect("evaluator registrations are consistent with the contract")
}

// ── doctor ────────────────────────────────────────────────────────────────────

/// The result of a full doctor pass.
///
/// `fail_count` is doctor's exit code (capped at 255 by the CLI shim — design-core
/// §10 divergence 13 rejects zsh's mod-256 wraparound).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorReport {
    pub outcomes: Vec<CheckOutcome>,
    pub fail_count: usize,
    pub warn_count: usize,
}

impl DoctorReport {
    /// Doctor's exit code, capped at 255.
    pub fn exit_code(&self) -> i32 {
        self.fail_count.min(255) as i32
    }
}

/// Run every contract check in contract order, streaming each outcome to `sink`
/// as it resolves, and return the aggregate report.
///
/// Order is load-bearing (doctor.sh section 3 resolves the bottle context that
/// later sections consume) — hence a plain sequential walk, not a join set.
pub fn run_doctor(ctx: &CheckCtx, mut sink: impl FnMut(CheckOutcome)) -> DoctorReport {
    run_doctor_with(&registry(), ctx, &mut sink)
}

/// [`run_doctor`] against a caller-supplied registry (single-check runs, tests).
pub fn run_doctor_with(
    registry: &Registry,
    ctx: &CheckCtx,
    sink: &mut impl FnMut(CheckOutcome),
) -> DoctorReport {
    let mut outcomes = Vec::with_capacity(registry.len());
    let mut fail_count = 0usize;
    let mut warn_count = 0usize;
    for check in registry.doctor_checks() {
        let outcome = check.evaluate(ctx);
        if outcome.status.counts_as_fail() {
            fail_count += 1;
        }
        if outcome.status.counts_as_warn() {
            warn_count += 1;
        }
        sink(outcome.clone());
        outcomes.push(outcome);
    }
    DoctorReport {
        outcomes,
        fail_count,
        warn_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_binds_in_contract_order_and_covers_every_slug() {
        let reg = registry();
        assert_eq!(reg.len(), contract().checks.len());
        let slugs: Vec<&str> = reg.checks().iter().map(|c| c.slug()).collect();
        assert_eq!(slugs, contract().check_slugs());
    }

    #[test]
    fn unknown_and_duplicate_registrations_are_errors_even_leniently() {
        // Every doctor-visible slug is bound, so BOTH builds must succeed —
        // strictness only exempts the run-only group (Phase 2 binds those on
        // the launch path, never here).
        assert!(build_registry(false).is_ok());
        assert!(build_registry(true).is_ok());
        // The only unbound slugs are the run-only preflights.
        for slug in registry().unbound() {
            assert_eq!(
                contract().check(slug).unwrap().group,
                NO_DOCTOR_ROW_GROUP,
                "unbound slug {slug} is not a run-only preflight"
            );
        }
    }

    #[test]
    fn doctor_walks_only_doctor_visible_checks() {
        // doctor.sh never evaluates or taps the run-only preflights; the
        // native doctor must emit exactly the doctor-visible subset, in
        // contract order, with no NotImplemented (all of them are bound).
        let ctx = CheckCtx::new(Paths::new("/nonexistent/repo"), CheckOptions::new());
        let reg = registry();
        let visible = reg.doctor_checks().count();
        assert_eq!(visible, reg.len() - reg.unbound().len());
        let mut seen = 0usize;
        let report = run_doctor_with(&reg, &ctx, &mut |_| seen += 1);
        assert_eq!(seen, visible);
        assert_eq!(report.outcomes.len(), visible);
        for o in &report.outcomes {
            assert_ne!(
                o.status,
                CheckStatus::NotImplemented,
                "doctor-visible slug {} has no evaluator",
                o.slug
            );
            assert_ne!(
                reg.get(&o.slug).unwrap().spec.group,
                NO_DOCTOR_ROW_GROUP,
                "run-only slug {} leaked into doctor output",
                o.slug
            );
        }
    }

    #[test]
    fn native_preflight_is_the_gating_subset() {
        let reg = registry();
        let pre: Vec<&str> = reg.native_preflight().iter().map(|c| c.slug()).collect();
        let want: Vec<&str> = contract()
            .native_preflight()
            .iter()
            .map(|s| s.slug.as_str())
            .collect();
        assert_eq!(pre, want);
        assert!(pre.contains(&"run.wine-exec"));
        assert!(!pre.contains(&"sys.arch"));
    }

    #[test]
    fn ctx_bottle_label_falls_back_to_the_doctor_placeholder() {
        let ctx = CheckCtx::new(Paths::new("/repo"), CheckOptions::new());
        assert_eq!(ctx.bottle_label(), "<name>");
        assert!(!ctx.bottle_requested);
        assert_eq!(ctx.prefix(), Path::new(""));

        let opts = CheckOptions {
            bottle_name: Some("NoSuchBottle".into()),
            ..CheckOptions::new()
        };
        let ctx = CheckCtx::new(Paths::new("/repo"), opts);
        assert_eq!(ctx.bottle_label(), "NoSuchBottle");
        assert!(ctx.bottle_requested);
        // Named but non-existent: requested, unresolved.
        assert!(ctx.bottle.is_none());
    }
}
