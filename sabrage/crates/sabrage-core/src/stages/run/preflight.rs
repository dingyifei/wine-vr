//! Contract-ordered launch gates. Reference: scripts/demo/run.sh — its
//! `# preflight:` / `# preflight-autofix:` tags name the slugs, and the
//! contract's `native_gate` column decides `block` / `warn` / `autofix` /
//! `none` per slug, so a per-side divergence is recorded in
//! `contract/pipeline.toml` rather than discovered by reading two
//! implementations.
//!
//! Each evaluated slug emits exactly one [`crate::events::StageEvent::Check`]
//! carrying its final outcome; for an `autofix` slug that is the re-check's,
//! preceded by the [`crate::events::StageEvent::AutoFixed`] describing what
//! changed.
//!
//! The walk follows contract order, which is doctor's, not run.sh's, and in
//! which the bottle context resolves before anything consumes it. Both sides
//! evaluate the same set and abort on the same conditions, so only which die
//! wins can differ. A check the shell would not have evaluated either is a
//! `Skipped` row that never blocks; an applicable check that reached no
//! verdict is Fatal, never a pass — launching on an unverified gate is how a
//! black window happens. Pinned by
//! tests::{the_slug_list_is_unique_gating_only_and_includes_the_run_only_slugs,
//! an_unverifiable_applicable_check_is_fatal_not_a_pass}; the order and gate
//! divergences are declared in PARITY.md § Run preflight (encoded in the
//! contract's per-side gates).

use std::path::Path;

use crate::checks::{CheckOutcome, CheckStatus, Registry};
use crate::contract::{contract, CheckSpec, Gate};
use crate::error::{Result, SabrageError};
use crate::events::{step, StageEvent};
use crate::fixes::{self, backend, FixAction, FixReport};
use crate::stages::{require_bottle, StageCtx};
use crate::util::bs_version;

use super::PreflightFacts;

/// The two `preflight-autofix`-gated helper slugs, in contract order. Both map
/// to `fix.restage-helper` and both are skipped under
/// `encoder_process = "inproc"`.
const HELPER_SLUGS: [&str; 2] = ["build.helper-staged", "build.helper-arm64"];

/// The launch-preflight slugs this side evaluates, in contract order.
///
/// Exactly `contract().native_preflight()` — every check whose `native_gate`
/// is gating
/// (tests::the_slug_list_is_unique_gating_only_and_includes_the_run_only_slugs).
/// Derived, never hand-written: the parity crate joins run.sh's
/// `# preflight:` tags against this list, and a hand-maintained second list is
/// how the two drift.
pub fn preflight_slugs() -> Vec<&'static str> {
    contract()
        .native_preflight()
        .into_iter()
        .map(|c| c.slug.as_str())
        .collect()
}

/// `oxrsys-runtime.toml` read once, before the checks that branch on it — the
/// same two facts run.sh captures with `awk`, resolved the way the runtime
/// resolves them (`read_toml_facts`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct TomlFacts {
    /// `[ -f "$TOML" ]`.
    present: bool,
    /// `$PROTOCOL` — the last value the runtime would **accept**, else the raw
    /// last assignment, and `""` when the key is absent or the file does not
    /// exist.
    protocol: String,
    /// `${ENCODER_PROC:-auto}` — already defaulted, exactly like the shell's
    /// own parameter expansion.
    encoder_process: String,
}

/// One key's **raw** last assignment via
/// [`crate::config::runtime_toml::effective_string`], with no accepted-set
/// filtering; an unassigned key is the empty string, which the callers below
/// already treat as "unset".
///
/// This is the right reader for a key whose accepted set Sabrage does not
/// model, and the fallback `read_toml_facts` uses when no occurrence at all
/// is one the runtime would accept: that is the value the die text has to
/// quote back to the user.
fn effective_string(toml_text: &str, key: &str) -> String {
    crate::config::runtime_toml::effective_string(toml_text, key).unwrap_or_default()
}

/// The two config facts a launch branches on, in one read of the file,
/// resolved the way the **runtime** resolves them rather than the way `awk`
/// does.
///
/// `protocol` and `encoder_process` go through
/// [`crate::config::runtime_toml::read_lines_like_the_runtime`]: the value the
/// launched runtime uses is the last assignment it would **accept**, across
/// `[table]` boundaries, so `protocol = "alvr"` followed by
/// `protocol = "banana"` still runs ALVR. When nothing is acceptable each key
/// falls back to its raw last assignment, so run.sh's die still quotes the
/// value back; an absent `encoder_process` reads as `auto`. The Settings
/// screen reads the same file through the same function, so the two cannot
/// name different backends.
///
/// One declared DIVERGENCE from run.sh, in this side's favour: an **unquoted**
/// value (`protocol = alvr`) is accepted here and reads as empty through
/// `awk -F'"'`. PARITY.md § Declared by the 2026-08-30 adversarial review
/// (round 1 fixes), "Config readers: doctor emulates `awk`, launch uses the
/// runtime's semantics."; pinned by
/// tests::{the_shadowed_invalid_last_fixture_launches_on_its_valid_values,
/// a_trailing_invalid_assignment_leaves_the_previous_valid_one_in_force}.
fn read_toml_facts(toml_path: &Path) -> TomlFacts {
    let present = toml_path.is_file();
    // An unreadable-but-present file degrades to empty captures, exactly like
    // the shell's unredirected `awk` failing silently.
    let text = if present {
        std::fs::read_to_string(toml_path).unwrap_or_default()
    } else {
        String::new()
    };
    let (values, _invalid, _shadowed) =
        crate::config::runtime_toml::read_lines_like_the_runtime(&text);
    let raw_encoder = values
        .encoder_process
        .map(|e| e.as_str().to_string())
        .unwrap_or_else(|| effective_string(&text, "encoder_process"));
    TomlFacts {
        present,
        protocol: values
            .protocol
            .map(|p| p.as_str().to_string())
            .unwrap_or_else(|| effective_string(&text, "protocol")),
        encoder_process: if raw_encoder.is_empty() {
            "auto".to_string()
        } else {
            raw_encoder
        },
    }
}

/// run.sh's `case "$ENCODER_PROC"`: does this configuration need the staged
/// arm64 helper, and does the shell print a line about it first?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EncoderMode {
    /// `native|auto` — helper required, nothing printed.
    HelperRequired,
    /// `inproc` — helper checks skipped, one `info` row.
    Inproc,
    /// Anything else — one `warn` row, then treated exactly like `auto`.
    UnrecognizedTreatedAsAuto,
}

fn encoder_mode(encoder_process: &str) -> EncoderMode {
    match encoder_process {
        "native" | "auto" => EncoderMode::HelperRequired,
        "inproc" => EncoderMode::Inproc,
        _ => EncoderMode::UnrecognizedTreatedAsAuto,
    }
}

impl EncoderMode {
    /// The helper pair applies unless the runtime encodes in-process.
    fn needs_helper(self) -> bool {
        self != EncoderMode::Inproc
    }
}

/// Why a slug was not evaluated at all. Rendered as the `Skipped` check row's
/// message, so the reason reaches the UI instead of an unexplained blank.
fn not_applicable_reason(ctx: &StageCtx, slug: &str, mode: EncoderMode) -> Option<&'static str> {
    match slug {
        // run.sh evaluates the whole `--wired` block only inside
        // `if [ -n "${WINEVR_WIRED:-}" ]`.
        "run.wired-adb" if !ctx.opts.wired => Some("not --wired"),
        // `inproc` never reaches run.sh's `ensure_helper_staged`.
        s if HELPER_SLUGS.contains(&s) && !mode.needs_helper() => {
            Some("encoder_process=inproc — the native helper is disabled")
        }
        _ => None,
    }
}

/// Run the whole preflight, applying the two permanent auto-fixes on the way.
///
/// Returns the config facts later steps branch on. Aborts with the shell's
/// `die` text on the first `block` failure.
pub async fn run(ctx: &StageCtx) -> Result<PreflightFacts> {
    // Mirrors run.sh's own `require_bottle`, just above its tagged preflight
    // block: this call is what enforces `bottle.named` + `bottle.exists`.
    // Their registry rows are still emitted below (they cannot fail once this
    // has passed), because a GUI preflight list with two silently-absent rows
    // reads like a bug.
    require_bottle(ctx)?;

    let facts = read_toml_facts(&ctx.paths.toml_path);
    let mode = encoder_mode(&facts.encoder_process);

    let registry = crate::checks::registry();
    // The preflight checks the bottle **this stage resolved**, not one the
    // check layer re-derives from `$HOME`: a second, independent resolution is
    // a way for the two to disagree. It is also what lets a test point the
    // whole preflight at a fixture bottle instead of the machine's real
    // `~/Library/Application Support/CrossOver/Bottles`.
    let check_ctx = {
        let mut c = ctx.check_ctx();
        c.bottle = ctx.bottle.clone();
        c.bs_dir = ctx.bs_dir.clone();
        c.bottle_requested = ctx.opts.bottle_name.is_some();
        c
    };
    let mut encoder_notice_emitted = false;

    for slug in preflight_slugs() {
        // Stop must be responsive here: an `adb devices` probe wakes a daemon
        // and can take seconds, and the preflight is the part of a launch a
        // user is most likely to abandon (they hit Run before plugging the
        // headset in). The executor guards its own mutations
        // ([`crate::executor`]); this guards the read-only walk between them.
        if ctx.cancel.is_cancelled() {
            return Err(SabrageError::Cancelled);
        }

        let spec = contract()
            .check(slug)
            .expect("preflight_slugs() comes from the contract");

        // run.sh prints its `inproc`/unrecognized line once, at the `case`,
        // whether or not the helper checks then run — so it must come before
        // the applicability decision, not after it.
        if HELPER_SLUGS.contains(&slug) && !encoder_notice_emitted {
            encoder_notice_emitted = true;
            emit_encoder_notice(ctx, mode, &facts);
        }

        if let Some(reason) = not_applicable_reason(ctx, slug, mode) {
            emit_check(
                ctx,
                CheckOutcome::skipped(slug, reason.into()),
                spec.native_gate,
            );
            continue;
        }

        let bound = *registry
            .get(slug)
            .expect("every contract slug has a bound evaluator");
        let outcome = evaluate(ctx, bound, &check_ctx).await?;

        match spec.native_gate {
            Gate::Autofix => {
                autofix(ctx, &registry, &check_ctx, spec, outcome).await?;
            }
            _ => {
                gate(ctx, spec, outcome, &facts)?;
            }
        }
    }

    Ok(PreflightFacts {
        protocol: facts.protocol,
        encoder_process: facts.encoder_process,
    })
}

/// The preflight slugs whose evaluator spawns a **child process** rather than
/// reading the filesystem.
///
/// Exactly one today: `run.wired-adb` runs `adb devices`, which wakes the adb
/// daemon and, on a bad USB state, can take the whole of its deadline
/// ([`crate::checks::run_only`]'s `ADB_PROBE_TIMEOUT`). Every other preflight
/// evaluator is a `stat`/`read`/`cmp` that answers immediately.
const CHILD_PROBE_SLUGS: [&str; 1] = ["run.wired-adb"];

/// Evaluate one check without pinning the launch to a blocking probe.
///
/// Evaluators are synchronous `fn(&CheckCtx)` (the doctor's shape), so a slow
/// one runs *inside* this future and the walk's cancellation checkpoint —
/// which sits between evaluators — cannot interrupt it. The stage holds
/// [`crate::stages::OPERATION_LOCK`] throughout, so the child-probe slugs run
/// on a blocking thread raced against the launch's token and Stop stays
/// responsive; the probe's own deadline bounds the orphaned thread. Everything
/// else is evaluated inline — a `stat` is not worth a thread hop, and doctor
/// keeps calling the same evaluators directly
/// (tests::a_cancel_during_the_wired_adb_probe_stops_the_walk_promptly).
async fn evaluate(
    ctx: &StageCtx,
    check: crate::checks::BoundCheck,
    check_ctx: &crate::checks::CheckCtx,
) -> Result<CheckOutcome> {
    if !CHILD_PROBE_SLUGS.contains(&check.slug()) {
        return Ok(check.evaluate(check_ctx));
    }
    let probe_ctx = check_ctx.clone();
    let mut probe = tokio::task::spawn_blocking(move || check.evaluate(&probe_ctx));
    tokio::select! {
        joined = &mut probe => match joined {
            Ok(outcome) => Ok(outcome),
            // A panicking evaluator is a bug in this process, not a launch
            // verdict: re-raise it where it would have been raised inline.
            Err(e) => match e.try_into_panic() {
                Ok(panic) => std::panic::resume_unwind(panic),
                Err(_) => Err(SabrageError::Cancelled),
            },
        },
        _ = ctx.cancel.cancelled() => Err(SabrageError::Cancelled),
    }
}

/// run.sh's `inproc` `info` line, verbatim.
/// `pub` (A1-3) so `sabrage-parity` can pin it against `run.sh` by calling the
/// real renderer instead of copying the sentence.
pub const INPROC_NOTICE: &str =
    "encoder_process=inproc — in-process x86_64 encode (native helper disabled)";

/// run.sh line 89's `warn`. `pub` (A1-3), same reason as [`INPROC_NOTICE`].
pub fn unrecognized_encoder_warn(encoder_process: &str) -> String {
    format!(
        "oxrsys-runtime.toml encoder_process='{encoder_process}' unrecognized — the runtime \
         treats unknown values as auto"
    )
}

/// run.sh's `warn` — the one `warn`-gated row's text, which is not
/// doctor's. `pub` (A1-3), same reason as [`INPROC_NOTICE`].
pub fn game_version_warn(version: &str) -> String {
    format!("Beat Saber version '{version}' != 1.29.4 — the Meta gate may block startup")
}

/// run.sh's `inproc` / unrecognized-encoder lines, verbatim.
fn emit_encoder_notice(ctx: &StageCtx, mode: EncoderMode, facts: &TomlFacts) {
    let row = ctx.step(step::RUN_PREFLIGHT);
    match mode {
        EncoderMode::HelperRequired => {}
        EncoderMode::Inproc => row.info(INPROC_NOTICE),
        EncoderMode::UnrecognizedTreatedAsAuto => {
            row.warn(unrecognized_encoder_warn(&facts.encoder_process));
        }
    }
}

/// Emit one [`StageEvent::Check`], always attributed to
/// [`step::RUN_PREFLIGHT`].
fn emit_check(ctx: &StageCtx, outcome: CheckOutcome, gate: Gate) {
    ctx.emit(StageEvent::Check {
        run_id: ctx.run_id,
        step: step::RUN_PREFLIGHT.to_string(),
        outcome,
        gate,
    });
}

/// `die`, carrying the contract's fix id so a GUI Fatal can offer the same
/// button doctor's row would.
///
/// Sabrage-only enrichment: `lib.sh`'s `die` has no remedy or fix slot at all,
/// so this adds structure to a line whose *text* stays the shell's verbatim.
fn die(ctx: &StageCtx, spec: &CheckSpec, message: String, remedy: Option<String>) -> SabrageError {
    let fix = spec.fix.as_deref().and_then(FixAction::from_contract_id);
    ctx.emit(StageEvent::Fatal {
        run_id: ctx.run_id,
        message: message.clone(),
        remedy: remedy.clone(),
        fix,
    });
    SabrageError::Fatal { message, remedy }
}

/// The `block` / `warn` gates (`autofix` is [`autofix`]).
fn gate(ctx: &StageCtx, spec: &CheckSpec, outcome: CheckOutcome, facts: &TomlFacts) -> Result<()> {
    let slug = spec.slug.as_str();

    // run.sh's goldberg gate is `sha256_ok "$GBE_DLL" … || [ -f "$GBE_DLL" ] ||
    // die`: the launch dies only when the dll is *gone*; a hash that differs
    // from the pinned build is tolerated (a user-supplied Goldberg build is a
    // legitimate setup). Doctor is stricter on purpose, so its verdict is
    // reported as-is and simply not gated when the file exists
    // (tests::goldberg_hash_mismatch_does_not_block_the_launch).
    if slug == "dep.goldberg" && ctx.paths.gbe_dll.is_file() {
        let outcome = if outcome.status == CheckStatus::Pass {
            outcome
        } else {
            // Only a row that would otherwise have been gated needs the
            // explanation; a clean Pass keeps doctor's own detail.
            outcome.with_detail(format!(
                "present at {} — run tolerates a hash mismatch (run.sh line 54); \
                 only a missing file blocks the launch",
                ctx.paths.gbe_dll.display()
            ))
        };
        emit_check(ctx, outcome, spec.native_gate);
        return Ok(());
    }

    // Both protocol rows are decided from the single `$PROTOCOL` read rather
    // than from the evaluator's own re-read, so they can never disagree with
    // each other or with `PreflightFacts`.
    if slug == "cfg.protocol.supported" || slug == "cfg.protocol.legacy-oxrsys" {
        return protocol_gate(ctx, spec, outcome, facts);
    }

    match outcome.status {
        CheckStatus::Pass | CheckStatus::Info => {
            emit_check(ctx, outcome, spec.native_gate);
            Ok(())
        }

        CheckStatus::Warn => {
            // `game.version` is the only `warn`-gated row, and its text is
            // *not* doctor's; every other Warn reaching a `block` gate is one
            // the shell's coarser test would have passed (`host.manifest`
            // pointing somewhere unexpected but present), so it is reported
            // and does not block.
            let text = if slug == "game.version" {
                game_version_warn(&bs_version(&ctx.bs_dir))
            } else {
                outcome.message.clone()
            };
            emit_check(ctx, outcome, spec.native_gate);
            ctx.step(step::RUN_PREFLIGHT).warn(text);
            Ok(())
        }

        CheckStatus::Skipped | CheckStatus::NotImplemented => {
            // Applicable (the caller filtered the not-applicable cases out)
            // but unverifiable. Never a pass.
            Err(cannot_verify(ctx, spec, outcome))
        }

        CheckStatus::Fail => {
            let (message, remedy) = block_die(ctx, slug, &outcome);
            emit_check(ctx, outcome, spec.native_gate);
            Err(die(ctx, spec, message, remedy))
        }
    }
}

/// "Applicable but unverifiable" — the row is emitted, and then the launch
/// stops: PARITY.md § Run preflight (encoded in the contract's per-side
/// gates), "A `Skipped` outcome that reaches a gate is a Fatal".
///
/// The reason the check gave is carried into the die text, with its remedy
/// appended when it has one, so the user reads why it could not be checked
/// rather than a bare slug.
fn cannot_verify(ctx: &StageCtx, spec: &CheckSpec, outcome: CheckOutcome) -> SabrageError {
    let reason = outcome.message.clone();
    let remedy = outcome.remedy.clone();
    emit_check(ctx, outcome, spec.native_gate);
    let mut message = format!("cannot verify {}: {reason}", spec.slug);
    if let Some(r) = &remedy {
        message.push_str(" — ");
        message.push_str(r);
    }
    die(ctx, spec, message, remedy)
}

/// run.sh's protocol gate, decided from the single `$PROTOCOL` capture.
fn protocol_gate(
    ctx: &StageCtx,
    spec: &CheckSpec,
    outcome: CheckOutcome,
    facts: &TomlFacts,
) -> Result<()> {
    let slug = spec.slug.as_str();
    let toml = ctx.paths.toml_path.display().to_string();

    // Every branch below emits this same row and then decides: the row reports
    // the *doctor* evaluator's verdict (`awk`, last raw assignment), the gate
    // reports this side's runtime-semantics fact. When the two disagree and
    // the launch proceeds anyway, the row would otherwise read as a red check
    // the launch silently ignored, so it says why instead. PARITY.md
    // § Declared by the 2026-08-30 adversarial review (round 1 fixes),
    // "Config readers: doctor emulates `awk`, launch uses the runtime's
    // semantics."
    let outcome = if facts.present
        && facts.protocol == "alvr"
        && matches!(outcome.status, CheckStatus::Fail | CheckStatus::Warn)
    {
        outcome.with_detail(format!(
            "the launch reads {toml} the way the runtime does: the last value it would accept \
             is protocol='alvr', so this row does not block the launch (doctor emulates \
             run.sh's awk, which sees the last assignment as written)"
        ))
    } else {
        outcome
    };
    emit_check(ctx, outcome, spec.native_gate);

    if !facts.present {
        // `[ -f "$TOML" ] || die "$TOML missing — ./demo.sh setup"` — one die
        // for the pair; the legacy row is never reached in the shell either.
        return Err(die(
            ctx,
            spec,
            format!("{toml} missing — ./demo.sh setup"),
            Some("./demo.sh setup".to_string()),
        ));
    }

    match (slug, facts.protocol.as_str()) {
        // `alvr) : ;;` — both rows pass silently.
        (_, "alvr") => Ok(()),

        // The supported-set row is happy with `oxrsys`; the legacy row is the
        // one that speaks.
        ("cfg.protocol.supported", "oxrsys") => Ok(()),

        // DECLARED DIVERGENCE (contract: shell_gate = warn, native_gate =
        // block) — PARITY.md § Run preflight (encoded in the contract's
        // per-side gates), "Launch refuses `protocol=oxrsys` outright". The
        // first line is run.sh's warn text verbatim; the second says what this
        // side does instead
        // (tests::an_oxrsys_protocol_blocks_the_launch_with_both_lines).
        ("cfg.protocol.legacy-oxrsys", "oxrsys") => Err(die(
            ctx,
            spec,
            format!(
                "protocol=oxrsys (legacy USB path) — the demo path is alvr\n       \
                     Sabrage does not launch the legacy protocol — use ./demo.sh run --bottle {}",
                ctx.bottle_name()
            ),
            Some(format!("set protocol = \"alvr\" in {toml}")),
        )),

        // Anything else: run.sh's two-line die, attributed to the supported-set
        // row. The legacy row is `tap … skipped` in the shell and never reached
        // here (the die above aborts first).
        ("cfg.protocol.supported", other) => Err(die(
            ctx,
            spec,
            format!(
                "oxrsys-runtime.toml protocol='{other}' is not valid for the demo\n       \
                     set protocol = \"alvr\" in {toml} (or delete the file and re-run \
                     ./demo.sh setup)"
            ),
            Some(format!("set protocol = \"alvr\" in {toml}")),
        )),

        // Unreachable: `cfg.protocol.supported` aborts on every non-alvr,
        // non-oxrsys value before this row is walked.
        ("cfg.protocol.legacy-oxrsys", _) => Ok(()),

        (_, _) => unreachable!("protocol_gate is only called for the two cfg.protocol.* slugs"),
    }
}

/// The `block`-gate die text for one slug — run.sh's `die` string verbatim,
/// with its interpolations.
///
/// Three have no shell counterpart at all (`overlay.dxmt-winemetal`,
/// `overlay.woxr-dll`, `overlay.woxr-so`) and reuse the sentence shape of the
/// `d3d11` die they extend: PARITY.md § Run preflight (encoded in the
/// contract's per-side gates), "Native preflight blocks on ALL four overlay
/// files".
/// `pub` (A1-3) so `sabrage-parity` can pin these against `run.sh` by calling
/// the real renderer instead of copying a substring per slug.
pub fn block_die(ctx: &StageCtx, slug: &str, outcome: &CheckOutcome) -> (String, Option<String>) {
    let bottle = ctx.bottle_name();
    let install = format!("./demo.sh install --bottle {bottle}");
    let install_remedy = Some(install.clone());

    match slug {
        // Unreachable — `require_bottle` above dies with lib.sh's own text
        // before the walk starts. Reproduced so the table is total.
        "bottle.named" => (
            format!(
                "CrossOver bottle name required: pass --bottle <name> or set WINEVR_BOTTLE.\n       \
                 Existing bottles: {}",
                crate::paths::list_bottles()
                    .into_iter()
                    .map(|b| format!("{b} "))
                    .collect::<String>()
            ),
            None,
        ),
        "bottle.exists" => (
            format!(
                "bottle '{bottle}' not found at {} — create it in CrossOver (win11_64) first",
                crate::paths::Bottle::unvalidated(bottle).prefix.display()
            ),
            None,
        ),

        "dep.goldberg" => (
            "Goldberg dll missing — ./demo.sh setup".to_string(),
            Some("./demo.sh setup".to_string()),
        ),

        "game.present" => (
            format!(
                "Beat Saber not found at {}\n       download 1.29.4: {}\n       \
                 (or pass --bs-dir / set WINEVR_BS_DIR)",
                ctx.bs_dir.display(),
                contract().depot_command(&ctx.bs_dir)
            ),
            None,
        ),

        // `dxmt-winemetal` is the same overlay, so it reuses the same sentence.
        "overlay.dxmt-d3d11" | "overlay.dxmt-winemetal" => (
            format!("CrossOver DXMT overlay stale (CrossOver update?) — {install}"),
            install_remedy,
        ),
        // Native-only rows: the global wineopenxr overlay, which run.sh never
        // re-checks at launch.
        "overlay.woxr-dll" | "overlay.woxr-so" => (
            format!("CrossOver wineopenxr overlay stale (CrossOver update?) — {install}"),
            install_remedy,
        ),

        "bottle.woxr-dll" => (
            format!("bottle wineopenxr.dll stale/missing — {install}"),
            install_remedy,
        ),
        "bottle.manifest" => (
            format!("bottle OpenXR manifest missing — {install}"),
            install_remedy,
        ),
        "bottle.registry" => (
            format!("bottle ActiveRuntime registry key missing — {install}"),
            install_remedy,
        ),
        "host.manifest" => (
            format!("host OpenXR registration missing — {install}"),
            install_remedy,
        ),

        // `checks::run_only` already carries each of these die strings whole,
        // because those slugs have no doctor row whose prose could compete with
        // run.sh's.
        "run.wine-exec" | "run.bridge-built" | "run.wired-adb" => {
            (outcome.message.clone(), outcome.remedy.clone())
        }

        // Not reachable through a `block` gate today; falling back on the
        // check's own words beats inventing a sentence.
        _ => (outcome.message.clone(), outcome.remedy.clone()),
    }
}

/// The `autofix` gate: apply the mapped fix, re-evaluate, and only then decide.
///
/// run.sh's two auto-fixing preflights are the `cxbottle.conf` backend rewrite
/// and `ensure_helper_staged`. Both are **permanent** mutations, never
/// unwound — see [`super`]'s "permanent vs guarded".
async fn autofix(
    ctx: &StageCtx,
    registry: &Registry,
    check_ctx: &crate::checks::CheckCtx,
    spec: &CheckSpec,
    outcome: CheckOutcome,
) -> Result<()> {
    if outcome.status != CheckStatus::Fail {
        // Nothing to fix — a Pass, or a Warn/Skipped that the shared gate
        // knows how to report.
        return gate_after_fix(ctx, spec, outcome);
    }

    let slug = spec.slug.as_str();
    let report = match apply_fix(ctx, spec).await {
        Ok(report) => report,
        Err(e) => return Err(fix_failed(ctx, spec, outcome, e)),
    };

    let rechecked = registry
        .get(slug)
        .expect("every contract slug has a bound evaluator")
        .evaluate(check_ctx);

    // A dry run planned the write instead of performing it, so the re-check
    // necessarily still fails. Reporting that as "the fix failed" would be a
    // lie about a run that deliberately touched nothing —
    // `fixes::helper::restage_helper` skips its own post-copy validation for
    // the same reason.
    if ctx.executor.is_dry_run() && report.changed && rechecked.status == CheckStatus::Fail {
        fixes::emit_auto_fixed(ctx, &ctx.sink, step::RUN_PREFLIGHT, &report);
        emit_check(
            ctx,
            CheckOutcome::info(
                slug,
                format!("{} — auto-fix planned (dry run)", rechecked.message),
            )
            .with_detail(report.description),
            spec.native_gate,
        );
        return Ok(());
    }

    if rechecked.status == CheckStatus::Fail || rechecked.status == CheckStatus::Skipped {
        emit_check(ctx, rechecked, spec.native_gate);
        let (message, remedy) = post_fix_die(ctx, slug);
        return Err(die(ctx, spec, message, remedy));
    }

    fixes::emit_auto_fixed(ctx, &ctx.sink, step::RUN_PREFLIGHT, &report);
    gate_after_fix(ctx, spec, rechecked)
}

/// The autofix itself failed — the fix's own error, turned back into the one
/// `Check` + one `Fatal` this module promises, so an event-only consumer never
/// sees a failed stage with an unresolved row.
///
/// * `Cancelled` — Stop, not a failure. Propagated untouched: no row, no die.
/// * `Fatal` — the fix already emitted its own (`helper::restage_helper`'s
///   "neither the staged copy nor the build output is arm64"). Its text is
///   run.sh's; only the missing `Check` is added.
/// * anything else — the io cause is surfaced as a stderr-shaped `Output` line
///   and the die is run.sh's post-fix text, the same shape
///   `actions::die_with_cause` uses
///   (tests::a_backend_autofix_that_cannot_write_still_emits_its_check_and_dies_run_shs_way).
fn fix_failed(
    ctx: &StageCtx,
    spec: &CheckSpec,
    pre_fix: CheckOutcome,
    e: SabrageError,
) -> SabrageError {
    if matches!(e, SabrageError::Cancelled) {
        return e;
    }
    // The pre-fix outcome IS the final one — the fix never landed — with the
    // cause on the row so the UI shows why rather than just "still failing".
    emit_check(ctx, pre_fix.with_detail(e.to_string()), spec.native_gate);
    if matches!(e, SabrageError::Fatal { .. }) {
        return e;
    }
    ctx.emit(StageEvent::Output {
        run_id: ctx.run_id,
        step: step::RUN_PREFLIGHT.to_string(),
        stream: crate::events::Stream::Stderr,
        chunk: e.to_string(),
        end: crate::process::ChunkEnd::Lf,
    });
    let (message, remedy) = post_fix_die(ctx, spec.slug.as_str());
    die(ctx, spec, message, remedy)
}

/// The non-Fail statuses of an `autofix` slug, reported like any other row.
fn gate_after_fix(ctx: &StageCtx, spec: &CheckSpec, outcome: CheckOutcome) -> Result<()> {
    match outcome.status {
        CheckStatus::Skipped | CheckStatus::NotImplemented => {
            Err(cannot_verify(ctx, spec, outcome))
        }
        CheckStatus::Warn => {
            let text = outcome.message.clone();
            emit_check(ctx, outcome, spec.native_gate);
            ctx.step(step::RUN_PREFLIGHT).warn(text);
            Ok(())
        }
        _ => {
            emit_check(ctx, outcome, spec.native_gate);
            Ok(())
        }
    }
}

/// The fix an `autofix`-gated slug maps to — **the contract's** `fix` id, not a
/// second slug→fix table maintained here.
///
/// The preflight already holds [`crate::stages::OPERATION_LOCK`], so this is
/// [`fixes::apply_holding_lock`] (the doctor Fix button's `fixes::apply`
/// would deadlock on the second acquire).
///
/// One deliberate override: `bottle.gfx-dxmt` uses
/// [`backend::set_graphics_backend_for_launch`], not the Fix button's refusing
/// variant, because run.sh rewrites `cxbottle.conf` here and the
/// `wineserver-reset` launch action kills that wineserver two blocks later.
///
/// A slug gated `autofix` with no mapped fix is a contract error, not a panic:
/// the launch dies with something a user can act on.
async fn apply_fix(ctx: &StageCtx, spec: &CheckSpec) -> Result<FixReport> {
    let Some(action) = spec.fix.as_deref().and_then(FixAction::from_contract_id) else {
        return Err(SabrageError::fatal(
            format!(
                "{} is gated autofix but names no fix this build can apply",
                spec.slug
            ),
            "./demo.sh doctor --bottle <name>",
        ));
    };
    match action {
        FixAction::SetGraphicsBackend => {
            backend::set_graphics_backend_for_launch(ctx, &ctx.sink).await
        }
        other => fixes::apply_holding_lock(other, ctx, &ctx.sink).await,
    }
}

/// run.sh's die text for "the auto-fix ran and the condition is still there".
/// `pub` (A1-3), same reason as [`block_die`].
pub fn post_fix_die(ctx: &StageCtx, slug: &str) -> (String, Option<String>) {
    match slug {
        "bottle.gfx-dxmt" => {
            let conf = ctx
                .bottle
                .as_ref()
                .map(|b| b.conf_path().display().to_string())
                .unwrap_or_default();
            (
                format!("could not force graphics backend to dxmt in {conf}"),
                None,
            )
        }
        _ => (
            format!(
                "encoder helper restage failed validation at {} — ./demo.sh build",
                ctx.paths.oxr_helper_staged.display()
            ),
            Some("./demo.sh build".to_string()),
        ),
    }
}

trait BottleName {
    /// `$WINEVR_BOTTLE` as the die strings interpolate it. Always `Some` after
    /// `require_bottle`; falls back to doctor's `<name>` placeholder so the
    /// table stays total.
    fn bottle_name(&self) -> &str;
}

impl BottleName for StageCtx {
    fn bottle_name(&self) -> &str {
        self.opts.bottle_name.as_deref().unwrap_or("<name>")
    }
}

#[cfg(test)]
mod tests;
